// src/vm/heap.rs

use crate::vm::error::VmError;
use crate::vm::value::Value;
use crate::vm::VM;
use std::collections::HashMap;
use tokio::sync::mpsc::Sender;

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
pub enum HeapObject {
    Array(Vec<Value>, usize),
    String(String, usize),
    Module {
        name: String,
        exports: HashMap<String, Value>,
        ref_count: usize,
    },
    NativeFunction(NativeFunction, usize),
    Actor(VM, Sender<Value>, usize),
    Supervisor(VM, Sender<Value>, usize),
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
            HeapObject::String(_, _)
            | HeapObject::NativeFunction(_, _)
            | HeapObject::Actor(_, _, _)
            | HeapObject::Supervisor(_, _, _) => Vec::new(),
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
