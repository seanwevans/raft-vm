//! Accessors, bytecode decoding, debug info, and standard-library formatting.

use raft::compiler::{Compiler, DebugInfo};
use raft::vm::execution::{ExecutionContext, ExecutionState};
use raft::vm::heap::{Heap, HeapObject};
use raft::vm::opcodes::{Bytecode, OpCode};
use raft::vm::supervision::{ExitReason, ExitSignal};
use raft::vm::value::{MessageValue, Value};
use raft::vm::{VmError, VM};

// --- bytecode --------------------------------------------------------------

#[test]
fn bytecode_reports_its_size() {
    let bytecode = Bytecode::new(vec![OpCode::Return, OpCode::Pop]);
    assert_eq!(bytecode.len(), 2);
    assert!(!bytecode.is_empty());
    assert!(Bytecode::new(Vec::new()).is_empty());
}

#[test]
fn bytecode_decodes_in_range_and_refuses_past_the_end() {
    let bytecode: Bytecode = vec![OpCode::Pop, OpCode::Return].into();
    assert!(matches!(
        *bytecode.decode(0).expect("decode 0"),
        OpCode::Pop
    ));
    assert!(matches!(
        *bytecode.decode(1).expect("decode 1"),
        OpCode::Return
    ));
    assert!(matches!(
        bytecode.decode(2),
        Err(VmError::ExecutionOutOfBounds)
    ));
}

#[test]
fn bytecode_round_trips_back_to_opcodes() {
    let original = vec![OpCode::PushConst(Value::Integer(1)), OpCode::Neg];
    let recovered = Bytecode::new(original.clone()).opcodes();
    assert_eq!(recovered.len(), original.len());
    assert!(matches!(recovered[0], OpCode::PushConst(Value::Integer(1))));
    assert!(matches!(recovered[1], OpCode::Neg));
}

// --- debug info ------------------------------------------------------------

#[test]
fn debug_info_returns_nothing_for_unknown_instructions() {
    let info = DebugInfo::default();
    assert!(info.span_for_instruction(0).is_none());
    assert!(info.location_for_instruction(0).is_none());
    assert!(info.instruction_spans().is_empty());
}

#[test]
fn debug_info_exposes_the_full_span_of_an_instruction() {
    let program = Compiler::compile_with_debug("1 2 +").expect("should compile");
    let span = program
        .debug_info
        .span_for_instruction(2)
        .expect("the third opcode should have a span");
    assert_eq!(span.start.line, 1);
    assert!(span.end.column >= span.start.column);
}

// --- execution context -----------------------------------------------------

#[test]
fn an_execution_context_halts_at_the_end_of_its_bytecode() {
    let mut execution = ExecutionContext::new(vec![OpCode::Pop]);
    let mut heap = Heap::new();
    execution.set_ip(1);
    assert!(matches!(
        execution.step(&mut heap),
        Ok(ExecutionState::Halted)
    ));
}

#[test]
fn an_execution_context_refuses_to_step_past_its_bytecode() {
    let mut execution = ExecutionContext::new(vec![OpCode::Pop]);
    let mut heap = Heap::new();
    execution.set_ip(9);
    assert!(matches!(
        execution.step(&mut heap),
        Err(VmError::ExecutionOutOfBounds)
    ));
}

#[test]
fn an_execution_context_exposes_its_pointer_locals_and_globals() {
    let mut execution = ExecutionContext::new_with_debug(vec![OpCode::Return], None);
    assert_eq!(execution.ip(), 0);
    execution.set_ip(1);
    assert_eq!(execution.ip(), 1);

    assert!(execution.locals().is_empty());
    execution.locals_mut().insert(0, Value::Integer(5));
    assert_eq!(execution.locals().get(&0), Some(&Value::Integer(5)));

    assert!(execution.globals().is_empty());
    execution
        .globals_mut()
        .insert("answer".to_string(), Value::Integer(42));
    assert_eq!(execution.globals().get("answer"), Some(&Value::Integer(42)));
}

#[tokio::test]
async fn a_context_built_with_a_mailbox_can_receive() {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let mut execution =
        ExecutionContext::with_mailbox_and_debug(vec![OpCode::ReceiveMessage], rx, None);
    tx.send(MessageValue::Integer(3))
        .await
        .expect("mailbox should accept");

    assert!(execution.mailbox_mut().try_recv().is_ok());
}

