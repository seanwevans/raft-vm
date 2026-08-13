//! Process handles, structured message conversion, and supervision state.

use std::sync::{Arc, Mutex};

use raft::vm::heap::{Heap, HeapObject, NativeFunction, ProcessHandle};
use raft::vm::opcodes::OpCode;
use raft::vm::value::{MessageValue, Value};
use raft::vm::{ChildSpec, SupervisorStrategy, VmError};

fn process_handle(final_stack: Vec<Value>) -> ProcessHandle {
    let task = tokio::spawn(async { Ok(()) });
    ProcessHandle::new(
        7,
        Some(2),
        3,
        vec![OpCode::Return],
        None,
        Vec::new(),
        true,
        task,
        Arc::new(Mutex::new(final_stack)),
    )
}

// --- process handles -------------------------------------------------------

#[tokio::test]
async fn a_process_handle_reports_what_it_was_built_with() {
    let handle = process_handle(Vec::new());
    assert_eq!(handle.process_id(), 7);
    assert_eq!(handle.parent(), Some(2));
    assert_eq!(handle.restart_ip(), 3);
    assert_eq!(
        handle.current_ip(),
        3,
        "a new handle starts at its start ip"
    );
    assert!(handle.trap_exits());
    assert!(handle.links().is_empty());
    assert!(handle.debug_info().is_none());
    assert_eq!(handle.bytecode().len(), 1);
}

#[tokio::test]
async fn a_process_handle_pointer_can_be_moved() {
    let mut handle = process_handle(Vec::new());
    handle.set_ip(11);
    assert_eq!(handle.current_ip(), 11);
    assert_eq!(handle.restart_ip(), 3, "the restart point does not move");
}

#[tokio::test]
async fn awaiting_a_finished_process_yields_its_result() {
    let mut handle = process_handle(Vec::new());
    handle.run().await.expect("the task should finish cleanly");
    handle
        .run()
        .await
        .expect("awaiting an already-joined process should be a no-op");
}

#[tokio::test]
async fn a_cancelled_process_is_not_reported_as_an_error() {
    let mut handle = process_handle(Vec::new());
    let replacement = tokio::spawn(async {
        // Long enough that the abort in `replace_runtime` lands first.
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        Ok(())
    });
    handle.replace_runtime(replacement, 5);
    assert_eq!(handle.restart_ip(), 5);
    assert_eq!(handle.current_ip(), 5);

    let further = tokio::spawn(async { Ok(()) });
    handle.replace_runtime(further, 0);
    handle
        .run()
        .await
        .expect("an aborted task should not surface as a failure");
}

#[tokio::test]
async fn a_process_handle_exposes_its_final_stack() {
    let mut handle = process_handle(vec![Value::Integer(1), Value::Integer(2)]);
    assert_eq!(handle.pop_stack().expect("pop"), Value::Integer(2));
    assert_eq!(handle.pop_stack().expect("pop"), Value::Integer(1));
    assert!(matches!(handle.pop_stack(), Err(VmError::StackUnderflow)));
}

#[tokio::test]
async fn a_process_handle_lists_the_references_in_its_final_stack() {
    let handle = process_handle(vec![
        Value::Reference(4),
        Value::Integer(1),
        Value::Reference(9),
    ]);
    assert_eq!(handle.heap_references(), vec![4, 9]);
}

#[tokio::test]
async fn a_process_handle_tracks_supervised_children() {
    let mut handle = process_handle(Vec::new());
    assert!(handle.supervised_children().is_empty());
    assert_eq!(handle.strategy(), SupervisorStrategy::OneForOne);

    handle.set_strategy(2);
    assert_eq!(handle.strategy(), SupervisorStrategy::RestForOne);

    let child = ChildSpec {
        reference: 1,
        start_ip: 0,
    };
    assert_eq!(handle.restart_targets(child), vec![child]);
    assert_eq!(handle.supervised_children(), &[child]);
}

// --- supervision state -----------------------------------------------------

#[test]
fn registering_a_known_child_updates_it_in_place() {
    let mut handle_state = raft::vm::supervision::SupervisorState::default();
    handle_state.ensure_child(ChildSpec {
        reference: 1,
        start_ip: 0,
    });
    handle_state.ensure_child(ChildSpec {
        reference: 1,
        start_ip: 8,
    });

    assert_eq!(
        handle_state.children(),
        &[ChildSpec {
            reference: 1,
            start_ip: 8
        }],
        "the same child should be updated, not duplicated"
    );
}

#[test]
fn each_strategy_selects_a_different_set_of_children() {
    let children: Vec<ChildSpec> = (0..3)
        .map(|reference| ChildSpec {
            reference,
            start_ip: 0,
        })
        .collect();

    let mut one_for_one = raft::vm::supervision::SupervisorState::default();
    for child in &children {
        one_for_one.ensure_child(*child);
    }
    assert_eq!(one_for_one.restart_targets(children[1]), vec![children[1]]);

    let mut one_for_all = raft::vm::supervision::SupervisorState::default();
    one_for_all.set_strategy(SupervisorStrategy::OneForAll);
    for child in &children {
        one_for_all.ensure_child(*child);
    }
    assert_eq!(one_for_all.restart_targets(children[1]), children);

    let mut rest_for_one = raft::vm::supervision::SupervisorState::default();
    rest_for_one.set_strategy(SupervisorStrategy::RestForOne);
    for child in &children {
        rest_for_one.ensure_child(*child);
    }
    assert_eq!(
        rest_for_one.restart_targets(children[1]),
        children[1..].to_vec()
    );
}

