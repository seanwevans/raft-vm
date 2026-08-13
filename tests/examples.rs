//! Every shipped example must compile and run.
//!
//! `examples/control_flow.raft` was broken for long enough to go unnoticed --
//! it fed an integer to `JumpIfFalse` and jumped past the end of its own
//! bytecode -- because nothing exercised the examples directory.

use std::fs;
use std::path::PathBuf;

fn example_paths() -> Vec<PathBuf> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut paths: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("reading {}: {error}", directory.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "raft"))
        .collect();
    paths.sort();
    paths
}

#[test]
fn the_examples_directory_is_not_empty() {
    assert!(
        !example_paths().is_empty(),
        "expected at least one .raft example to check"
    );
}

#[tokio::test]
async fn every_example_compiles_and_runs() {
    for path in example_paths() {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));

        raft::Compiler::compile(&source)
            .unwrap_or_else(|error| panic!("{} should compile: {error}", path.display()));

        raft::run(&source)
            .await
            .unwrap_or_else(|error| panic!("{} should run: {error}", path.display()));
    }
}
