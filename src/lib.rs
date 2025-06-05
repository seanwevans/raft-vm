// src/lib.rs

pub mod vm;
pub mod compiler;
pub mod runtime;

pub use vm::VM;
pub use compiler::Compiler;
pub use runtime::Actor;

use crate::vm::Backend;


pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Runs a Raft program from source code
pub async fn run(source: &str) -> Result<(), String> {
    use log;    
    
    let bytecode = match Compiler::compile(source) {
        Ok(bc) => bc,
        Err(e) => {
            log::error!("Compilation error: {}", e);
            return Err(format!("Compilation Error: {}", e));
        }
    };

    let (mut vm, _tx) = VM::new(bytecode, None, Backend::default());
    match vm.run().await {
        Ok(_) => {
            log::info!("Program executed successfully");
            Ok(())
        }
        Err(e) => {
            log::error!("VM execution error: {}", e);
            Err(format!("VM Error: {}", e))
        }
    }
}
