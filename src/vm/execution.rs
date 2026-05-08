// src/vm/execution.rs

use std::collections::HashMap;

use crate::vm::error::VmError;
use crate::vm::heap::Heap;
use crate::vm::opcodes::OpCode;
use crate::vm::value::Value;

use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Debug, Clone)]
pub struct ProcessContext {
    pub process_id: usize,
    pub self_sender: Sender<Value>,
    pub trap_exits: bool,
}

#[derive(Debug)]
pub struct ExecutionContext {
    pub stack: Vec<Value>,
    pub locals: HashMap<usize, Value>,
    pub globals: HashMap<String, Value>,
    pub ip: usize,
    pub call_stack: Vec<usize>,
    pub bytecode: Vec<OpCode>,
}

impl ExecutionContext {
    pub fn new(bytecode: Vec<OpCode>) -> Self {
        Self {
            stack: Vec::new(),
            locals: HashMap::new(),
            globals: HashMap::new(),
            ip: 0,
            call_stack: Vec::new(),
            bytecode,
        }
    }

    pub async fn step(
        &mut self,
        heap: &mut Heap,
        mailbox: &mut Receiver<Value>,
    ) -> Result<(), VmError> {
        self.step_inner(heap, mailbox, None).await
    }

    pub async fn step_with_process(
        &mut self,
        heap: &mut Heap,
        mailbox: &mut Receiver<Value>,
        process_id: usize,
        self_sender: Sender<Value>,
        trap_exits: bool,
    ) -> Result<(), VmError> {
        self.step_inner(
            heap,
            mailbox,
            Some(ProcessContext {
                process_id,
                self_sender,
                trap_exits,
            }),
        )
        .await
    }

    async fn step_inner(
        &mut self,
        heap: &mut Heap,
        mailbox: &mut Receiver<Value>,
        process: Option<ProcessContext>,
    ) -> Result<(), VmError> {
        if self.ip >= self.bytecode.len() {
            log::error!("Instruction pointer out of bounds: {}", self.ip);
            return Err(VmError::ExecutionOutOfBounds);
        }

        let opcode = self.bytecode[self.ip];
        // advance instruction pointer unless opcode modified it
        self.ip += 1;
        log::info!("Executing opcode: {:?}", opcode);
        opcode
            .execute_with_process(self, heap, mailbox, process)
            .await
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

    pub fn globals(&self) -> &HashMap<String, Value> {
        &self.globals
    }

    pub fn globals_mut(&mut self) -> &mut HashMap<String, Value> {
        &mut self.globals
    }
}
