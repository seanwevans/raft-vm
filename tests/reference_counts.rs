use raft::vm::execution::ExecutionContext;
use raft::vm::heap::{Heap, HeapObject};
use raft::vm::opcodes::OpCode;
use raft::vm::value::Value;
use tokio::sync::mpsc::channel;

fn actor_ref_count(heap: &Heap, address: usize) -> usize {
    match heap.get(address) {
        Some(HeapObject::Actor(_, _, rc)) => *rc,
        _ => panic!("Expected actor at address {address}"),
    }
}

#[tokio::test]
async fn actor_reference_lifecycle_on_stack() {
    let mut execution = ExecutionContext::new(vec![]);
    let mut heap = Heap::new();
    let (_tx, mut mailbox) = channel(1);

    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap, &mut mailbox)
        .await
        .unwrap();

    let address = match execution.stack.last().copied() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("Expected actor reference on stack, got {other:?}"),
    };

    assert_eq!(actor_ref_count(&heap, address), 1);

    OpCode::Dup
        .execute(&mut execution, &mut heap, &mut mailbox)
        .await
        .unwrap();
    assert_eq!(actor_ref_count(&heap, address), 2);

    OpCode::Pop
        .execute(&mut execution, &mut heap, &mut mailbox)
        .await
        .unwrap();
    assert_eq!(actor_ref_count(&heap, address), 1);

    OpCode::Pop
        .execute(&mut execution, &mut heap, &mut mailbox)
        .await
        .unwrap();
    assert_eq!(actor_ref_count(&heap, address), 0);

    heap.collect_garbage();
    assert!(heap.get(address).is_none());
}

#[tokio::test]
async fn send_and_receive_message_updates_reference_counts() {
    let mut execution = ExecutionContext::new(vec![]);
    let mut heap = Heap::new();
    let (tx, mut mailbox) = channel(1);

    // Spawn target actor (A) and message actor (B)
    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap, &mut mailbox)
        .await
        .unwrap();
    let actor_a = match execution.stack.last().copied() {
        Some(Value::Reference(addr)) => addr,
        _ => panic!("Expected actor reference for target"),
    };

    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap, &mut mailbox)
        .await
        .unwrap();
    let actor_b = match execution.stack.last().copied() {
        Some(Value::Reference(addr)) => addr,
        _ => panic!("Expected actor reference for message"),
    };

    OpCode::Swap
        .execute(&mut execution, &mut heap, &mut mailbox)
        .await
        .unwrap();

    OpCode::SendMessage
        .execute(&mut execution, &mut heap, &mut mailbox)
        .await
        .unwrap();

    assert_eq!(actor_ref_count(&heap, actor_a), 1, "actor on stack");
    assert_eq!(actor_ref_count(&heap, actor_b), 1, "message queued");

    // Simulate the message arriving back to this VM's mailbox.
    let message = {
        let actor_entry = heap.get_mut(actor_a).expect("Expected actor VM for target");
        match actor_entry {
            HeapObject::Actor(vm, _, _) => vm.mailbox.recv().await,
            _ => panic!("Expected actor VM for target"),
        }
    }
    .expect("Actor mailbox was empty");

    if let Value::Reference(addr) = message {
        if let Some(object) = heap.get_mut(addr) {
            object.decrement_ref();
            object.increment_ref();
        } else {
            panic!("Message reference not found in heap");
        }
    }

    tx.send(message).await.unwrap();

    OpCode::ReceiveMessage
        .execute(&mut execution, &mut heap, &mut mailbox)
        .await
        .unwrap();

    assert_eq!(actor_ref_count(&heap, actor_b), 1, "message now on stack");

    // Drop both stack references and collect.
    OpCode::Pop
        .execute(&mut execution, &mut heap, &mut mailbox)
        .await
        .unwrap();
    OpCode::Pop
        .execute(&mut execution, &mut heap, &mut mailbox)
        .await
        .unwrap();
    assert_eq!(actor_ref_count(&heap, actor_b), 0);

    heap.collect_garbage();
    assert!(heap.get(actor_b).is_none());
}
