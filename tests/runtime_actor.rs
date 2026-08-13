//! Coverage for the `runtime::Actor` wrapper, which had none.

use raft::vm::opcodes::OpCode;
use raft::vm::value::{MessageValue, Value};
use raft::vm::{ExitReason, VmError};
use raft::Actor;

fn halting_actor() -> Actor {
    Actor::new(vec![OpCode::Return])
}

#[tokio::test]
async fn each_actor_gets_its_own_process_id() {
    let first = halting_actor();
    let second = halting_actor();
    assert_ne!(first.process_id(), second.process_id());
    assert!(second.process_id() > first.process_id());
}

#[tokio::test]
async fn an_actor_runs_its_bytecode() {
    let mut actor = Actor::new(vec![
        OpCode::PushConst(Value::Integer(2)),
        OpCode::PushConst(Value::Integer(3)),
        OpCode::Add,
    ]);
    actor.run().await.expect("arithmetic should succeed");
}

#[tokio::test]
async fn a_failing_actor_reports_its_error() {
    let mut actor = Actor::new(vec![
        OpCode::PushConst(Value::Integer(1)),
        OpCode::PushConst(Value::Integer(0)),
        OpCode::Div,
    ]);
    let error = actor.run().await.expect_err("division by zero should fail");
    assert!(matches!(error, VmError::DivisionByZero));
}

#[tokio::test]
async fn a_sent_value_arrives_in_the_mailbox() {
    let mut actor = halting_actor();
    actor
        .send(Value::Integer(42))
        .await
        .expect("sending should succeed");

    assert_eq!(actor.handle_next_message().await, Some(Value::Integer(42)));
}

#[tokio::test]
async fn every_scalar_round_trips_through_the_mailbox() {
    let mut actor = halting_actor();
    for value in [
        Value::Integer(-1),
        Value::Float(2.5),
        Value::Boolean(true),
        Value::Null,
    ] {
        actor
            .send(value.clone())
            .await
            .expect("sending should succeed");
        assert_eq!(actor.handle_next_message().await, Some(value));
    }
}

#[tokio::test]
async fn the_sender_handle_reaches_the_same_mailbox() {
    let mut actor = halting_actor();
    actor
        .sender()
        .send(MessageValue::Boolean(true))
        .await
        .expect("the sender handle should deliver");

    assert_eq!(
        actor.handle_next_message().await,
        Some(Value::Boolean(true))
    );
}

/// `handle_next_message` only maps scalars back to values; a structured
/// message is dropped rather than materialized onto the actor's heap.
#[tokio::test]
async fn a_structured_message_yields_nothing() {
    let mut actor = halting_actor();
    actor
        .sender()
        .send(MessageValue::Array(vec![MessageValue::Integer(1)]))
        .await
        .expect("the sender handle should deliver");

    assert_eq!(actor.handle_next_message().await, None);
}

#[tokio::test]
async fn sending_a_dangling_reference_fails() {
    let actor = halting_actor();
    let error = actor
        .send(Value::Reference(9_999))
        .await
        .expect_err("a reference to nothing should not be sendable");
    assert!(matches!(error, VmError::InvalidReference));
}

#[tokio::test]
async fn a_linked_actor_receives_an_exit_signal() {
    let mut observer = halting_actor();
    let mut failing = Actor::new(vec![
        OpCode::PushConst(Value::Integer(1)),
        OpCode::PushConst(Value::Integer(0)),
        OpCode::Div,
    ]);
    let failing_id = failing.process_id();

    observer.link_to(&mut failing);
    failing
        .run()
        .await
        .expect_err("the linked actor should fail");

    match observer.handle_next_message().await {
        Some(Value::ExitSignal(signal)) => {
            assert_eq!(signal.from, failing_id);
            assert_eq!(signal.reason, ExitReason::DivisionByZero);
        }
        other => panic!("expected an exit signal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_monitoring_actor_receives_an_exit_signal() {
    let mut observer = halting_actor();
    let mut failing = Actor::new(vec![
        OpCode::PushConst(Value::Integer(1)),
        OpCode::PushConst(Value::Boolean(true)),
        OpCode::Add,
    ]);

    observer.monitor(&mut failing);
    failing
        .run()
        .await
        .expect_err("the monitored actor should fail");

    match observer.handle_next_message().await {
        Some(Value::ExitSignal(signal)) => assert_eq!(signal.reason, ExitReason::TypeMismatch),
        other => panic!("expected an exit signal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_successful_actor_sends_no_exit_signal() {
    let mut observer = halting_actor();
    let mut succeeding = Actor::new(vec![OpCode::PushConst(Value::Integer(1))]);

    observer.link_to(&mut succeeding);
    succeeding.run().await.expect("the actor should succeed");

    observer
        .send(Value::Integer(7))
        .await
        .expect("sending should succeed");
    assert_eq!(
        observer.handle_next_message().await,
        Some(Value::Integer(7)),
        "a clean exit should not put a signal ahead of this message"
    );
}
