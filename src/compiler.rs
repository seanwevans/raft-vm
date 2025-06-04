// src/compiler/compiler.rs

use crate::vm::opcodes::OpCode;
use crate::vm::value::Value;

pub struct Compiler;

impl Compiler {
    pub fn compile(source: &str) -> Result<Vec<OpCode>, String> {
        let mut bytecode = Vec::new();

        for token in source.split_whitespace() {
            if let Ok(num) = token.parse::<i32>() {
                bytecode.push(OpCode::PushConst(Value::Integer(num)));
            } else {
                match token {
                    "+" | "Add" => bytecode.push(OpCode::Add),
                    _ => return Err(format!("Unknown token: {}", token)),
                }
            }
        }

        Ok(bytecode)
    }
}
