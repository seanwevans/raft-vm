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

/// Deepest chain of heap references `value_to_message` will follow before it
/// gives up. Bounded so a pathological value cannot exhaust the host stack.
pub const MAX_MESSAGE_DEPTH: usize = 512;

#[derive(Debug)]
pub struct Heap {
    objects: Vec<Option<HeapObject>>,
    free_list: Vec<usize>,
    allocations_since_collection: usize,
}

#[derive(Debug, Clone)]
pub struct NativeFunction {
    pub name: String,
    pub arity: usize,
    pub function: fn(Vec<Value>) -> Result<Value, VmError>,
}

/// A handle to a process spawned onto the Tokio runtime.
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

    /// Place `child` under this supervisor, or update it if already tracked.
    ///
    /// Registration is what makes the `OneForAll` and `RestForOne` strategies
    /// meaningful: they act on the supervisor's full child list, so a child
    /// that was never registered can never be restarted alongside its siblings.
    pub fn supervise_child(&mut self, child: ChildSpec) {
        self.supervisor_state.ensure_child(child);
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
            allocations_since_collection: 0,
        }
    }

    pub fn allocate(&mut self, object: HeapObject) -> usize {
        self.allocations_since_collection += 1;
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

    /// Number of allocations made since the last collection. Drives the VM's
    /// decision about when to collect.
    pub fn allocations_since_collection(&self) -> usize {
        self.allocations_since_collection
    }

    /// Number of heap slots currently holding an object.
    pub fn live_object_count(&self) -> usize {
        self.objects.iter().filter(|slot| slot.is_some()).count()
    }

    /// Total number of heap slots, including free slots awaiting reuse.
    pub fn slot_count(&self) -> usize {
        self.objects.len()
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
        self.allocations_since_collection = 0;
        let object_count = self.objects.len();
        let mut internal_incoming = vec![0usize; object_count];

        for address in 0..object_count {
            let Some(object) = self.objects[address].as_ref() else {
                continue;
            };
            // Only count edges from objects that are themselves still
            // referenced. A dead object awaiting collection still holds its
            // children in place, and counting those edges would cancel out the
            // child's own count -- making a value that a stack slot still holds
            // look unrooted and collecting it out from under the program.
            if !object.is_alive() {
                continue;
            }
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

    /// Drop one reference to `address`, cascading into children whose last
    /// reference this was.
    ///
    /// Traversal is iterative: nesting depth is chosen by the running program,
    /// so recursing here would let a deeply nested value overflow the host
    /// stack. Cycles terminate on their own because an object's children are
    /// only followed while its count is still 1, and the count reaches 0 before
    /// the cycle closes.
    pub fn release_reference(&mut self, address: usize) -> Result<(), VmError> {
        let mut pending = vec![address];

        while let Some(address) = pending.pop() {
            match self.get_mut(address) {
                Some(object) => {
                    if object.ref_count() == 1 {
                        let children = object.references();
                        object.decrement_ref();
                        pending.extend(children);
                    } else {
                        object.decrement_ref();
                    }
                }
                None => return Err(VmError::InvalidReference),
            }
        }

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
        let mut visiting = Vec::new();
        self.value_to_message_at(value, &mut visiting)
    }

    fn value_to_message_at(
        &self,
        value: Value,
        visiting: &mut Vec<usize>,
    ) -> Result<MessageValue, VmError> {
        match value {
            Value::Integer(v) => Ok(MessageValue::Integer(v)),
            Value::Float(v) => Ok(MessageValue::Float(v)),
            Value::Boolean(v) => Ok(MessageValue::Boolean(v)),
            Value::ExitSignal(v) => Ok(MessageValue::ExitSignal(v)),
            Value::Null => Ok(MessageValue::Null),
            Value::Reference(address) => self.reference_to_message(address, visiting),
        }
    }

    /// Convert a heap reference into a message, refusing values that cannot be
    /// represented as the tree-shaped `MessageValue`.
    ///
    /// `visiting` holds the addresses on the current traversal path. A repeat
    /// visit means the value is cyclic, and an over-long path means the value
    /// nests deeper than the host stack can safely handle. Both are reported as
    /// ordinary `VmError`s so a misbehaving program fails its own process
    /// instead of overflowing the stack and aborting the whole runtime.
    fn reference_to_message(
        &self,
        address: usize,
        visiting: &mut Vec<usize>,
    ) -> Result<MessageValue, VmError> {
        if visiting.contains(&address) {
            return Err(VmError::CyclicReference(address));
        }
        if visiting.len() >= MAX_MESSAGE_DEPTH {
            return Err(VmError::MessageTooDeep(MAX_MESSAGE_DEPTH));
        }

        visiting.push(address);
        let message = self.reference_to_message_unchecked(address, visiting);
        visiting.pop();
        message
    }

    fn reference_to_message_unchecked(
        &self,
        address: usize,
        visiting: &mut Vec<usize>,
    ) -> Result<MessageValue, VmError> {
        match self.get(address).ok_or(VmError::InvalidReference)? {
            HeapObject::Array(values, _) => Ok(MessageValue::Array(
                values
                    .iter()
                    .map(|value| self.value_to_message_at(value.clone(), visiting))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            HeapObject::String(value, _) => Ok(MessageValue::String(value.clone())),
            HeapObject::Module { exports, .. } => Ok(MessageValue::Module(
                exports
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.value_to_message_at(v.clone(), visiting)?)))
                    .collect::<Result<HashMap<_, _>, VmError>>()?,
            )),
            HeapObject::NativeFunction(_, _)
            | HeapObject::Actor(_, _, _)
            | HeapObject::Supervisor(_, _, _) => Err(VmError::TypeMismatch(
                "SendMessage unsupported reference type",
            )),
        }
    }

    /// Materialize a received message onto this heap.
    ///
    /// Every allocated object comes back with a count of 1, which covers the
    /// reference its parent holds. The outermost object's count covers the
    /// caller, so callers push the result without retaining it again.
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
            // Actor/Supervisor heaps are isolated from the parent heap. Any references in the
            // child's final stack are addresses in the child heap and are not valid indices for
            // the parent heap during parent GC traversal.
            HeapObject::Actor(_, _, _) | HeapObject::Supervisor(_, _, _) => Vec::new(),
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
    fn gc_does_not_treat_actor_heap_addresses_as_parent_heap_references() {
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
            heap.get(array_address).is_none(),
            "child heap addresses in actor final stack must not pin parent heap objects"
        );
        assert!(
            heap.get(string_address).is_none(),
            "parent transitive references from a child heap address must be reclaimable"
        );
    }
}
