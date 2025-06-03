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

