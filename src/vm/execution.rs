// src/vm/execution.rs

use std::collections::HashMap;

use crate::vm::error::VmError;
use crate::vm::heap::Heap;
use crate::vm::opcodes::{Bytecode, ExecutionState, OpCode};
use crate::vm::value::Value;

use tokio::sync::mpsc::Receiver;

#[derive(Debug)]
pub struct ExecutionContext {
    pub stack: Vec<Value>,
    pub locals: HashMap<usize, Value>,
    pub ip: usize,
    pub call_stack: Vec<usize>,
    pub bytecode: Bytecode,
    pub mailbox: Receiver<Value>,
    pub(crate) pending_receive: Option<Value>,
}

impl ExecutionContext {
    pub fn new(bytecode: Vec<OpCode>) -> Self {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        Self::with_mailbox(bytecode, rx)
    }

    pub fn from_bytecode(bytecode: Bytecode, mailbox: Receiver<Value>) -> Self {
        Self {
            stack: Vec::new(),
            locals: HashMap::new(),
            ip: 0,
            call_stack: Vec::new(),
            bytecode,
            mailbox,
            pending_receive: None,
        }
    }

    pub fn with_mailbox<B>(bytecode: B, mailbox: Receiver<Value>) -> Self
    where
        B: Into<Bytecode>,
    {
        Self::from_bytecode(bytecode.into(), mailbox)
    }

    pub fn step(&mut self, heap: &mut Heap) -> Result<ExecutionState, VmError> {
        if self.ip >= self.bytecode.instruction_len() {
            if self.ip == self.bytecode.instruction_len() {
                return Ok(ExecutionState::Halted);
            }
            log::error!("Instruction pointer out of bounds: {}", self.ip);
            return Err(VmError::ExecutionOutOfBounds);
        }

        let opcode = self.bytecode.decode_at(self.ip)?;
        self.ip += 1;
        log::info!("Executing opcode: {:?}", opcode);
        opcode.execute(self, heap)
    }

    pub fn ip(&self) -> usize {
        self.ip
    }

    pub fn set_ip(&mut self, value: usize) {
        self.ip = value;
    }

    pub fn locals(&self) -> &HashMap<usize, Value> {
        &self.locals
    }

    pub fn locals_mut(&mut self) -> &mut HashMap<usize, Value> {
        &mut self.locals
    }
}
