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

    pub fn debug_info(&self) -> Option<DebugInfo> {
        self.debug_info.clone()
    }

    pub fn links(&self) -> Vec<Sender<Value>> {
        self.links.clone()
    }

    pub fn trap_exits(&self) -> bool {
        self.trap_exits
    }

    pub fn replace_task(&mut self, task: JoinHandle<Result<(), VmError>>, start_ip: usize) {
        if let Some(task) = &self.task {
            task.abort();
        }
        self.task = Some(task);
        self.start_ip = start_ip;
        self.current_ip = start_ip;
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
        let before = self.live_count();
        let live = self.trace_live_objects();
        self.sweep_unmarked(&live);
        let collected = before - self.live_count();
        if collected > 0 {
            log::info!("Collected {} unreachable heap objects", collected);
        }
    }

    fn live_count(&self) -> usize {
        self.objects
            .iter()
            .filter(|object| object.is_some())
            .count()
    }

    fn trace_live_objects(&self) -> Vec<bool> {
        let internal_references = self.internal_reference_counts();
        let mut live = vec![false; self.objects.len()];
        let mut worklist = Vec::new();

        for (address, object) in self.objects.iter().enumerate() {
            let Some(object) = object else {
                continue;
            };

            let ref_count = object.ref_count();
            if ref_count > internal_references[address] {
                live[address] = true;
                worklist.push(address);
            }
        }

        while let Some(address) = worklist.pop() {
            let Some(object) = self.get(address) else {
                continue;
            };

            for referenced_address in object.references() {
                if referenced_address < live.len()
                    && self.objects[referenced_address].is_some()
                    && !live[referenced_address]
                {
                    live[referenced_address] = true;
                    worklist.push(referenced_address);
                }
            }
        }

        live
    }

    fn internal_reference_counts(&self) -> Vec<usize> {
        let mut counts = vec![0; self.objects.len()];
        for object in self.objects.iter().flatten() {
            for referenced_address in object.references() {
                if let Some(count) = counts.get_mut(referenced_address) {
                    *count += 1;
                }
            }
        }
        counts
    }

    fn sweep_unmarked(&mut self, live: &[bool]) {
        let mut references_to_release = Vec::new();
        let mut addresses_to_free = Vec::new();

        for (address, object) in self.objects.iter().enumerate() {
            let Some(object) = object else {
                continue;
            };

            if !live.get(address).copied().unwrap_or(false) {
                references_to_release.extend(object.references().into_iter().filter(
                    |referenced_address| live.get(*referenced_address).copied().unwrap_or(false),
                ));
                addresses_to_free.push(address);
            }
        }

        for referenced_address in references_to_release {
            if let Some(object) = self.get_mut(referenced_address) {
                object.decrement_ref();
            }
        }

        for address in addresses_to_free {
            if self.objects[address].take().is_some() {
                self.free_list.push(address);
            }
        }
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
