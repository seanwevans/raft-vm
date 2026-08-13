//! Opcode behaviours and error paths not reached by the existing suites.

use raft::vm::execution::ExecutionContext;
use raft::vm::heap::{Heap, HeapObject, NativeFunction};
use raft::vm::opcodes::OpCode;
use raft::vm::value::Value;
use raft::vm::VmError;

fn context() -> (ExecutionContext, Heap) {
    (ExecutionContext::new(vec![OpCode::Return]), Heap::new())
}

fn exec(execution: &mut ExecutionContext, heap: &mut Heap, opcodes: &[OpCode]) {
    for opcode in opcodes {
        opcode
            .execute(execution, heap)
            .unwrap_or_else(|error| panic!("{opcode:?} should succeed: {error}"));
    }
}

fn err(execution: &mut ExecutionContext, heap: &mut Heap, opcode: OpCode) -> VmError {
    opcode
        .execute(execution, heap)
        .expect_err("expected this opcode to fail")
}

fn add_native(args: Vec<Value>) -> Result<Value, VmError> {
    match args.as_slice() {
        [Value::Integer(a), Value::Integer(b)] => Ok(Value::Integer(a + b)),
        _ => Err(VmError::TypeMismatch("native add")),
    }
}

fn failing_native(_args: Vec<Value>) -> Result<Value, VmError> {
    Err(VmError::Message("native exploded".to_string()))
}

fn native(name: &str, arity: usize, function: fn(Vec<Value>) -> Result<Value, VmError>) -> OpCode {
    OpCode::MakeNativeFunction(NativeFunction {
        name: name.to_string(),
        arity,
        function,
    })
}

// --- arithmetic ------------------------------------------------------------

#[tokio::test]
async fn modulo_by_zero_and_on_wrong_types_is_rejected() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(5)),
            OpCode::PushConst(Value::Integer(0)),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::Mod),
        VmError::DivisionByZero
    ));

    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Float(5.0)),
            OpCode::PushConst(Value::Float(2.0)),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::Mod),
        VmError::TypeMismatch("Mod")
    ));
}

#[tokio::test]
async fn modulo_of_integers_produces_the_remainder() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(7)),
            OpCode::PushConst(Value::Integer(3)),
            OpCode::Mod,
        ],
    );
    assert_eq!(execution.stack.pop(), Some(Value::Integer(1)));
}

#[tokio::test]
async fn exponentiation_covers_floats_and_wrong_types() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Float(2.0)),
            OpCode::PushConst(Value::Float(3.0)),
            OpCode::Exp,
        ],
    );
    match execution.stack.pop() {
        Some(Value::Float(value)) => assert!((value - 8.0).abs() < f64::EPSILON),
        other => panic!("expected 8.0, got {other:?}"),
    }

    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(2)),
            OpCode::PushConst(Value::Boolean(true)),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::Exp),
        VmError::TypeMismatch("Exp")
    ));
}

#[tokio::test]
async fn negation_handles_floats_and_the_integer_boundary() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[OpCode::PushConst(Value::Float(2.5)), OpCode::Neg],
    );
    match execution.stack.pop() {
        Some(Value::Float(value)) => assert!((value + 2.5).abs() < f64::EPSILON),
        other => panic!("expected -2.5, got {other:?}"),
    }

    exec(
        &mut execution,
        &mut heap,
        &[OpCode::PushConst(Value::Integer(i32::MIN))],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::Neg),
        VmError::IntegerOverflow
    ));
}

// --- stack -----------------------------------------------------------------

#[tokio::test]
async fn duplicating_an_empty_stack_underflows() {
    let (mut execution, mut heap) = context();
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::Dup),
        VmError::StackUnderflow
    ));
}

#[tokio::test]
async fn binary_operations_underflow_on_a_single_operand() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[OpCode::PushConst(Value::Integer(1))],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::Add),
        VmError::StackUnderflow
    ));
}

#[tokio::test]
async fn loading_an_unset_local_fails() {
    let (mut execution, mut heap) = context();
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::LoadVar(7)),
        VmError::VariableNotFound(7)
    ));
}

#[tokio::test]
async fn storing_over_a_local_releases_the_previous_value() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[OpCode::MakeString("first".to_string()), OpCode::StoreVar(0)],
    );
    let first = match execution.locals.get(&0) {
        Some(Value::Reference(address)) => *address,
        other => panic!("expected a stored reference, got {other:?}"),
    };

    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::MakeString("second".to_string()),
            OpCode::StoreVar(0),
        ],
    );
    assert_eq!(
        heap.get(first).map(HeapObject::ref_count),
        Some(0),
        "the replaced value should have been released"
    );
}

// --- arrays ----------------------------------------------------------------

#[tokio::test]
async fn make_array_underflows_when_the_stack_is_short() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[OpCode::PushConst(Value::Integer(1))],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::MakeArray(3)),
        VmError::StackUnderflowFor("MakeArray")
    ));
}

#[tokio::test]
async fn reading_past_the_end_of_an_array_is_rejected() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(1)),
            OpCode::MakeArray(1),
            OpCode::PushConst(Value::Integer(4)),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::ArrayGet),
        VmError::IndexOutOfBounds {
            index: 4,
            length: 1
        }
    ));
}

