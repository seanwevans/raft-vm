use raft::vm::error::VmError;
use raft::vm::execution::ExecutionContext;
use raft::vm::heap::{Heap, HeapObject};
use raft::vm::opcodes::OpCode;
use raft::vm::value::Value;
use tokio::sync::mpsc::channel;

#[tokio::test]
async fn jump_if_false_errors_on_non_boolean() {
    let mut ctx = ExecutionContext::new(vec![]);
    ctx.stack.push(Value::Integer(42));
    let mut heap = Heap::new();
    let (_tx, mut rx) = channel(1);
    let result = OpCode::JumpIfFalse(0)
        .execute(&mut ctx, &mut heap, &mut rx)
        .await;
    assert!(matches!(result, Err(VmError::TypeMismatch("JumpIfFalse"))));
}

#[tokio::test]
async fn jump_if_false_errors_on_empty_stack() {
    let mut ctx = ExecutionContext::new(vec![]);
    let mut heap = Heap::new();
    let (_tx, mut rx) = channel(1);
    let result = OpCode::JumpIfFalse(0)
        .execute(&mut ctx, &mut heap, &mut rx)
        .await;
    assert!(matches!(result, Err(VmError::StackUnderflow)));
}

fn actor_ref_count(heap: &Heap, address: usize) -> usize {
    match heap.get(address) {
        Some(HeapObject::Actor(_, _, rc)) => *rc,
        _ => panic!("Expected actor at address {address}"),
    }
}

#[tokio::test]
async fn jump_if_false_skips_jump_when_true() {
    let mut ctx = ExecutionContext::new(vec![]);
    ctx.stack.push(Value::Boolean(true));
    ctx.ip = 7;
    let mut heap = Heap::new();
    let (_tx, mut rx) = channel(1);

    OpCode::JumpIfFalse(99)
        .execute(&mut ctx, &mut heap, &mut rx)
        .await
        .unwrap();

    assert_eq!(ctx.ip, 7);
    assert!(ctx.stack.is_empty());
}

#[tokio::test]
async fn jump_if_false_drops_reference_on_type_mismatch() {
    let mut ctx = ExecutionContext::new(vec![]);
    let mut heap = Heap::new();
    let (_tx, mut rx) = channel(1);

    OpCode::SpawnActor(0)
        .execute(&mut ctx, &mut heap, &mut rx)
        .await
        .unwrap();

    let address = match ctx.stack.last().copied() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("Expected actor reference on stack, got {other:?}"),
    };

    assert_eq!(actor_ref_count(&heap, address), 1);

    let result = OpCode::JumpIfFalse(0)
        .execute(&mut ctx, &mut heap, &mut rx)
        .await;

    assert!(matches!(result, Err(VmError::TypeMismatch("JumpIfFalse"))));
    assert!(ctx.stack.is_empty());
    assert_eq!(actor_ref_count(&heap, address), 0);

    heap.collect_garbage();
    assert!(heap.get(address).is_none());
}
