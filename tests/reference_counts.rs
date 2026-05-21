use raft::vm::execution::ExecutionContext;
use raft::vm::heap::{Heap, HeapObject};
use raft::vm::opcodes::OpCode;
use raft::vm::value::{MessageValue, Value};
use raft::vm::vm::VM;
use tokio::sync::mpsc::channel;

fn actor_ref_count(heap: &Heap, address: usize) -> usize {
    match heap.get(address) {
        Some(HeapObject::Actor(_, _, rc)) => *rc,
        _ => panic!("Expected actor at address {address}"),
    }
}

#[tokio::test]
async fn actor_reference_lifecycle_on_stack() {
    let mut execution = ExecutionContext::new(vec![OpCode::Return]);
    let mut heap = Heap::new();

    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap)
        .unwrap();

    let address = match execution.stack.last().copied() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("Expected actor reference on stack, got {other:?}"),
    };

    assert_eq!(actor_ref_count(&heap, address), 1);

    OpCode::Dup.execute(&mut execution, &mut heap).unwrap();
    assert_eq!(actor_ref_count(&heap, address), 2);

    OpCode::Pop.execute(&mut execution, &mut heap).unwrap();
    assert_eq!(actor_ref_count(&heap, address), 1);

    OpCode::Pop.execute(&mut execution, &mut heap).unwrap();
    assert_eq!(actor_ref_count(&heap, address), 0);

    heap.collect_garbage();
    assert!(heap.get(address).is_none());
}

#[tokio::test]
async fn send_and_receive_message_updates_reference_counts() {
    let (_tx, mailbox) = channel(1);
    let mut heap = Heap::new();
    let mut execution = ExecutionContext::with_mailbox(vec![OpCode::Return], mailbox);

    // Spawn target actor (A) and message array (M)
    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap)
        .unwrap();
    let actor_a = match execution.stack.last().copied() {
        Some(Value::Reference(addr)) => addr,
        _ => panic!("Expected actor reference for target"),
    };

    let message_addr = heap.allocate(HeapObject::Array(vec![], 0));
    OpCode::PushConst(Value::Reference(message_addr))
        .execute(&mut execution, &mut heap)
        .unwrap();
    let message_ref = message_addr;

    OpCode::Swap.execute(&mut execution, &mut heap).unwrap();

    OpCode::SendMessage
        .execute(&mut execution, &mut heap)
        .unwrap();

    assert_eq!(actor_ref_count(&heap, actor_a), 1, "actor on stack");
    match heap.get(message_ref) {
        Some(HeapObject::Array(_, rc)) => assert_eq!(*rc, 1, "message queued"),
        other => panic!("expected message array, got {other:?}"),
    }

    // Drop both stack references and collect.
    OpCode::Pop.execute(&mut execution, &mut heap).unwrap();
    OpCode::Pop.execute(&mut execution, &mut heap).unwrap();
    assert_eq!(actor_ref_count(&heap, actor_a), 0);
    match heap.get(message_ref) {
        Some(HeapObject::Array(_, rc)) => assert_eq!(*rc, 0),
        other => panic!("expected message array, got {other:?}"),
    }

    heap.collect_garbage();
    assert!(heap.get(actor_a).is_none());
    assert!(heap.get(message_ref).is_none());
}

#[tokio::test]
async fn receive_message_updates_reference_counts() {
    let (tx, mailbox) = channel(1);
    let mut execution = ExecutionContext::with_mailbox(vec![OpCode::Return], mailbox);
    let mut heap = Heap::new();

    tx.send(MessageValue::Array(vec![]))
        .await
        .expect("sending message into mailbox should succeed");

    OpCode::ReceiveMessage
        .execute(&mut execution, &mut heap)
        .expect("ReceiveMessage should push message onto stack");

    let message_addr = match execution.stack.as_slice() {
        [Value::Reference(addr)] => *addr,
        other => panic!("expected a single message reference on stack, got {other:?}"),
    };
    match heap
        .get(message_addr)
        .expect("message object should be allocated")
    {
        HeapObject::Array(_, rc) => assert_eq!(*rc, 2, "message should be retained on stack"),
        other => panic!("expected array at message_addr, got {other:?}"),
    }
}

#[tokio::test]
async fn vm_pop_stack_drops_actor_reference() {
    let (mut vm, _tx) = VM::new(vec![OpCode::SpawnActor(0)], None);
    vm.run().await.unwrap();

    let actor_addr = match vm.pop_stack().expect("pop_stack should succeed") {
        Value::Reference(addr) => addr,
        other => panic!("Expected actor reference, got {other:?}"),
    };

    let ref_count = vm
        .heap_ref_count(actor_addr)
        .expect("expected actor to remain allocated");
    assert_eq!(
        ref_count, 0,
        "actor reference count should drop to zero after pop"
    );

    vm.collect_garbage();
    assert!(
        vm.heap_ref_count(actor_addr).is_none(),
        "actor should be collected after reference count reaches zero"
    );
}

