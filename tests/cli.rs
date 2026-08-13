//! End-to-end tests for the `raft` binary: subcommands, exit codes, and the
//! REPL's line handling.

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn raft() -> Command {
    Command::new(env!("CARGO_BIN_EXE_raft"))
}

fn scratch_file(name: &str, contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("raft-cli-{}-{name}", std::process::id()));
    std::fs::write(&path, contents).expect("writing the scratch program should succeed");
    path
}

/// Drive the REPL with `input` on stdin and collect its output.
fn repl(input: &str) -> Output {
    let mut child = raft()
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the repl should start");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(input.as_bytes())
        .expect("writing to the repl should succeed");
    child.wait_with_output().expect("the repl should exit")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn version_reports_the_crate_version() {
    let output = raft().arg("version").output().expect("version should run");
    assert!(output.status.success());
    assert_eq!(
        stdout_of(&output).trim(),
        format!("Raft version {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn no_subcommand_prints_help() {
    let output = raft().output().expect("bare invocation should run");
    assert!(output.status.success());
    let stdout = stdout_of(&output);
    assert!(
        stdout.contains("Usage"),
        "expected usage text, got: {stdout}"
    );
    assert!(
        stdout.contains("repl"),
        "expected the repl subcommand listed"
    );
}

#[test]
fn run_executes_a_program() {
    let path = scratch_file("run.raft", "42 io.print CallNative 1\n");
    let output = raft()
        .arg("run")
        .arg(&path)
        .output()
        .expect("run should execute");

    assert!(output.status.success(), "stderr: {}", stderr_of(&output));
    assert_eq!(stdout_of(&output).trim(), "42");
    let _ = std::fs::remove_file(path);
}

#[test]
fn run_reports_a_missing_file_and_exits_nonzero() {
    let output = raft()
        .arg("run")
        .arg("definitely-not-a-real-program.raft")
        .output()
        .expect("run should execute");

    assert!(!output.status.success());
    assert!(
        stderr_of(&output).contains("File error"),
        "expected a file error, got: {}",
        stderr_of(&output)
    );
}

#[test]
fn run_reports_a_compile_error_and_exits_nonzero() {
    let path = scratch_file("bad-compile.raft", "NotAnInstruction\n");
    let output = raft()
        .arg("run")
        .arg(&path)
        .output()
        .expect("run should execute");

    assert!(!output.status.success());
    assert!(
        stderr_of(&output).contains("Invalid token"),
        "expected an invalid token error, got: {}",
        stderr_of(&output)
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn run_reports_a_runtime_error_and_exits_nonzero() {
    let path = scratch_file("bad-runtime.raft", "1 0 /\n");
    let output = raft()
        .arg("run")
        .arg(&path)
        .output()
        .expect("run should execute");

    assert!(!output.status.success());
    assert!(
        stderr_of(&output).contains("Division by zero"),
        "expected a division error, got: {}",
        stderr_of(&output)
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn an_unknown_subcommand_fails() {
    let output = raft()
        .arg("frobnicate")
        .output()
        .expect("the binary should run");
    assert!(!output.status.success());
}

#[test]
fn the_repl_evaluates_a_line_and_exits() {
    let output = repl("1 2 +\nexit\n");
    assert!(output.status.success());
    assert!(
        stdout_of(&output).contains("Success"),
        "expected a success line, got: {}",
        stdout_of(&output)
    );
}

#[test]
fn the_repl_survives_a_compiler_error() {
    let output = repl("NotAnInstruction\n1 2 +\nexit\n");
    assert!(output.status.success());
    assert!(
        stderr_of(&output).contains("Invalid token"),
        "expected the error to be reported"
    );
    assert!(
        stdout_of(&output).contains("Success"),
        "the repl should keep going after a compiler error"
    );
}

#[test]
fn the_repl_survives_a_runtime_error() {
    let output = repl("1 0 /\n1 2 +\nexit\n");
    assert!(output.status.success());
    assert!(
        stderr_of(&output).contains("Division by zero"),
        "expected the runtime error to be reported"
    );
    assert!(
        stdout_of(&output).contains("Success"),
        "the repl should keep going after a runtime error"
    );
}

#[test]
fn the_repl_continues_an_explicitly_continued_line() {
    // A trailing backslash joins the next line before compiling.
    let output = repl("1 \\\n2 +\nexit\n");
    assert!(output.status.success());
    assert!(
        stdout_of(&output).contains("Success"),
        "expected the joined line to compile, stderr: {}",
        stderr_of(&output)
    );
    assert!(
        stdout_of(&output).contains("...>"),
        "expected the continuation prompt"
    );
}

#[test]
fn the_repl_continues_an_incomplete_instruction() {
    // `StoreVar` needs an operand, so the repl waits for the next line.
    let output = repl("StoreVar\n0\nexit\n");
    assert!(output.status.success());
    assert!(
        stdout_of(&output).contains("...>"),
        "expected the continuation prompt, got: {}",
        stdout_of(&output)
    );
}

#[test]
fn the_repl_ignores_blank_lines() {
    let output = repl("\n\n1 2 +\nexit\n");
    assert!(output.status.success());
    assert_eq!(
        stdout_of(&output).matches("Success").count(),
        1,
        "blank lines should not be evaluated"
    );
}

#[test]
fn the_repl_exits_on_end_of_input() {
    // No `exit`, just a closed stdin.
    let output = repl("1 2 +\n");
    assert!(output.status.success());
}

#[test]
fn the_repl_reports_a_comment_only_line_without_running_it() {
    let output = repl("# just a comment\nexit\n");
    assert!(output.status.success());
    assert!(
        !stdout_of(&output).contains("Success"),
        "an empty program should not report success"
    );
}
