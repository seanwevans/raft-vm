// src/vm/mod.rs

pub mod opcodes;
pub mod execution;
pub mod heap;
pub mod backend;
pub mod value;
pub mod vm;

pub use crate::vm::value::Value;
pub use crate::vm::opcodes::OpCode;
pub use crate::vm::heap::{Heap, HeapObject};
pub use crate::vm::execution::ExecutionContext;
pub use crate::vm::vm::VM;
pub use crate::vm::backend::Backend;

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;
    use tokio::sync::mpsc;

    #[test]
    fn test_basic_arithmetic() {
        let code = vec![
            OpCode::PushConst(Value::Integer(5)),
            OpCode::PushConst(Value::Integer(3)),
            OpCode::Add,
        ];

        let mut ctx = ExecutionContext::new(code);
        let mut heap = Heap::new();
        let (_tx, mut mailbox) = mpsc::channel(1);
        let rt = Runtime::new().unwrap();
        rt.block_on(async {
            while ctx.ip() < ctx.bytecode.len() {
                ctx.step(&mut heap, &mut mailbox).await.unwrap();
            }
        });

        assert_eq!(ctx.stack.pop(), Some(Value::Integer(8)));
    }
}
