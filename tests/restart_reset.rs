use raft::vm::opcodes::OpCode;
use raft::vm::value::{MessageValue, Value};
use raft::vm::VM;

#[tokio::test]
async fn a_restarted_process_can_still_receive_messages() {
    let (mut vm, _original_tx) = VM::new(vec![OpCode::ReceiveMessage], None);

    let tx = vm.reset_for_restart(0);
    tx.send(MessageValue::Integer(7))
        .await
        .expect("the restarted mailbox should accept messages");

    vm.run()
        .await
        .expect("a restarted process should receive on its new mailbox");
    assert_eq!(vm.stack(), &vec![Value::Integer(7)]);
}

#[tokio::test]
async fn the_senders_view_of_the_restarted_process_matches() {
    let (mut vm, _original_tx) = VM::new(vec![OpCode::ReceiveMessage], None);

    let returned = vm.reset_for_restart(0);
    vm.sender()
        .send(MessageValue::Integer(1))
        .await
        .expect("the VM's own sender should reach the new mailbox");
    assert!(!returned.is_closed(), "the returned sender should be live");

    vm.run().await.expect("run should receive the message");
    assert_eq!(vm.stack(), &vec![Value::Integer(1)]);
}

#[tokio::test]
async fn restarting_keeps_the_standard_library_available() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::PushConst(Value::Integer(42)),
            OpCode::LoadGlobal("io".to_string()),
            OpCode::GetExport("print".to_string()),
            OpCode::CallNative(1),
        ],
        None,
    );

    let _tx = vm.reset_for_restart(0);
    vm.run()
        .await
        .expect("io.print should still resolve after a restart");
}

#[tokio::test]
async fn restarting_clears_the_stack_and_locals() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::PushConst(Value::Integer(1)),
            OpCode::PushConst(Value::Integer(2)),
            OpCode::StoreVar(0),
            OpCode::Return,
        ],
        None,
    );
    vm.run().await.expect("program should run");
    assert!(!vm.stack().is_empty(), "precondition: the stack has a value");

    let _tx = vm.reset_for_restart(0);
    assert!(vm.stack().is_empty(), "a restarted process starts empty");
}

#[tokio::test]
async fn restarting_clears_locals() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::PushConst(Value::Integer(1)),
            OpCode::StoreVar(0),
            OpCode::LoadVar(0),
        ],
        None,
    );
    vm.run().await.expect("program should run");

    // Rerunning from the LoadVar must now fail: the local is gone.
    let _tx = vm.reset_for_restart(2);
    assert!(
        vm.run().await.is_err(),
        "locals should not survive a restart"
    );
}

#[tokio::test]
async fn restarting_releases_values_the_process_was_holding() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::MakeString("held on the stack".to_string()),
            OpCode::MakeString("held in a local".to_string()),
            OpCode::StoreVar(0),
        ],
        None,
    );
    vm.run().await.expect("program should run");

    let stacked = match vm.stack().first() {
        Some(Value::Reference(address)) => *address,
        other => panic!("expected a string on the stack, got {other:?}"),
    };

    let _tx = vm.reset_for_restart(0);
    vm.collect_garbage();

    assert!(
        vm.heap_ref_count(stacked).is_none(),
        "values a restarted process was holding must become collectable"
    );
}

#[tokio::test]
async fn restarting_resumes_from_the_requested_instruction() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::PushConst(Value::Integer(1)),
            OpCode::PushConst(Value::Integer(2)),
        ],
        None,
    );

    let _tx = vm.reset_for_restart(1);
    assert_eq!(vm.current_ip(), 1);
    assert_eq!(
        vm.restart_ip(),
        1,
        "a further restart should return to the same instruction"
    );

    vm.run().await.expect("program should run");
    assert_eq!(
        vm.stack(),
        &vec![Value::Integer(2)],
        "execution should have skipped the first instruction"
    );
}
