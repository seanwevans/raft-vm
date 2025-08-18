use std::fmt;
use tokio::sync::mpsc::error::SendError;

use crate::vm::value::Value;

#[derive(Debug, Clone)]
pub enum VmError {
    Message(String),
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
        }
    }
}

impl std::error::Error for VmError {}
