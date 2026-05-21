// src/vm/execution.rs

use std::collections::HashMap;

use crate::compiler::DebugInfo;
use crate::vm::error::VmError;
use crate::vm::heap::Heap;
use crate::vm::opcodes::{Bytecode, OpCode};
use crate::vm::value::{MessageValue, Value};

use tokio::sync::mpsc::{Receiver, Sender};

#[derive(Debug)]
pub enum BlockingOperation {
    ReceiveMessage,
    SendMessage {
        sender: Sender<MessageValue>,
        actor_address: usize,
        message: MessageValue,
    },
}

#[derive(Debug)]
pub enum ExecutionState {
    Continue,
    Yield(BlockingOperation),
    Halted,
}

#[derive(Debug, Clone)]
pub struct ProcessContext {
    pub process_id: usize,
    pub self_sender: Sender<MessageValue>,
    pub trap_exits: bool,
}

#[derive(Debug)]
pub struct ExecutionContext {
    pub stack: Vec<Value>,
    pub locals: HashMap<usize, Value>,
    pub globals: HashMap<String, Value>,
    pub ip: usize,
    pub call_stack: Vec<usize>,
    pub bytecode: Bytecode,
    pub debug_info: Option<DebugInfo>,
    pub mailbox: Receiver<MessageValue>,
}

impl ExecutionContext {
    pub fn new(bytecode: Vec<OpCode>) -> Self {
        let (_tx, rx) = tokio::sync::mpsc::channel(100);
        Self::with_mailbox(bytecode, rx)
    }

    pub fn with_mailbox(bytecode: impl Into<Bytecode>, mailbox: Receiver<MessageValue>) -> Self {
        Self {
            stack: Vec::new(),
            locals: HashMap::new(),
            globals: HashMap::new(),
            ip: 0,
            call_stack: Vec::new(),
            bytecode: bytecode.into(),
            debug_info: None,
            mailbox,
        }
    }

    pub fn new_with_debug(bytecode: impl Into<Bytecode>, debug_info: Option<DebugInfo>) -> Self {
        let (_tx, rx) = tokio::sync::mpsc::channel(100);
        Self::with_mailbox_and_debug(bytecode, rx, debug_info)
    }

    pub fn with_mailbox_and_debug(
        bytecode: impl Into<Bytecode>,
        mailbox: Receiver<MessageValue>,
        debug_info: Option<DebugInfo>,
    ) -> Self {
        Self {
            stack: Vec::new(),
            locals: HashMap::new(),
            globals: HashMap::new(),
            ip: 0,
            call_stack: Vec::new(),
            bytecode: bytecode.into(),
            debug_info,
            mailbox,
        }
    }

    pub fn step(&mut self, heap: &mut Heap) -> Result<ExecutionState, VmError> {
        self.step_inner(heap, None)
    }

    pub fn step_with_process(
        &mut self,
        heap: &mut Heap,
        process_id: usize,
        self_sender: Sender<MessageValue>,
        trap_exits: bool,
    ) -> Result<ExecutionState, VmError> {
        self.step_inner(
            heap,
            Some(ProcessContext {
                process_id,
                self_sender,
                trap_exits,
            }),
        )
    }

    fn step_inner(
        &mut self,
        heap: &mut Heap,
        process: Option<ProcessContext>,
    ) -> Result<ExecutionState, VmError> {
        if self.ip == self.bytecode.len() {
            return Ok(ExecutionState::Halted);
        }
        if self.ip > self.bytecode.len() {
            log::error!("Instruction pointer out of bounds: {}", self.ip);
            return Err(VmError::ExecutionOutOfBounds);
        }

        let instruction_ip = self.ip;
        let opcode = self.bytecode.decode(self.ip)?;
        // advance instruction pointer unless opcode modified it
        self.ip += 1;
        log::info!("Executing opcode: {:?}", opcode);
        opcode
            .execute_with_process(self, heap, process)
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

    pub fn mailbox_mut(&mut self) -> &mut Receiver<MessageValue> {
        &mut self.mailbox
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
