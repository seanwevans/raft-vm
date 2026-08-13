use raft::vm::heap::{Heap, HeapObject, MAX_MESSAGE_DEPTH};
use raft::vm::opcodes::OpCode;
use raft::vm::value::{MessageValue, Value};
use raft::vm::{VmError, VM};

/// Bytecode that builds a self-referential array (`a[0] = a`) and leaves it on
/// the stack. `ArraySet` pops value, index, then the array reference.
fn build_self_referential_array() -> Vec<OpCode> {
    vec![
        OpCode::PushConst(Value::Null),
        OpCode::MakeArray(1),
        OpCode::Dup,
        OpCode::PushConst(Value::Integer(0)),
        OpCode::Swap,
        OpCode::ArraySet,
    ]
}

fn innermost_error(error: &VmError) -> &VmError {
    match error {
        VmError::RuntimeError { source, .. } => innermost_error(source),
        other => other,
    }
}

#[test]
fn value_to_message_reports_cycle_instead_of_overflowing_the_stack() {
    let mut heap = Heap::new();
    let address = heap.allocate(HeapObject::Array(vec![Value::Null], 1));
    if let Some(HeapObject::Array(elements, _)) = heap.get_mut(address) {
        elements[0] = Value::Reference(address);
    }

    match heap.value_to_message(Value::Reference(address)) {
        Err(VmError::CyclicReference(reported)) => assert_eq!(reported, address),
        other => panic!("expected CyclicReference, got {other:?}"),
    }
}

#[test]
fn value_to_message_reports_indirect_cycles() {
    let mut heap = Heap::new();
    let outer = heap.allocate(HeapObject::Array(vec![Value::Null], 1));
    let inner = heap.allocate(HeapObject::Array(vec![Value::Reference(outer)], 1));
    if let Some(HeapObject::Array(elements, _)) = heap.get_mut(outer) {
        elements[0] = Value::Reference(inner);
    }

    assert!(matches!(
        heap.value_to_message(Value::Reference(outer)),
        Err(VmError::CyclicReference(_))
    ));
}

#[test]
fn value_to_message_allows_the_same_child_twice() {
    let mut heap = Heap::new();
    let shared = heap.allocate(HeapObject::String("shared".to_string(), 2));
    let parent = heap.allocate(HeapObject::Array(
        vec![Value::Reference(shared), Value::Reference(shared)],
        1,
    ));

    // Sharing is not a cycle: a diamond must still convert cleanly.
    match heap.value_to_message(Value::Reference(parent)) {
        Ok(MessageValue::Array(elements)) => assert_eq!(
            elements,
            vec![
                MessageValue::String("shared".to_string()),
                MessageValue::String("shared".to_string()),
            ]
        ),
        other => panic!("expected a two-element array message, got {other:?}"),
    }
}

#[test]
fn value_to_message_rejects_values_nested_past_the_depth_limit() {
    let mut heap = Heap::new();
    let mut address = heap.allocate(HeapObject::String("leaf".to_string(), 1));
    for _ in 0..MAX_MESSAGE_DEPTH + 1 {
        address = heap.allocate(HeapObject::Array(vec![Value::Reference(address)], 1));
    }

    assert!(matches!(
        heap.value_to_message(Value::Reference(address)),
        Err(VmError::MessageTooDeep(MAX_MESSAGE_DEPTH))
    ));
}

#[test]
fn value_to_message_accepts_values_within_the_depth_limit() {
    let mut heap = Heap::new();
    let mut address = heap.allocate(HeapObject::String("leaf".to_string(), 1));
    for _ in 0..MAX_MESSAGE_DEPTH - 1 {
        address = heap.allocate(HeapObject::Array(vec![Value::Reference(address)], 1));
    }

    assert!(heap.value_to_message(Value::Reference(address)).is_ok());
}

#[test]
fn releasing_a_deeply_nested_value_does_not_overflow_the_stack() {
    let mut heap = Heap::new();
    let mut address = heap.allocate(HeapObject::String("leaf".to_string(), 1));
    for _ in 0..200_000 {
        address = heap.allocate(HeapObject::Array(vec![Value::Reference(address)], 1));
    }

    heap.release_reference(address)
        .expect("releasing a deep chain should succeed");
    assert_eq!(heap.get(address).map(HeapObject::ref_count), Some(0));
}

#[test]
fn releasing_a_reference_cycle_terminates() {
    let mut heap = Heap::new();
    let left = heap.allocate(HeapObject::Array(vec![Value::Null], 1));
    let right = heap.allocate(HeapObject::Array(vec![Value::Reference(left)], 1));
    if let Some(HeapObject::Array(elements, _)) = heap.get_mut(left) {
        elements[0] = Value::Reference(right);
    }

    heap.release_reference(left)
        .expect("releasing a cycle should succeed");
    assert_eq!(heap.get(left).map(HeapObject::ref_count), Some(0));
    assert_eq!(heap.get(right).map(HeapObject::ref_count), Some(0));
}

#[tokio::test]
async fn sending_a_cyclic_message_fails_the_process_instead_of_the_runtime() {
    let mut code = build_self_referential_array();
    let child_ip = code.len() + 2;
    code.push(OpCode::SpawnActor(child_ip));
    code.push(OpCode::SendMessage);
    code.push(OpCode::ReceiveMessage);

    let (mut vm, _tx) = VM::new(code, None);
    let error = vm
        .run()
        .await
        .expect_err("sending a cyclic value should fail");

    assert!(
        matches!(innermost_error(&error), VmError::CyclicReference(_)),
        "expected CyclicReference, got {error:?}"
    );
}

#[tokio::test]
async fn a_failed_cyclic_send_leaves_the_actor_on_the_stack() {
    let mut code = build_self_referential_array();
    let child_ip = code.len() + 2;
    code.push(OpCode::SpawnActor(child_ip));
    code.push(OpCode::SendMessage);
    code.push(OpCode::ReceiveMessage);

    let (mut vm, _tx) = VM::new(code, None);
    let _ = vm.run().await;

    let actor_address = match vm.stack().last() {
        Some(Value::Reference(address)) => *address,
        other => panic!("expected the actor reference on the stack, got {other:?}"),
    };
    assert_eq!(
        vm.heap_ref_count(actor_address),
        Some(1),
        "the actor reference left on the stack should be counted exactly once"
    );
}
