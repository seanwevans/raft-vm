use raft::compiler::Compiler;
use raft::vm::opcodes::OpCode;
use raft::vm::value::Value;

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
