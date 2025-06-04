// src/vm/execution.rs

use std::collections::HashMap;

use crate::vm::heap::Heap;
use crate::vm::opcodes::OpCode;
use crate::vm::value::Value;

use tokio::sync::mpsc::Receiver;

#[derive(Debug)]
pub struct Frame {
    pub return_address: usize,
    pub locals: HashMap<usize, Value>,
}

#[derive(Debug)]
pub struct ExecutionContext {
    pub stack: Vec<Value>,
    pub locals: HashMap<usize, Value>,
    pub ip: usize,
    pub call_stack: Vec<usize>,
    pub bytecode: Vec<OpCode>,
}

impl ExecutionContext {
    pub fn new(bytecode: Vec<OpCode>) -> Self {
        Self {
            stack: Vec::new(),
            locals: HashMap::new(),
            ip: 0,
            call_stack: Vec::new(),
            bytecode,
        }
    }

    pub async fn step(
        &mut self,
        heap: &mut Heap,
        mailbox: &mut Receiver<Value>,
    ) -> Result<(), String> {
        if self.ip >= self.bytecode.len() {
            log::error!("Instruction pointer out of bounds: {}", self.ip);
            return Err("Execution out of bounds".to_string());
        }

        let opcode = self.bytecode[self.ip];
        log::info!("Executing opcode: {:?}", opcode);

        let stack = &mut self.stack;
        let call_stack = &mut self.call_stack;
        let ip = self.ip;
        let locals = &mut self.locals;

        // Clone the opcode to avoid immutable borrow issues.
        let opcode = self.bytecode[ip].clone();

        opcode.execute(self, heap, mailbox).await?;

        if self.ip == ip {
            self.ip += 1;
        }

        Ok(())
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
