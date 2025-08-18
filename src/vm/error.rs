use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum VmError {
    Message(String),
    TypeMismatch,
}

impl<T: Into<String>> From<T> for VmError {
    fn from(value: T) -> Self {
        VmError::Message(value.into())
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
