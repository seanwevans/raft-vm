// src/vm/value.rs

use crate::vm::error::VmError;
use log;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Integer(i32),
    Float(f64),
    Boolean(bool),
    Reference(usize),
    Null,
}

impl Value {
    pub fn add(self, other: Value) -> Result<Value, VmError> {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a + b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
            _ => Err("Type mismatch for Add".into()),
        }
    }

    pub fn sub(self, other: Value) -> Result<Value, VmError> {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a - b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
            _ => Err("Type mismatch for Sub".into()),
        }
    }

    pub fn mul(self, other: Value) -> Result<Value, VmError> {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(a * b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
            _ => Err("Type mismatch for Mul".into()),
        }
    }

    pub fn div(self, other: Value) -> Result<Value, VmError> {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => {
                if b == 0 {
                    log::error!("Division by zero: {}/{}", a, b);
                    Err("Division by zero".into())
                } else {
                    Ok(Value::Integer(a / b))
                }
            }
            (Value::Float(a), Value::Float(b)) => {
                if b == 0.0 {
                    log::error!("Division by zero: {}/{}", a, b);
                    Err("Division by zero".into())
                } else {
                    Ok(Value::Float(a / b))
                }
            }
            _ => {
                log::error!("Div type mismatch: {:?} / {:?}", self, other);
                Err("Type mismatch for Div".into())
            }
        }
    }
}
