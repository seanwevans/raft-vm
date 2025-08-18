use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum VmError {
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

