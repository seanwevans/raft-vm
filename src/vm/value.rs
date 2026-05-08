// src/vm/value.rs

use crate::vm::error::VmError;
use crate::vm::supervision::ExitSignal;
use log;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Integer(i32),
    Float(f64),
    Boolean(bool),
    Reference(usize),
    ExitSignal(ExitSignal),
    Null,
}

#[allow(clippy::should_implement_trait)]
impl Value {
    pub fn checked_add(self, other: Value) -> Result<Value, VmError> {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a + b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
            _ => Err(VmError::TypeMismatch("Add")),
        }
    }

    pub fn checked_sub(self, other: Value) -> Result<Value, VmError> {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a - b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
            _ => Err(VmError::TypeMismatch("Sub")),
        }
    }

    pub fn checked_mul(self, other: Value) -> Result<Value, VmError> {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a * b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
            _ => Err(VmError::TypeMismatch("Mul")),
        }
    }

    pub fn checked_div(self, other: Value) -> Result<Value, VmError> {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => {
                if b == 0 {
                    log::error!("Division by zero: {}/{}", a, b);
                    Err(VmError::DivisionByZero)
                } else {
                    Ok(Value::Integer(a / b))
                }
            }
            (Value::Float(a), Value::Float(b)) => {
                if b == 0.0 {
                    log::error!("Division by zero: {}/{}", a, b);
                    Err(VmError::DivisionByZero)
                } else {
                    Ok(Value::Float(a / b))
                }
            }
            _ => {
                log::error!("Div type mismatch: {:?} / {:?}", self, other);
                Err(VmError::TypeMismatch("Div"))
            }
        }
    }
}
