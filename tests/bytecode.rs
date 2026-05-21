use raft::vm::opcodes::{Bytecode, OpCode};
use raft::vm::value::Value;

#[test]
fn simple_opcodes_round_trip() {
    let bytecode = Bytecode::new(vec![OpCode::Add, OpCode::Pop, OpCode::Return]);

    assert!(matches!(*bytecode.decode(0).unwrap(), OpCode::Add));
    assert!(matches!(*bytecode.decode(1).unwrap(), OpCode::Pop));
    assert!(matches!(*bytecode.decode(2).unwrap(), OpCode::Return));
}

#[test]
fn push_const_round_trips_without_reencoding() {
    let bytecode = Bytecode::new(vec![OpCode::PushConst(Value::Integer(42))]);

    assert!(matches!(
        *bytecode.decode(0).unwrap(),
        OpCode::PushConst(Value::Integer(42))
    ));
}

#[test]
fn array_and_string_adjacent_opcodes_round_trip() {
    let bytecode = Bytecode::new(vec![
        OpCode::ArrayGet,
        OpCode::ArraySet,
        OpCode::MakeString("hello".to_string()),
    ]);

    assert!(matches!(*bytecode.decode(0).unwrap(), OpCode::ArrayGet));
    assert!(matches!(*bytecode.decode(1).unwrap(), OpCode::ArraySet));
    assert!(matches!(
        bytecode.decode(2).unwrap().as_ref(),
        OpCode::MakeString(value) if value == "hello"
    ));
}
