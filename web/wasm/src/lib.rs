//! The Raft VM behind a C ABI, for hosting in a browser.
//!
//! The playground loads this module with plain `WebAssembly.instantiate`: no
//! bindgen, no generated glue, nothing to keep in version lockstep. The host
//! hands over a program as UTF-8 bytes and gets back a JSON report of what
//! running it did -- printed output, the final stack, the compiled bytecode,
//! and any error, with the source location that produced it.
//!
//! Every entry point returns the length of the report and leaves it in a
//! buffer the host reads with [`raft_result_ptr`]. Reading it means two calls
//! rather than one, but it keeps the ABI to 32-bit values that JavaScript can
//! hold exactly.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::rc::Rc;

use raft::compiler::{CompiledProgram, Compiler, CompilerError};
use raft::stdlib;
use raft::vm::value::MessageValue;
use raft::vm::{Value, VmError, VM};

/// Lines of program output the report will carry. A program that prints from
/// inside a loop can produce far more than a page can show, and the report has
/// to fit in memory before the host ever sees it.
const MAX_OUTPUT_LINES: usize = 2_000;

/// Stack entries rendered into the report, counted from the top of the stack.
const MAX_STACK_ENTRIES: usize = 256;

/// How deep a value is rendered before it is summarised.
const MAX_RENDER_DEPTH: usize = 8;

/// Scheduler ticks given to processes the program spawned once it has halted.
///
/// Enough for a child holding a message its parent already sent to run and
/// print; far too few to wait out one that is blocked forever.
const DRAIN_TICKS: usize = 32;

/// Bytes set aside for the message a panicking instance leaves behind.
const PANIC_LOG_CAPACITY: usize = 512;

/// Where a panicking instance leaves its message.
///
/// wasm cannot unwind and a browser has no stderr, so a panic arrives at the
/// host as a bare trap -- and a trapped instance cannot be called again to ask
/// what happened. The message goes in a buffer at a fixed address instead: the
/// host takes the address with [`raft_panic_log`] before running anything, and
/// reads the message out of the module's memory afterwards, which stays
/// readable once the instance is dead.
///
/// The first four bytes are the message length, little-endian; the rest is
/// UTF-8. Declaring an imported function to call instead would be the obvious
/// way to do this, and it does not link: an undefined symbol is an error for
/// this target, not an import.
static mut PANIC_LOG: [u8; PANIC_LOG_CAPACITY] = [0; PANIC_LOG_CAPACITY];

