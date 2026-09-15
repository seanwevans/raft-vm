use std::cell::RefCell;
use std::collections::HashMap;

use crate::vm::error::VmError;
use crate::vm::execution::ExecutionContext;
use crate::vm::heap::{Heap, HeapObject, NativeFunction};
use crate::vm::value::Value;

/// A destination for the lines `io.print` writes.
pub type PrintSink = Box<dyn FnMut(&str)>;

thread_local! {
    /// Where `io.print` writes on this thread. `None` is stdout.
    static PRINT_SINK: RefCell<Option<PrintSink>> = const { RefCell::new(None) };
}

/// Route `io.print` on this thread to `sink`, returning whatever it replaced.
///
/// A host without a usable stdout -- the wasm playground is one, since a
/// browser has no console to inherit -- installs a sink to collect what a
/// program prints. The sink belongs to the calling thread, so a VM running on
/// another thread keeps printing to stdout.
pub fn set_print_sink(sink: PrintSink) -> Option<PrintSink> {
    PRINT_SINK.with(|cell| cell.borrow_mut().replace(sink))
}

/// Remove this thread's sink and hand it back. `io.print` returns to stdout.
pub fn take_print_sink() -> Option<PrintSink> {
    PRINT_SINK.with(|cell| cell.borrow_mut().take())
}

/// Write one line to wherever this thread's `io.print` output goes.
fn emit(line: &str) {
    // The sink is taken for the duration of the call: a sink that prints while
    // printing would otherwise re-enter the `RefCell` it is borrowed from and
    // panic. Nested printing falls through to stdout instead.
    let Some(mut sink) = take_print_sink() else {
        println!("{line}");
        return;
    };

    sink(line);

    PRINT_SINK.with(|cell| {
        let mut slot = cell.borrow_mut();
        // Only restore if nothing was installed while the sink ran; the newer
        // sink is the one the host wants.
        if slot.is_none() {
            *slot = Some(sink);
        }
    });
}

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
    emit(&display_value(value));
    Ok(Value::Null)
}

/// Render a value the way `io.print` writes it.
pub fn display_value(value: &Value) -> String {
    match value {
        Value::Integer(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::Boolean(value) => value.to_string(),
        Value::Reference(address) => format!("<ref:{address}>"),
        Value::ExitSignal(signal) => format!("<exit:{}:{:?}>", signal.from, signal.reason),
        Value::Null => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    /// Collect printed lines into a buffer the test can read afterwards.
    fn capture() -> Rc<RefCell<Vec<String>>> {
        let lines = Rc::new(RefCell::new(Vec::new()));
        let sink_lines = Rc::clone(&lines);
        set_print_sink(Box::new(move |line| {
            sink_lines.borrow_mut().push(line.to_string());
        }));
        lines
    }

    #[test]
    fn printing_goes_to_the_installed_sink() {
        let lines = capture();

        io_print(vec![Value::Integer(42)]).expect("print should succeed");
        io_print(vec![Value::Boolean(true)]).expect("print should succeed");

        assert_eq!(*lines.borrow(), vec!["42".to_string(), "true".to_string()]);
        take_print_sink();
    }

    #[test]
    fn taking_the_sink_returns_printing_to_stdout() {
        let lines = capture();
        take_print_sink();

        io_print(vec![Value::Integer(1)]).expect("print should succeed");

        assert!(
            lines.borrow().is_empty(),
            "a removed sink should stop receiving output"
        );
    }

    #[test]
    fn a_sink_that_prints_does_not_re_enter_itself() {
        let lines = Rc::new(RefCell::new(Vec::new()));
        let sink_lines = Rc::clone(&lines);
        set_print_sink(Box::new(move |line| {
            sink_lines.borrow_mut().push(line.to_string());
            // Nested print: it must not deadlock or panic on the borrow.
            if line != "nested" {
                emit("nested");
            }
        }));

        io_print(vec![Value::Integer(7)]).expect("print should succeed");

        assert_eq!(*lines.borrow(), vec!["7".to_string()]);
        take_print_sink();
    }

    #[test]
    fn printing_without_arguments_is_an_error() {
        assert!(matches!(
            io_print(Vec::new()),
            Err(VmError::StackUnderflowFor("io.print"))
        ));
    }

    #[test]
    fn values_render_by_kind() {
        assert_eq!(display_value(&Value::Float(1.5)), "1.5");
        assert_eq!(display_value(&Value::Null), "null");
        assert_eq!(display_value(&Value::Reference(3)), "<ref:3>");
    }
}
