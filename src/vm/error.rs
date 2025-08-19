use thiserror::Error;
use tokio::sync::mpsc::error::SendError;

use crate::vm::value::Value;

#[derive(Debug, Error, Clone)]
pub enum VmError {
    #[error("{0}")]
    Message(String),
    #[error("Stack underflow")]
    StackUnderflow,
    #[error("Type mismatch in {0}")]
    TypeMismatch(&'static str),
    #[error("Division by zero")]
    DivisionByZero,
    #[error("Execution out of bounds")]
    ExecutionOutOfBounds,
    #[error("No bytecode to execute")]
    NoBytecode,
    #[error("Variable at index {0} not found")]
    VariableNotFound(usize),
    #[error("Invalid reference")]
    InvalidReference,
    #[error("Mailbox empty")]
    MailboxEmpty,
    #[error("Channel send error: {0}")]
    ChannelSend(String),
    #[error("Compilation error: {0}")]
    CompilationError(String),
}

impl From<String> for VmError {
    fn from(value: String) -> Self {
        VmError::Message(value)
    }
}

impl From<&str> for VmError {
    fn from(value: &str) -> Self {
        VmError::Message(value.to_string())
    }
}

impl From<SendError<Value>> for VmError {
    fn from(err: SendError<Value>) -> Self {
        VmError::ChannelSend(err.to_string())
    }
}
