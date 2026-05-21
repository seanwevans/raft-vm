// src/vm/vm.rs

use crate::compiler::DebugInfo;
use crate::stdlib;
use crate::vm::error::VmError;
use crate::vm::execution::{BlockingOperation, ExecutionContext, ExecutionState};
use crate::vm::heap::{Heap, HeapObject};
use crate::vm::opcodes::{Bytecode, OpCode};
use crate::vm::supervision::{ExitReason, ExitSignal};
use crate::vm::value::{MessageValue, Value};

use std::cell::Cell;
use tokio::sync::mpsc::{self, Receiver, Sender};

thread_local! {
    static NEXT_PROCESS_ID: Cell<usize> = const { Cell::new(1) };
}
const REDUCTION_QUOTA: usize = 2_000;

fn allocate_process_id() -> Result<usize, VmError> {
    NEXT_PROCESS_ID.with(|next| {
        let process_id = next.get();
        let next_id = process_id
            .checked_add(1)
            .ok_or(VmError::ProcessIdExhausted)?;
        next.set(next_id);
        Ok(process_id)
    })
}

#[derive(Debug)]
pub struct VM {
    execution: ExecutionContext,
    heap: Heap,
    self_sender: Sender<MessageValue>,
    process_id: usize,
    parent: Option<usize>,
    restart_ip: usize,
    links: Vec<Sender<MessageValue>>,
    trap_exits: bool,
    reductions: usize,
    _supervisor: Option<Sender<usize>>,
}

impl VM {
    pub fn new(
        bytecode: Vec<OpCode>,
        supervisor: Option<Sender<usize>>,
    ) -> (Self, Sender<MessageValue>) {
        Self::new_with_debug(bytecode, None, supervisor)
            .expect("failed to allocate process id for VM")
    }

    pub fn new_with_debug(
        bytecode: impl Into<Bytecode>,
        debug_info: Option<DebugInfo>,
        supervisor: Option<Sender<usize>>,
    ) -> Result<(Self, Sender<MessageValue>), VmError> {
        let (tx, rx) = mpsc::channel(100);
        let bytecode: Bytecode = bytecode.into();
        log::info!("Initializing VM with {} opcodes", bytecode.len());
        let mut execution = ExecutionContext::with_mailbox(bytecode, rx);
        execution.debug_info = debug_info;
        let mut heap = Heap::new();
        stdlib::install(&mut heap, &mut execution);

        Ok((
            VM {
                execution,
                heap,
                self_sender: tx.clone(),
                process_id: allocate_process_id()?,
                parent: None,
                restart_ip: 0,
                links: Vec::new(),
                trap_exits: false,
                reductions: 0,
                _supervisor: supervisor,
            },
            tx,
        ))
    }

    pub fn pop_stack(&mut self) -> Result<Value, VmError> {
        match self.execution.stack.pop() {
            Some(value) => {
                if let Value::Reference(address) = value {
                    if let Some(object) = self.heap.get_mut(address) {
                        object.decrement_ref();
                    } else {
                        log::error!("Attempted to pop invalid heap reference: {}", address);
                        return Err(VmError::InvalidReference);
                    }
                }
                Ok(value)
            }
            None => {
                log::error!("Attempted to pop value from an empty stack");
                Err(VmError::StackUnderflow)
            }
        }
    }

    pub fn collect_garbage(&mut self) {
        self.heap.collect_garbage();
    }

    pub fn global(&self, name: &str) -> Option<Value> {
        self.execution.globals().get(name).cloned()
    }

    pub fn heap_ref_count(&self, address: usize) -> Option<usize> {
        self.heap.get(address).map(|object| match object {
            HeapObject::Array(_, rc)
            | HeapObject::String(_, rc)
            | HeapObject::NativeFunction(_, rc)
            | HeapObject::Actor(_, _, rc)
            | HeapObject::Supervisor(_, _, rc) => *rc,
            HeapObject::Module { ref_count, .. } => *ref_count,
        })
    }

    pub fn set_ip(&mut self, ip: usize) {
        self.execution.ip = ip;
    }

    pub fn current_ip(&self) -> usize {
        self.execution.ip
    }

    pub fn restart_ip(&self) -> usize {
        self.restart_ip
    }

    pub fn set_restart_ip(&mut self, ip: usize) {
        self.restart_ip = ip;
    }

    pub async fn run(&mut self) -> Result<(), VmError> {
        if self.execution.bytecode.is_empty() {
            log::warn!("Attempted to run VM with empty bytecode");
            let error = VmError::NoBytecode;
            self.notify_links(&error).await;
            return Err(error);
        }

        loop {
            let state = self.execution.step_with_process(
                &mut self.heap,
                self.process_id,
                self.self_sender.clone(),
                self.trap_exits,
            );

            match state {
                Ok(ExecutionState::Continue) => {
                    self.reductions += 1;
                    if self.reductions >= REDUCTION_QUOTA {
                        self.reductions = 0;
                        tokio::task::yield_now().await;
                    }
                }
                Ok(ExecutionState::Halted) => break,
                Ok(ExecutionState::Yield(operation)) => {
                    self.reductions = 0;
                    if let Err(e) = self.await_blocking_operation(operation).await {
                        log::error!("Execution error at ip {}: {}", self.execution.ip, e);
                        self.notify_links(&e).await;
                        return Err(e);
                    }
                }
                Err(e) => {
                    log::error!("Execution error at ip {}: {}", self.execution.ip, e);
                    self.notify_links(&e).await;
                    return Err(e);
                }
            }
        }

        log::info!("VM execution completed successfully");
        Ok(())
    }