thread_local! {
    /// The report the host is about to read. Held until the next call replaces
    /// it, since the host reads it after the call that produced it returns.
    static RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Reserve `len` bytes for the host to write a program into.
///
/// # Safety
///
/// The returned pointer is owned by the caller until it is handed back to
/// [`raft_eval`] or [`raft_dealloc`]; freeing it any other way leaks it.
#[no_mangle]
pub extern "C" fn raft_alloc(len: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(len);
    let pointer = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    pointer
}

/// Release a buffer obtained from [`raft_alloc`].
///
/// # Safety
///
/// `ptr` must come from [`raft_alloc`] with the same `len`, and must not have
/// been released already.
#[no_mangle]
pub unsafe extern "C" fn raft_dealloc(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    drop(Vec::from_raw_parts(ptr, len, len));
}

/// Compile and run a program, returning the length of the JSON report.
///
/// `budget` bounds how many instructions the program may execute; `0` runs it
/// unbounded, which only a host that can kill the instance should ask for.
/// The source buffer is consumed: the caller must not free it again.
///
/// # Safety
///
/// `ptr` must point at `len` bytes obtained from [`raft_alloc`].
#[no_mangle]
pub unsafe extern "C" fn raft_eval(ptr: *mut u8, len: usize, budget: u32) -> usize {
    install_panic_hook();

    let source = if ptr.is_null() || len == 0 {
        String::new()
    } else {
        String::from_utf8_lossy(&Vec::from_raw_parts(ptr, len, len)).into_owned()
    };

    store(evaluate(&source, budget))
}

/// Compile a program without running it, returning the length of the report.
///
/// # Safety
///
/// As [`raft_eval`].
#[no_mangle]
pub unsafe extern "C" fn raft_compile(ptr: *mut u8, len: usize) -> usize {
    install_panic_hook();

    let source = if ptr.is_null() || len == 0 {
        String::new()
    } else {
        String::from_utf8_lossy(&Vec::from_raw_parts(ptr, len, len)).into_owned()
    };

    let report = match Compiler::compile_with_debug(&source) {
        Ok(program) => {
            let mut json = String::from("{\"ok\":true,\"stage\":\"compile\",");
            write_bytecode(&mut json, &program);
            json.push('}');
            json
        }
        Err(error) => compile_error_report(&error),
    };

    store(report)
}

/// Describe the build: the VM's version and the limits it was given.
#[no_mangle]
pub extern "C" fn raft_info() -> usize {
    let mut json = String::from("{\"version\":");
    write_json_string(&mut json, raft::VERSION);
    let _ = write!(json, ",\"maxOutputLines\":{MAX_OUTPUT_LINES}");
    let _ = write!(json, ",\"maxStackEntries\":{MAX_STACK_ENTRIES}");
    json.push('}');
    store(json)
}

/// The address of the panic buffer, which the host reads after a trap.
///
/// The buffer starts zeroed, so a length of zero means the instance has not
/// panicked.
#[no_mangle]
pub extern "C" fn raft_panic_log() -> *const u8 {
    (&raw const PANIC_LOG) as *const u8
}

/// The start of the report produced by the last call.
///
/// Valid until the next call into this module. The host must re-read the
/// module's memory after every call, since running a program can grow it and
/// leave an earlier view detached.
#[no_mangle]
pub extern "C" fn raft_result_ptr() -> *const u8 {
    RESULT.with(|result| result.borrow().as_ptr())
}

fn store(json: String) -> usize {
    let bytes = json.into_bytes();
    let len = bytes.len();
    RESULT.with(|result| *result.borrow_mut() = bytes);
    len
}

fn install_panic_hook() {
    thread_local! {
        static INSTALLED: RefCell<bool> = const { RefCell::new(false) };
    }

    INSTALLED.with(|installed| {
        if std::mem::replace(&mut *installed.borrow_mut(), true) {
            return;
        }

        std::panic::set_hook(Box::new(|info| write_panic_log(&info.to_string())));
    });
}

/// Record a panic message for the host, truncated to what the buffer holds.
fn write_panic_log(message: &str) {
    let capacity = PANIC_LOG_CAPACITY - 4;
    let mut end = message.len().min(capacity);
    while end > 0 && !message.is_char_boundary(end) {
        end -= 1;
    }

    // Safety: a wasm instance runs on one thread, and the host only reads the
    // buffer once that instance has stopped. The length is written last, so a
    // host reading a partly written buffer sees the previous length rather than
    // a length that runs past the bytes behind it.
    unsafe {
        let base = (&raw mut PANIC_LOG) as *mut u8;
        std::ptr::copy_nonoverlapping(message.as_ptr(), base.add(4), end);
        std::ptr::copy_nonoverlapping((end as u32).to_le_bytes().as_ptr(), base, 4);
    }
}

/// Compile and run `source`, reporting everything the page shows about it.
fn evaluate(source: &str, budget: u32) -> String {
    let program = match Compiler::compile_with_debug(source) {
        Ok(program) => program,
        Err(error) => return compile_error_report(&error),
    };

    let (mut vm, _mailbox) = match VM::new_with_debug(
        program.bytecode.clone(),
        Some(program.debug_info.clone()),
        None,
    ) {
        Ok(vm) => vm,
        Err(error) => return runtime_error_report(&error, &program, &Output::default(), &[]),
    };

    if budget > 0 {
        vm.set_instruction_budget(Some(u64::from(budget)));
    }

    let output = Output::default();
    let sink = output.clone();
    stdlib::set_print_sink(Box::new(move |line| sink.push(line)));

    let result = run_to_completion(&mut vm);

    stdlib::take_print_sink();

    let stack = render_stack(&vm);

    match result {
        Ok(()) => {
            let mut json = String::from("{\"ok\":true,\"stage\":\"run\",");
            write_run_details(&mut json, &vm, &program, &output, &stack);
            json.push('}');
            json
        }
        Err(error) => {
            let mut json = String::from("{\"ok\":false,\"stage\":\"run\",");
            write_error(&mut json, &error);
            json.push(',');
            write_run_details(&mut json, &vm, &program, &output, &stack);
            json.push('}');
            json
        }
    }
}

/// Drive the VM to a halt on a single-threaded scheduler.
///
/// Actors are Tokio tasks, so running a program that spawns them needs a
/// runtime even though a browser gives us exactly one thread to run it on.
fn run_to_completion(vm: &mut VM) -> Result<(), VmError> {
    let runtime = match tokio::runtime::Builder::new_current_thread().build() {
        Ok(runtime) => runtime,
        Err(error) => return Err(VmError::Message(format!("no runtime: {error}"))),
    };

    let result = runtime.block_on(vm.run());
    drain_spawned_processes(&runtime);
    result
}

/// Let the processes a program spawned finish what they were already doing.
///
/// A halting program does not stop its actors: they are tasks on this runtime,
/// and dropping it cancels them wherever they happen to be. Each tick here
/// hands the scheduler the thread, so a child holding a message its parent
/// already sent gets to run and print. Ticking a bounded number of times is
/// what keeps a child that is blocked forever from taking the page with it --
/// and the yield always wakes itself, so the scheduler never tries to park a
/// thread the browser does not have.
fn drain_spawned_processes(runtime: &tokio::runtime::Runtime) {
    runtime.block_on(async {
        for _ in 0..DRAIN_TICKS {
            tokio::task::yield_now().await;
        }
    });
}

/// Program output, shared between the print sink and the report.
#[derive(Clone, Default)]
struct Output {
    lines: Rc<RefCell<Vec<String>>>,
    written: Rc<RefCell<usize>>,
}

impl Output {
    fn push(&self, line: &str) {
        *self.written.borrow_mut() += 1;
        let mut lines = self.lines.borrow_mut();
        if lines.len() < MAX_OUTPUT_LINES {
            lines.push(line.to_string());
        }
    }

    /// Lines beyond the cap, which the report names rather than carries.
    fn dropped(&self) -> usize {
        self.written
            .borrow()
            .saturating_sub(self.lines.borrow().len())
    }
}

fn write_run_details(
    json: &mut String,
    vm: &VM,
    program: &CompiledProgram,
    output: &Output,
    stack: &[(&'static str, String)],
) {
    json.push_str("\"output\":[");
    for (index, line) in output.lines.borrow().iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        write_json_string(json, line);
    }
    json.push(']');

    let _ = write!(json, ",\"outputDropped\":{}", output.dropped());

    json.push_str(",\"stack\":[");
    for (index, (kind, text)) in stack.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        json.push_str("{\"kind\":");
        write_json_string(json, kind);
        json.push_str(",\"text\":");
        write_json_string(json, text);
        json.push('}');
    }
    json.push(']');

    let _ = write!(
        json,
        ",\"stackDepth\":{},\"instructions\":{},\"heap\":{{\"live\":{},\"slots\":{}}},",
        vm.stack().len(),
        vm.instructions_executed(),
        vm.heap_live_object_count(),
        vm.heap_slot_count()
    );

    write_bytecode(json, program);
}

fn write_bytecode(json: &mut String, program: &CompiledProgram) {
    json.push_str("\"bytecode\":[");
    for (index, opcode) in program.bytecode.iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        let _ = write!(json, "{{\"index\":{index},\"text\":");
        write_json_string(json, &format!("{opcode:?}"));
        if let Some(location) = program.debug_info.location_for_instruction(index) {
            let _ = write!(json, ",\"line\":{}", location.line);
        }
        json.push('}');
    }
    json.push(']');
}

