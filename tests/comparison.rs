//! Comparison and logic opcodes, and the loops they make possible.

use raft::vm::opcodes::OpCode;
use raft::vm::value::Value;
use raft::vm::{VmError, VM};
use raft::Compiler;

async fn eval(source: &str) -> Vec<Value> {
    let bytecode = Compiler::compile(source)
        .unwrap_or_else(|error| panic!("{source:?} should compile: {error}"));
    let (mut vm, _tx) = VM::new(bytecode, None);
    vm.run()
        .await
        .unwrap_or_else(|error| panic!("{source:?} should run: {error}"));
    vm.stack().clone()
}

async fn eval_error(source: &str) -> VmError {
    let bytecode = Compiler::compile(source).expect("source should compile");
    let (mut vm, _tx) = VM::new(bytecode, None);
    vm.run().await.expect_err("expected this program to fail")
}

async fn boolean(source: &str) -> bool {
    match eval(source).await.as_slice() {
        [Value::Boolean(value)] => *value,
        other => panic!("{source:?} should leave one boolean, got {other:?}"),
    }
}

fn innermost(error: &VmError) -> &VmError {
    match error {
        VmError::RuntimeError { source, .. } => innermost(source),
        other => other,
    }
}

// --- ordering --------------------------------------------------------------

#[tokio::test]
async fn integers_compare_in_every_direction() {
    for (source, expected) in [
        ("1 2 Lt", true),
        ("2 1 Lt", false),
        ("2 2 Lt", false),
        ("2 2 Le", true),
        ("3 2 Le", false),
        ("3 2 Gt", true),
        ("2 3 Gt", false),
        ("2 2 Ge", true),
        ("1 2 Ge", false),
    ] {
        assert_eq!(boolean(source).await, expected, "{source}");
    }
}

#[tokio::test]
async fn floats_compare_in_every_direction() {
    for (source, expected) in [
        ("1.5 2.5 Lt", true),
        ("2.5 1.5 Gt", true),
        ("2.5 2.5 Le", true),
        ("2.5 2.5 Ge", true),
    ] {
        assert_eq!(boolean(source).await, expected, "{source}");
    }
}

#[tokio::test]
async fn negative_operands_order_correctly() {
    assert!(boolean("0 5 Sub 0 Lt").await, "0 - 5 should be below zero");
}

#[tokio::test]
async fn ordering_requires_matching_numeric_types() {
    for source in ["1 2.0 Lt", "true false Lt", "1 true Gt"] {
        let error = eval_error(source).await;
        assert!(
            matches!(innermost(&error), VmError::TypeMismatch(_)),
            "{source} should be a type error, got {error:?}"
        );
    }
}

// --- equality --------------------------------------------------------------

#[tokio::test]
async fn equality_works_within_each_type() {
    for (source, expected) in [
        ("1 1 Eq", true),
        ("1 2 Eq", false),
        ("1 2 Ne", true),
        ("1.5 1.5 Eq", true),
        ("true true Eq", true),
        ("true false Eq", false),
        ("true false Ne", true),
    ] {
        assert_eq!(boolean(source).await, expected, "{source}");
    }
}

/// Unlike ordering, equality is total: comparing across types answers `false`
/// rather than failing, so a program can test a value without knowing its type.
#[tokio::test]
async fn equality_across_types_is_false_rather_than_an_error() {
    assert!(!boolean("1 true Eq").await);
    assert!(boolean("1 true Ne").await);
    assert!(!boolean("1 1.0 Eq").await);
}

