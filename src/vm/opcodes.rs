// src/vm/opcodes.rs

use crate::vm::execution::ExecutionContext;
use crate::vm::heap::{Heap};
use crate::vm::value::Value;
use tokio::sync::mpsc::Receiver;


fn unary_op<F>(stack: &mut Vec<Value>, f: F) -> Result<(), String>
where
    F: Fn(Value) -> Result<Value, String>,
{
    if let Some(v) = stack.pop() {
        let result = f(v)?;
        stack.push(result);
        Ok(())
    } else {
        log::error!("Stack underflow during unary operation");
        Err("Stack underflow for unary operation".to_string())
    }
}


fn binary_op<F>(stack: &mut Vec<Value>, f: F) -> Result<(), String>
where
    F: Fn(Value, Value) -> Result<Value, String>,
{
    if stack.len() < 2 {
        log::error!("Stack underflow during binary operation");
        return Err("Stack underflow for binary operation".to_string());
    }
    let b = stack.pop().unwrap();
    let a = stack.pop().unwrap();
    let result = f(a, b)?;
    stack.push(result);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub enum OpCode {
    // Variables
    StoreVar(usize),
    LoadVar(usize),
    
    // Stack
    PushConst(Value),
    Pop,
    Dup,
    Swap,
    Peek,
    
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,
    Exp,
    
    // Control Flow
    Jump(usize),
    JumpIfFalse(usize),
    Call(usize),
    Return,
    
    // Actors
    SpawnActor(usize),
    SendMessage(usize),
    ReceiveMessage,
    
    // Supervisor
    SpawnSupervisor(usize),
    SetStrategy(usize),
    RestartChild(usize),
}

impl OpCode {
    pub async fn execute(
        &self,
        execution: &mut ExecutionContext,        
        _heap: &mut Heap,
        mailbox: &mut Receiver<Value>,        
    ) -> Result<(), String> {        
        match self {
            OpCode::Add => binary_op(&mut execution.stack, |a, b| a.add(b)),
            OpCode::Sub => binary_op(&mut execution.stack, |a, b| a.sub(b)),
            OpCode::Mul => binary_op(&mut execution.stack, |a, b| a.mul(b)),
            OpCode::Div => binary_op(&mut execution.stack, |a, b| a.div(b)),
            OpCode::Neg => unary_op(&mut execution.stack, |a| match a {
                Value::Integer(i) => Ok(Value::Integer(-i)),
                Value::Float(f) => Ok(Value::Float(-f)),
                _ => Err("Cannot negate non-numeric value".to_string()),
            }),
            OpCode::PushConst(v) => {
                execution.stack.push(*v);
                Ok(())
            }
            OpCode::Pop => {
                execution.stack.pop().ok_or("Stack underflow".to_string()).map(|_| ())
            }
            OpCode::Dup => {
                if let Some(&v) = execution.stack.last() {
                    execution.stack.push(v);
                    Ok(())
                } else {
                    Err("Stack underflow".to_string())
                }
            }
            OpCode::Swap => {
                if execution.stack.len() < 2 {
                    return Err("Stack underflow for Swap".to_string());
                }
                let len = execution.stack.len();
                execution.stack.swap(len - 1, len - 2);
                Ok(())
            }
            OpCode::StoreVar(index) => {
                if let Some(value) = execution.stack.pop() {
                    execution.locals.insert(*index, value);
                    Ok(())
                } else {
                    Err("Stack underflow for StoreVar".to_string())
                }
            }
            OpCode::Peek => {
                if let Some(v) = execution.stack.last() {
                    execution.stack.push(*v);
                    Ok(())
                } else {
                    Err("Stack underflow for Peek".to_string())
                }
            }
            OpCode::LoadVar(index) => {
                if let Some(value) = execution.locals.get(index) {
                    execution.stack.push(*value);
                    Ok(())
                } else {
                    Err(format!("Variable at index {} not found", index))
                }
            }
            OpCode::Mod => binary_op(&mut execution.stack, |a, b| match (a, b) {
                (Value::Integer(x), Value::Integer(y)) => {
                    if y == 0 {
                        Err("Modulo by zero".to_string())
                    } else {
                        Ok(Value::Integer(x % y))
                    }
                }
                _ => Err("Type mismatch for Mod".to_string()),
            }),
            OpCode::Exp => binary_op(&mut execution.stack, |a, b| match (a, b) {
                (Value::Integer(x), Value::Integer(y)) => Ok(Value::Integer(x.pow(y as u32))),
                (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x.powf(y))),
                _ => Err("Type mismatch for Exp".to_string()),
            }),
            OpCode::Jump(target) => {
                execution.ip = *target;
                Ok(())
            }
            OpCode::JumpIfFalse(target) => {
                if let Some(Value::Boolean(false)) = execution.stack.pop() {
                    execution.ip = *target;
                }
                Ok(())
            }
            OpCode::Call(addr) => {
                execution.call_stack.push(execution.ip);
                execution.ip = *addr;
                Ok(())
            }
            OpCode::Return => {
                if let Some(return_addr) = execution.call_stack.pop() {
                    execution.ip = return_addr;
                    Ok(())
                } else {
                    Err("Call stack underflow on Return".to_string())
                }
            }
            OpCode::ReceiveMessage => {
                if let Some(message) = mailbox.recv().await {
                    log::info!("Received message: {:?}", message);
                    execution.stack.push(message);
                    Ok(())
                } else {
                    log::warn!("Mailbox is empty or closed");
                    Err("Mailbox empty".to_string())
                }
            }            
            _ => Err("Opcode not implemented".to_string()),
        }
    }
}
