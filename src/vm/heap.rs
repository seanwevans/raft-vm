// src/vm/heap.rs

use crate::vm::error::VmError;
use crate::vm::value::Value;
use crate::vm::VM;
use std::collections::HashMap;
use tokio::sync::mpsc::Sender;

#[derive(Debug)]
pub struct Heap {
    objects: HashMap<usize, HeapObject>,
    next_address: usize,
}

#[derive(Debug)]
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

impl Heap {
    pub fn new() -> Self {
        Self {
            objects: HashMap::new(),
            next_address: 0,
        }
    }

    pub fn allocate(&mut self, object: HeapObject) -> usize {
        let address = self.next_address;
        self.objects.insert(address, object);
        log::info!("Allocated object at address {}", address);
        self.next_address += 1;
        address
    }

    pub fn get(&self, address: usize) -> Option<&HeapObject> {
        if let Some(obj) = self.objects.get(&address) {
            Some(obj)
        } else {
            log::warn!("Attempted to access invalid heap address: {}", address);
            None
        }
    }

    pub fn get_mut(&mut self, address: usize) -> Option<&mut HeapObject> {
        self.objects.get_mut(&address)
    }

    pub fn collect_garbage(&mut self) {
        let before = self.objects.len();
        self.objects.retain(|_, obj| obj.is_alive());
        let collected = before - self.objects.len();
        if collected > 0 {
            log::info!("Collected {} unreachable heap objects", collected);
        }
    }
}

impl HeapObject {
    pub fn is_alive(&self) -> bool {
        match self {
            HeapObject::Array(_, rc)
            | HeapObject::String(_, rc)
            | HeapObject::NativeFunction(_, rc)
            | HeapObject::Actor(_, _, rc)
            | HeapObject::Supervisor(_, _, rc) => *rc > 0,
            HeapObject::Module { ref_count, .. } => *ref_count > 0,
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
