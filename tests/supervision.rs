use raft::vm::execution::ExecutionContext;
use raft::vm::heap::{Heap, HeapObject};
use raft::vm::value::MessageValue;
use raft::vm::{ExitReason, OpCode, SupervisorStrategy, Value, VmError, VM};

#[tokio::test]
async fn linked_parent_receives_division_by_zero_exit_signal() {
    let (mut parent, parent_tx) = VM::new(vec![OpCode::Return], None);
    let (mut child, _child_tx) = VM::new(
        vec![
            OpCode::PushConst(Value::Integer(1)),
            OpCode::PushConst(Value::Integer(0)),
            OpCode::Div,
        ],
        None,
    );

    child.link(parent_tx);
    let err = child.run().await.expect_err("child should fail");
    assert!(matches!(err, VmError::DivisionByZero));

    let signal = parent
        .mailbox_mut()
        .recv()
        .await
        .expect("linked parent should receive an exit signal");
    match signal {
        MessageValue::ExitSignal(signal) => {
            assert_eq!(signal.from, child.process_id());
            assert_eq!(signal.reason, ExitReason::DivisionByZero);
        }
        other => panic!("expected exit signal, got {other:?}"),
    }
}

#[tokio::test]
async fn linked_parent_receives_type_mismatch_exit_signal() {
    let (mut parent, parent_tx) = VM::new(vec![OpCode::Return], None);
    let (mut child, _child_tx) = VM::new(
        vec![
            OpCode::PushConst(Value::Integer(1)),
            OpCode::PushConst(Value::Boolean(true)),
            OpCode::Add,
        ],
        None,
    );

    child.link(parent_tx);
    let err = child.run().await.expect_err("child should fail");
    assert!(matches!(err, VmError::TypeMismatch("Add")));

    let signal = parent
        .mailbox_mut()
        .recv()
        .await
        .expect("linked parent should receive an exit signal");
    match signal {
        MessageValue::ExitSignal(signal) => {
            assert_eq!(signal.from, child.process_id());
            assert_eq!(signal.reason, ExitReason::TypeMismatch);
        }
        other => panic!("expected exit signal, got {other:?}"),
    }
}

#[tokio::test]
async fn supervisor_restart_child_uses_one_for_all_strategy() {
    let mut execution = ExecutionContext::new(vec![OpCode::Return]);
    let mut heap = Heap::new();
    OpCode::SpawnSupervisor(0)
        .execute(&mut execution, &mut heap)
        .expect("spawn supervisor should succeed");
    let supervisor_addr = match execution.stack.last().cloned() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("expected supervisor reference, got {other:?}"),
    };

    OpCode::SetStrategy(1)
        .execute(&mut execution, &mut heap)
        .expect("set strategy should succeed");
    match heap.get(supervisor_addr).expect("supervisor should exist") {
        HeapObject::Supervisor(vm, _, _) => {
            assert_eq!(vm.strategy(), SupervisorStrategy::OneForAll)
        }
        other => panic!("expected supervisor, got {other:?}"),
    }

    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap)
        .expect("spawn first actor should succeed");
    let first_child = match execution.stack.pop() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("expected first actor reference, got {other:?}"),
    };
    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap)
        .expect("spawn second actor should succeed");
    let second_child = match execution.stack.pop() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("expected second actor reference, got {other:?}"),
    };

    match heap.get_mut(first_child).expect("first child should exist") {
        HeapObject::Actor(vm, _, _) => vm.set_ip(1),
        other => panic!("expected first actor, got {other:?}"),
    }
    match heap
        .get_mut(second_child)
        .expect("second child should exist")
    {
        HeapObject::Actor(vm, _, _) => vm.set_ip(1),
        other => panic!("expected second actor, got {other:?}"),
    }

    // Register the second child first, then restart the first child. One-for-all
    // should reset both tracked children to their start instruction pointers.
    execution.stack.push(Value::Reference(supervisor_addr));
    OpCode::RestartChild(second_child)
        .execute(&mut execution, &mut heap)
        .expect("registering second child should succeed");
    execution.stack.push(Value::Reference(supervisor_addr));
    OpCode::RestartChild(first_child)
        .execute(&mut execution, &mut heap)
        .expect("one-for-all restart should succeed");

    match heap.get(first_child).expect("first child should exist") {
        HeapObject::Actor(vm, _, _) => assert_eq!(vm.current_ip(), 0),
        other => panic!("expected first actor, got {other:?}"),
    }
    match heap.get(second_child).expect("second child should exist") {
        HeapObject::Actor(vm, _, _) => assert_eq!(vm.current_ip(), 0),
        other => panic!("expected second actor, got {other:?}"),
    }
}

#[tokio::test]
async fn spawn_opcodes_allocate_one_process_id_each() {
    let mut execution = ExecutionContext::new(vec![OpCode::Return]);
    let mut heap = Heap::new();

    OpCode::SpawnSupervisor(0)
        .execute(&mut execution, &mut heap)
        .expect("spawn supervisor should succeed");
    let supervisor_addr = match execution.stack.pop() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("expected supervisor reference, got {other:?}"),
    };

    let supervisor_pid = match heap.get(supervisor_addr).expect("supervisor should exist") {
        HeapObject::Supervisor(handle, _, _) => handle.process_id(),
        other => panic!("expected supervisor, got {other:?}"),
    };

    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap)
        .expect("spawn first actor should succeed");
    let first_actor_addr = match execution.stack.pop() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("expected actor reference, got {other:?}"),
    };
    let first_actor_pid = match heap
        .get(first_actor_addr)
        .expect("first actor should exist")
    {
        HeapObject::Actor(handle, _, _) => handle.process_id(),
        other => panic!("expected actor, got {other:?}"),
    };

    OpCode::SpawnActor(0)
        .execute(&mut execution, &mut heap)
        .expect("spawn second actor should succeed");
    let second_actor_addr = match execution.stack.pop() {
        Some(Value::Reference(addr)) => addr,
        other => panic!("expected actor reference, got {other:?}"),
    };
    let second_actor_pid = match heap
        .get(second_actor_addr)
        .expect("second actor should exist")
    {
        HeapObject::Actor(handle, _, _) => handle.process_id(),
        other => panic!("expected actor, got {other:?}"),
    };

    assert!(
        first_actor_pid > supervisor_pid,
        "first actor pid should be allocated after supervisor pid"
    );
    assert!(
        second_actor_pid > first_actor_pid,
        "second actor pid should be allocated after first actor pid"
    );
}
