//! Interpreter throughput: opcode dispatch, control flow, and message passing.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use raft::vm::execution::{ExecutionContext, ExecutionState};
use raft::vm::heap::Heap;
use raft::vm::opcodes::OpCode;
use raft::vm::value::{MessageValue, Value};
use raft::vm::VM;

/// Run a context to completion. The program must neither fail nor block.
fn drive(execution: &mut ExecutionContext, heap: &mut Heap) {
    execution.ip = 0;
    execution.stack.clear();
    loop {
        match execution
            .step(heap)
            .expect("benchmark program should not fail")
        {
            ExecutionState::Halted => break,
            ExecutionState::Continue => {}
            ExecutionState::Yield(_) => panic!("benchmark program should not block"),
        }
    }
}

/// Time `bytecode` as raw dispatch, reported per opcode executed.
fn bench_dispatch(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    name: &str,
    bytecode: Vec<OpCode>,
) {
    let opcodes = bytecode.len() as u64;
    let mut execution = ExecutionContext::new(bytecode);
    group.throughput(Throughput::Elements(opcodes));
    group.bench_function(BenchmarkId::from_parameter(name), |b| {
        // A fresh heap per iteration. Sharing one would let allocating
        // programs grow it without bound across the measurement, so the
        // numbers would drift upward instead of describing a single run.
        b.iter_batched(
            Heap::new,
            |mut heap| {
                drive(black_box(&mut execution), black_box(&mut heap));
                heap
            },
            BatchSize::SmallInput,
        )
    });
}

fn repeat(pattern: &[OpCode], times: usize) -> Vec<OpCode> {
    pattern
        .iter()
        .cloned()
        .cycle()
        .take(pattern.len() * times)
        .collect()
}

fn bench_opcode_dispatch(c: &mut Criterion) {
    const ITERATIONS: usize = 2_000;
    let mut group = c.benchmark_group("interpreter/dispatch");

    bench_dispatch(
        &mut group,
        "arithmetic",
        repeat(
            &[
                OpCode::PushConst(Value::Integer(7)),
                OpCode::PushConst(Value::Integer(11)),
                OpCode::Add,
                OpCode::Pop,
            ],
            ITERATIONS,
        ),
    );

    bench_dispatch(
        &mut group,
        "stack_shuffle",
        repeat(
            &[
                OpCode::PushConst(Value::Integer(1)),
                OpCode::Dup,
                OpCode::Swap,
                OpCode::Pop,
                OpCode::Pop,
            ],
            ITERATIONS,
        ),
    );

    bench_dispatch(
        &mut group,
        "locals",
        repeat(
            &[
                OpCode::PushConst(Value::Integer(3)),
                OpCode::StoreVar(0),
                OpCode::LoadVar(0),
                OpCode::Pop,
            ],
            ITERATIONS,
        ),
    );

    // Every jump lands on the next instruction, so this measures the branch
    // itself rather than the work between branches.
    let jumps = (1..=ITERATIONS).map(OpCode::Jump).collect::<Vec<_>>();
    bench_dispatch(&mut group, "jumps", jumps);

    group.finish();
}

fn bench_call_return(c: &mut Criterion) {
    const CALLS: usize = 2_000;
    let mut group = c.benchmark_group("interpreter/call_return");

    // CALLS calls, then a jump over a subroutine that pushes, pops and returns.
    let subroutine = CALLS + 1;
    let mut bytecode: Vec<OpCode> = (0..CALLS).map(|_| OpCode::Call(subroutine)).collect();
    bytecode.push(OpCode::Jump(CALLS + 4));
    bytecode.push(OpCode::PushConst(Value::Integer(1)));
    bytecode.push(OpCode::Pop);
    bytecode.push(OpCode::Return);

    bench_dispatch(&mut group, "call_return", bytecode);
    group.finish();
}

fn bench_heap_opcodes(c: &mut Criterion) {
    const ITERATIONS: usize = 500;
    let mut group = c.benchmark_group("interpreter/heap_opcodes");

    bench_dispatch(
        &mut group,
        "make_string",
        repeat(
            &[OpCode::MakeString("benchmark".to_string()), OpCode::Pop],
            ITERATIONS,
        ),
    );

    bench_dispatch(
        &mut group,
        "make_array",
        repeat(
            &[
                OpCode::PushConst(Value::Integer(1)),
                OpCode::PushConst(Value::Integer(2)),
                OpCode::PushConst(Value::Integer(3)),
                OpCode::MakeArray(3),
                OpCode::Pop,
            ],
            ITERATIONS,
        ),
    );

    bench_dispatch(
        &mut group,
        "array_get",
        {
            let mut bytecode = vec![
                OpCode::PushConst(Value::Integer(1)),
                OpCode::PushConst(Value::Integer(2)),
                OpCode::MakeArray(2),
            ];
            bytecode.extend(repeat(
                &[
                    OpCode::Dup,
                    OpCode::PushConst(Value::Integer(1)),
                    OpCode::ArrayGet,
                    OpCode::Pop,
                ],
                ITERATIONS,
            ));
            bytecode.push(OpCode::Pop);
            bytecode
        },
    );

    group.finish();
}

/// End-to-end cost of standing a process up and running it, including the
/// standard library install and the async loop.
fn bench_process_lifecycle(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("interpreter/process");

    let program = repeat(
        &[
            OpCode::PushConst(Value::Integer(7)),
            OpCode::PushConst(Value::Integer(11)),
            OpCode::Add,
            OpCode::Pop,
        ],
        500,
    );

    group.bench_function("new", |b| {
        // Clone in setup: this measures process construction, not the cost of
        // copying the bytecode vector.
        b.iter_batched(
            || program.clone(),
            |bytecode| black_box(VM::new(bytecode, None)),
            BatchSize::SmallInput,
        )
    });

    group.bench_function("new_and_run", |b| {
        b.iter_batched(
            || VM::new(program.clone(), None),
            |(mut vm, _tx)| runtime.block_on(async { vm.run().await.expect("program should run") }),
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

fn bench_messaging(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("interpreter/messaging");

    // Deliver a message through the mailbox and materialize it on the stack.
    for message in [
        ("integer", MessageValue::Integer(42)),
        (
            "array",
            MessageValue::Array(vec![MessageValue::Integer(1), MessageValue::Integer(2)]),
        ),
        ("string", MessageValue::String("benchmark".to_string())),
    ] {
        let (label, message) = message;
        group.bench_function(BenchmarkId::new("receive", label), |b| {
            b.iter_batched(
                || {
                    let (vm, tx) = VM::new(vec![OpCode::ReceiveMessage], None);
                    runtime
                        .block_on(tx.send(message.clone()))
                        .expect("mailbox should accept the message");
                    vm
                },
                |mut vm| runtime.block_on(async { vm.run().await.expect("receive should succeed") }),
                BatchSize::SmallInput,
            )
        });
    }

    // Spawn a child process and send it one message.
    group.bench_function("spawn_and_send", |b| {
        let program = vec![
            OpCode::PushConst(Value::Integer(42)),
            OpCode::SpawnActor(4),
            OpCode::SendMessage,
            OpCode::Jump(5),
            OpCode::ReceiveMessage,
        ];
        b.iter_batched(
            || VM::new(program.clone(), None),
            |(mut vm, _tx)| runtime.block_on(async { vm.run().await.expect("spawn should run") }),
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_opcode_dispatch,
    bench_call_return,
    bench_heap_opcodes,
    bench_process_lifecycle,
    bench_messaging
);
criterion_main!(benches);
