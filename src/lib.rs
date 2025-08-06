// src/lib.rs

pub mod compiler;
pub mod runtime;
pub mod vm;

pub use compiler::Compiler;
pub use runtime::Actor;
pub use vm::VM;

use crate::vm::{Backend, VmError};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Runs a Raft program from source code
pub async fn run(source: &str) -> Result<(), VmError> {
    let bytecode = Compiler::compile(source)
        .map_err(|e| VmError::from(format!("Compilation Error: {}", e)))?;

    let (mut vm, _tx) = VM::new(bytecode, None, Backend::default());
    vm.run().await
}