// --- heap accessors --------------------------------------------------------

#[test]
fn an_empty_heap_has_nothing_at_any_address() {
    let mut heap = Heap::default();
    assert!(heap.get(0).is_none());
    assert!(heap.get_mut(0).is_none());
}

#[test]
fn a_module_message_materializes_onto_the_heap() {
    let mut heap = Heap::new();
    let mut exports = std::collections::HashMap::new();
    exports.insert("answer".to_string(), MessageValue::Integer(42));

    let value = heap
        .message_to_value(MessageValue::Module(exports))
        .expect("a module message should materialize");
    match value {
        Value::Reference(address) => match heap.get(address) {
            Some(HeapObject::Module { name, exports, .. }) => {
                assert_eq!(name, "message_module");
                assert_eq!(exports.get("answer"), Some(&Value::Integer(42)));
            }
            other => panic!("expected a module, got {other:?}"),
        },
        other => panic!("expected a reference, got {other:?}"),
    }
}

#[test]
fn scalar_messages_convert_in_both_directions() {
    let mut heap = Heap::new();
    let signal = ExitSignal {
        from: 3,
        reason: ExitReason::Normal,
    };
    for (value, message) in [
        (Value::Integer(1), MessageValue::Integer(1)),
        (Value::Float(2.0), MessageValue::Float(2.0)),
        (Value::Boolean(true), MessageValue::Boolean(true)),
        (Value::Null, MessageValue::Null),
        (Value::ExitSignal(signal), MessageValue::ExitSignal(signal)),
    ] {
        assert_eq!(
            heap.value_to_message(value.clone()).expect("to message"),
            message
        );
        assert_eq!(
            heap.message_to_value(message).expect("to value"),
            value,
            "round trip should be lossless"
        );
    }
}

#[test]
fn converting_a_dangling_reference_is_rejected() {
    let heap = Heap::new();
    assert!(matches!(
        heap.value_to_message(Value::Reference(7)),
        Err(VmError::InvalidReference)
    ));
}

// --- vm accessors ----------------------------------------------------------

#[tokio::test]
async fn a_vm_with_no_bytecode_refuses_to_run() {
    let (mut vm, _tx) = VM::new(Vec::new(), None);
    assert!(matches!(vm.run().await, Err(VmError::NoBytecode)));
}

#[tokio::test]
async fn popping_an_empty_vm_stack_underflows() {
    let (mut vm, _tx) = VM::new(vec![OpCode::Return], None);
    assert!(matches!(vm.pop_stack(), Err(VmError::StackUnderflow)));
}

#[tokio::test]
async fn a_vm_exposes_its_globals_and_bytecode() {
    let code = vec![OpCode::Return, OpCode::Pop];
    let (vm, _tx) = VM::new(code.clone(), None);

    assert!(
        matches!(vm.global("io"), Some(Value::Reference(_))),
        "the standard library should be installed"
    );
    assert!(vm.global("missing").is_none());
    assert_eq!(vm.bytecode().len(), code.len());
    assert!(vm.debug_info().is_none());
    assert!(vm.links().is_empty());
    assert!(vm.stack().is_empty());
}

#[tokio::test]
async fn a_vm_tracks_its_pointers_parent_and_exit_trapping() {
    let (mut vm, _tx) = VM::new(vec![OpCode::Return], None);

    assert_eq!(vm.current_ip(), 0);
    vm.set_ip(1);
    assert_eq!(vm.current_ip(), 1);

    assert_eq!(vm.restart_ip(), 0);
    vm.set_restart_ip(4);
    assert_eq!(vm.restart_ip(), 4);

    assert!(vm.parent().is_none());
    vm.set_parent(11);
    assert_eq!(vm.parent(), Some(11));

    assert!(!vm.trap_exits());
    vm.set_trap_exits(true);
    assert!(vm.trap_exits());
}

#[tokio::test]
async fn a_vm_reports_reference_counts_only_for_live_addresses() {
    let (mut vm, _tx) = VM::new(vec![OpCode::MakeString("live".to_string())], None);
    vm.run().await.expect("program should run");

    let address = match vm.stack().last() {
        Some(Value::Reference(address)) => *address,
        other => panic!("expected a string reference, got {other:?}"),
    };
    assert_eq!(vm.heap_ref_count(address), Some(1));
    assert_eq!(vm.heap_ref_count(9_999), None);
}