    async fn await_blocking_operation(
        &mut self,
        operation: BlockingOperation,
    ) -> Result<(), VmError> {
        match operation {
            BlockingOperation::ReceiveMessage => {
                let message = self
                    .execution
                    .mailbox_mut()
                    .recv()
                    .await
                    .ok_or(VmError::MailboxDisconnected)?;
                let value = self.heap.message_to_value(message)?;
                self.push_runtime_value(value)
            }
            BlockingOperation::SendMessage {
                sender,
                actor_address,
                message,
            } => match sender.send(message).await {
                Ok(()) => self.push_runtime_value(Value::Reference(actor_address)),
                Err(err) => {
                    let error = err.to_string();
                    let _value = err.0;
                    self.push_runtime_value(Value::Reference(actor_address))?;
                    Err(VmError::ChannelSend {
                        error,
                        value: Value::Null,
                    })
                }
            },
        }
    }

    fn push_runtime_value(&mut self, value: Value) -> Result<(), VmError> {
        if let Value::Reference(address) = value {
            self.increment_reference(address)?;
        }
        self.execution.stack.push(value);
        Ok(())
    }

    fn release_runtime_value(&mut self, value: Value) -> Result<(), VmError> {
        if let Value::Reference(address) = value {
            self.release_reference(address)?;
        }
        Ok(())
    }

    fn increment_reference(&mut self, address: usize) -> Result<(), VmError> {
        if let Some(object) = self.heap.get_mut(address) {
            object.increment_ref();
            Ok(())
        } else {
            Err(VmError::InvalidReference)
        }
    }

    fn release_reference(&mut self, address: usize) -> Result<(), VmError> {
        if self.heap.get(address).is_some() {
            self.heap.release_reference(address)
        } else {
            Err(VmError::InvalidReference)
        }
    }

    /// Expose a reference to the execution stack for testing or inspection.
    pub fn stack(&self) -> &Vec<Value> {
        &self.execution.stack
    }

