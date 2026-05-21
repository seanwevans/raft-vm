// src/vm/heap.rs

use crate::vm::error::VmError;
use crate::vm::value::{MessageValue, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Sender;
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

/// A handle to a process spawned onto the Tokio runtime.
///
/// ### Option 3 tradeoff (current contract)
///
/// **Buys:** lightweight process tracking that can await task completion and
/// inspect captured post-mortem state.
///
/// **Costs:** no live mailbox ownership through the handle while the process is
/// running.
///
/// A `ProcessHandle` represents a process that has been spawned onto the Tokio
/// runtime. While the process is running, its mailbox is owned by the VM inside
/// the spawned task; the handle does not provide a way to peek at or close the
/// mailbox in-flight. To send messages, use the `Sender<MessageValue>` stored
/// alongside the handle in `HeapObject::Actor`.
///
/// To inspect a process post-mortem, await [`ProcessHandle::run`] (which awaits
/// the underlying `JoinHandle`) and then read [`ProcessHandle::pop_stack`] or
/// [`ProcessHandle::heap_references`] from the captured `final_stack`.
///
/// This makes the limitation explicit and gives Phase 1 a clear target:
/// introduce a proper `Process` type that is not moved into a Tokio task and
/// can therefore provide live mailbox access.
#[derive(Debug)]
/// While a process is running, the only way to communicate with it from outside is via the
/// `Sender<MessageValue>` stored alongside the handle in `HeapObject::Actor`. The handle itself
/// exposes no live mailbox view; the running VM owns it. Post-mortem state is available via
/// `pop_stack` once `run` has been awaited.
pub struct ProcessHandle {
    process_id: usize,
    parent: Option<usize>,
    start_ip: usize,
    current_ip: usize,
    bytecode: Vec<OpCode>,
    debug_info: Option<DebugInfo>,
    links: Vec<Sender<MessageValue>>,
    trap_exits: bool,
    supervisor_state: SupervisorState,
    task: Option<JoinHandle<Result<(), VmError>>>,
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
    Actor(ProcessHandle, Sender<MessageValue>, usize),
    Supervisor(ProcessHandle, Sender<MessageValue>, usize),
}

impl ProcessHandle {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        process_id: usize,
        parent: Option<usize>,
        start_ip: usize,
        bytecode: Vec<OpCode>,
        debug_info: Option<DebugInfo>,
        links: Vec<Sender<MessageValue>>,
        trap_exits: bool,
        task: JoinHandle<Result<(), VmError>>,
        final_stack: Arc<Mutex<Vec<Value>>>,
    ) -> Self {
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

    pub fn links(&self) -> Vec<Sender<MessageValue>> {
        self.links.clone()
    }

    pub fn trap_exits(&self) -> bool {
        self.trap_exits
    }

    pub fn replace_runtime(&mut self, task: JoinHandle<Result<(), VmError>>, start_ip: usize) {
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

    pub fn heap_references(&self) -> Vec<usize> {
        self.final_stack
            .lock()
            .map(|stack| {
                stack
                    .iter()
                    .filter_map(|value| match value {
                        Value::Reference(address) => Some(*address),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
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

    #[allow(clippy::needless_range_loop)]
    pub fn collect_garbage(&mut self) {
        let object_count = self.objects.len();
        let mut internal_incoming = vec![0usize; object_count];

        for address in 0..object_count {
            let Some(object) = self.objects[address].as_ref() else {
                continue;
            };
            for child in object.references() {
                if child < object_count && self.objects[child].is_some() {
                    internal_incoming[child] += 1;
                }
            }
        }

        let mut reachable = vec![false; object_count];
        let mut worklist = VecDeque::new();

        for address in 0..object_count {
            let Some(object) = self.objects[address].as_ref() else {
                continue;
            };
            let external_incoming = object
                .ref_count()
                .saturating_sub(internal_incoming[address]);
            if external_incoming > 0 {
                reachable[address] = true;
                worklist.push_back(address);
            }
        }

        while let Some(address) = worklist.pop_front() {
            let Some(object) = self.objects[address].as_ref() else {
                continue;
            };
            for child in object.references() {
                if child < object_count && self.objects[child].is_some() && !reachable[child] {
                    reachable[child] = true;
                    worklist.push_back(child);
                }
            }
        }

        let mut pending_release = VecDeque::new();
        for address in 0..object_count {
            if self.objects[address].is_some() && !reachable[address] {
                pending_release.push_back(address);
            }
        }

        let mut reclaimed = 0usize;
        while let Some(address) = pending_release.pop_front() {
            if self.try_release(address) {
                reclaimed += 1;
                continue;
            }

            let child_references = self
                .objects
                .get(address)
                .and_then(|object| object.as_ref())
                .map(|object| object.references())
                .unwrap_or_default();

            self.objects[address] = None;
            self.free_list.push(address);
            reclaimed += 1;

            for child in child_references {
                if self.release_reference(child).is_ok() && self.try_release(child) {
                    pending_release.push_back(child);
                }
            }
        }

        if reclaimed > 0 {
            log::info!("Reclaimed {} unreachable heap objects", reclaimed);
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

    pub fn value_to_message(&self, value: Value) -> Result<MessageValue, VmError> {
        match value {
            Value::Integer(v) => Ok(MessageValue::Integer(v)),
            Value::Float(v) => Ok(MessageValue::Float(v)),
            Value::Boolean(v) => Ok(MessageValue::Boolean(v)),
            Value::ExitSignal(v) => Ok(MessageValue::ExitSignal(v)),
            Value::Null => Ok(MessageValue::Null),
            Value::Reference(address) => self.reference_to_message(address),
        }
    }

    fn reference_to_message(&self, address: usize) -> Result<MessageValue, VmError> {
        match self.get(address).ok_or(VmError::InvalidReference)? {
            HeapObject::Array(values, _) => Ok(MessageValue::Array(
                values
                    .iter()
                    .map(|value| self.value_to_message(*value))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            HeapObject::String(value, _) => Ok(MessageValue::String(value.clone())),
            HeapObject::Module { exports, .. } => Ok(MessageValue::Module(
                exports
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.value_to_message(*v)?)))
                    .collect::<Result<HashMap<_, _>, VmError>>()?,
            )),
            HeapObject::NativeFunction(_, _) | HeapObject::Actor(_, _, _) | HeapObject::Supervisor(_, _, _) => {
                Err(VmError::TypeMismatch("SendMessage unsupported reference type"))
            }
        }
    }

    pub fn message_to_value(&mut self, message: MessageValue) -> Result<Value, VmError> {
        match message {
            MessageValue::Integer(v) => Ok(Value::Integer(v)),
            MessageValue::Float(v) => Ok(Value::Float(v)),
            MessageValue::Boolean(v) => Ok(Value::Boolean(v)),
            MessageValue::ExitSignal(v) => Ok(Value::ExitSignal(v)),
            MessageValue::Null => Ok(Value::Null),
            MessageValue::String(v) => {
                let address = self.allocate(HeapObject::String(v, 1));
                Ok(Value::Reference(address))
            }
            MessageValue::Array(values) => {
                let materialized = values
                    .into_iter()
                    .map(|value| self.message_to_value(value))
                    .collect::<Result<Vec<_>, _>>()?;
                let address = self.allocate(HeapObject::Array(materialized, 1));
                Ok(Value::Reference(address))
            }
            MessageValue::Module(exports) => {
                let materialized_exports = exports
                    .into_iter()
                    .map(|(k, v)| Ok((k, self.message_to_value(v)?)))
                    .collect::<Result<HashMap<_, _>, VmError>>()?;
                let address = self.allocate(HeapObject::Module {
                    name: "message_module".to_string(),
                    exports: materialized_exports,
                    ref_count: 1,
                });
                Ok(Value::Reference(address))
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
    use std::sync::{Arc, Mutex};

    #[test]
    fn gc_preserves_objects_reachable_from_live_actor_vm_stack() {
        let mut heap = Heap::new();
        let string_address = heap.allocate(HeapObject::String("actor string".to_string(), 0));
        let array_address =
            heap.allocate(HeapObject::Array(vec![Value::Reference(string_address)], 0));

        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (actor_sender, _actor_mailbox) = tokio::sync::mpsc::channel(1);
        let final_stack = Arc::new(Mutex::new(vec![Value::Reference(array_address)]));
        let task = runtime.spawn(async { Ok(()) });
        let actor = ProcessHandle::new(
            1,
            None,
            0,
            Vec::new(),
            None,
            Vec::new(),
            false,
            task,
            final_stack,
        );
        let actor_address = heap.allocate(HeapObject::Actor(actor, actor_sender, 1));

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
