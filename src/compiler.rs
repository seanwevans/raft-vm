// src/compiler/compiler.rs

use crate::vm::opcodes::OpCode;
use crate::vm::value::Value;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum CompilerError {
    #[error("Invalid token: {0}")]
    InvalidToken(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Parse error: {0}")]
    ParseError(String),
}

pub struct Compiler;

impl Compiler {
    pub fn compile(source: &str) -> Result<Vec<OpCode>, CompilerError> {
        let mut bytecode = Vec::new();

        let mut tokens = source.split_whitespace();
        while let Some(token) = tokens.next() {
            if token == "true" || token == "false" {
                bytecode.push(OpCode::PushConst(Value::Boolean(token == "true")));
            } else if token.contains('.') {
                let num = token
                    .parse::<f64>()
                    .map_err(|_| CompilerError::ParseError(format!("Invalid float: {}", token)))?;
                bytecode.push(OpCode::PushConst(Value::Float(num)));
            } else if let Ok(num) = token.parse::<i32>() {
                bytecode.push(OpCode::PushConst(Value::Integer(num)));
            } else {
                match token {
                    "+" | "Add" => bytecode.push(OpCode::Add),
                    "-" | "Sub" => bytecode.push(OpCode::Sub),
                    "*" | "Mul" => bytecode.push(OpCode::Mul),
                    "/" | "Div" => bytecode.push(OpCode::Div),
                    "%" | "Mod" => bytecode.push(OpCode::Mod),
                    "Neg" => bytecode.push(OpCode::Neg),
                    "Exp" | "^" => bytecode.push(OpCode::Exp),
                    "Jump" => {
                        let addr_token = tokens
                            .next()
                            .ok_or_else(|| CompilerError::InvalidAddress("expected address after Jump".into()))?;
                        let addr = addr_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(addr_token.to_string()))?;
                        bytecode.push(OpCode::Jump(addr));
                    }
                    "JumpIfFalse" => {
                        let addr_token = tokens
                            .next()
                            .ok_or_else(|| CompilerError::InvalidAddress("expected address after JumpIfFalse".into()))?;
                        let addr = addr_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(addr_token.to_string()))?;
                        bytecode.push(OpCode::JumpIfFalse(addr));
                    }
                    "Call" => {
                        let addr_token = tokens
                            .next()
                            .ok_or_else(|| CompilerError::InvalidAddress("expected address after Call".into()))?;
                        let addr = addr_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(addr_token.to_string()))?;
                        bytecode.push(OpCode::Call(addr));
                    }
                    "Return" => bytecode.push(OpCode::Return),
                    _ => return Err(CompilerError::InvalidToken(token.to_string())),
                }
            }
        }

        Ok(bytecode)
    }
}
