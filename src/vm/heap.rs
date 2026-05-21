// src/vm/heap.rs

use crate::vm::error::VmError;
use crate::vm::value::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::task::JoinHandle;

use crate::compiler::DebugInfo;
use crate::vm::opcodes::OpCode;
use crate::vm::supervision::{ChildSpec, SupervisorState, SupervisorStrategy};

#[derive(Debug)]
pub struct Heap {
    objects: Vec<Option<HeapObject>>,
    free_list: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct NativeFunction {
    pub name: String,
    pub arity: usize,
    pub function: fn(Vec<Value>) -> Result<Value, VmError>,
}

#[derive(Debug)]
pub struct ProcessHandle {
    process_id: usize,
    parent: Option<usize>,
    start_ip: usize,
    current_ip: usize,
    bytecode: Vec<OpCode>,
    debug_info: Option<DebugInfo>,
    links: Vec<Sender<Value>>,
    trap_exits: bool,
    supervisor_state: SupervisorState,
    task: Option<JoinHandle<Result<(), VmError>>>,
    mailbox: Receiver<Value>,
    mailbox_sender: Sender<Value>,
    final_stack: Arc<Mutex<Vec<Value>>>,
}

#[derive(Debug)]
pub enum HeapObject {
    Array(Vec<Value>, usize),
    String(String, usize),
    Module {
        name: String,
        exports: HashMap<String, Value>,
        ref_count: usize,
    },
    NativeFunction(NativeFunction, usize),
    Actor(ProcessHandle, Sender<Value>, usize),
    Supervisor(ProcessHandle, Sender<Value>, usize),
}

impl ProcessHandle {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        process_id: usize,
        parent: Option<usize>,
        start_ip: usize,
        bytecode: Vec<OpCode>,
        debug_info: Option<DebugInfo>,
        links: Vec<Sender<Value>>,
        trap_exits: bool,
        task: JoinHandle<Result<(), VmError>>,
        final_stack: Arc<Mutex<Vec<Value>>>,
    ) -> Self {
        let (mailbox_sender, mailbox) = tokio::sync::mpsc::channel(1);
        Self {
            process_id,
            parent,
            start_ip,
            current_ip: start_ip,
            bytecode,
            debug_info,
            links,
            trap_exits,
            supervisor_state: SupervisorState::default(),
            task: Some(task),
            mailbox,
            mailbox_sender,
            final_stack,
        }
    }

    pub fn process_id(&self) -> usize {
        self.process_id
    }

    pub fn parent(&self) -> Option<usize> {
        self.parent
    }

    pub fn restart_ip(&self) -> usize {
        self.start_ip
    }

    pub fn current_ip(&self) -> usize {
        self.current_ip
    }

    pub fn set_ip(&mut self, ip: usize) {
        self.current_ip = ip;
    }

    pub fn bytecode(&self) -> Vec<OpCode> {
        self.bytecode.clone()
    }

    pub fn heap_references(&self) -> Vec<usize> {
        Vec::new()
    }

    pub fn debug_info(&self) -> Option<DebugInfo> {
        self.debug_info.clone()
    }

    pub fn links(&self) -> Vec<Sender<Value>> {
        self.links.clone()
    }

    pub fn trap_exits(&self) -> bool {
        self.trap_exits
    }

    pub fn replace_runtime(
        &mut self,
        task: JoinHandle<Result<(), VmError>>,
        start_ip: usize,
        mailbox: Receiver<Value>,
        mailbox_sender: Sender<Value>,
    ) {
        if let Some(task) = &self.task {
            task.abort();
        }
        self.task = Some(task);
        self.start_ip = start_ip;
        self.current_ip = start_ip;
        self.mailbox = mailbox;
        self.mailbox_sender = mailbox_sender;
    }

    pub async fn run(&mut self) -> Result<(), VmError> {
        match self.task.take() {
            Some(task) => match task.await {
                Ok(result) => result,
                Err(err) if err.is_cancelled() => Ok(()),
                Err(err) => Err(VmError::Message(err.to_string())),
            },
            None => Ok(()),
        }
    }

    pub fn pop_stack(&mut self) -> Result<Value, VmError> {
        self.final_stack
            .lock()
            .map_err(|err| VmError::Message(err.to_string()))?
            .pop()
            .ok_or(VmError::StackUnderflow)
    }

    pub fn mailbox_mut(&mut self) -> &mut Receiver<Value> {
        &mut self.mailbox
    }

    pub fn is_mailbox_closed(&self) -> bool {
        self.mailbox.is_closed()
    }

    pub fn mailbox_sender(&self) -> Sender<Value> {
        self.mailbox_sender.clone()
    }

    pub fn set_strategy(&mut self, strategy: usize) {
        let strategy = SupervisorStrategy::from_usize(strategy);
        self.supervisor_state.set_strategy(strategy);
        log::info!("Set supervisor strategy to {:?}", strategy);
    }

    pub fn strategy(&self) -> SupervisorStrategy {
        self.supervisor_state.strategy()
    }

    pub fn supervised_children(&self) -> &[ChildSpec] {
        self.supervisor_state.children()
    }

