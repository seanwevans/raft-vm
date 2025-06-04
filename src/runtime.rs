// src/runtime/runtime.rs

use tokio::sync::mpsc::Sender;

use crate::vm::value::Value;
use crate::vm::{Backend, OpCode, VM};

/// A lightweight wrapper around a `VM` that exposes a mailbox
/// for message passing.
pub struct Actor {
    vm: VM,
    sender: Sender<Value>,
}

impl Actor {
    /// Create a new actor from bytecode.
    pub fn new(bytecode: Vec<OpCode>, backend: Backend) -> Self {
        let (vm, tx) = VM::new(bytecode, None, backend);
        Actor { vm, sender: tx }
    }

    /// Obtain a sender that can be used to send messages to this actor.
    pub fn sender(&self) -> Sender<Value> {
        self.sender.clone()
    }

    /// Send a message to the actor's mailbox.
    pub async fn send(&self, msg: Value) -> Result<(), String> {
        self.sender.send(msg).await.map_err(|e| e.to_string())
    }

    /// Execute the actor until its VM halts.
    pub async fn run(&mut self) -> Result<(), String> {
        self.vm.run().await
    }

    /// Receive the next message if available.
    pub async fn handle_next_message(&mut self) -> Option<Value> {
        self.vm.mailbox.recv().await
    }
}
