// src/vm/opcodes.rs

use crate::vm::error::VmError;
use crate::vm::execution::ExecutionContext;
use crate::vm::heap::{Heap, HeapObject};
use crate::vm::value::Value;
use crate::vm::vm::VM;
use tokio::sync::mpsc::Receiver;

fn unary_op<F>(stack: &mut Vec<Value>, f: F) -> Result<(), VmError>
where
    F: Fn(Value) -> Result<Value, VmError>,
{
    if let Some(v) = stack.pop() {
        let result = f(v)?;
        stack.push(result);
        Ok(())
    } else {
        log::error!("Stack underflow during unary operation");
        Err("Stack underflow for unary operation".into())
    }
}

fn binary_op<F>(stack: &mut Vec<Value>, f: F) -> Result<(), VmError>
where
    F: Fn(Value, Value) -> Result<Value, VmError>,
{
    if stack.len() < 2 {
        log::error!("Stack underflow during binary operation");
        return Err("Stack underflow for binary operation".into());
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
    SendMessage,
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
    ) -> Result<(), VmError> {
        match self {
            OpCode::Add => binary_op(&mut execution.stack, |a, b| a.add(b)),
            OpCode::Sub => binary_op(&mut execution.stack, |a, b| a.sub(b)),
            OpCode::Mul => binary_op(&mut execution.stack, |a, b| a.mul(b)),
            OpCode::Div => binary_op(&mut execution.stack, |a, b| a.div(b)),
            OpCode::Neg => unary_op(&mut execution.stack, |a| match a {
                Value::Integer(i) => Ok(Value::Integer(-i)),
                Value::Float(f) => Ok(Value::Float(-f)),
                _ => Err("Cannot negate non-numeric value".into()),
            }),
            OpCode::PushConst(v) => {
                execution.stack.push(*v);
                Ok(())
            }
            OpCode::Pop => {
                execution
                    .stack
                    .pop()
                    .ok_or_else(|| VmError::from("Stack underflow"))?;
                Ok(())
            }
            OpCode::Dup => {
                if let Some(&v) = execution.stack.last() {
                    execution.stack.push(v);
                    Ok(())
                } else {
                    Err("Stack underflow".into())
                }
            }
            OpCode::Swap => {
                if execution.stack.len() < 2 {
                    return Err("Stack underflow for Swap".into());
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
                    Err("Stack underflow for StoreVar".into())
                }
            }
            OpCode::LoadVar(index) => {
                if let Some(value) = execution.locals.get(index) {
                    execution.stack.push(*value);
                    Ok(())
                } else {
                    Err(format!("Variable at index {} not found", index).into())
                }
            }
            OpCode::Mod => binary_op(&mut execution.stack, |a, b| match (a, b) {
                (Value::Integer(x), Value::Integer(y)) => {
                    if y == 0 {
                        Err("Modulo by zero".into())
                    } else {
                        Ok(Value::Integer(x % y))
                    }
                }
                _ => Err("Type mismatch for Mod".into()),
            }),
            OpCode::Exp => binary_op(&mut execution.stack, |a, b| match (a, b) {
                (Value::Integer(x), Value::Integer(y)) => Ok(Value::Integer(x.pow(y as u32))),
                (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x.powf(y))),
                _ => Err("Type mismatch for Exp".into()),
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
                    Err("Call stack underflow on Return".into())
                }
            }
            OpCode::ReceiveMessage => {
                if let Some(message) = mailbox.recv().await {
                    log::info!("Received message: {:?}", message);
                    execution.stack.push(message);
                    Ok(())
                } else {
                    log::warn!("Mailbox is empty or closed");
                    Err("Mailbox empty".into())
                }
            }

            OpCode::SpawnActor(addr) => {
                let bytecode = execution.bytecode.clone();
                let (mut vm, tx) = VM::new(bytecode, None);
                vm.set_ip(*addr);
                let address = _heap.allocate(HeapObject::Actor(vm, tx, 1));
                execution.stack.push(Value::Reference(address));
                Ok(())
            }
            OpCode::SendMessage => {
                let actor_ref = execution
                    .stack
                    .pop()
                    .ok_or_else(|| VmError::from("Stack underflow for SendMessage"))?;
                let message = execution
                    .stack
                    .pop()
                    .ok_or_else(|| VmError::from("Stack underflow for SendMessage"))?;
                if let Value::Reference(address) = actor_ref {
                    if let Some(HeapObject::Actor(_actor_vm, sender, _)) = _heap.get(address) {
                        sender.send(message).await.map_err(|e| e.to_string())?;
                        execution.stack.push(Value::Reference(address));
                        Ok(())
                    } else {
                        Err("Invalid actor reference".to_string().into())
                    }
                } else {
                    Err("Invalid actor reference".to_string().into())

                }
            }
            OpCode::SpawnSupervisor(addr) => {
                let bytecode = execution.bytecode.clone();
                let (mut vm, tx) = VM::new(bytecode, None);
                vm.set_ip(*addr);
                let address = _heap.allocate(HeapObject::Supervisor(vm, tx, 1));
                execution.stack.push(Value::Reference(address));
                Ok(())
            }
            OpCode::SetStrategy(strategy) => {
                let sup_ref = execution
                    .stack
                    .pop()
                    .ok_or_else(|| VmError::from("Stack underflow for SetStrategy"))?;
                if let Value::Reference(addr) = sup_ref {
                    if let Some(HeapObject::Supervisor(vm, _, _)) = _heap.get_mut(addr) {
                        vm.set_strategy(*strategy);
                        execution.stack.push(Value::Reference(addr));
                        Ok(())
                    } else {
                        Err("Invalid supervisor reference".to_string().into())
                    }
                } else {
                    Err("Invalid supervisor reference".to_string().into())

                }
            }
            OpCode::RestartChild(child) => {
                let sup_ref = execution
                    .stack
                    .pop()
                    .ok_or_else(|| VmError::from("Stack underflow for RestartChild"))?;
                if let Value::Reference(addr) = sup_ref {
                    if let Some(HeapObject::Supervisor(vm, _, _)) = _heap.get_mut(addr) {
                        vm.restart_child(*child);
                        execution.stack.push(Value::Reference(addr));
                        Ok(())
                    } else {
                        Err("Invalid supervisor reference".to_string().into())
                    }
                } else {
                    Err("Invalid supervisor reference".to_string().into())
                }
            }

        }
    }
}