    pub fn restart_targets(&mut self, child: ChildSpec) -> Vec<ChildSpec> {
        self.supervisor_state.restart_targets(child)
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    pub fn new() -> Self {
        Self {
            objects: Vec::new(),
            free_list: Vec::new(),
        }
    }

    pub fn allocate(&mut self, object: HeapObject) -> usize {
        if let Some(address) = self.free_list.pop() {
            self.objects[address] = Some(object);
            log::info!("Allocated object in reused heap slot {}", address);
            address
        } else {
            let address = self.objects.len();
            self.objects.push(Some(object));
            log::info!("Allocated object at new heap slot {}", address);
            address
        }
    }

    pub fn get(&self, address: usize) -> Option<&HeapObject> {
        if let Some(Some(obj)) = self.objects.get(address) {
            Some(obj)
        } else {
            log::warn!("Attempted to access invalid heap address: {}", address);
            None
        }
    }

    pub fn get_mut(&mut self, address: usize) -> Option<&mut HeapObject> {
        self.objects.get_mut(address).and_then(Option::as_mut)
    }

    pub fn collect_garbage(&mut self) {
        let mut reclaimed = 0usize;
        for address in 0..self.objects.len() {
            if self.try_release(address) {
                reclaimed += 1;
            }
        }
        if reclaimed > 0 {
            log::info!("Reclaimed {} zero-ref heap objects", reclaimed);
        }
    }

    pub fn release_reference(&mut self, address: usize) -> Result<(), VmError> {
        let child_references = match self.get_mut(address) {
            Some(object) => {
                let refs = if object.ref_count() == 1 {
                    object.references()
                } else {
                    Vec::new()
                };
                object.decrement_ref();
                refs
            }
            None => return Err(VmError::InvalidReference),
        };

        for child in child_references {
            self.release_reference(child)?;
        }
        let _ = self.try_release(address);
        Ok(())
    }

    fn try_release(&mut self, address: usize) -> bool {
        let should_release =
            matches!(self.objects.get(address), Some(Some(object)) if object.ref_count() == 0);
        if !should_release {
            return false;
        }

        let child_references = self
            .objects
            .get(address)
            .and_then(|object| object.as_ref())
            .map(|object| object.references())
            .unwrap_or_default();
        self.objects[address] = None;
        self.free_list.push(address);
        for child in child_references {
            let _ = self.release_reference(child);
        }
        true
    }
}

impl HeapObject {
    pub fn is_alive(&self) -> bool {
        self.ref_count() > 0
    }

    pub fn ref_count(&self) -> usize {
        match self {
            HeapObject::Array(_, rc)
            | HeapObject::String(_, rc)
            | HeapObject::NativeFunction(_, rc)
            | HeapObject::Actor(_, _, rc)
            | HeapObject::Supervisor(_, _, rc) => *rc,
            HeapObject::Module { ref_count, .. } => *ref_count,
        }
    }

    pub fn references(&self) -> Vec<usize> {
        match self {
            HeapObject::Array(values, _) => values
                .iter()
                .filter_map(|value| match value {
                    Value::Reference(address) => Some(*address),
                    _ => None,
                })
                .collect(),
            HeapObject::Module { exports, .. } => exports
                .values()
                .filter_map(|value| match value {
                    Value::Reference(address) => Some(*address),
                    _ => None,
                })
                .collect(),
            HeapObject::Actor(vm, _, _) | HeapObject::Supervisor(vm, _, _) => vm.heap_references(),
            HeapObject::String(_, _) | HeapObject::NativeFunction(_, _) => Vec::new(),
        }
    }

    pub fn increment_ref(&mut self) {
        match self {
            HeapObject::Array(_, rc)
            | HeapObject::String(_, rc)
            | HeapObject::NativeFunction(_, rc)
            | HeapObject::Actor(_, _, rc)
            | HeapObject::Supervisor(_, _, rc) => *rc += 1,
            HeapObject::Module {
                ref mut ref_count, ..
            } => *ref_count += 1,
        }
    }

    pub fn decrement_ref(&mut self) {
        match self {
            HeapObject::Array(_, rc)
            | HeapObject::String(_, rc)
            | HeapObject::NativeFunction(_, rc)
            | HeapObject::Actor(_, _, rc)
            | HeapObject::Supervisor(_, _, rc) => {
                if *rc > 0 {
                    *rc -= 1;
                }
            }
            HeapObject::Module {
                ref mut ref_count, ..
            } => {
                if *ref_count > 0 {
                    *ref_count -= 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gc_preserves_objects_reachable_from_live_actor_vm_stack() {
        let mut heap = Heap::new();
        let string_address = heap.allocate(HeapObject::String("actor string".to_string(), 0));
        let array_address =
            heap.allocate(HeapObject::Array(vec![Value::Reference(string_address)], 0));

        let (mut actor_vm, actor_sender) = VM::new(Vec::new(), None);
        actor_vm.push_stack_value_for_test(Value::Reference(array_address));
        let actor_address = heap.allocate(HeapObject::Actor(actor_vm, actor_sender, 1));

        heap.collect_garbage();

        assert!(
            matches!(heap.get(actor_address), Some(HeapObject::Actor(_, _, 1))),
            "live actor should not be reclaimed"
        );
        assert!(
            matches!(heap.get(array_address), Some(HeapObject::Array(_, 0))),
            "array referenced by actor VM stack should not be reclaimed"
        );
        assert!(
            matches!(heap.get(string_address), Some(HeapObject::String(value, 0)) if value == "actor string"),
            "string referenced by actor VM array should not be reclaimed"
        );
    }
}