    /// Return heap addresses referenced by this VM's execution roots.
    ///
    /// This intentionally exposes only copied heap addresses instead of the
    /// mutable stack, locals, or globals collections so the GC can trace VMs
    /// embedded in heap objects without giving callers access to VM internals.
    pub fn heap_references(&self) -> Vec<usize> {
        self.execution
            .stack
            .iter()
            .chain(self.execution.locals().values())
            .chain(self.execution.globals().values())
            .filter_map(|value| match value {
                Value::Reference(address) => Some(*address),
                _ => None,
            })
            .collect()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn push_stack_value_for_test(&mut self, value: Value) {
        self.execution.stack.push(value);
    }

    pub fn process_id(&self) -> usize {
        self.process_id
    }

    pub fn parent(&self) -> Option<usize> {
        self.parent
    }

    pub fn sender(&self) -> Sender<MessageValue> {
        self.self_sender.clone()
    }

    pub fn value_to_message(&self, value: Value) -> Result<MessageValue, VmError> {
        self.heap.value_to_message(value)
    }

    pub fn link(&mut self, sender: Sender<MessageValue>) {
        self.links.push(sender);
    }

    pub fn set_parent(&mut self, parent: usize) {
        self.parent = Some(parent);
    }

    pub fn set_trap_exits(&mut self, trap_exits: bool) {
        self.trap_exits = trap_exits;
    }

    pub fn trap_exits(&self) -> bool {
        self.trap_exits
    }

    pub fn reset_for_restart(&mut self, start_ip: usize) {
        let bytecode = self.execution.bytecode.clone();
        let debug_info = self.execution.debug_info.clone();
        let (_tx, rx) = mpsc::channel(100);
        self.execution = ExecutionContext::with_mailbox_and_debug(bytecode, rx, debug_info);
        self.execution.ip = start_ip;
    }

    pub fn mailbox_mut(&mut self) -> &mut Receiver<MessageValue> {
        self.execution.mailbox_mut()
    }

    pub fn bytecode(&self) -> Vec<OpCode> {
        self.execution.bytecode.opcodes()
    }

    pub fn debug_info(&self) -> Option<DebugInfo> {
        self.execution.debug_info.clone()
    }

    pub fn links(&self) -> Vec<Sender<MessageValue>> {
        self.links.clone()
    }

    async fn notify_links(&self, error: &VmError) {
        let signal = MessageValue::ExitSignal(ExitSignal {
            from: self.process_id,
            reason: ExitReason::from(error),
        });

        for link in &self.links {
            if let Err(err) = link.send(signal.clone()).await {
                log::warn!("Failed to deliver exit signal: {}", err);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::opcodes::OpCode;
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
    async fn test_negative_integer_exponent_produces_float_result() {
        let code = vec![
            OpCode::PushConst(Value::Integer(2)),
            OpCode::PushConst(Value::Integer(-3)),
            OpCode::Exp,
        ];

        let (mut vm, _tx) = VM::new(code, None);
        vm.run().await.unwrap();

        match vm.execution.stack.pop() {
            Some(Value::Float(result)) => {
                assert!((result - 0.125).abs() < f64::EPSILON);
            }
            other => panic!("Expected Some(Float(_)), got {:?}", other),
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

        ctx.step(&mut heap).unwrap();
        assert_eq!(ctx.ip, 1);

        ctx.step(&mut heap).unwrap();
        assert_eq!(ctx.ip, 2);

        ctx.step(&mut heap).unwrap();
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

        ctx.step(&mut heap).unwrap();
        assert_eq!(ctx.ip, 2);

        // Test Call
        let mut ctx = ExecutionContext::new(vec![
            OpCode::Call(2),
            OpCode::PushConst(Value::Integer(99)),
            OpCode::Return,
        ]);
        let mut heap = Heap::new();

        ctx.step(&mut heap).unwrap();
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
        let actor_addr = match vm.pop_stack().expect("pop_stack failed") {
            Value::Reference(addr) => addr,
            other => panic!("Expected actor reference, got {:?}", other),
        };

        // Retrieve actor from heap and run it to process message
        let actor_entry = vm.heap.get_mut(actor_addr).expect("actor not found");
        if let HeapObject::Actor(actor_vm, _sender, _) = actor_entry {
            actor_vm.run().await.unwrap();
            assert_eq!(
                actor_vm.pop_stack().expect("child pop_stack failed"),
                Value::Integer(42)
            );
        } else {
            panic!("Expected HeapObject::Actor");
        }
    }

    #[tokio::test]
    async fn test_send_message_failure() {
        use crate::vm::error::VmError;
        use crate::vm::HeapObject;

        let code = vec![
            OpCode::PushConst(Value::Null),
            OpCode::SpawnActor(4),
            OpCode::SendMessage,
            OpCode::Jump(5),
            OpCode::ReceiveMessage,
        ];

        let (mut vm, _tx) = VM::new(code, None);

        // Execute PushConst and SpawnActor
        vm.execution.step(&mut vm.heap).unwrap();
        vm.execution.step(&mut vm.heap).unwrap();

        // Swap actor sender with a closed channel to force send failure
        let actor_addr = match vm.execution.stack.last() {
            Some(Value::Reference(addr)) => *addr,
            other => panic!("Expected actor reference, got {:?}", other),
        };
        let _ = actor_addr;
        if let Some(HeapObject::Actor(_, actor_sender, _)) = vm.heap.get_mut(actor_addr) {
            let (closed_sender, closed_receiver) = mpsc::channel(1);
            drop(closed_receiver);
            *actor_sender = closed_sender;
        } else {
            panic!("Expected HeapObject::Actor");
        }

        // SendMessage should now fail
        let result = vm.execution.step(&mut vm.heap);

        match result {
            Err(VmError::ChannelSend { value, .. }) => {
                assert_eq!(value, Value::Null);
            }
            other => panic!("Expected ChannelSend error, got {:?}", other),
        }

        if let Some(HeapObject::Actor(_, _, rc)) = vm.heap.get(actor_addr) {
            assert_eq!(
                *rc, 1,
                "actor reference count should stay on stack after failure"
            );
        } else {
            panic!("Expected HeapObject::Actor");
        }
    }

    #[tokio::test]
    async fn blocking_send_failure_releases_retained_message_reference() {
        use crate::vm::error::VmError;
        use crate::vm::heap::ProcessHandle;
        use crate::vm::HeapObject;
        use std::sync::{Arc, Mutex};

        let (mut vm, _tx) = VM::new(vec![OpCode::Return], None);
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);

        let (actor_sender, _actor_mailbox) = mpsc::channel(1);
        let task = tokio::spawn(async { Ok(()) });
        let final_stack = Arc::new(Mutex::new(Vec::new()));
        let actor_vm = ProcessHandle::new(
            1,
            None,
            0,
            vec![OpCode::Return],
            None,
            Vec::new(),
            false,
            task,
            final_stack,
        );
        let actor_addr = vm
            .heap
            .allocate(HeapObject::Actor(actor_vm, actor_sender, 0));
        let err = vm
            .await_blocking_operation(BlockingOperation::SendMessage {
                sender,
                actor_address: actor_addr,
                message: crate::vm::value::MessageValue::Array(Vec::new()),
            })
            .await
            .expect_err("closed blocking send should fail");

        match err {
            VmError::ChannelSend { value, .. } => {
                assert_eq!(value, Value::Null);
            }
            other => panic!("Expected ChannelSend error, got {other:?}"),
        }

        assert_eq!(
            vm.execution.stack,
            vec![Value::Reference(actor_addr)],
            "actor reference should be restored on blocking send failure",
        );
        assert_eq!(
            vm.heap_ref_count(actor_addr),
            Some(1),
            "actor reference count should stay on stack after blocking send failure",
        );
    }
}
