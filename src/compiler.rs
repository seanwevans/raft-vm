// src/compiler/compiler.rs

use crate::vm::opcodes::OpCode;
use crate::vm::value::Value;

pub struct Compiler;

impl Compiler {
    pub fn compile(source: &str) -> Result<Vec<OpCode>, String> {
        let mut bytecode = Vec::new();

        let mut tokens = source.split_whitespace();
        while let Some(token) = tokens.next() {
            if let Ok(num) = token.parse::<i32>() {
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
                        let addr_token = tokens.next().ok_or("Expected address after Jump")?;
                        let addr = addr_token
                            .parse::<usize>()
                            .map_err(|_| format!("Invalid address: {}", addr_token))?;
                        bytecode.push(OpCode::Jump(addr));
                    }
                    "JumpIfFalse" => {
                        let addr_token =
                            tokens.next().ok_or("Expected address after JumpIfFalse")?;
                        let addr = addr_token
                            .parse::<usize>()
                            .map_err(|_| format!("Invalid address: {}", addr_token))?;
                        bytecode.push(OpCode::JumpIfFalse(addr));
                    }
                    "Call" => {
                        let addr_token = tokens.next().ok_or("Expected address after Call")?;
                        let addr = addr_token
                            .parse::<usize>()
                            .map_err(|_| format!("Invalid address: {}", addr_token))?;
                        bytecode.push(OpCode::Call(addr));
                    }
                    "Return" => bytecode.push(OpCode::Return),
                    _ => return Err(format!("Unknown token: {}", token)),
                }
            }
        }

        Ok(bytecode)
    }
}
