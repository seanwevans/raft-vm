use raft::vm::execution::ExecutionContext;
use raft::vm::heap::{Heap, HeapObject};
use raft::vm::{ChildSpec, OpCode, Value, VmError};

/// Spawn a supervisor plus `count` actors, leaving the supervisor reference on
/// the stack. Every actor is stepped off its start ip so a restart is visible.
fn supervisor_with_children(count: usize) -> (ExecutionContext, Heap, usize, Vec<usize>) {
    let mut execution = ExecutionContext::new(vec![OpCode::Return, OpCode::Return]);
    let mut heap = Heap::new();

    OpCode::SpawnSupervisor(0)
        .execute(&mut execution, &mut heap)
        .expect("spawn supervisor");
    let supervisor = match execution.stack.last().cloned() {
        Some(Value::Reference(address)) => address,
        other => panic!("expected supervisor reference, got {other:?}"),
    };

    let mut children = Vec::with_capacity(count);
    for _ in 0..count {
        OpCode::SpawnActor(0)
            .execute(&mut execution, &mut heap)
            .expect("spawn actor");
        let child = match execution.stack.pop() {
            Some(Value::Reference(address)) => address,
            other => panic!("expected actor reference, got {other:?}"),
        };
        match heap.get_mut(child).expect("child should exist") {
            HeapObject::Actor(actor, _, _) => actor.set_ip(1),
            other => panic!("expected actor, got {other:?}"),
        }
        children.push(child);
    }

    (execution, heap, supervisor, children)
}

fn supervise(execution: &mut ExecutionContext, heap: &mut Heap, child: usize) {
    OpCode::SuperviseChild(child)
        .execute(execution, heap)
        .expect("supervising a child should succeed");
}

fn current_ip(heap: &Heap, child: usize) -> usize {
    match heap.get(child).expect("child should exist") {
        HeapObject::Actor(actor, _, _) => actor.current_ip(),
        other => panic!("expected actor, got {other:?}"),
    }
}

fn restarted(heap: &Heap, child: usize) -> bool {
    current_ip(heap, child) == 0
}

#[tokio::test]
async fn supervise_child_records_the_child_on_the_supervisor() {
    let (mut execution, mut heap, supervisor, children) = supervisor_with_children(2);

    supervise(&mut execution, &mut heap, children[0]);
    supervise(&mut execution, &mut heap, children[1]);

    match heap.get(supervisor).expect("supervisor should exist") {
        HeapObject::Supervisor(process, _, _) => assert_eq!(
            process.supervised_children(),
            &[
                ChildSpec {
                    reference: children[0],
                    start_ip: 0
                },
                ChildSpec {
                    reference: children[1],
                    start_ip: 0
                },
            ]
        ),
        other => panic!("expected supervisor, got {other:?}"),
    }
}

#[tokio::test]
async fn supervise_child_is_idempotent() {
    let (mut execution, mut heap, supervisor, children) = supervisor_with_children(1);

    supervise(&mut execution, &mut heap, children[0]);
    supervise(&mut execution, &mut heap, children[0]);

    match heap.get(supervisor).expect("supervisor should exist") {
        HeapObject::Supervisor(process, _, _) => assert_eq!(
            process.supervised_children().len(),
            1,
            "registering the same child twice should not duplicate it"
        ),
        other => panic!("expected supervisor, got {other:?}"),
    }
}

#[tokio::test]
async fn one_for_all_restarts_registered_siblings() {
    let (mut execution, mut heap, supervisor, children) = supervisor_with_children(3);

    OpCode::SetStrategy(1)
        .execute(&mut execution, &mut heap)
        .expect("set one-for-all");
    for child in &children {
        supervise(&mut execution, &mut heap, *child);
    }

    // Only the first child fails; one-for-all should take all three down.
    OpCode::RestartChild(children[0])
        .execute(&mut execution, &mut heap)
        .expect("restart should succeed");

    for child in &children {
        assert!(
            restarted(&heap, *child),
            "one-for-all should restart every registered child, {child} was not restarted"
        );
    }
    let _ = supervisor;
}

#[tokio::test]
async fn rest_for_one_restarts_the_child_and_later_siblings_only() {
    let (mut execution, mut heap, _supervisor, children) = supervisor_with_children(3);

    OpCode::SetStrategy(2)
        .execute(&mut execution, &mut heap)
        .expect("set rest-for-one");
    for child in &children {
        supervise(&mut execution, &mut heap, *child);
    }

    OpCode::RestartChild(children[1])
        .execute(&mut execution, &mut heap)
        .expect("restart should succeed");

    assert!(
        !restarted(&heap, children[0]),
        "a child registered before the failure should be left alone"
    );
    assert!(restarted(&heap, children[1]), "the failed child restarts");
    assert!(
        restarted(&heap, children[2]),
        "children registered after the failure restart too"
    );
}

#[tokio::test]
async fn one_for_one_leaves_siblings_running() {
    let (mut execution, mut heap, _supervisor, children) = supervisor_with_children(3);

    OpCode::SetStrategy(0)
        .execute(&mut execution, &mut heap)
        .expect("set one-for-one");
    for child in &children {
        supervise(&mut execution, &mut heap, *child);
    }

    OpCode::RestartChild(children[1])
        .execute(&mut execution, &mut heap)
        .expect("restart should succeed");

    assert!(restarted(&heap, children[1]), "the failed child restarts");
    assert!(!restarted(&heap, children[0]), "siblings keep running");
    assert!(!restarted(&heap, children[2]), "siblings keep running");
}

#[tokio::test]
async fn supervise_child_rejects_addresses_that_are_not_actors() {
    let (mut execution, mut heap, _supervisor, _children) = supervisor_with_children(0);
    let not_an_actor = heap.allocate(HeapObject::String("not an actor".to_string(), 1));

    let error = OpCode::SuperviseChild(not_an_actor)
        .execute(&mut execution, &mut heap)
        .expect_err("supervising a non-actor should fail");
    assert!(matches!(error, VmError::InvalidReference));
}

#[tokio::test]
async fn supervise_child_leaves_the_supervisor_on_the_stack() {
    let (mut execution, mut heap, supervisor, children) = supervisor_with_children(1);

    supervise(&mut execution, &mut heap, children[0]);

    assert_eq!(
        execution.stack.last(),
        Some(&Value::Reference(supervisor)),
        "SuperviseChild should leave the supervisor reference for chaining"
    );
    match heap.get(supervisor).expect("supervisor should exist") {
        HeapObject::Supervisor(_, _, rc) => assert_eq!(*rc, 1),
        other => panic!("expected supervisor, got {other:?}"),
    }
}

#[test]
fn compiler_accepts_supervise_child() {
    let bytecode = raft::Compiler::compile("SpawnSupervisor 0 SuperviseChild 3")
        .expect("SuperviseChild should compile");
    assert!(matches!(
        bytecode.as_slice(),
        [OpCode::SpawnSupervisor(0), OpCode::SuperviseChild(3)]
    ));
}