#[tokio::test]
async fn a_vm_reports_the_heap_addresses_its_roots_hold() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::MakeString("on the stack".to_string()),
            OpCode::MakeString("in a local".to_string()),
            OpCode::StoreVar(0),
        ],
        None,
    );
    vm.run().await.expect("program should run");

    let references = vm.heap_references();
    // The stack value, the local, and the io module installed in globals.
    assert_eq!(references.len(), 3, "got {references:?}");
}

#[tokio::test]
async fn a_vm_converts_values_for_its_own_mailbox() {
    let (vm, tx) = VM::new(vec![OpCode::Return], None);
    let message = vm
        .value_to_message(Value::Integer(5))
        .expect("a scalar should convert");
    assert_eq!(message, MessageValue::Integer(5));

    vm.sender()
        .send(message.clone())
        .await
        .expect("the vm's own sender should deliver");
    tx.send(message)
        .await
        .expect("the returned sender should deliver");
}

#[tokio::test]
async fn receiving_blocks_until_a_message_arrives() {
    let (mut vm, tx) = VM::new(vec![OpCode::ReceiveMessage], None);

    // The mailbox is empty, so ReceiveMessage yields and the VM awaits.
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        tx.send(MessageValue::Integer(5))
            .await
            .expect("the mailbox should still be open");
    });

    vm.run().await.expect("the blocked receive should complete");
    assert_eq!(vm.stack(), &vec![Value::Integer(5)]);
}

#[tokio::test]
async fn a_failing_process_tolerates_links_that_have_gone_away() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::PushConst(Value::Integer(1)),
            OpCode::PushConst(Value::Integer(0)),
            OpCode::Div,
        ],
        None,
    );

    let (live_sender, mut live_mailbox) = tokio::sync::mpsc::channel(1);
    let (dead_sender, dead_mailbox) = tokio::sync::mpsc::channel(1);
    drop(dead_mailbox);
    vm.link(dead_sender);
    vm.link(live_sender);
    assert_eq!(vm.links().len(), 2);

    vm.run().await.expect_err("the program should fail");

    // The undeliverable signal must not stop the live link from being notified.
    assert!(matches!(
        live_mailbox.try_recv(),
        Ok(MessageValue::ExitSignal(_))
    ));
}

#[tokio::test]
async fn reference_counts_are_reported_for_every_kind_of_heap_object() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::SpawnSupervisor(4),
            OpCode::SpawnActor(4),
            OpCode::MakeString("string".to_string()),
            OpCode::MakeArray(1),
            OpCode::Return,
        ],
        None,
    );
    vm.run().await.expect("program should run");

    // The standard library installs a module and a native function first.
    assert_eq!(vm.heap_ref_count(0), Some(1), "the io module");
    assert_eq!(vm.heap_ref_count(1), Some(1), "the io.print function");

    let counts: Vec<Option<usize>> = (2..5).map(|address| vm.heap_ref_count(address)).collect();
    assert!(
        counts.iter().all(|count| count.is_some()),
        "supervisor, actor and array should all report a count: {counts:?}"
    );
}

// --- standard library ------------------------------------------------------

#[tokio::test]
async fn io_print_renders_every_value_kind() {
    let signal = ExitSignal {
        from: 1,
        reason: ExitReason::Normal,
    };
    for value in [
        Value::Integer(-3),
        Value::Float(1.5),
        Value::Boolean(false),
        Value::Null,
        Value::ExitSignal(signal),
    ] {
        let (mut vm, _tx) = VM::new(
            vec![
                OpCode::PushConst(value.clone()),
                OpCode::LoadGlobal("io".to_string()),
                OpCode::GetExport("print".to_string()),
                OpCode::CallNative(1),
            ],
            None,
        );
        vm.run()
            .await
            .unwrap_or_else(|error| panic!("printing {value:?} should succeed: {error}"));
    }
}

#[tokio::test]
async fn io_print_renders_a_heap_reference() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::MakeString("a string".to_string()),
            OpCode::LoadGlobal("io".to_string()),
            OpCode::GetExport("print".to_string()),
            OpCode::CallNative(1),
        ],
        None,
    );
    vm.run().await.expect("printing a reference should succeed");
}
