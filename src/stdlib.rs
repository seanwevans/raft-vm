use std::collections::HashMap;

use crate::vm::error::VmError;
use crate::vm::execution::ExecutionContext;
use crate::vm::heap::{Heap, HeapObject, NativeFunction};
use crate::vm::value::Value;

pub fn install(heap: &mut Heap, execution: &mut ExecutionContext) {
    install_io_module(heap, execution);
}

fn install_io_module(heap: &mut Heap, execution: &mut ExecutionContext) {
    let print_address = heap.allocate(HeapObject::NativeFunction(
        NativeFunction {
            name: "io.print".to_string(),
            arity: 1,
            function: io_print,
        },
        1,
    ));

    let mut exports = HashMap::new();
    exports.insert("print".to_string(), Value::Reference(print_address));

    let module_address = heap.allocate(HeapObject::Module {
        name: "io".to_string(),
        exports,
        ref_count: 1,
    });

    execution
        .globals_mut()
        .insert("io".to_string(), Value::Reference(module_address));
}

fn io_print(args: Vec<Value>) -> Result<Value, VmError> {
    let value = args.first().ok_or(VmError::StackUnderflowFor("io.print"))?;
    println!("{}", display_value(value));
    Ok(Value::Null)
}

fn display_value(value: &Value) -> String {
    match value {
        Value::Integer(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::Boolean(value) => value.to_string(),
        Value::Reference(address) => format!("<ref:{address}>"),
        Value::ExitSignal(signal) => format!("<exit:{}:{:?}>", signal.from, signal.reason),
        Value::Null => "null".to_string(),
    }
}
