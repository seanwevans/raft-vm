// src/lib.rs

pub mod compiler;
pub mod runtime;
pub mod vm;

pub use compiler::{
    CompiledProgram, Compiler, CompilerError, DebugInfo, SourceLocation, SourceSpan,
};
pub use runtime::Actor;
pub use vm::VM;

use crate::vm::VmError;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Runs a Raft program from source code
pub async fn run(source: &str) -> Result<(), VmError> {
    let program = Compiler::compile_with_debug(source)?;

    let (mut vm, _tx) = VM::new_with_debug(program.bytecode, Some(program.debug_info), None);
    vm.run().await
}
