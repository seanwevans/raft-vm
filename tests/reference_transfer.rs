//! Reference counts must survive moving a value into and out of a container.
//!
//! Popping a value with a release, then re-retaining it, is not a no-op: the
//! release cascades into the value's own children when it held the last
//! reference, and the later retain only restores the top object. These tests
//! pin the counts of values nested one level down, where that difference shows.

use raft::vm::execution::ExecutionContext;
use raft::vm::heap::Heap;
use raft::vm::opcodes::OpCode;
use raft::vm::value::Value;
use raft::vm::VM;

fn run(code: Vec<OpCode>) -> (ExecutionContext, Heap) {
    let mut execution = ExecutionContext::new(vec![OpCode::Return]);
    let mut heap = Heap::new();
    for opcode in code {
        opcode
            .execute(&mut execution, &mut heap)
            .unwrap_or_else(|error| panic!("opcode failed: {error}"));
    }
    (execution, heap)
}

fn ref_count(heap: &Heap, address: usize) -> usize {
    heap.get(address)
        .unwrap_or_else(|| panic!("expected an object at {address}"))
        .ref_count()
}

fn top_address(execution: &ExecutionContext) -> usize {
    match execution.stack.last() {
        Some(Value::Reference(address)) => *address,
        other => panic!("expected a reference on top of the stack, got {other:?}"),
    }
}

#[tokio::test]
async fn nesting_an_array_preserves_its_children() {
    let (mut execution, mut heap) = run(vec![OpCode::MakeString("leaf".to_string())]);
    let leaf = top_address(&execution);

    OpCode::MakeArray(1)
        .execute(&mut execution, &mut heap)
        .unwrap();
    let inner = top_address(&execution);
    OpCode::MakeArray(1)
        .execute(&mut execution, &mut heap)
        .unwrap();
    let outer = top_address(&execution);

    assert_eq!(ref_count(&heap, outer), 1, "outer array held by the stack");
    assert_eq!(ref_count(&heap, inner), 1, "inner array held by the outer");
    assert_eq!(
        ref_count(&heap, leaf),
        1,
        "leaf is still held by the inner array"
    );
}

#[tokio::test]
async fn nesting_a_module_preserves_its_exports() {
    let (mut execution, mut heap) = run(vec![OpCode::MakeString("leaf".to_string())]);
    let leaf = top_address(&execution);

    OpCode::MakeModule(vec!["inner".to_string()])
        .execute(&mut execution, &mut heap)
        .unwrap();
    let inner = top_address(&execution);
    OpCode::MakeModule(vec!["outer".to_string()])
        .execute(&mut execution, &mut heap)
        .unwrap();

    assert_eq!(ref_count(&heap, inner), 1, "inner module held by the outer");
    assert_eq!(
        ref_count(&heap, leaf),
        1,
        "leaf is still held by the inner module"
    );
}

#[tokio::test]
async fn reading_an_element_out_of_its_last_owner_keeps_it_alive() {
    let (mut execution, mut heap) = run(vec![
        OpCode::MakeString("leaf".to_string()),
        OpCode::MakeArray(1),
        OpCode::PushConst(Value::Integer(0)),
    ]);

    // The array on the stack is the leaf's only owner, and ArrayGet consumes it.
    OpCode::ArrayGet
        .execute(&mut execution, &mut heap)
        .unwrap();

    let leaf = top_address(&execution);
    assert_eq!(
        ref_count(&heap, leaf),
        1,
        "the element must survive its container being dropped"
    );
    heap.collect_garbage();
    assert!(
        heap.get(leaf).is_some(),
        "the element must not be collected out from under the stack"
    );
}

#[tokio::test]
async fn reading_an_export_out_of_its_last_owner_keeps_it_alive() {
    let (mut execution, mut heap) = run(vec![
        OpCode::MakeString("leaf".to_string()),
        OpCode::MakeModule(vec!["leaf".to_string()]),
    ]);

    OpCode::ModuleGet("leaf".to_string())
        .execute(&mut execution, &mut heap)
        .unwrap();

    let leaf = top_address(&execution);
    assert_eq!(ref_count(&heap, leaf), 1);
    heap.collect_garbage();
    assert!(heap.get(leaf).is_some());
}

