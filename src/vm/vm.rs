// src/vm/vm.rs

use crate::vm::error::VmError;
use crate::vm::execution::ExecutionContext;
use crate::vm::heap::Heap;
use crate::vm::opcodes::OpCode;
use crate::vm::value::Value;

use tokio::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug)]
pub struct VM {
    execution: ExecutionContext,
    heap: Heap,
    pub mailbox: Receiver<Value>,
    _supervisor: Option<Sender<usize>>,
}

impl VM {
    pub fn new(bytecode: Vec<OpCode>, supervisor: Option<Sender<usize>>) -> (Self, Sender<Value>) {
        let (tx, rx) = mpsc::channel(100);
        log::info!("Initializing VM with {} opcodes", bytecode.len());
        (
            VM {
                execution: ExecutionContext::new(bytecode),
                heap: Heap::new(),
                mailbox: rx,
                _supervisor: supervisor,
            },
            tx,
        )
    }

    pub fn pop_stack(&mut self) -> Option<Value> {
        self.execution.stack.pop()
    }

    pub fn set_ip(&mut self, ip: usize) {
        self.execution.ip = ip;
    }

    pub async fn run(&mut self) -> Result<(), VmError> {
        if self.execution.bytecode.is_empty() {
            log::warn!("Attempted to run VM with empty bytecode");
            return Err(VmError::NoBytecode);
        }

        while self.execution.ip < self.execution.bytecode.len() {
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
    use crate::vm::value::Value;

    #[tokio::test]
    async fn test_basic_arithmetic() {
        let code = vec![
            OpCode::PushConst(Value::Integer(5)),
            OpCode::PushConst(Value::Integer(3)),
            OpCode::Add,
        ];

        let (mut vm, _tx) = VM::new(code, None);
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

    #[tokio::test]
    async fn test_spawn_actor_and_message_delivery() {
        use crate::vm::HeapObject;

        // Parent code: send 42 to spawned actor
        let code = vec![
            OpCode::PushConst(Value::Integer(42)), // message
            OpCode::SpawnActor(4),                 // spawn actor starting at 4
            OpCode::SendMessage,                   // send message
            OpCode::Jump(5),                       // skip child code
            // Child actor code starts here (index 4)
            OpCode::ReceiveMessage,
        ];

        let (mut vm, _tx) = VM::new(code, None);
        vm.run().await.unwrap();

        // Actor reference should remain on stack after sending
        let actor_addr = match vm.pop_stack() {
            Some(Value::Reference(addr)) => addr,
            other => panic!("Expected actor reference, got {:?}", other),
        };

        // Retrieve actor from heap and run it to process message
        let actor_entry = vm.heap.get_mut(actor_addr).expect("actor not found");
        if let HeapObject::Actor(actor_vm, _sender, _) = actor_entry {
            actor_vm.run().await.unwrap();
            assert_eq!(actor_vm.pop_stack(), Some(Value::Integer(42)));
        } else {
            panic!("Expected HeapObject::Actor");
        }
    }

    #[tokio::test]
    async fn test_send_message_failure() {
        use crate::vm::HeapObject;

        let code = vec![
            OpCode::PushConst(Value::Integer(1)),
            OpCode::SpawnActor(4),
            OpCode::SendMessage,
            OpCode::Jump(5),
            OpCode::ReceiveMessage,
        ];

        let (mut vm, _tx) = VM::new(code, None);

        // Execute PushConst and SpawnActor
        vm.execution
            .step(&mut vm.heap, &mut vm.mailbox)
            .await
            .unwrap();
        vm.execution
            .step(&mut vm.heap, &mut vm.mailbox)
            .await
            .unwrap();

        // Close actor mailbox to force send failure
        let actor_addr = match vm.execution.stack.last() {
            Some(Value::Reference(addr)) => *addr,
            other => panic!("Expected actor reference, got {:?}", other),
        };
        if let Some(HeapObject::Actor(actor_vm, _, _)) = vm.heap.get_mut(actor_addr) {
            actor_vm.mailbox.close();
        } else {
            panic!("Expected HeapObject::Actor");
        }

        // SendMessage should now fail
        let result = vm.execution.step(&mut vm.heap, &mut vm.mailbox).await;
        assert!(result.is_err());
    }
}