#[tokio::test]
async fn a_negative_array_index_is_a_type_error() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(1)),
            OpCode::MakeArray(1),
            OpCode::PushConst(Value::Integer(-1)),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::ArrayGet),
        VmError::TypeMismatch("ArrayGet")
    ));
}

#[tokio::test]
async fn indexing_a_non_array_is_rejected() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::MakeString("not an array".to_string()),
            OpCode::PushConst(Value::Integer(0)),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::ArrayGet),
        VmError::InvalidReference
    ));

    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(3)),
            OpCode::PushConst(Value::Integer(0)),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::ArrayGet),
        VmError::TypeMismatch("ArrayGet")
    ));
}

#[tokio::test]
async fn writing_past_the_end_of_an_array_is_rejected() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(1)),
            OpCode::MakeArray(1),
            OpCode::PushConst(Value::Integer(9)),
            OpCode::PushConst(Value::Integer(0)),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::ArraySet),
        VmError::IndexOutOfBounds {
            index: 9,
            length: 1
        }
    ));
}

#[tokio::test]
async fn writing_into_a_non_array_is_rejected() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::MakeString("not an array".to_string()),
            OpCode::PushConst(Value::Integer(0)),
            OpCode::PushConst(Value::Integer(1)),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::ArraySet),
        VmError::InvalidReference
    ));
}

// --- strings ---------------------------------------------------------------

#[tokio::test]
async fn push_string_and_make_string_behave_alike() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::MakeString("same".to_string()),
            OpCode::PushString("same".to_string()),
        ],
    );

    let addresses: Vec<usize> = execution
        .stack
        .iter()
        .map(|value| match value {
            Value::Reference(address) => *address,
            other => panic!("expected references, got {other:?}"),
        })
        .collect();
    assert_ne!(addresses[0], addresses[1], "each push allocates its own");
    for address in addresses {
        match heap.get(address) {
            Some(HeapObject::String(value, rc)) => {
                assert_eq!(value, "same");
                assert_eq!(*rc, 1);
            }
            other => panic!("expected a string, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn concatenating_non_strings_is_rejected() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::MakeString("left".to_string()),
            OpCode::PushConst(Value::Integer(1)),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::StringConcat),
        VmError::TypeMismatch("StringConcat")
    ));

    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(1)),
            OpCode::MakeString("right".to_string()),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::StringConcat),
        VmError::TypeMismatch("StringConcat")
    ));
}

#[tokio::test]
async fn concatenating_a_non_string_reference_is_rejected() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::MakeString("left".to_string()),
            OpCode::PushConst(Value::Integer(1)),
            OpCode::MakeArray(1),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::StringConcat),
        VmError::InvalidReference
    ));
}

// --- modules ---------------------------------------------------------------

#[tokio::test]
async fn make_module_underflows_when_the_stack_is_short() {
    let (mut execution, mut heap) = context();
    assert!(matches!(
        err(
            &mut execution,
            &mut heap,
            OpCode::MakeModule(vec!["a".to_string(), "b".to_string()])
        ),
        VmError::StackUnderflowFor("MakeModule")
    ));
}

#[tokio::test]
async fn reading_a_missing_module_export_is_rejected() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(1)),
            OpCode::MakeModule(vec!["present".to_string()]),
        ],
    );
    let error = err(
        &mut execution,
        &mut heap,
        OpCode::ModuleGet("absent".to_string()),
    );
    assert!(
        error
            .to_string()
            .contains("Module export not found: absent"),
        "got: {error}"
    );
}

#[tokio::test]
async fn module_operations_reject_non_modules() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[OpCode::MakeString("not a module".to_string())],
    );
    assert!(matches!(
        err(
            &mut execution,
            &mut heap,
            OpCode::ModuleGet("any".to_string())
        ),
        VmError::InvalidReference
    ));

    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[OpCode::PushConst(Value::Integer(1))],
    );
    assert!(matches!(
        err(
            &mut execution,
            &mut heap,
            OpCode::ModuleGet("any".to_string())
        ),
        VmError::TypeMismatch("ModuleGet")
    ));

    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(1)),
            OpCode::PushConst(Value::Integer(2)),
        ],
    );
    assert!(matches!(
        err(
            &mut execution,
            &mut heap,
            OpCode::ModuleSet("any".to_string())
        ),
        VmError::TypeMismatch("ModuleSet")
    ));
}

