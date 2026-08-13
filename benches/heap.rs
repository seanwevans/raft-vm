//! Heap costs: allocation, collection, reference release, and message
//! conversion.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use raft::vm::heap::{Heap, HeapObject};
use raft::vm::value::{MessageValue, Value};

/// A heap holding `live` rooted strings and `garbage` unrooted ones.
fn populated_heap(live: usize, garbage: usize) -> Heap {
    let mut heap = Heap::new();
    for index in 0..live {
        heap.allocate(HeapObject::String(format!("live {index}"), 1));
    }
    for index in 0..garbage {
        heap.allocate(HeapObject::String(format!("garbage {index}"), 0));
    }
    heap
}

/// A chain of `depth` nested single-element arrays around one string, with the
/// root retained.
fn nested_chain(depth: usize) -> (Heap, usize) {
    let mut heap = Heap::new();
    let mut address = heap.allocate(HeapObject::String("leaf".to_string(), 1));
    for _ in 0..depth {
        address = heap.allocate(HeapObject::Array(vec![Value::Reference(address)], 1));
    }
    (heap, address)
}

/// A balanced two-level array of strings: `breadth` children, each holding
/// `breadth` strings.
fn wide_tree(breadth: usize) -> (Heap, usize) {
    let mut heap = Heap::new();
    let mut children = Vec::with_capacity(breadth);
    for index in 0..breadth {
        let mut leaves = Vec::with_capacity(breadth);
        for leaf in 0..breadth {
            let address = heap.allocate(HeapObject::String(format!("{index}:{leaf}"), 1));
            leaves.push(Value::Reference(address));
        }
        children.push(Value::Reference(heap.allocate(HeapObject::Array(leaves, 1))));
    }
    let root = heap.allocate(HeapObject::Array(children, 1));
    (heap, root)
}

fn bench_allocate(c: &mut Criterion) {
    let mut group = c.benchmark_group("heap/allocate");
    for count in [64usize, 1_024] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                // Build the payloads in setup so the measurement is allocation
                // and slot bookkeeping, not string formatting.
                || {
                    (
                        Heap::new(),
                        (0..count).map(|index| index.to_string()).collect::<Vec<_>>(),
                    )
                },
                |(mut heap, payloads): (Heap, Vec<String>)| {
                    for payload in payloads {
                        black_box(heap.allocate(HeapObject::String(payload, 1)));
                    }
                    heap
                },
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

/// Collection walks every slot twice, so its cost tracks heap size rather than
/// the amount of garbage. Both axes are measured.
fn bench_collect_garbage(c: &mut Criterion) {
    let mut group = c.benchmark_group("heap/collect_garbage");

    for (live, garbage) in [(1_000usize, 0usize), (500, 500), (0, 1_000)] {
        group.throughput(Throughput::Elements((live + garbage) as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("live{live}_garbage{garbage}")),
            &(live, garbage),
            |b, &(live, garbage)| {
                b.iter_batched(
                    || populated_heap(live, garbage),
                    |mut heap| {
                        heap.collect_garbage();
                        heap
                    },
                    BatchSize::SmallInput,
                )
            },
        );
    }

    // Collection also has to trace through structure, not just flat slots.
    for breadth in [16usize, 32] {
        group.throughput(Throughput::Elements((breadth * breadth + breadth + 1) as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("tree{breadth}")),
            &breadth,
            |b, &breadth| {
                b.iter_batched(
                    || wide_tree(breadth).0,
                    |mut heap| {
                        heap.collect_garbage();
                        heap
                    },
                    BatchSize::SmallInput,
                )
            },
        );
    }

    group.finish();
}

fn bench_release_reference(c: &mut Criterion) {
    let mut group = c.benchmark_group("heap/release_reference");
    for depth in [64usize, 1_024] {
        group.throughput(Throughput::Elements(depth as u64));
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            b.iter_batched(
                || nested_chain(depth),
                |(mut heap, root)| {
                    heap.release_reference(root).expect("release should succeed");
                    heap
                },
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

fn bench_message_conversion(c: &mut Criterion) {
    let mut group = c.benchmark_group("heap/message_conversion");

    for breadth in [8usize, 24] {
        let (heap, root) = wide_tree(breadth);
        let elements = (breadth * breadth) as u64;

        group.throughput(Throughput::Elements(elements));
        group.bench_with_input(
            BenchmarkId::new("value_to_message", breadth),
            &(heap, root),
            |b, (heap, root)| {
                b.iter(|| {
                    heap.value_to_message(black_box(Value::Reference(*root)))
                        .expect("tree should convert")
                })
            },
        );

        let (heap, root) = wide_tree(breadth);
        let message = heap
            .value_to_message(Value::Reference(root))
            .expect("tree should convert");
        group.throughput(Throughput::Elements(elements));
        group.bench_with_input(
            BenchmarkId::new("message_to_value", breadth),
            &message,
            |b, message| {
                b.iter_batched(
                    || (Heap::new(), message.clone()),
                    |(mut heap, message): (Heap, MessageValue)| {
                        heap.message_to_value(message)
                            .expect("message should materialize")
                    },
                    BatchSize::SmallInput,
                )
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_allocate,
    bench_collect_garbage,
    bench_release_reference,
    bench_message_conversion
);
criterion_main!(benches);