#[tokio::test]
async fn references_compare_by_identity() {
    // Dup leaves the same address twice, so the two are equal.
    assert!(
        boolean(r#""same" Dup Eq"#).await,
        "a reference should equal itself"
    );
    // Two separately allocated strings with equal contents are distinct objects.
    assert!(
        !boolean(r#""same" "same" Eq"#).await,
        "distinct allocations should not be equal"
    );
}

// --- logic -----------------------------------------------------------------

#[tokio::test]
async fn logical_operators_combine_booleans() {
    for (source, expected) in [
        ("true Not", false),
        ("false Not", true),
        ("true true And", true),
        ("true false And", false),
        ("false false Or", false),
        ("true false Or", true),
    ] {
        assert_eq!(boolean(source).await, expected, "{source}");
    }
}

#[tokio::test]
async fn logical_operators_reject_non_booleans() {
    for source in ["1 Not", "1 true And", "true 1 Or"] {
        let error = eval_error(source).await;
        assert!(
            matches!(innermost(&error), VmError::TypeMismatch(_)),
            "{source} should be a type error, got {error:?}"
        );
    }
}

#[tokio::test]
async fn comparisons_and_logic_compose() {
    assert!(boolean("1 2 Lt 3 2 Gt And").await);
    assert!(!boolean("1 2 Gt 3 2 Lt Or").await);
    assert!(boolean("1 2 Gt Not").await);
}

// --- what this unlocks -----------------------------------------------------

/// Before this change no loop could ever terminate on a computed value:
/// `JumpIfFalse` needs a boolean, and nothing but `PushConst` produced one, so
/// every branch was decided when the program was written.
#[tokio::test]
async fn a_counted_loop_terminates() {
    let sum = eval(
        r#"
        # sum = 0, counter = 5
        0 StoreVar 0
        5 StoreVar 1

        .loop
        LoadVar 1 0 Gt
        JumpIfFalse .done
          LoadVar 0 LoadVar 1 + StoreVar 0
          LoadVar 1 1 - StoreVar 1
        Jump .loop

        .done
        LoadVar 0
        "#,
    )
    .await;

    assert_eq!(sum, vec![Value::Integer(15)], "5 + 4 + 3 + 2 + 1");
}

#[tokio::test]
async fn a_loop_body_can_be_skipped_entirely() {
    let sum = eval(
        r#"
        0 StoreVar 0
        0 StoreVar 1

        .loop
        LoadVar 1 0 Gt
        JumpIfFalse .done
          LoadVar 0 LoadVar 1 + StoreVar 0
          LoadVar 1 1 - StoreVar 1
        Jump .loop

        .done
        LoadVar 0
        "#,
    )
    .await;

    assert_eq!(
        sum,
        vec![Value::Integer(0)],
        "a zero count runs no iterations"
    );
}

/// A recursive subroutine needs a computed base case, which was equally
/// impossible before.
#[tokio::test]
async fn a_recursive_subroutine_reaches_its_base_case() {
    let result = eval(
        r#"
        # factorial(5) via a recursive Call, accumulating in slot 0
        1 StoreVar 0
        5 StoreVar 1
        Call .factorial
        Jump .end

        .factorial
        LoadVar 1 1 Gt
        JumpIfFalse .base
          LoadVar 0 LoadVar 1 * StoreVar 0
          LoadVar 1 1 - StoreVar 1
          Call .factorial
        .base
        Return

        .end
        LoadVar 0
        "#,
    )
    .await;

    assert_eq!(result, vec![Value::Integer(120)], "5!");
}

#[tokio::test]
async fn a_comparison_can_drive_a_branch_on_received_data() {
    // The branch depends on a value the program computed, not a literal.
    let taken = eval(
        r#"
        7 StoreVar 0
        LoadVar 0 5 Gt
        JumpIfFalse .small
          1
        Jump .end
        .small
          0
        .end
        "#,
    )
    .await;

    assert_eq!(taken, vec![Value::Integer(1)]);
}

// --- compilation -----------------------------------------------------------

#[test]
fn every_comparison_keyword_compiles() {
    let bytecode = Compiler::compile("1 1 Eq 1 1 Ne 1 1 Lt 1 1 Le 1 1 Gt 1 1 Ge")
        .expect("comparison keywords should compile");
    let operators: Vec<&OpCode> = bytecode
        .iter()
        .filter(|opcode| !matches!(opcode, OpCode::PushConst(_)))
        .collect();
    assert!(matches!(
        operators.as_slice(),
        [
            OpCode::Eq,
            OpCode::Ne,
            OpCode::Lt,
            OpCode::Le,
            OpCode::Gt,
            OpCode::Ge
        ]
    ));
}

#[test]
fn every_logic_keyword_compiles() {
    let bytecode =
        Compiler::compile("true Not true true And true true Or").expect("logic should compile");
    let operators: Vec<&OpCode> = bytecode
        .iter()
        .filter(|opcode| !matches!(opcode, OpCode::PushConst(_)))
        .collect();
    assert!(matches!(
        operators.as_slice(),
        [OpCode::Not, OpCode::And, OpCode::Or]
    ));
}

// --- stack discipline ------------------------------------------------------

#[tokio::test]
async fn comparisons_underflow_on_a_short_stack() {
    for opcode in [
        OpCode::Eq,
        OpCode::Ne,
        OpCode::Lt,
        OpCode::Le,
        OpCode::Gt,
        OpCode::Ge,
        OpCode::And,
        OpCode::Or,
    ] {
        let (mut vm, _tx) = VM::new(
            vec![OpCode::PushConst(Value::Integer(1)), opcode.clone()],
            None,
        );
        let error = vm
            .run()
            .await
            .expect_err("a binary operator with one operand should underflow");
        assert!(
            matches!(innermost(&error), VmError::StackUnderflow),
            "{opcode:?} should underflow, got {error:?}"
        );
    }
}

#[tokio::test]
async fn not_underflows_on_an_empty_stack() {
    let (mut vm, _tx) = VM::new(vec![OpCode::Not], None);
    assert!(matches!(
        innermost(&vm.run().await.expect_err("Not should underflow")),
        VmError::StackUnderflow
    ));
}

#[tokio::test]
async fn a_comparison_consumes_both_operands() {
    assert_eq!(eval("9 1 2 Lt").await.len(), 2, "one spare plus the result");
}
