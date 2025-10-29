use raft::compiler::{Compiler, CompilerError};
use raft::run;
use raft::vm::opcodes::OpCode;
use raft::vm::value::Value;
use raft::vm::VmError;

#[test]
fn compile_arithmetic_tokens() {
    let source = "5 3 - 2 * 4 / 10 3 % Neg";
    let bytecode = Compiler::compile(source).unwrap();
    assert_eq!(bytecode.len(), 11);
    assert!(matches!(bytecode[0], OpCode::PushConst(Value::Integer(5))));
    assert!(matches!(bytecode[1], OpCode::PushConst(Value::Integer(3))));
    assert!(matches!(bytecode[2], OpCode::Sub));
    assert!(matches!(bytecode[3], OpCode::PushConst(Value::Integer(2))));
    assert!(matches!(bytecode[4], OpCode::Mul));
    assert!(matches!(bytecode[5], OpCode::PushConst(Value::Integer(4))));
    assert!(matches!(bytecode[6], OpCode::Div));
    assert!(matches!(bytecode[7], OpCode::PushConst(Value::Integer(10))));
    assert!(matches!(bytecode[8], OpCode::PushConst(Value::Integer(3))));
    assert!(matches!(bytecode[9], OpCode::Mod));
    assert!(matches!(bytecode[10], OpCode::Neg));
}

#[test]
fn compile_control_flow_tokens() {
    let source = "1 JumpIfFalse 4 Call 6 Jump 8 Return";
    let bytecode = Compiler::compile(source).unwrap();
    assert_eq!(bytecode.len(), 5);
    assert!(matches!(bytecode[0], OpCode::PushConst(Value::Integer(1))));
    assert!(matches!(bytecode[1], OpCode::JumpIfFalse(4)));
    assert!(matches!(bytecode[2], OpCode::Call(6)));
    assert!(matches!(bytecode[3], OpCode::Jump(8)));
    assert!(matches!(bytecode[4], OpCode::Return));
}

#[test]
fn compile_float_tokens() {
    let source = "3.14 2.0 +";
    let bytecode = Compiler::compile(source).unwrap();
    assert_eq!(bytecode.len(), 3);
    assert!(
        matches!(bytecode[0], OpCode::PushConst(Value::Float(f)) if (f - 3.14).abs() < f64::EPSILON)
    );
    assert!(
        matches!(bytecode[1], OpCode::PushConst(Value::Float(f)) if (f - 2.0).abs() < f64::EPSILON)
    );
    assert!(matches!(bytecode[2], OpCode::Add));
}

#[test]
fn compile_boolean_tokens() {
    let source = "true false";
    let bytecode = Compiler::compile(source).unwrap();
    assert_eq!(bytecode.len(), 2);
    assert!(matches!(
        bytecode[0],
        OpCode::PushConst(Value::Boolean(true))
    ));
    assert!(matches!(
        bytecode[1],
        OpCode::PushConst(Value::Boolean(false))
    ));
}

#[test]
fn compile_variable_and_stack_tokens() {
    let source = "1 StoreVar 0 LoadVar 0 Dup Swap Pop";
    let bytecode = Compiler::compile(source).unwrap();
    assert_eq!(bytecode.len(), 6);
    assert!(matches!(bytecode[0], OpCode::PushConst(Value::Integer(1))));
    assert!(matches!(bytecode[1], OpCode::StoreVar(0)));
    assert!(matches!(bytecode[2], OpCode::LoadVar(0)));
    assert!(matches!(bytecode[3], OpCode::Dup));
    assert!(matches!(bytecode[4], OpCode::Swap));
    assert!(matches!(bytecode[5], OpCode::Pop));
}

#[test]
fn variable_tokens_reject_invalid_operands() {
    let missing_store = Compiler::compile("StoreVar").unwrap_err();
    assert!(matches!(
        missing_store,
        CompilerError::InvalidAddress(msg) if msg.contains("StoreVar")
    ));

    let missing_load = Compiler::compile("LoadVar").unwrap_err();
    assert!(matches!(
        missing_load,
        CompilerError::InvalidAddress(msg) if msg.contains("LoadVar")
    ));

    let invalid_index = Compiler::compile("LoadVar foo").unwrap_err();
    assert!(matches!(
        invalid_index,
        CompilerError::InvalidAddress(msg) if msg == "foo"
    ));
}

#[test]
fn compile_actor_and_supervisor_tokens() {
    let source =
        "SpawnActor 4 SendMessage ReceiveMessage SpawnSupervisor 8 SetStrategy 2 RestartChild 1";
    let bytecode = Compiler::compile(source).unwrap();
    assert_eq!(bytecode.len(), 6);
    assert!(matches!(bytecode[0], OpCode::SpawnActor(4)));
    assert!(matches!(bytecode[1], OpCode::SendMessage));
    assert!(matches!(bytecode[2], OpCode::ReceiveMessage));
    assert!(matches!(bytecode[3], OpCode::SpawnSupervisor(8)));
    assert!(matches!(bytecode[4], OpCode::SetStrategy(2)));
    assert!(matches!(bytecode[5], OpCode::RestartChild(1)));
}

#[test]
fn invalid_token_returns_error() {
    let err = Compiler::compile("1 foo").unwrap_err();
    assert!(matches!(err, CompilerError::InvalidToken(t) if t == "foo"));
}

#[test]
fn invalid_address_returns_error() {
    let err = Compiler::compile("Jump abc").unwrap_err();
    assert!(matches!(err, CompilerError::InvalidAddress(a) if a == "abc"));
}

#[test]
fn parse_error_returns_error() {
    let err = Compiler::compile("1..").unwrap_err();
    assert!(matches!(err, CompilerError::ParseError(m) if m.contains("1..")));
}

#[tokio::test]
async fn run_propagates_compiler_error() {
    let err = run("bogus").await.expect_err("expected compile error");
    assert!(matches!(
        err,
        VmError::CompilationError(CompilerError::InvalidToken(_))
    ));
}
