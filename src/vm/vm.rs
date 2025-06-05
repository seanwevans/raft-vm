// src/vm/vm.rs

use crate::vm::backend::Backend;
use crate::vm::execution::ExecutionContext;
use crate::vm::heap::Heap;
use crate::vm::opcodes::OpCode;
use crate::vm::value::Value;

use tokio::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug)]
pub struct VM {
    execution: ExecutionContext,
    heap: Heap,
    bytecode: Vec<OpCode>,
    pub mailbox: Receiver<Value>,
    supervisor: Option<Sender<usize>>,
    backend: Backend,
}

impl VM {
    pub fn new(
        bytecode: Vec<OpCode>,
        supervisor: Option<Sender<usize>>,
        backend: Backend,
    ) -> (Self, Sender<Value>) {
        let (tx, rx) = mpsc::channel(100);
        log::info!("Initializing VM with {} opcodes", bytecode.len());
        (
            VM {
                execution: ExecutionContext::new(bytecode.clone()),
                heap: Heap::new(),
                bytecode,
                mailbox: rx,
                supervisor,
                backend,
            },
            tx,
        )
    }

    pub fn pop_stack(&mut self) -> Option<Value> {
        self.execution.stack.pop()
    }

    pub async fn run(&mut self) -> Result<(), String> {
        if self.bytecode.is_empty() {
            log::warn!("Attempted to run VM with empty bytecode");
            return Err("No bytecode to execute".to_string());
        }

        while self.execution.ip < self.bytecode.len() {
            if let Err(e) = self.execution.step(&mut self.heap, &mut self.mailbox).await {
                log::error!("Execution error at ip {}: {}", self.execution.ip, e);
                return Err(e);
            }
        }
        log::info!("VM execution completed successfully");
        Ok(())
    }

    /// Expose a reference to the execution stack for testing or inspection.
    pub fn stack(&self) -> &Vec<Value> {
        &self.execution.stack
    }

    pub fn set_strategy(&mut self, _strategy: usize) {
        log::info!("Set supervisor strategy to {}", _strategy);
    }

    pub fn restart_child(&mut self, _child_ref: usize) {
        log::info!("Restarted child at {}", _child_ref);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::backend::Backend;
    use crate::vm::value::Value;

    #[tokio::test]
    async fn test_basic_arithmetic() {
        let code = vec![
            OpCode::PushConst(Value::Integer(5)),
            OpCode::PushConst(Value::Integer(3)),
            OpCode::Add,
        ];

        let (mut vm, _tx) = VM::new(code, None, Backend::default());
        vm.run().await.unwrap();

        match vm.execution.stack.pop() {
            Some(Value::Integer(8)) => {}
            other => panic!("Expected Some(Integer(8)), got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_sequential_ip_increment() {
        let code = vec![
            OpCode::PushConst(Value::Integer(1)),
            OpCode::PushConst(Value::Integer(2)),
            OpCode::Add,
        ];

        let mut ctx = ExecutionContext::new(code);
        let mut heap = Heap::new();
        let (_tx, mut rx) = tokio::sync::mpsc::channel(1);

        ctx.step(&mut heap, &mut rx).await.unwrap();
        assert_eq!(ctx.ip, 1);

        ctx.step(&mut heap, &mut rx).await.unwrap();
        assert_eq!(ctx.ip, 2);

        ctx.step(&mut heap, &mut rx).await.unwrap();
        assert_eq!(ctx.ip, 3);
    }

    #[tokio::test]
    async fn test_jump_and_call_modify_ip() {
        // Test Jump
        let mut ctx = ExecutionContext::new(vec![
            OpCode::Jump(2),
            OpCode::PushConst(Value::Integer(0)),
            OpCode::PushConst(Value::Integer(1)),
        ]);
        let mut heap = Heap::new();
        let (_tx, mut rx) = tokio::sync::mpsc::channel(1);

        ctx.step(&mut heap, &mut rx).await.unwrap();
        assert_eq!(ctx.ip, 2);

        // Test Call
        let mut ctx = ExecutionContext::new(vec![
            OpCode::Call(2),
            OpCode::PushConst(Value::Integer(99)),
            OpCode::Return,
        ]);
        let mut heap = Heap::new();
        let (_tx, mut rx) = tokio::sync::mpsc::channel(1);

        ctx.step(&mut heap, &mut rx).await.unwrap();
        assert_eq!(ctx.ip, 2);
        assert_eq!(ctx.call_stack, vec![1]);
    }
}
