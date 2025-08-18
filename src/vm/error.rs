use std::fmt;
use tokio::sync::mpsc::error::SendError;

use crate::vm::value::Value;

#[derive(Debug, Error, Clone)]
pub enum VmError {
    Message(String),
    TypeMismatch,
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
        VmError::Message(err.to_string())
    }
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmError::Message(msg) => write!(f, "{}", msg),
            VmError::TypeMismatch => write!(f, "Type mismatch"),
        }
    }
}

impl std::error::Error for VmError {}
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