#[tokio::test]
async fn get_export_reports_a_missing_export_by_module_name() {
    let (mut execution, mut heap) = context();
    let module = heap.allocate(HeapObject::Module {
        name: "io".to_string(),
        exports: std::collections::HashMap::new(),
        ref_count: 1,
    });
    execution.stack.push(Value::Reference(module));

    match err(
        &mut execution,
        &mut heap,
        OpCode::GetExport("read".to_string()),
    ) {
        VmError::ExportNotFound { module, export } => {
            assert_eq!(module, "io");
            assert_eq!(export, "read");
        }
        other => panic!("expected ExportNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn get_export_rejects_values_that_are_not_modules() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[OpCode::PushConst(Value::Integer(1))],
    );
    assert!(matches!(
        err(
            &mut execution,
            &mut heap,
            OpCode::GetExport("any".to_string())
        ),
        VmError::InvalidReference
    ));
}

#[tokio::test]
async fn loading_a_missing_global_is_rejected() {
    let (mut execution, mut heap) = context();
    match err(
        &mut execution,
        &mut heap,
        OpCode::LoadGlobal("nope".to_string()),
    ) {
        VmError::GlobalNotFound(name) => assert_eq!(name, "nope"),
        other => panic!("expected GlobalNotFound, got {other:?}"),
    }
}

// --- native calls ----------------------------------------------------------

#[tokio::test]
async fn a_native_function_may_sit_below_its_arguments() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            native("add", 2, add_native),
            OpCode::PushConst(Value::Integer(2)),
            OpCode::PushConst(Value::Integer(3)),
            OpCode::CallNative(2),
        ],
    );
    assert_eq!(execution.stack.pop(), Some(Value::Integer(5)));
}

#[tokio::test]
async fn calling_a_native_with_the_wrong_arity_is_rejected() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(1)),
            native("add", 2, add_native),
        ],
    );
    match err(&mut execution, &mut heap, OpCode::CallNative(1)) {
        VmError::NativeArityMismatch { expected, actual } => {
            assert_eq!((expected, actual), (2, 1));
        }
        other => panic!("expected NativeArityMismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn calling_a_native_without_enough_stack_is_rejected() {
    let (mut execution, mut heap) = context();
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::CallNative(2)),
        VmError::StackUnderflowFor("CallNative")
    ));
}

#[tokio::test]
async fn calling_something_that_is_not_a_native_is_rejected() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(1)),
            OpCode::PushConst(Value::Integer(2)),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::CallNative(1)),
        VmError::InvalidReference
    ));
}

#[tokio::test]
async fn a_native_that_fails_propagates_its_error() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(1)),
            native("boom", 1, failing_native),
        ],
    );
    let error = err(&mut execution, &mut heap, OpCode::CallNative(1));
    assert!(
        error.to_string().contains("native exploded"),
        "got: {error}"
    );
}

// --- control flow ----------------------------------------------------------

#[tokio::test]
async fn return_resumes_at_the_saved_address() {
    let mut execution =
        ExecutionContext::new(vec![OpCode::Call(2), OpCode::Return, OpCode::Return]);
    let mut heap = Heap::new();

    execution.step(&mut heap).expect("call should succeed");
    assert_eq!(execution.ip, 2);
    assert_eq!(execution.call_stack, vec![1]);

    execution.step(&mut heap).expect("return should succeed");
    assert_eq!(execution.ip, 1, "return should resume after the call");
    assert!(execution.call_stack.is_empty());
}

#[tokio::test]
async fn jumping_to_the_end_of_the_program_halts_it() {
    let mut execution = ExecutionContext::new(vec![OpCode::Jump(1)]);
    let mut heap = Heap::new();
    execution.step(&mut heap).expect("jump should succeed");
    assert!(matches!(
        execution.step(&mut heap),
        Ok(raft::vm::execution::ExecutionState::Halted)
    ));
}

// --- supervision -----------------------------------------------------------

#[tokio::test]
async fn supervisor_opcodes_reject_non_supervisors() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[OpCode::MakeString("not a supervisor".to_string())],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::SetStrategy(1)),
        VmError::InvalidReference
    ));

    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[OpCode::PushConst(Value::Integer(1))],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::SetStrategy(1)),
        VmError::InvalidReference
    ));
}

#[tokio::test]
async fn restarting_a_child_that_is_not_an_actor_is_rejected() {
    let (mut execution, mut heap) = context();
    exec(&mut execution, &mut heap, &[OpCode::SpawnSupervisor(0)]);
    let not_an_actor = heap.allocate(HeapObject::String("not an actor".to_string(), 1));

    assert!(matches!(
        err(
            &mut execution,
            &mut heap,
            OpCode::RestartChild(not_an_actor)
        ),
        VmError::InvalidReference
    ));
}

// --- messaging -------------------------------------------------------------

#[tokio::test]
async fn sending_to_something_that_is_not_an_actor_is_rejected() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(1)),
            OpCode::MakeString("not an actor".to_string()),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::SendMessage),
        VmError::InvalidReference
    ));

    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[
            OpCode::PushConst(Value::Integer(1)),
            OpCode::PushConst(Value::Integer(2)),
        ],
    );
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::SendMessage),
        VmError::InvalidReference
    ));
}

#[tokio::test]
async fn an_actor_handle_cannot_be_sent_as_a_message() {
    let (mut execution, mut heap) = context();
    exec(
        &mut execution,
        &mut heap,
        &[OpCode::SpawnActor(0), OpCode::Dup],
    );

    // Both operands are the same actor: the message is a handle, which has no
    // mailbox representation.
    assert!(matches!(
        err(&mut execution, &mut heap, OpCode::SendMessage),
        VmError::TypeMismatch("SendMessage unsupported reference type")
    ));
}