fn compile_error_report(error: &CompilerError) -> String {
    let mut json = String::from("{\"ok\":false,\"stage\":\"compile\",\"error\":");
    write_json_string(&mut json, &error.to_string());

    if let CompilerError::ParseErrorAt {
        message,
        line,
        column,
    } = error
    {
        json.push_str(",\"detail\":");
        write_json_string(&mut json, message);
        let _ = write!(json, ",\"line\":{line},\"column\":{column}");
    }

    json.push('}');
    json
}

fn runtime_error_report(
    error: &VmError,
    program: &CompiledProgram,
    output: &Output,
    stack: &[(&'static str, String)],
) -> String {
    let mut json = String::from("{\"ok\":false,\"stage\":\"run\",");
    write_error(&mut json, error);
    json.push(',');
    json.push_str("\"output\":[");
    for (index, line) in output.lines.borrow().iter().enumerate() {
        if index > 0 {
            json.push(',');
        }
        write_json_string(&mut json, line);
    }
    json.push_str("],\"outputDropped\":0,\"stack\":[],\"stackDepth\":0,\"instructions\":0,");
    json.push_str("\"heap\":{\"live\":0,\"slots\":0},");
    write_bytecode(&mut json, program);
    json.push('}');
    let _ = stack;
    json
}

/// Write the `"error"` field, unwrapping the location the VM attached to it.
fn write_error(json: &mut String, error: &VmError) {
    json.push_str("\"error\":");
    match error {
        VmError::RuntimeError { location, source } => {
            write_json_string(json, &source.to_string());
            let _ = write!(
                json,
                ",\"line\":{},\"column\":{}",
                location.line, location.column
            );
        }
        other => write_json_string(json, &other.to_string()),
    }
}

/// Render the stack top-first, the way the page lists it.
fn render_stack(vm: &VM) -> Vec<(&'static str, String)> {
    vm.stack()
        .iter()
        .rev()
        .take(MAX_STACK_ENTRIES)
        .map(|value| render_value(vm, value))
        .collect()
}

fn render_value(vm: &VM, value: &Value) -> (&'static str, String) {
    match value {
        Value::Integer(value) => ("integer", value.to_string()),
        Value::Float(value) => ("float", value.to_string()),
        Value::Boolean(value) => ("boolean", value.to_string()),
        Value::Null => ("null", "null".to_string()),
        Value::ExitSignal(signal) => (
            "exit",
            format!("exit from process {} ({:?})", signal.from, signal.reason),
        ),
        // A reference is rendered through the message conversion, which already
        // refuses cycles and over-deep values rather than chasing them.
        Value::Reference(address) => match vm.value_to_message(value.clone()) {
            Ok(message) => ("reference", render_message(&message, 0)),
            Err(_) => ("reference", format!("<ref:{address}>")),
        },
    }
}

fn render_message(message: &MessageValue, depth: usize) -> String {
    if depth >= MAX_RENDER_DEPTH {
        return "...".to_string();
    }

    match message {
        MessageValue::Integer(value) => value.to_string(),
        MessageValue::Float(value) => value.to_string(),
        MessageValue::Boolean(value) => value.to_string(),
        MessageValue::Null => "null".to_string(),
        MessageValue::String(value) => format!("{value:?}"),
        MessageValue::ExitSignal(signal) => {
            format!("exit from process {} ({:?})", signal.from, signal.reason)
        }
        MessageValue::Array(items) => {
            let rendered: Vec<String> = items
                .iter()
                .map(|item| render_message(item, depth + 1))
                .collect();
            format!("[{}]", rendered.join(", "))
        }
        MessageValue::Module(exports) => {
            let mut names: Vec<&String> = exports.keys().collect();
            names.sort();
            let rendered: Vec<String> = names
                .iter()
                .map(|name| {
                    let value = render_message(&exports[*name], depth + 1);
                    format!("{name}: {value}")
                })
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

fn write_json_string(json: &mut String, value: &str) {
    json.push('"');
    for character in value.chars() {
        match character {
            '"' => json.push_str("\\\""),
            '\\' => json.push_str("\\\\"),
            '\n' => json.push_str("\\n"),
            '\r' => json.push_str("\\r"),
            '\t' => json.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                let _ = write!(json, "\\u{:04x}", character as u32);
            }
            character => json.push(character),
        }
    }
    json.push('"');
}
