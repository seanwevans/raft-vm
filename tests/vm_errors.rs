use raft::vm::execution::ExecutionContext;
use raft::vm::heap::{Heap, HeapObject};
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

#[tokio::test]
async fn spawn_actor_at_bytecode_len_returns_error() {
    let code = vec![OpCode::SpawnActor(1)];
    let (mut vm, _tx) = VM::new(code, None);
    let err = vm
        .run()
        .await
        .expect_err("expected execution out of bounds for spawn actor at bytecode length");
    assert!(matches!(err, VmError::ExecutionOutOfBounds));
}

#[tokio::test]
async fn spawn_supervisor_at_bytecode_len_returns_error() {
    let code = vec![OpCode::SpawnSupervisor(1)];
    let (mut vm, _tx) = VM::new(code, None);
    let err = vm
        .run()
        .await
        .expect_err("expected execution out of bounds for spawn supervisor at bytecode length");
    assert!(matches!(err, VmError::ExecutionOutOfBounds));
}

#[tokio::test]
async fn send_message_failure_preserves_actor_on_stack_and_ref_counts() {
    let mut execution = ExecutionContext::new(vec![OpCode::Return]);
    let mut heap = Heap::new();
    // Stack layout expected by SendMessage is [message, actor_ref].
    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap)
        .expect("spawn actor should succeed");
    let actor_addr = match execution.stack.last().copied() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("expected actor reference, got {other:?}"),
    };

    let message_addr = heap.allocate(HeapObject::Array(vec![], 0));
    OpCode::PushConst(Value::Reference(message_addr))
        .execute(&mut execution, &mut heap)
        .expect("push message reference should succeed");
    OpCode::Swap
        .execute(&mut execution, &mut heap)
        .expect("swap should succeed");

    // Close actor mailbox to force ChannelSend error.
    let actor_entry = heap
        .get_mut(actor_addr)
        .expect("spawned actor should be in heap");
    match actor_entry {
        HeapObject::Actor(actor_vm, _, _) => actor_vm.mailbox_mut().close(),
        other => panic!("expected actor in heap, got {other:?}"),
    }

    let err = OpCode::SendMessage
        .execute(&mut execution, &mut heap)
        .expect_err("send should fail when mailbox is closed");

    match err {
        VmError::ChannelSend { value, .. } => {
            assert_eq!(value, Value::Reference(message_addr));
        }
        other => panic!("expected ChannelSend error, got {other:?}"),
    }

    assert_eq!(
        execution.stack,
        vec![Value::Reference(actor_addr)],
        "actor reference should be restored on failure",
    );

    match heap.get(actor_addr).expect("actor should remain allocated") {
        HeapObject::Actor(_, _, rc) => assert_eq!(*rc, 1, "actor should have stack ref"),
        other => panic!("expected actor at actor_addr, got {other:?}"),
    }
    match heap
        .get(message_addr)
        .expect("message object should remain allocated")
    {
        HeapObject::Array(_, rc) => {
            assert_eq!(
                *rc, 0,
                "failed send should release the retained message reference",
            )
        }
        other => panic!("expected array at message_addr, got {other:?}"),
    }
}

#[tokio::test]
async fn top_level_return_gracefully_halts_vm() {
    let code = vec![OpCode::Return];
    let (mut vm, _tx) = VM::new(code, None);
    let result = vm.run().await;
    assert!(
        result.is_ok(),
        "expected top-level return to halt gracefully"
    );
}

#[tokio::test]
async fn native_io_print_is_available_from_standard_library() {
    let code = vec![
        OpCode::PushConst(Value::Integer(42)),
        OpCode::LoadGlobal("io".to_string()),
        OpCode::GetExport("print".to_string()),
        OpCode::CallNative(1),
    ];
    let (mut vm, _tx) = VM::new(code, None);

    vm.run().await.expect("native io.print should run");

    assert_eq!(vm.stack().last().copied(), Some(Value::Null));
}

#[tokio::test]
async fn missing_native_global_returns_error() {
    let code = vec![OpCode::LoadGlobal("missing".to_string())];
    let (mut vm, _tx) = VM::new(code, None);

    let err = vm.run().await.expect_err("missing global should fail");

    assert!(matches!(err, VmError::GlobalNotFound(name) if name == "missing"));
}
