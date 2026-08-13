//! Front-end throughput: lexing, parsing, and emission.

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use raft::compiler::{Compiler, Lexer, Parser};

/// A source with `instructions` arithmetic instructions and a comment per line.
fn arithmetic_source(instructions: usize) -> String {
    let mut source = String::new();
    for index in 0..instructions {
        source.push_str("# push a pair and combine them\n");
        source.push_str(&format!("{} {} +\n", index % 100, index % 7));
    }
    source
}

/// A source whose jumps all resolve through labels, exercising the two-pass
/// emitter's label table.
fn labelled_source(labels: usize) -> String {
    let mut source = String::from("Jump .block0\n");
    for index in 0..labels {
        source.push_str(&format!(".block{index}\n"));
        source.push_str(&format!("{index}\n"));
        source.push_str(&format!("Jump .block{}\n", index + 1));
    }
    source.push_str(&format!(".block{labels}\nReturn\n"));
    source
}

fn bench_lexing(c: &mut Criterion) {
    let mut group = c.benchmark_group("compiler/lex");
    for instructions in [64usize, 1_024] {
        let source = arithmetic_source(instructions);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(instructions),
            &source,
            |b, source| {
                b.iter(|| {
                    Lexer::new(black_box(source))
                        .lex()
                        .expect("benchmark source should lex")
                })
            },
        );
    }
    group.finish();
}

fn bench_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("compiler/parse");
    for instructions in [64usize, 1_024] {
        let source = arithmetic_source(instructions);
        let tokens = Lexer::new(&source).lex().expect("source should lex");
        group.throughput(Throughput::Elements(tokens.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(instructions),
            &tokens,
            |b, tokens| {
                b.iter(|| {
                    Parser::new(black_box(tokens.clone()))
                        .parse()
                        .expect("benchmark tokens should parse")
                })
            },
        );
    }
    group.finish();
}

fn bench_full_compile(c: &mut Criterion) {
    let mut group = c.benchmark_group("compiler/compile");
    for instructions in [64usize, 1_024] {
        let source = arithmetic_source(instructions);
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(instructions),
            &source,
            |b, source| {
                b.iter(|| {
                    Compiler::compile_with_debug(black_box(source)).expect("source should compile")
                })
            },
        );
    }
    group.finish();
}

/// Label resolution is a second pass over the AST; this shows how it scales
/// with the number of distinct labels.
fn bench_label_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("compiler/labels");
    for labels in [16usize, 256] {
        let source = labelled_source(labels);
        group.throughput(Throughput::Elements(labels as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(labels),
            &source,
            |b, source| {
                b.iter(|| Compiler::compile(black_box(source)).expect("labels should resolve"))
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_lexing,
    bench_parsing,
    bench_full_compile,
    bench_label_resolution
);
criterion_main!(benches);