// --- structured message conversion -----------------------------------------

#[test]
fn an_array_converts_to_a_message_and_back() {
    let mut heap = Heap::new();
    let string = heap.allocate(HeapObject::String("nested".to_string(), 1));
    let array = heap.allocate(HeapObject::Array(
        vec![
            Value::Integer(1),
            Value::Reference(string),
            Value::Boolean(true),
        ],
        1,
    ));

    let message = heap
        .value_to_message(Value::Reference(array))
        .expect("array should convert");
    assert_eq!(
        message,
        MessageValue::Array(vec![
            MessageValue::Integer(1),
            MessageValue::String("nested".to_string()),
            MessageValue::Boolean(true),
        ])
    );

    let mut receiver = Heap::new();
    match receiver
        .message_to_value(message)
        .expect("array message should materialize")
    {
        Value::Reference(address) => match receiver.get(address) {
            Some(HeapObject::Array(values, _)) => assert_eq!(values.len(), 3),
            other => panic!("expected an array, got {other:?}"),
        },
        other => panic!("expected a reference, got {other:?}"),
    }
}

#[test]
fn a_module_converts_to_a_message() {
    let mut heap = Heap::new();
    let mut exports = std::collections::HashMap::new();
    exports.insert("answer".to_string(), Value::Integer(42));
    let module = heap.allocate(HeapObject::Module {
        name: "answers".to_string(),
        exports,
        ref_count: 1,
    });

    match heap
        .value_to_message(Value::Reference(module))
        .expect("module should convert")
    {
        MessageValue::Module(exports) => {
            assert_eq!(exports.get("answer"), Some(&MessageValue::Integer(42)));
        }
        other => panic!("expected a module message, got {other:?}"),
    }
}

#[test]
fn a_native_function_cannot_be_sent_as_a_message() {
    fn noop(_args: Vec<Value>) -> Result<Value, VmError> {
        Ok(Value::Null)
    }

    let mut heap = Heap::new();
    let address = heap.allocate(HeapObject::NativeFunction(
        NativeFunction {
            name: "noop".to_string(),
            arity: 0,
            function: noop,
        },
        1,
    ));

    assert!(matches!(
        heap.value_to_message(Value::Reference(address)),
        Err(VmError::TypeMismatch(
            "SendMessage unsupported reference type"
        ))
    ));
}

#[test]
fn an_array_holding_a_dangling_reference_will_not_convert() {
    let mut heap = Heap::new();
    let array = heap.allocate(HeapObject::Array(vec![Value::Reference(999)], 1));
    assert!(matches!(
        heap.value_to_message(Value::Reference(array)),
        Err(VmError::InvalidReference)
    ));
}

// --- heap object bookkeeping -----------------------------------------------

#[test]
fn references_are_listed_for_containers_only() {
    let array = HeapObject::Array(
        vec![Value::Reference(1), Value::Integer(0), Value::Reference(2)],
        1,
    );
    assert_eq!(array.references(), vec![1, 2]);

    let mut exports = std::collections::HashMap::new();
    exports.insert("a".to_string(), Value::Reference(5));
    exports.insert("b".to_string(), Value::Null);
    let module = HeapObject::Module {
        name: "m".to_string(),
        exports,
        ref_count: 1,
    };
    assert_eq!(module.references(), vec![5]);

    assert!(HeapObject::String("s".to_string(), 1)
        .references()
        .is_empty());
}

#[test]
fn reference_counts_move_up_and_stop_at_zero() {
    let mut object = HeapObject::String("counted".to_string(), 0);
    assert!(!object.is_alive());

    object.increment_ref();
    assert_eq!(object.ref_count(), 1);
    assert!(object.is_alive());

    object.decrement_ref();
    assert_eq!(object.ref_count(), 0);
    object.decrement_ref();
    assert_eq!(object.ref_count(), 0, "counts must not wrap below zero");
}

#[test]
fn module_reference_counts_move_the_same_way() {
    let mut module = HeapObject::Module {
        name: "m".to_string(),
        exports: std::collections::HashMap::new(),
        ref_count: 0,
    };
    module.increment_ref();
    assert_eq!(module.ref_count(), 1);
    module.decrement_ref();
    module.decrement_ref();
    assert_eq!(module.ref_count(), 0);
}

#[test]
fn releasing_an_address_that_holds_nothing_is_an_error() {
    let mut heap = Heap::new();
    assert!(matches!(
        heap.release_reference(3),
        Err(VmError::InvalidReference)
    ));
}
