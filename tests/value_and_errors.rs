//! Arithmetic on `Value`, error conversions, and the small supervision types.

use raft::compiler::{CompilerError, SourceLocation};
use raft::vm::value::Value;
use raft::vm::{ExitReason, SupervisorStrategy, VmError};

#[test]
fn integer_arithmetic_is_checked_in_both_directions() {
    assert_eq!(
        Value::Integer(2).checked_add(Value::Integer(3)).unwrap(),
        Value::Integer(5)
    );
    assert_eq!(
        Value::Integer(2).checked_sub(Value::Integer(3)).unwrap(),
        Value::Integer(-1)
    );
    assert_eq!(
        Value::Integer(6).checked_mul(Value::Integer(7)).unwrap(),
        Value::Integer(42)
    );
    assert_eq!(
        Value::Integer(7).checked_div(Value::Integer(2)).unwrap(),
        Value::Integer(3)
    );

    for overflowing in [
        Value::Integer(i32::MAX).checked_add(Value::Integer(1)),
        Value::Integer(i32::MIN).checked_sub(Value::Integer(1)),
        Value::Integer(i32::MAX).checked_mul(Value::Integer(2)),
    ] {
        assert!(matches!(overflowing, Err(VmError::IntegerOverflow)));
    }
}

#[test]
fn float_arithmetic_works_on_matching_operands() {
    match Value::Float(1.5).checked_add(Value::Float(2.0)).unwrap() {
        Value::Float(value) => assert!((value - 3.5).abs() < f64::EPSILON),
        other => panic!("expected a float, got {other:?}"),
    }
    match Value::Float(1.5).checked_sub(Value::Float(2.0)).unwrap() {
        Value::Float(value) => assert!((value + 0.5).abs() < f64::EPSILON),
        other => panic!("expected a float, got {other:?}"),
    }
    match Value::Float(1.5).checked_mul(Value::Float(2.0)).unwrap() {
        Value::Float(value) => assert!((value - 3.0).abs() < f64::EPSILON),
        other => panic!("expected a float, got {other:?}"),
    }
    match Value::Float(3.0).checked_div(Value::Float(2.0)).unwrap() {
        Value::Float(value) => assert!((value - 1.5).abs() < f64::EPSILON),
        other => panic!("expected a float, got {other:?}"),
    }
}

#[test]
fn division_by_zero_is_rejected_for_both_numeric_types() {
    assert!(matches!(
        Value::Integer(1).checked_div(Value::Integer(0)),
        Err(VmError::DivisionByZero)
    ));
    assert!(matches!(
        Value::Float(1.0).checked_div(Value::Float(0.0)),
        Err(VmError::DivisionByZero)
    ));
}

#[test]
fn mixed_operand_types_are_rejected() {
    assert!(matches!(
        Value::Integer(1).checked_add(Value::Float(1.0)),
        Err(VmError::TypeMismatch("Add"))
    ));
    assert!(matches!(
        Value::Boolean(true).checked_sub(Value::Integer(1)),
        Err(VmError::TypeMismatch("Sub"))
    ));
    assert!(matches!(
        Value::Null.checked_mul(Value::Integer(1)),
        Err(VmError::TypeMismatch("Mul"))
    ));
    assert!(matches!(
        Value::Reference(0).checked_div(Value::Integer(1)),
        Err(VmError::TypeMismatch("Div"))
    ));
}

#[test]
fn errors_can_be_built_from_strings() {
    assert!(matches!(
        VmError::from("borrowed".to_string()),
        VmError::Message(message) if message == "borrowed"
    ));
    assert!(matches!(
        VmError::from("literal"),
        VmError::Message(message) if message == "literal"
    ));
}

#[test]
fn a_failed_channel_send_carries_the_value_it_could_not_deliver() {
    let error: VmError = tokio::sync::mpsc::error::SendError(Value::Integer(9)).into();
    match error {
        VmError::ChannelSend { error, value } => {
            assert_eq!(value, Value::Integer(9));
            assert!(!error.is_empty(), "the channel error should be described");
        }
        other => panic!("expected ChannelSend, got {other:?}"),
    }
}

#[test]
fn a_compiler_error_converts_into_a_vm_error() {
    let error: VmError = CompilerError::InvalidToken("nope".to_string()).into();
    assert!(matches!(error, VmError::CompilationError(_)));
    assert!(error.to_string().contains("Invalid token: nope"));
}

#[test]
fn a_runtime_error_renders_its_source_location() {
    let error = VmError::RuntimeError {
        location: SourceLocation { line: 4, column: 9 },
        source: Box::new(VmError::DivisionByZero),
    };
    assert_eq!(error.to_string(), "Division by zero at 4:9");
}

#[test]
fn error_messages_name_what_went_wrong() {
    assert_eq!(VmError::StackUnderflow.to_string(), "Stack underflow");
    assert_eq!(
        VmError::StackUnderflowFor("Swap").to_string(),
        "Stack underflow for Swap"
    );
    assert_eq!(
        VmError::TypeMismatch("Add").to_string(),
        "Type mismatch in Add"
    );
    assert_eq!(VmError::IntegerOverflow.to_string(), "Integer overflow");
    assert_eq!(
        VmError::ExecutionOutOfBounds.to_string(),
        "Execution out of bounds"
    );
    assert_eq!(VmError::NoBytecode.to_string(), "No bytecode to execute");
    assert_eq!(
        VmError::VariableNotFound(3).to_string(),
        "Variable at index 3 not found"
    );
    assert_eq!(
        VmError::GlobalNotFound("io".to_string()).to_string(),
        "Global `io` not found"
    );
    assert_eq!(
        VmError::ExportNotFound {
            module: "io".to_string(),
            export: "read".to_string(),
        }
        .to_string(),
        "Export `read` not found in module `io`"
    );
    assert_eq!(VmError::InvalidReference.to_string(), "Invalid reference");
    assert_eq!(
        VmError::IndexOutOfBounds {
            index: 5,
            length: 2
        }
        .to_string(),
        "Index out of bounds: index 5, length 2"
    );
    assert_eq!(
        VmError::NativeArityMismatch {
            expected: 1,
            actual: 2
        }
        .to_string(),
        "Native arity mismatch: expected 1, got 2"
    );
    assert_eq!(
        VmError::MailboxDisconnected.to_string(),
        "Mailbox disconnected"
    );
    assert_eq!(
        VmError::ChannelSend {
            error: "closed".to_string(),
            value: Value::Null,
        }
        .to_string(),
        "Channel send error: closed"
    );
}

#[test]
fn exit_reasons_classify_the_errors_that_produced_them() {
    assert_eq!(
        ExitReason::from(&VmError::DivisionByZero),
        ExitReason::DivisionByZero
    );
    assert_eq!(
        ExitReason::from(&VmError::TypeMismatch("Add")),
        ExitReason::TypeMismatch
    );
    assert_eq!(
        ExitReason::from(&VmError::StackUnderflow),
        ExitReason::Error
    );
}

#[test]
fn strategy_codes_map_onto_strategies_and_default_to_one_for_one() {
    assert_eq!(
        SupervisorStrategy::from_usize(0),
        SupervisorStrategy::OneForOne
    );
    assert_eq!(
        SupervisorStrategy::from_usize(1),
        SupervisorStrategy::OneForAll
    );
    assert_eq!(
        SupervisorStrategy::from_usize(2),
        SupervisorStrategy::RestForOne
    );
    assert_eq!(
        SupervisorStrategy::from_usize(99),
        SupervisorStrategy::OneForOne,
        "an unrecognized code should fall back to one-for-one"
    );
}
