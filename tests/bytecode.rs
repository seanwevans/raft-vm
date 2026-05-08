use raft::vm::{Bytecode, OpCode, Value};

#[test]
fn bytecode_encodes_simple_opcodes_as_single_bytes() {
    let bytecode = Bytecode::from_opcodes(vec![OpCode::Add, OpCode::Dup, OpCode::Pop]);

    assert_eq!(bytecode.instruction_len(), 3);
    assert_eq!(bytecode.bytes().len(), 3);
    assert!(bytecode.constants().is_empty());
}

#[test]
fn push_const_uses_constant_pool_operand_instead_of_inline_value() {
    let bytecode = Bytecode::from_opcodes(vec![OpCode::PushConst(Value::Integer(42))]);

    assert_eq!(bytecode.instruction_len(), 1);
    assert_eq!(bytecode.bytes().len(), 5);
    assert_eq!(bytecode.constants(), &[Value::Integer(42)]);
    assert_eq!(
        bytecode.decode_at(0).unwrap(),
        OpCode::PushConst(Value::Integer(42))
    );
}
