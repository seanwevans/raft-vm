// src/vm/execution.rs

use std::collections::HashMap;

use crate::compiler::DebugInfo;
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
    pub debug_info: Option<DebugInfo>,
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
            debug_info: None,
        }
    }

    pub fn new_with_debug(bytecode: Vec<OpCode>, debug_info: Option<DebugInfo>) -> Self {
        Self {
            stack: Vec::new(),
            locals: HashMap::new(),
            globals: HashMap::new(),
            ip: 0,
            call_stack: Vec::new(),
            bytecode,
            debug_info,
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

        let instruction_ip = self.ip;
        let opcode = self.bytecode[self.ip].clone();
        // advance instruction pointer unless opcode modified it
        self.ip += 1;
        log::info!("Executing opcode: {:?}", opcode);
        opcode
            .execute_with_process(self, heap, mailbox, process)
            .await
            .map_err(|error| self.with_debug_location(error, instruction_ip))
    }

    fn with_debug_location(&self, error: VmError, instruction_ip: usize) -> VmError {
        if matches!(error, VmError::RuntimeError { .. }) {
            return error;
        }

        match self
            .debug_info
            .as_ref()
            .and_then(|debug_info| debug_info.location_for_instruction(instruction_ip))
        {
            Some(location) => VmError::RuntimeError {
                location,
                source: Box::new(error),
            },
            None => error,
        }
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