#[tokio::test]
async fn get_export_keeps_the_export_alive_when_it_drops_the_module() {
    let (mut vm, _tx) = VM::new(
        vec![
            OpCode::LoadGlobal("io".to_string()),
            OpCode::GetExport("print".to_string()),
        ],
        None,
    );
    vm.run().await.expect("loading io.print should succeed");

    let print = match vm.stack().last() {
        Some(Value::Reference(address)) => *address,
        other => panic!("expected the print function on the stack, got {other:?}"),
    };
    vm.collect_garbage();
    assert!(
        vm.heap_ref_count(print).is_some(),
        "the loaded export must survive collection"
    );
}

#[tokio::test]
async fn overwriting_an_element_releases_only_the_old_value() {
    let (mut execution, mut heap) = run(vec![
        OpCode::MakeString("old".to_string()),
        OpCode::MakeArray(1),
    ]);
    let array = top_address(&execution);

    OpCode::PushConst(Value::Integer(0))
        .execute(&mut execution, &mut heap)
        .unwrap();
    OpCode::MakeString("new".to_string())
        .execute(&mut execution, &mut heap)
        .unwrap();
    let new = match execution.stack.last() {
        Some(Value::Reference(address)) => *address,
        other => panic!("expected the new string, got {other:?}"),
    };

    OpCode::ArraySet
        .execute(&mut execution, &mut heap)
        .unwrap();

    assert_eq!(ref_count(&heap, array), 1, "the array is still on the stack");
    assert_eq!(
        ref_count(&heap, new),
        1,
        "the stored value is held once, by the array"
    );
}

#[tokio::test]
async fn setting_a_module_export_does_not_double_count_the_module() {
    let (mut execution, mut heap) = run(vec![
        OpCode::PushConst(Value::Integer(1)),
        OpCode::MakeModule(vec!["answer".to_string()]),
    ]);
    let module = top_address(&execution);

    OpCode::PushConst(Value::Integer(2))
        .execute(&mut execution, &mut heap)
        .unwrap();
    OpCode::ModuleSet("answer".to_string())
        .execute(&mut execution, &mut heap)
        .unwrap();

    assert_eq!(
        ref_count(&heap, module),
        1,
        "ModuleSet returns the same reference it consumed"
    );
}

#[tokio::test]
async fn set_strategy_does_not_double_count_the_supervisor() {
    let (mut execution, mut heap) = run(vec![OpCode::SpawnSupervisor(0)]);
    let supervisor = top_address(&execution);

    OpCode::SetStrategy(1)
        .execute(&mut execution, &mut heap)
        .unwrap();

    assert_eq!(
        ref_count(&heap, supervisor),
        1,
        "SetStrategy returns the same reference it consumed"
    );
}

#[tokio::test]
async fn a_nested_value_is_collected_once_its_root_is_dropped() {
    let (mut execution, mut heap) = run(vec![OpCode::MakeString("leaf".to_string())]);
    let leaf = top_address(&execution);
    OpCode::MakeArray(1)
        .execute(&mut execution, &mut heap)
        .unwrap();
    let inner = top_address(&execution);
    OpCode::MakeArray(1)
        .execute(&mut execution, &mut heap)
        .unwrap();
    let outer = top_address(&execution);

    OpCode::Pop.execute(&mut execution, &mut heap).unwrap();
    heap.collect_garbage();

    assert!(heap.get(outer).is_none(), "the root should be reclaimed");
    assert!(heap.get(inner).is_none(), "the inner array should be reclaimed");
    assert!(heap.get(leaf).is_none(), "the leaf should be reclaimed too");
}

#[tokio::test]
async fn duplicate_export_names_do_not_strand_the_shadowed_value() {
    let (mut execution, mut heap) = run(vec![
        OpCode::MakeString("first".to_string()),
        OpCode::MakeString("second".to_string()),
    ]);
    let shadowed = match execution.stack.first() {
        Some(Value::Reference(address)) => *address,
        other => panic!("expected the first string, got {other:?}"),
    };

    OpCode::MakeModule(vec!["name".to_string(), "name".to_string()])
        .execute(&mut execution, &mut heap)
        .unwrap();

    assert_eq!(
        ref_count(&heap, shadowed),
        0,
        "a value shadowed by a duplicate export name is unreachable and must be released"
    );
    heap.collect_garbage();
    assert!(heap.get(shadowed).is_none());
}
