use raft::vm::{opcodes::OpCode, value::Value, vm::VM};

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
    assert_eq!(err.to_string(), "Stack underflow");
}
