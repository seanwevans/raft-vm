use raft::vm::opcodes::OpCode;
use raft::vm::execution::ExecutionContext;
use raft::vm::heap::Heap;
use raft::vm::value::Value;
use raft::vm::error::VmError;
use tokio::sync::mpsc::channel;

#[tokio::test]
async fn jump_if_false_errors_on_non_boolean() {
    let mut ctx = ExecutionContext::new(vec![]);
    ctx.stack.push(Value::Integer(42));
    let mut heap = Heap::new();
    let (_tx, mut rx) = channel(1);
    let result = OpCode::JumpIfFalse(0).execute(&mut ctx, &mut heap, &mut rx).await;
    assert!(matches!(result, Err(VmError::TypeMismatch)));
}
