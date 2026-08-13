use raft::vm::opcodes::OpCode;
use raft::vm::value::Value;
use raft::vm::VM;
use raft::Compiler;

fn compile(source: &str) -> Vec<OpCode> {
    Compiler::compile(source).unwrap_or_else(|error| panic!("{source:?} should compile: {error}"))
}

async fn eval(source: &str) -> Vec<Value> {
    let (mut vm, _tx) = VM::new(compile(source), None);
    vm.run()
        .await
        .unwrap_or_else(|error| panic!("{source:?} should run: {error}"));
    vm.stack().clone()
}

#[test]
fn a_negative_integer_is_a_literal() {
    assert!(matches!(
        compile("-5").as_slice(),
        [OpCode::PushConst(Value::Integer(-5))]
    ));
}

#[test]
fn a_negative_float_is_a_literal() {
    match compile("-3.14").as_slice() {
        [OpCode::PushConst(Value::Float(value))] => assert!((value + 3.14).abs() < f64::EPSILON),
        other => panic!("expected a single float literal, got {other:?}"),
    }
}

#[test]
fn the_most_negative_integer_is_a_literal() {
    assert!(matches!(
        compile("-2147483648").as_slice(),
        [OpCode::PushConst(Value::Integer(i32::MIN))]
    ));
}

#[test]
fn a_spaced_minus_is_still_subtraction() {
    assert!(matches!(
        compile("5 3 -").as_slice(),
        [
            OpCode::PushConst(Value::Integer(5)),
            OpCode::PushConst(Value::Integer(3)),
            OpCode::Sub,
        ]
    ));
}

#[test]
fn a_minus_before_a_non_digit_is_still_subtraction() {
    assert!(matches!(
        compile("5 3 - Neg").as_slice(),
        [
            OpCode::PushConst(Value::Integer(5)),
            OpCode::PushConst(Value::Integer(3)),
            OpCode::Sub,
            OpCode::Neg,
        ]
    ));
}

#[test]
fn a_trailing_minus_is_still_subtraction() {
    assert!(matches!(compile("-").as_slice(), [OpCode::Sub]));
}

#[test]
fn existing_arithmetic_still_compiles_the_same_way() {
    // The shipped arithmetic example, which spaces every operator.
    assert!(matches!(
        compile("5 3 - 2 * 4 / 10 3 % Neg").as_slice(),
        [
            OpCode::PushConst(Value::Integer(5)),
            OpCode::PushConst(Value::Integer(3)),
            OpCode::Sub,
            OpCode::PushConst(Value::Integer(2)),
            OpCode::Mul,
            OpCode::PushConst(Value::Integer(4)),
            OpCode::Div,
            OpCode::PushConst(Value::Integer(10)),
            OpCode::PushConst(Value::Integer(3)),
            OpCode::Mod,
            OpCode::Neg,
        ]
    ));
}

#[tokio::test]
async fn adding_a_negative_literal_evaluates_correctly() {
    // Previously compiled to [Push 3, Sub, Push 5, Add]: a silent
    // miscompilation that underflowed the stack at runtime.
    assert_eq!(eval("3 -5 +").await, vec![Value::Integer(-2)]);
}

#[tokio::test]
async fn a_negative_literal_can_be_stored_and_reloaded() {
    assert_eq!(
        eval("-7 StoreVar 0 LoadVar 0").await,
        vec![Value::Integer(-7)]
    );
}

#[tokio::test]
async fn negation_and_negative_literals_agree() {
    assert_eq!(eval("5 Neg").await, eval("-5").await);
}

#[tokio::test]
async fn subtraction_still_evaluates_correctly() {
    assert_eq!(eval("5 3 -").await, vec![Value::Integer(2)]);
}
