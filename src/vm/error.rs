use std::fmt;

#[derive(Debug, Clone)]
pub enum VmError {
    Message(String),
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
        }
    }
}

impl std::error::Error for VmError {}
