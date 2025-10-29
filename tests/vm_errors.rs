use raft::vm::{error::VmError, opcodes::OpCode, value::Value, vm::VM};

#[tokio::test]
async fn division_by_zero_returns_error() {
    let code = vec![
        OpCode::PushConst(Value::Integer(4)),
        OpCode::PushConst(Value::Integer(0)),
        OpCode::Div,
    ];
    let (mut vm, _tx) = VM::new(code, None);
    let err = vm.run().await.expect_err("expected division by zero error");
    assert_eq!(err.to_string(), "Division by zero");
}

#[tokio::test]
async fn pop_on_empty_stack_returns_error() {
    let code = vec![OpCode::Pop];
    let (mut vm, _tx) = VM::new(code, None);
    let err = vm.run().await.expect_err("expected stack underflow");
    assert_eq!(err.to_string(), "Stack underflow");
}

#[tokio::test]
async fn swap_on_single_element_stack_returns_error() {
    let code = vec![OpCode::PushConst(Value::Integer(1)), OpCode::Swap];
    let (mut vm, _tx) = VM::new(code, None);
    let err = vm
        .run()
        .await
        .expect_err("expected stack underflow for swap");
    assert_eq!(err.to_string(), "Stack underflow for Swap");
}

#[tokio::test]
async fn jump_out_of_bounds_returns_error() {
    let code = vec![OpCode::Jump(5)];
    let (mut vm, _tx) = VM::new(code, None);
    let err = vm
        .run()
        .await
        .expect_err("expected execution out of bounds for jump");
    assert!(matches!(err, VmError::ExecutionOutOfBounds));
}

#[tokio::test]
async fn jump_if_false_out_of_bounds_returns_error() {
    let code = vec![
        OpCode::PushConst(Value::Boolean(false)),
        OpCode::JumpIfFalse(10),
    ];
    let (mut vm, _tx) = VM::new(code, None);
    let err = vm
        .run()
        .await
        .expect_err("expected execution out of bounds for jump if false");
    assert!(matches!(err, VmError::ExecutionOutOfBounds));
}

#[tokio::test]
async fn call_out_of_bounds_returns_error() {
    let code = vec![OpCode::Call(3)];
    let (mut vm, _tx) = VM::new(code, None);
    let err = vm
        .run()
        .await
        .expect_err("expected execution out of bounds for call");
    assert!(matches!(err, VmError::ExecutionOutOfBounds));
}

#[tokio::test]
async fn spawn_actor_out_of_bounds_returns_error() {
    let code = vec![OpCode::SpawnActor(4)];
    let (mut vm, _tx) = VM::new(code, None);
    let err = vm
        .run()
        .await
        .expect_err("expected execution out of bounds for spawn actor");
    assert!(matches!(err, VmError::ExecutionOutOfBounds));
}

#[tokio::test]
async fn spawn_supervisor_out_of_bounds_returns_error() {
    let code = vec![OpCode::SpawnSupervisor(4)];
    let (mut vm, _tx) = VM::new(code, None);
    let err = vm
        .run()
        .await
        .expect_err("expected execution out of bounds for spawn supervisor");
    assert!(matches!(err, VmError::ExecutionOutOfBounds));
}
