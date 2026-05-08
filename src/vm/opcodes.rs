// src/vm/opcodes.rs

use std::convert::TryFrom;

use crate::vm::error::VmError;
use crate::vm::execution::ExecutionContext;
use crate::vm::heap::{Heap, HeapObject};
use crate::vm::value::Value;
use crate::vm::vm::VM;
use tokio::sync::mpsc::{error::TrySendError, Sender};

const OPERAND_WIDTH: usize = std::mem::size_of::<u32>();

#[derive(Debug, Clone, PartialEq)]
pub struct Bytecode {
    code: Vec<u8>,
    constants: Vec<Value>,
    offsets: Vec<usize>,
}

impl Bytecode {
    pub fn new(code: Vec<u8>, constants: Vec<Value>, offsets: Vec<usize>) -> Self {
        Self {
            code,
            constants,
            offsets,
        }
    }

    pub fn from_opcodes(opcodes: Vec<OpCode>) -> Self {
        let mut code = Vec::with_capacity(opcodes.len());
        let mut constants = Vec::new();
        let mut offsets = Vec::with_capacity(opcodes.len());

        for opcode in opcodes {
            offsets.push(code.len());
            opcode.encode(&mut code, &mut constants);
        }

        Self::new(code, constants, offsets)
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn instruction_len(&self) -> usize {
        self.offsets.len()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.code
    }

    pub fn constants(&self) -> &[Value] {
        &self.constants
    }

    pub fn patch_instruction(&mut self, index: usize, opcode: OpCode) -> Result<(), VmError> {
        if index >= self.offsets.len() {
            return Err(VmError::ExecutionOutOfBounds);
        }
        let mut opcodes = self.to_opcodes()?;
        opcodes[index] = opcode;
        *self = Self::from_opcodes(opcodes);
        Ok(())
    }

    pub fn decode_at(&self, instruction_index: usize) -> Result<OpCode, VmError> {
        let Some(&offset) = self.offsets.get(instruction_index) else {
            return Err(VmError::ExecutionOutOfBounds);
        };
        let tag = *self.code.get(offset).ok_or(VmError::ExecutionOutOfBounds)?;
        let kind = DenseOp::try_from(tag)?;
        let operand = if kind.has_operand() {
            let start = offset + 1;
            let end = start + OPERAND_WIDTH;
            let bytes: [u8; OPERAND_WIDTH] = self
                .code
                .get(start..end)
                .ok_or(VmError::ExecutionOutOfBounds)?
                .try_into()
                .map_err(|_| VmError::ExecutionOutOfBounds)?;
            Some(u32::from_le_bytes(bytes) as usize)
        } else {
            None
        };

        match kind {
            DenseOp::StoreVar => Ok(OpCode::StoreVar(operand.unwrap())),
            DenseOp::LoadVar => Ok(OpCode::LoadVar(operand.unwrap())),
            DenseOp::PushConst => {
                let index = operand.unwrap();
                let value = *self
                    .constants
                    .get(index)
                    .ok_or(VmError::ExecutionOutOfBounds)?;
                Ok(OpCode::PushConst(value))
            }
            DenseOp::Pop => Ok(OpCode::Pop),
            DenseOp::Dup => Ok(OpCode::Dup),
            DenseOp::Swap => Ok(OpCode::Swap),
            DenseOp::Add => Ok(OpCode::Add),
            DenseOp::Sub => Ok(OpCode::Sub),
            DenseOp::Mul => Ok(OpCode::Mul),
            DenseOp::Div => Ok(OpCode::Div),
            DenseOp::Mod => Ok(OpCode::Mod),
            DenseOp::Neg => Ok(OpCode::Neg),
            DenseOp::Exp => Ok(OpCode::Exp),
            DenseOp::Jump => Ok(OpCode::Jump(operand.unwrap())),
            DenseOp::JumpIfFalse => Ok(OpCode::JumpIfFalse(operand.unwrap())),
            DenseOp::Call => Ok(OpCode::Call(operand.unwrap())),
            DenseOp::Return => Ok(OpCode::Return),
            DenseOp::SpawnActor => Ok(OpCode::SpawnActor(operand.unwrap())),
            DenseOp::SendMessage => Ok(OpCode::SendMessage),
            DenseOp::ReceiveMessage => Ok(OpCode::ReceiveMessage),
            DenseOp::SpawnSupervisor => Ok(OpCode::SpawnSupervisor(operand.unwrap())),
            DenseOp::SetStrategy => Ok(OpCode::SetStrategy(operand.unwrap())),
            DenseOp::RestartChild => Ok(OpCode::RestartChild(operand.unwrap())),
        }
    }

    pub fn to_opcodes(&self) -> Result<Vec<OpCode>, VmError> {
        (0..self.instruction_len())
            .map(|index| self.decode_at(index))
            .collect()
    }
}

impl From<Vec<OpCode>> for Bytecode {
    fn from(opcodes: Vec<OpCode>) -> Self {
        Self::from_opcodes(opcodes)
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
enum DenseOp {
    StoreVar = 0x01,
    LoadVar = 0x02,
    PushConst = 0x03,
    Pop = 0x04,
    Dup = 0x05,
    Swap = 0x06,
    Add = 0x10,
    Sub = 0x11,
    Mul = 0x12,
    Div = 0x13,
    Mod = 0x14,
    Neg = 0x15,
    Exp = 0x16,
    Jump = 0x20,
    JumpIfFalse = 0x21,
    Call = 0x22,
    Return = 0x23,
    SpawnActor = 0x30,
    SendMessage = 0x31,
    ReceiveMessage = 0x32,
    SpawnSupervisor = 0x40,
    SetStrategy = 0x41,
    RestartChild = 0x42,
}

impl DenseOp {
    fn has_operand(self) -> bool {
        matches!(
            self,
            DenseOp::StoreVar
                | DenseOp::LoadVar
                | DenseOp::PushConst
                | DenseOp::Jump
                | DenseOp::JumpIfFalse
                | DenseOp::Call
                | DenseOp::SpawnActor
                | DenseOp::SpawnSupervisor
                | DenseOp::SetStrategy
                | DenseOp::RestartChild
        )
    }
}

impl TryFrom<u8> for DenseOp {
    type Error = VmError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(DenseOp::StoreVar),
            0x02 => Ok(DenseOp::LoadVar),
            0x03 => Ok(DenseOp::PushConst),
            0x04 => Ok(DenseOp::Pop),
            0x05 => Ok(DenseOp::Dup),
            0x06 => Ok(DenseOp::Swap),
            0x10 => Ok(DenseOp::Add),
            0x11 => Ok(DenseOp::Sub),
            0x12 => Ok(DenseOp::Mul),
            0x13 => Ok(DenseOp::Div),
            0x14 => Ok(DenseOp::Mod),
            0x15 => Ok(DenseOp::Neg),
            0x16 => Ok(DenseOp::Exp),
            0x20 => Ok(DenseOp::Jump),
            0x21 => Ok(DenseOp::JumpIfFalse),
            0x22 => Ok(DenseOp::Call),
            0x23 => Ok(DenseOp::Return),
            0x30 => Ok(DenseOp::SpawnActor),
            0x31 => Ok(DenseOp::SendMessage),
            0x32 => Ok(DenseOp::ReceiveMessage),
            0x40 => Ok(DenseOp::SpawnSupervisor),
            0x41 => Ok(DenseOp::SetStrategy),
            0x42 => Ok(DenseOp::RestartChild),
            _ => Err(VmError::ExecutionOutOfBounds),
        }
    }
}

#[derive(Debug)]
pub enum ExecutionState {
    Running,
    Yield(BlockingOperation),
    Halted,
}

#[derive(Debug)]
pub enum BlockingOperation {
    ReceiveMessage,
    SendMessage {
        sender: Sender<Value>,
        message: Value,
        actor_address: usize,
    },
}

fn unary_op<F>(execution: &mut ExecutionContext, heap: &mut Heap, f: F) -> Result<(), VmError>
where
    F: Fn(Value) -> Result<Value, VmError>,
{
    let v = pop_value(execution, heap)?;
    let result = f(v)?;
    execution.stack.push(result);
    Ok(())
}

fn binary_op<F>(execution: &mut ExecutionContext, heap: &mut Heap, f: F) -> Result<(), VmError>
where
    F: Fn(Value, Value) -> Result<Value, VmError>,
{
    if execution.stack.len() < 2 {
        log::error!("Stack underflow during binary operation");
        return Err(VmError::StackUnderflow);
    }
    let b = pop_value(execution, heap)?;
    let a = pop_value(execution, heap)?;
    let result = f(a, b)?;
    execution.stack.push(result);
    Ok(())
}

fn increment_reference(heap: &mut Heap, address: usize) -> Result<(), VmError> {
    if let Some(object) = heap.get_mut(address) {
        object.increment_ref();
        Ok(())
    } else {
        Err(VmError::InvalidReference)
    }
}

fn decrement_reference(heap: &mut Heap, address: usize) -> Result<(), VmError> {
    if let Some(object) = heap.get_mut(address) {
        object.decrement_ref();
        Ok(())
    } else {
        Err(VmError::InvalidReference)
    }
}

pub(crate) fn push_value(
    execution: &mut ExecutionContext,
    heap: &mut Heap,
    value: Value,
) -> Result<(), VmError> {
    if let Value::Reference(address) = value {
        increment_reference(heap, address)?;
    }
    execution.stack.push(value);
    Ok(())
}

pub(crate) fn pop_value(
    execution: &mut ExecutionContext,
    heap: &mut Heap,
) -> Result<Value, VmError> {
    if let Some(value) = execution.stack.pop() {
        if let Value::Reference(address) = value {
            decrement_reference(heap, address)?;
        }
        Ok(value)
    } else {
        Err(VmError::StackUnderflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpCode {
    StoreVar(usize),
    LoadVar(usize),
    PushConst(Value),
    Pop,
    Dup,
    Swap,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,
    Exp,
    Jump(usize),
    JumpIfFalse(usize),
    Call(usize),
    Return,
    SpawnActor(usize),
    SendMessage,
    ReceiveMessage,
    SpawnSupervisor(usize),
    SetStrategy(usize),
    RestartChild(usize),
}

impl OpCode {
    fn encode(self, code: &mut Vec<u8>, constants: &mut Vec<Value>) {
        let (dense, operand) = match self {
            OpCode::StoreVar(index) => (DenseOp::StoreVar, Some(index)),
            OpCode::LoadVar(index) => (DenseOp::LoadVar, Some(index)),
            OpCode::PushConst(value) => {
                let index = constants.len();
                constants.push(value);
                (DenseOp::PushConst, Some(index))
            }
            OpCode::Pop => (DenseOp::Pop, None),
            OpCode::Dup => (DenseOp::Dup, None),
            OpCode::Swap => (DenseOp::Swap, None),
            OpCode::Add => (DenseOp::Add, None),
            OpCode::Sub => (DenseOp::Sub, None),
            OpCode::Mul => (DenseOp::Mul, None),
            OpCode::Div => (DenseOp::Div, None),
            OpCode::Mod => (DenseOp::Mod, None),
            OpCode::Neg => (DenseOp::Neg, None),
            OpCode::Exp => (DenseOp::Exp, None),
            OpCode::Jump(target) => (DenseOp::Jump, Some(target)),
            OpCode::JumpIfFalse(target) => (DenseOp::JumpIfFalse, Some(target)),
            OpCode::Call(addr) => (DenseOp::Call, Some(addr)),
            OpCode::Return => (DenseOp::Return, None),
            OpCode::SpawnActor(addr) => (DenseOp::SpawnActor, Some(addr)),
            OpCode::SendMessage => (DenseOp::SendMessage, None),
            OpCode::ReceiveMessage => (DenseOp::ReceiveMessage, None),
            OpCode::SpawnSupervisor(addr) => (DenseOp::SpawnSupervisor, Some(addr)),
            OpCode::SetStrategy(strategy) => (DenseOp::SetStrategy, Some(strategy)),
            OpCode::RestartChild(child) => (DenseOp::RestartChild, Some(child)),
        };

        code.push(dense as u8);
        if let Some(operand) = operand {
            code.extend_from_slice(&(operand as u32).to_le_bytes());
        }
    }

    pub fn execute(
        self,
        execution: &mut ExecutionContext,
        heap: &mut Heap,
    ) -> Result<ExecutionState, VmError> {
        match self {
            OpCode::Add => binary_op(execution, heap, |a, b| a.add(b))?,
            OpCode::Sub => binary_op(execution, heap, |a, b| a.sub(b))?,
            OpCode::Mul => binary_op(execution, heap, |a, b| a.mul(b))?,
            OpCode::Div => binary_op(execution, heap, |a, b| a.div(b))?,
            OpCode::Neg => unary_op(execution, heap, |a| match a {
                Value::Integer(i) => Ok(Value::Integer(-i)),
                Value::Float(f) => Ok(Value::Float(-f)),
                _ => Err(VmError::TypeMismatch("Neg")),
            })?,
            OpCode::PushConst(v) => push_value(execution, heap, v)?,
            OpCode::Pop => {
                pop_value(execution, heap)?;
            }
            OpCode::Dup => {
                if let Some(&value) = execution.stack.last() {
                    push_value(execution, heap, value)?;
                } else {
                    return Err(VmError::StackUnderflow);
                }
            }
            OpCode::Swap => {
                if execution.stack.len() < 2 {
                    return Err(VmError::StackUnderflowFor("Swap"));
                }
                let len = execution.stack.len();
                execution.stack.swap(len - 1, len - 2);
            }
            OpCode::StoreVar(index) => {
                let value = pop_value(execution, heap)?;
                if let Some(Value::Reference(address)) = execution.locals.insert(index, value) {
                    decrement_reference(heap, address)?;
                }
                if let Value::Reference(address) = value {
                    increment_reference(heap, address)?;
                }
            }
            OpCode::LoadVar(index) => {
                if let Some(value) = execution.locals.get(&index) {
                    push_value(execution, heap, *value)?;
                } else {
                    return Err(VmError::VariableNotFound(index));
                }
            }
            OpCode::Mod => binary_op(execution, heap, |a, b| match (a, b) {
                (Value::Integer(x), Value::Integer(y)) => {
                    if y == 0 {
                        Err(VmError::DivisionByZero)
                    } else {
                        Ok(Value::Integer(x % y))
                    }
                }
                _ => Err(VmError::TypeMismatch("Mod")),
            })?,
            OpCode::Exp => binary_op(execution, heap, |a, b| match (a, b) {
                (Value::Integer(x), Value::Integer(y)) => {
                    if y < 0 {
                        Ok(Value::Float((x as f64).powi(y)))
                    } else {
                        Ok(Value::Integer(x.pow(y as u32)))
                    }
                }
                (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x.powf(y))),
                _ => Err(VmError::TypeMismatch("Exp")),
            })?,
            OpCode::Jump(target) => {
                validate_target(execution, target, true)?;
                execution.ip = target;
            }
            OpCode::JumpIfFalse(target) => {
                let value = pop_value(execution, heap)?;
                match value {
                    Value::Boolean(false) => {
                        validate_target(execution, target, true)?;
                        execution.ip = target;
                    }
                    Value::Boolean(true) => {}
                    _ => return Err(VmError::TypeMismatch("JumpIfFalse")),
                }
            }
            OpCode::Call(addr) => {
                validate_target(execution, addr, false)?;
                execution.call_stack.push(execution.ip);
                execution.ip = addr;
            }
            OpCode::Return => {
                if let Some(return_addr) = execution.call_stack.pop() {
                    execution.ip = return_addr;
                } else {
                    execution.ip = execution.bytecode.instruction_len();
                    return Ok(ExecutionState::Halted);
                }
            }
            OpCode::ReceiveMessage => {
                return match execution
                    .pending_receive
                    .take()
                    .or_else(|| execution.mailbox.try_recv().ok())
                {
                    Some(message) => {
                        log::info!("Received message: {:?}", message);
                        if let Value::Reference(address) = message {
                            decrement_reference(heap, address)?;
                        }
                        push_value(execution, heap, message)?;
                        Ok(ExecutionState::Running)
                    }
                    None => Ok(ExecutionState::Yield(BlockingOperation::ReceiveMessage)),
                };
            }
            OpCode::SpawnActor(addr) => {
                validate_target(execution, addr, false)?;
                let bytecode = execution.bytecode.clone();
                let (mut vm, tx) = VM::new(bytecode, None);
                vm.set_ip(addr);
                let address = heap.allocate(HeapObject::Actor(vm, tx, 0));
                push_value(execution, heap, Value::Reference(address))?;
            }
            OpCode::SendMessage => {
                let actor_ref = pop_value(execution, heap)?;
                let message = pop_value(execution, heap)?;
                if let Value::Reference(address) = actor_ref {
                    let sender = match heap.get(address) {
                        Some(HeapObject::Actor(_actor_vm, sender, _)) => sender.clone(),
                        _ => return Err(VmError::InvalidReference),
                    };
                    if let Value::Reference(message_address) = message {
                        increment_reference(heap, message_address)?;
                    }
                    return match sender.try_send(message) {
                        Ok(()) => {
                            push_value(execution, heap, Value::Reference(address))?;
                            Ok(ExecutionState::Running)
                        }
                        Err(TrySendError::Full(message)) => {
                            Ok(ExecutionState::Yield(BlockingOperation::SendMessage {
                                sender,
                                message,
                                actor_address: address,
                            }))
                        }
                        Err(TrySendError::Closed(failed_message)) => {
                            push_value(execution, heap, Value::Reference(address))?;
                            Err(VmError::ChannelSend {
                                error: "channel closed".to_string(),
                                value: failed_message,
                            })
                        }
                    };
                } else {
                    return Err(VmError::InvalidReference);
                }
            }
            OpCode::SpawnSupervisor(addr) => {
                validate_target(execution, addr, false)?;
                let bytecode = execution.bytecode.clone();
                let (mut vm, tx) = VM::new(bytecode, None);
                vm.set_ip(addr);
                let address = heap.allocate(HeapObject::Supervisor(vm, tx, 0));
                push_value(execution, heap, Value::Reference(address))?;
            }
            OpCode::SetStrategy(strategy) => {
                let sup_ref = pop_value(execution, heap)?;
                if let Value::Reference(addr) = sup_ref {
                    if let Some(HeapObject::Supervisor(vm, _, _)) = heap.get_mut(addr) {
                        vm.set_strategy(strategy);
                    } else {
                        return Err(VmError::InvalidReference);
                    }
                    push_value(execution, heap, Value::Reference(addr))?;
                } else {
                    return Err(VmError::InvalidReference);
                }
            }
            OpCode::RestartChild(child) => {
                let sup_ref = pop_value(execution, heap)?;
                if let Value::Reference(addr) = sup_ref {
                    if let Some(HeapObject::Supervisor(vm, _, _)) = heap.get_mut(addr) {
                        vm.restart_child(child);
                    } else {
                        return Err(VmError::InvalidReference);
                    }
                    push_value(execution, heap, Value::Reference(addr))?;
                } else {
                    return Err(VmError::InvalidReference);
                }
            }
        }

        Ok(ExecutionState::Running)
    }
}

fn validate_target(
    execution: &ExecutionContext,
    target: usize,
    allow_end: bool,
) -> Result<(), VmError> {
    let len = execution.bytecode.instruction_len();
    if target > len || (!allow_end && target == len) {
        log::error!(
            "Instruction target {} out of bounds (instruction length {})",
            target,
            len
        );
        Err(VmError::ExecutionOutOfBounds)
    } else {
        Ok(())
    }
}
