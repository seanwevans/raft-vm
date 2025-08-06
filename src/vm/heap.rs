// src/vm/heap.rs

use crate::vm::error::VmError;
use crate::vm::value::Value;
use crate::vm::VM;
use std::collections::HashMap;

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
    Array(Vec<Value>),
    String(String),
    Module {
        name: String,
        exports: HashMap<String, Value>,
    },
    NativeFunction(NativeFunction),
    Actor(VM),
    Supervisor(VM),
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

    pub fn is_alive(&mut self) -> bool {
        true // Placeholder logic for now
    }

    pub fn get_mut(&mut self, address: usize) -> Option<&mut HeapObject> {
        self.objects.get_mut(&address)
    }

    pub fn collect_garbage(&mut self) {
        self.objects.retain(|_, obj| obj.is_alive());
    }
}

impl HeapObject {
    pub fn is_alive(&self) -> bool {
        true // Placeholder logic for now
    }
}