#[tokio::test]
async fn neg_type_mismatch_consumes_reference_operand() {
    let mut execution = ExecutionContext::new(vec![OpCode::Return]);
    let mut heap = Heap::new();

    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap)
        .unwrap();

    let address = match execution.stack.last().copied() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("Expected actor reference on stack, got {other:?}"),
    };

    let err = OpCode::Neg.execute(&mut execution, &mut heap).unwrap_err();
    assert!(matches!(err, raft::vm::error::VmError::TypeMismatch("Neg")));
    assert!(execution.stack.is_empty(), "operand should be consumed");
    assert_eq!(actor_ref_count(&heap, address), 0);

    heap.collect_garbage();
    assert!(heap.get(address).is_none());
}

#[tokio::test]
async fn add_and_mod_type_mismatch_do_not_leak_references() {
    let mut execution = ExecutionContext::new(vec![OpCode::Return]);
    let mut heap = Heap::new();

    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap)
        .unwrap();
    let add_ref = match execution.stack.last().copied() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("Expected actor reference on stack, got {other:?}"),
    };
    execution.stack.push(Value::Integer(1));

    let add_err = OpCode::Add.execute(&mut execution, &mut heap).unwrap_err();
    assert!(matches!(
        add_err,
        raft::vm::error::VmError::TypeMismatch("Add")
    ));
    assert!(execution.stack.is_empty(), "operands should be consumed");
    assert_eq!(actor_ref_count(&heap, add_ref), 0);

    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap)
        .unwrap();
    let mod_ref = match execution.stack.last().copied() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("Expected actor reference on stack, got {other:?}"),
    };
    execution.stack.push(Value::Integer(2));

    let mod_err = OpCode::Mod.execute(&mut execution, &mut heap).unwrap_err();
    assert!(matches!(
        mod_err,
        raft::vm::error::VmError::TypeMismatch("Mod")
    ));
    assert!(execution.stack.is_empty(), "operands should be consumed");
    assert_eq!(actor_ref_count(&heap, mod_ref), 0);

    heap.collect_garbage();
    assert!(heap.get(add_ref).is_none());
    assert!(heap.get(mod_ref).is_none());
}

#[test]
fn heap_reuses_freed_slab_slots() {
    let mut heap = Heap::new();

    let first = heap.allocate(HeapObject::String("first".to_string(), 0));
    heap.collect_garbage();

    assert!(heap.get(first).is_none());

    let second = heap.allocate(HeapObject::String("second".to_string(), 0));
    assert_eq!(second, first, "freed slab slot should be reused");
}

#[test]
fn garbage_collection_collects_unrooted_reference_cycle() {
    let mut heap = Heap::new();

    let left = heap.allocate(HeapObject::Array(vec![], 0));
    let right = heap.allocate(HeapObject::Array(vec![], 0));

    if let Some(HeapObject::Array(values, _)) = heap.get_mut(left) {
        values.push(Value::Reference(right));
    }
    if let Some(HeapObject::Array(values, _)) = heap.get_mut(right) {
        values.push(Value::Reference(left));
    }
    heap.get_mut(left).unwrap().increment_ref();
    heap.get_mut(right).unwrap().increment_ref();

    assert!(heap.get(left).unwrap().is_alive());
    assert!(heap.get(right).unwrap().is_alive());

    heap.collect_garbage();

    assert!(heap.get(left).is_none());
    assert!(heap.get(right).is_none());
}

#[test]
fn garbage_collection_preserves_cycles_reachable_from_external_references() {
    let mut heap = Heap::new();

    let left = heap.allocate(HeapObject::Array(vec![], 0));
    let right = heap.allocate(HeapObject::Array(vec![], 0));

    if let Some(HeapObject::Array(values, _)) = heap.get_mut(left) {
        values.push(Value::Reference(right));
    }
    if let Some(HeapObject::Array(values, _)) = heap.get_mut(right) {
        values.push(Value::Reference(left));
    }
    heap.get_mut(left).unwrap().increment_ref();
    heap.get_mut(right).unwrap().increment_ref();
    heap.get_mut(left).unwrap().increment_ref();

    heap.collect_garbage();

    assert!(heap.get(left).is_some());
    assert!(heap.get(right).is_some());

    heap.get_mut(left).unwrap().decrement_ref();
    heap.collect_garbage();

    assert!(heap.get(left).is_none());
    assert!(heap.get(right).is_none());
}
