use raft::vm::opcodes::{Bytecode, BytecodeConstant, OpCode};
use raft::vm::value::Value;

#[test]
fn simple_opcodes_use_single_byte_encoding() {
    let bytecode = Bytecode::new(vec![OpCode::Add, OpCode::Pop, OpCode::Return]);

    assert_eq!(bytecode.instructions().len(), 3);
    assert_eq!(bytecode.offsets(), &[0, 1, 2]);
    assert!(matches!(bytecode.decode(0).unwrap(), OpCode::Add));
    assert!(matches!(bytecode.decode(1).unwrap(), OpCode::Pop));
    assert!(matches!(bytecode.decode(2).unwrap(), OpCode::Return));
}

#[test]
fn push_const_uses_constant_pool_operand() {
    let bytecode = Bytecode::new(vec![OpCode::PushConst(Value::Integer(42))]);

    assert_eq!(bytecode.instructions().len(), 5);
    assert_eq!(bytecode.constants().len(), 1);
    assert!(matches!(
        bytecode.constants().first(),
        Some(BytecodeConstant::Value(Value::Integer(42)))
    ));
    assert!(matches!(
        bytecode.decode(0).unwrap(),
        OpCode::PushConst(Value::Integer(42))
    ));
}
