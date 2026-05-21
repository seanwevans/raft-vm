// src/runtime/runtime.rs

use tokio::sync::mpsc::Sender;

use crate::vm::error::VmError;
use crate::vm::value::{MessageValue, Value};
use crate::vm::{OpCode, VM};

/// A lightweight wrapper around a `VM` that exposes a mailbox
/// for message passing.
pub struct Actor {
    vm: VM,
    sender: Sender<MessageValue>,
}

impl Actor {
    /// Create a new actor from bytecode.
    pub fn new(bytecode: Vec<OpCode>) -> Self {
        let (vm, tx) = VM::new(bytecode, None);
        Actor { vm, sender: tx }
    }

    /// Obtain a sender that can be used to send messages to this actor.
    pub fn sender(&self) -> Sender<MessageValue> {
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
        let message = self.vm.value_to_message(msg)?;
        self.sender.send(message).await.map_err(|e| {
            let error = e.to_string();
            let value = match e.0 {
                MessageValue::Integer(v) => Value::Integer(v),
                MessageValue::Float(v) => Value::Float(v),
                MessageValue::Boolean(v) => Value::Boolean(v),
                MessageValue::ExitSignal(v) => Value::ExitSignal(v),
                _ => Value::Null,
            };
            VmError::ChannelSend { error, value }
        })
    }

    /// Execute the actor until its VM halts.
    pub async fn run(&mut self) -> Result<(), VmError> {
        self.vm.run().await
    }

    /// Receive the next message if available.
    pub async fn handle_next_message(&mut self) -> Option<Value> {
        self.vm
            .mailbox_mut()
            .recv()
            .await
            .and_then(|message| match message {
                MessageValue::Integer(v) => Some(Value::Integer(v)),
                MessageValue::Float(v) => Some(Value::Float(v)),
                MessageValue::Boolean(v) => Some(Value::Boolean(v)),
                MessageValue::ExitSignal(v) => Some(Value::ExitSignal(v)),
                MessageValue::Null => Some(Value::Null),
                _ => None,
            })
    }
}
