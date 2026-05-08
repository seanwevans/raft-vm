// src/runtime/runtime.rs

use tokio::sync::mpsc::Sender;

use crate::vm::error::VmError;
use crate::vm::value::Value;
use crate::vm::{OpCode, VM};

/// A lightweight wrapper around a `VM` that exposes a mailbox
/// for message passing.
pub struct Actor {
    vm: VM,
    sender: Sender<Value>,
}

impl Actor {
    /// Create a new actor from bytecode.
    pub fn new(bytecode: Vec<OpCode>) -> Self {
        let (vm, tx) = VM::new(bytecode, None);
        Actor { vm, sender: tx }
    }

    /// Obtain a sender that can be used to send messages to this actor.
    pub fn sender(&self) -> Sender<Value> {
        self.sender.clone()
    }

    /// Obtain this actor's runtime process id.
    pub fn process_id(&self) -> usize {
        self.vm.process_id()
    }

    /// Link this actor to another actor so this actor receives exit signals
    /// when the other actor terminates with an error.
    pub fn link_to(&mut self, other: &mut Actor) {
        other.vm.link(self.sender());
    }

    /// Monitor another actor. Monitoring is one-way and currently delivers the
    /// same mailbox exit signal shape as links.
    pub fn monitor(&mut self, other: &mut Actor) {
        self.link_to(other);
    }

    /// Send a message to the actor's mailbox.
    pub async fn send(&self, msg: Value) -> Result<(), VmError> {
        self.sender.send(msg).await.map_err(|e| {
            let error = e.to_string();
            let value = e.0;
            VmError::ChannelSend { error, value }
        })
    }

    /// Execute the actor until its VM halts.
    pub async fn run(&mut self) -> Result<(), VmError> {
        self.vm.run().await
    }

    /// Receive the next message if available.
    pub async fn handle_next_message(&mut self) -> Option<Value> {
        self.vm.mailbox_mut().recv().await
    }
}
