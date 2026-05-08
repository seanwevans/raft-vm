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
            } else if token == "io.print" {
                bytecode.push(OpCode::LoadGlobal("io".to_string()));
                bytecode.push(OpCode::GetExport("print".to_string()));
            } else if token.contains('.') {
                let num = token
                    .parse::<f64>()
                    .map_err(|_| CompilerError::ParseError(format!("Invalid float: {}", token)))?;
                bytecode.push(OpCode::PushConst(Value::Float(num)));
            } else if let Ok(num) = token.parse::<i32>() {
                bytecode.push(OpCode::PushConst(Value::Integer(num)));
            } else {
                match token {
                    "StoreVar" => {
                        let index_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress(
                                "expected variable index after StoreVar".into(),
                            )
                        })?;
                        let index = index_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(index_token.to_string()))?;
                        bytecode.push(OpCode::StoreVar(index));
                    }
                    "LoadVar" => {
                        let index_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress(
                                "expected variable index after LoadVar".into(),
                            )
                        })?;
                        let index = index_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(index_token.to_string()))?;
                        bytecode.push(OpCode::LoadVar(index));
                    }
                    "MakeArray" => {
                        let length_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress("expected length after MakeArray".into())
                        })?;
                        let length = length_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(length_token.to_string()))?;
                        bytecode.push(OpCode::MakeArray(length));
                    }
                    "ArrayGet" => bytecode.push(OpCode::ArrayGet),
                    "ArraySet" => bytecode.push(OpCode::ArraySet),
                    "MakeString" => {
                        let value = tokens.next().ok_or_else(|| {
                            CompilerError::ParseError(
                                "expected string token after MakeString".into(),
                            )
                        })?;
                        bytecode.push(OpCode::MakeString(value.to_string()));
                    }
                    "StringConcat" => bytecode.push(OpCode::StringConcat),
                    "MakeModule" => {
                        let export_count_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress(
                                "expected export count after MakeModule".into(),
                            )
                        })?;
                        let export_count = export_count_token.parse::<usize>().map_err(|_| {
                            CompilerError::InvalidAddress(export_count_token.to_string())
                        })?;
                        let mut exports = Vec::with_capacity(export_count);
                        for _ in 0..export_count {
                            exports.push(
                                tokens
                                    .next()
                                    .ok_or_else(|| {
                                        CompilerError::ParseError(
                                            "expected export name after MakeModule".into(),
                                        )
                                    })?
                                    .to_string(),
                            );
                        }
                        bytecode.push(OpCode::MakeModule(exports));
                    }
                    "ModuleGet" => {
                        let name = tokens.next().ok_or_else(|| {
                            CompilerError::ParseError("expected export name after ModuleGet".into())
                        })?;
                        bytecode.push(OpCode::ModuleGet(name.to_string()));
                    }
                    "ModuleSet" => {
                        let name = tokens.next().ok_or_else(|| {
                            CompilerError::ParseError("expected export name after ModuleSet".into())
                        })?;
                        bytecode.push(OpCode::ModuleSet(name.to_string()));
                    }
                    "CallNative" => {
                        let arity_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress("expected arity after CallNative".into())
                        })?;
                        let arity = arity_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(arity_token.to_string()))?;
                        bytecode.push(OpCode::CallNative(arity));
                    "LoadGlobal" => {
                        let name = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress(
                                "expected global name after LoadGlobal".into(),
                            )
                        })?;
                        bytecode.push(OpCode::LoadGlobal(name.to_string()));
                    }
                    "GetExport" => {
                        let name = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress(
                                "expected export name after GetExport".into(),
                            )
                        })?;
                        bytecode.push(OpCode::GetExport(name.to_string()));
                    }
                    "Pop" => bytecode.push(OpCode::Pop),
                    "Dup" => bytecode.push(OpCode::Dup),
                    "Swap" => bytecode.push(OpCode::Swap),
                    "+" | "Add" => bytecode.push(OpCode::Add),
                    "-" | "Sub" => bytecode.push(OpCode::Sub),
                    "*" | "Mul" => bytecode.push(OpCode::Mul),
                    "/" | "Div" => bytecode.push(OpCode::Div),
                    "%" | "Mod" => bytecode.push(OpCode::Mod),
                    "Neg" => bytecode.push(OpCode::Neg),
                    "Exp" | "^" => bytecode.push(OpCode::Exp),
                    "Jump" => {
                        let addr_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress("expected address after Jump".into())
                        })?;
                        let addr = addr_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(addr_token.to_string()))?;
                        bytecode.push(OpCode::Jump(addr));
                    }
                    "JumpIfFalse" => {
                        let addr_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress(
                                "expected address after JumpIfFalse".into(),
                            )
                        })?;
                        let addr = addr_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(addr_token.to_string()))?;
                        bytecode.push(OpCode::JumpIfFalse(addr));
                    }
                    "Call" => {
                        let addr_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress("expected address after Call".into())
                        })?;
                        let addr = addr_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(addr_token.to_string()))?;
                        bytecode.push(OpCode::Call(addr));
                    }
                    "CallNative" => {
                        let arity_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress("expected arity after CallNative".into())
                        })?;
                        let arity = arity_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(arity_token.to_string()))?;
                        bytecode.push(OpCode::CallNative(arity));
                    }
                    "SpawnActor" => {
                        let addr_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress(
                                "expected address after SpawnActor".into(),
                            )
                        })?;
                        let addr = addr_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(addr_token.to_string()))?;
                        bytecode.push(OpCode::SpawnActor(addr));
                    }
                    "SendMessage" => {
                        bytecode.push(OpCode::SendMessage);
                    }
                    "ReceiveMessage" => {
                        bytecode.push(OpCode::ReceiveMessage);
                    }
                    "SpawnSupervisor" => {
                        let addr_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress(
                                "expected address after SpawnSupervisor".into(),
                            )
                        })?;
                        let addr = addr_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(addr_token.to_string()))?;
                        bytecode.push(OpCode::SpawnSupervisor(addr));
                    }
                    "SetStrategy" => {
                        let strategy_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress(
                                "expected strategy after SetStrategy".into(),
                            )
                        })?;
                        let strategy = strategy_token.parse::<usize>().map_err(|_| {
                            CompilerError::InvalidAddress(strategy_token.to_string())
                        })?;
                        bytecode.push(OpCode::SetStrategy(strategy));
                    }
                    "RestartChild" => {
                        let child_token = tokens.next().ok_or_else(|| {
                            CompilerError::InvalidAddress(
                                "expected child index after RestartChild".into(),
                            )
                        })?;
                        let child = child_token
                            .parse::<usize>()
                            .map_err(|_| CompilerError::InvalidAddress(child_token.to_string()))?;
                        bytecode.push(OpCode::RestartChild(child));
                    }
                    "Return" => bytecode.push(OpCode::Return),
                    _ => return Err(CompilerError::InvalidToken(token.to_string())),
                }
            }
        }

        Ok(bytecode)
    }
}
