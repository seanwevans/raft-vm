use raft::vm::opcodes::OpCode;
use raft::vm::supervision::ExitReason;
use raft::vm::value::Value;
use raft::vm::{VmError, VM};

/// A program that never halts on its own.
fn spin() -> Vec<OpCode> {
    vec![OpCode::Jump(0)]
}

#[tokio::test]
async fn a_vm_is_unbounded_by_default() {
    let (vm, _tx) = VM::new(vec![OpCode::Return], None);
    assert_eq!(vm.instruction_budget(), None);
}

#[tokio::test]
async fn a_program_that_fits_its_budget_runs_to_completion() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::PushConst(Value::Integer(5)),
            OpCode::PushConst(Value::Integer(3)),
            OpCode::Add,
        ],
        None,
    );
    vm.set_instruction_budget(Some(10));

    vm.run().await.expect("three instructions fit in ten");

    assert_eq!(vm.stack().last(), Some(&Value::Integer(8)));
}

#[tokio::test]
async fn an_endless_loop_stops_once_its_budget_is_spent() {
    let (mut vm, _tx) = VM::new(spin(), None);
    vm.set_instruction_budget(Some(512));

    let error = vm.run().await.expect_err("an endless loop cannot finish");

    assert!(
        matches!(error, VmError::InstructionBudgetExhausted(512)),
        "expected an exhausted budget, got {error:?}"
    );
}

#[tokio::test]
async fn a_budget_stops_the_program_at_the_instruction_it_runs_out() {
    // Four instructions, a budget of three: the fourth never runs, so the sum
    // is missing from the stack.
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::PushConst(Value::Integer(1)),
            OpCode::PushConst(Value::Integer(2)),
            OpCode::PushConst(Value::Integer(3)),
            OpCode::Add,
        ],
        None,
    );
    vm.set_instruction_budget(Some(3));

    let error = vm.run().await.expect_err("the budget runs out first");

    assert!(
        matches!(error, VmError::InstructionBudgetExhausted(3)),
        "expected an exhausted budget, got {error:?}"
    );
    assert_eq!(
        vm.stack(),
        &vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)],
        "the three instructions that fit should have run"
    );
}

#[tokio::test]
async fn a_program_that_fits_its_budget_exactly_still_finishes() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::PushConst(Value::Integer(1)),
            OpCode::PushConst(Value::Integer(2)),
            OpCode::Add,
        ],
        None,
    );
    vm.set_instruction_budget(Some(3));

    vm.run().await.expect("three instructions fit in three");

    assert_eq!(vm.stack().last(), Some(&Value::Integer(3)));
    assert_eq!(vm.instructions_executed(), 3);
}

#[tokio::test]
async fn a_run_counts_the_instructions_it_executed() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::PushConst(Value::Integer(0)),
            OpCode::Jump(2),
            OpCode::Pop,
        ],
        None,
    );

    vm.run().await.expect("the program halts");

    // PushConst, Jump, Pop -- reaching the end of the program is not an
    // instruction, and neither is the jump's destination being the last one.
    assert_eq!(vm.instructions_executed(), 3);
}

#[tokio::test]
async fn clearing_a_budget_makes_a_vm_unbounded_again() {
    let (mut vm, _tx) = VM::new(vec![OpCode::PushConst(Value::Integer(1))], None);
    vm.set_instruction_budget(Some(0));
    vm.run().await.expect_err("a budget of zero runs nothing");

    vm.set_instruction_budget(None);
    vm.set_ip(0);

    vm.run().await.expect("an unbounded VM runs the program");
    assert_eq!(vm.instruction_budget(), None);
}

/// A child that spins must not outlive the bound its parent runs under: the
/// budget travels with the spawn, and the child's exit reaches the parent
/// because spawning links the two.
#[tokio::test]
async fn a_spawned_actor_inherits_the_budget_of_the_process_that_spawned_it() {
    let code = vec![
        OpCode::SpawnActor(3),
        OpCode::ReceiveMessage,
        OpCode::Jump(4),
        // The child starts here and never halts on its own.
        OpCode::Jump(3),
    ];

    let (mut vm, _tx) = VM::new(code, None);
    vm.set_instruction_budget(Some(256));

    vm.run()
        .await
        .expect("the parent halts once the child's exit arrives");

    match vm.stack().last() {
        Some(Value::ExitSignal(signal)) => {
            assert_eq!(
                signal.reason,
                ExitReason::Error,
                "an exhausted budget is a runtime error"
            );
            assert_ne!(
                signal.from,
                vm.process_id(),
                "the exit should come from the child"
            );
        }
        other => panic!("expected the child's exit signal on the stack, got {other:?}"),
    }
}
