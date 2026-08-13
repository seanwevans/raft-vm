use raft::vm::opcodes::OpCode;
use raft::vm::value::Value;
use raft::vm::VM;

/// Allocate and immediately discard `count` strings.
fn churn(count: usize) -> Vec<OpCode> {
    let mut code = Vec::with_capacity(count * 2);
    for _ in 0..count {
        code.push(OpCode::MakeString("garbage".to_string()));
        code.push(OpCode::Pop);
    }
    code
}

/// A process's heap must be bounded by how much garbage it holds at once, not
/// by how long it has been running. Collection is quota-driven, so up to one
/// quota of garbage can be outstanding at any moment -- but no more, however
/// many allocations the program makes in total.
#[tokio::test]
async fn heap_size_is_bounded_by_live_data_not_program_length() {
    let (mut short, _short_tx) = VM::new(churn(2_000), None);
    short.run().await.expect("short churn program should run");

    let (mut long, _long_tx) = VM::new(churn(50_000), None);
    long.run().await.expect("long churn program should run");

    assert!(
        long.heap_slot_count() < 2 * short.heap_slot_count(),
        "25x the allocations should not scale the heap: {} slots vs {}",
        long.heap_slot_count(),
        short.heap_slot_count()
    );
    assert!(
        long.heap_slot_count() < 2_048,
        "heap should stay within a small multiple of the collection quota, found {} slots",
        long.heap_slot_count()
    );
}

#[tokio::test]
async fn discarded_values_are_reclaimed_during_execution() {
    let (mut vm, _tx) = VM::new(churn(8_000), None);
    vm.run().await.expect("churn program should run");

    // Nothing is rooted, so a collection now must reclaim everything the
    // program allocated, leaving only the standard library.
    vm.collect_garbage();
    assert_eq!(
        vm.heap_live_object_count(),
        2,
        "only the io module and its print export should remain"
    );
}

#[tokio::test]
async fn collection_keeps_values_held_in_locals() {
    let mut code = vec![
        OpCode::MakeString("keep me".to_string()),
        OpCode::StoreVar(0),
    ];
    code.extend(churn(4_000));
    code.push(OpCode::LoadVar(0));

    let (mut vm, _tx) = VM::new(code, None);
    vm.run().await.expect("program should run");

    let address = match vm.stack().last() {
        Some(Value::Reference(address)) => *address,
        other => panic!("expected the retained string on the stack, got {other:?}"),
    };
    assert!(
        vm.heap_ref_count(address).is_some(),
        "a value held in a local must survive collection"
    );
}

#[tokio::test]
async fn collection_keeps_values_held_on_the_stack() {
    let mut code = vec![OpCode::MakeString("keep me".to_string())];
    code.extend(churn(4_000));

    let (mut vm, _tx) = VM::new(code, None);
    vm.run().await.expect("program should run");

    let address = match vm.stack().first() {
        Some(Value::Reference(address)) => *address,
        other => panic!("expected the retained string at the bottom of the stack, got {other:?}"),
    };
    assert_eq!(
        vm.heap_ref_count(address),
        Some(1),
        "a value held on the stack must survive collection"
    );
}

#[tokio::test]
async fn collection_keeps_the_standard_library_reachable() {
    let mut code = churn(4_000);
    code.extend([
        OpCode::PushConst(Value::Integer(1)),
        OpCode::LoadGlobal("io".to_string()),
        OpCode::GetExport("print".to_string()),
        OpCode::CallNative(1),
    ]);

    let (mut vm, _tx) = VM::new(code, None);
    vm.run()
        .await
        .expect("io.print should still resolve after collection");
}

#[tokio::test]
async fn nested_values_survive_collection_through_their_parent() {
    let mut code = vec![
        OpCode::MakeString("leaf".to_string()),
        OpCode::MakeArray(1),
        OpCode::StoreVar(0),
    ];
    code.extend(churn(4_000));
    code.extend([
        OpCode::LoadVar(0),
        OpCode::PushConst(Value::Integer(0)),
        OpCode::ArrayGet,
    ]);

    let (mut vm, _tx) = VM::new(code, None);
    vm.run().await.expect("program should run");

    let address = match vm.stack().last() {
        Some(Value::Reference(address)) => *address,
        other => panic!("expected the nested leaf string on the stack, got {other:?}"),
    };
    assert!(
        vm.heap_ref_count(address).is_some(),
        "a value reachable only through its parent must survive collection"
    );
}
