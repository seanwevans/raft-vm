// src/vm/mod.rs

pub mod backend;
pub mod error;
pub mod execution;
pub mod heap;
pub mod opcodes;
pub mod value;
pub mod vm;

pub use crate::vm::backend::Backend;
pub use crate::vm::error::VmError;
pub use crate::vm::execution::ExecutionContext;
pub use crate::vm::heap::{Heap, HeapObject};
pub use crate::vm::opcodes::OpCode;
pub use crate::vm::value::Value;
pub use crate::vm::vm::VM;

#[cfg(test)]
mod tests {
    use super::*;

    use crate::vm::backend::Backend;

    #[tokio::test]
    async fn test_basic_arithmetic() {
        let code = vec![
            OpCode::PushConst(Value::Integer(5)),
            OpCode::PushConst(Value::Integer(3)),
            OpCode::Add,
        ];

        let (mut vm, _tx) = VM::new(code, None, Backend::default());
        vm.run().await.unwrap();

        assert_eq!(vm.stack().last().cloned(), Some(Value::Integer(8)));
    }
}
