// src/main.rs

// Example usage:
//   $ raft run example.raft
//   $ raft repl
//   $ raft version
//   $ raft help [command]

use clap::{CommandFactory, Parser, Subcommand};
use std::fs;
use std::process;

use raft::compiler::{Compiler, CompilerError};
use raft::vm::value::Value;
use raft::vm::{VmError, VM};

use std::io::Write;
use tokio::io::{self, AsyncBufReadExt};

#[derive(Parser)]
#[command(name = "raft", author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Run { filename: String },
    Repl,
    Version,
}

#[tokio::main]
async fn main() {
    // Initialize env_logger so log output respects RUST_LOG
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Run { filename }) => handle_run(&filename).await,
        Some(Commands::Repl) => start_repl().await,
        Some(Commands::Version) => print_version(),
        None => print_help(),
    }
}

fn print_help() {
    Cli::command().print_long_help().unwrap();
    println!();
}

fn print_version() {
    println!("Raft version {}", raft::VERSION);
}

async fn handle_run(filename: &str) {
    match fs::read_to_string(filename) {
        Ok(source) => {
            let bytecode = match Compiler::compile(&source) {
                Ok(b) => b,
                Err(e) => {
                    let err: VmError = e.into();
                    eprintln!("{}", err);
                    process::exit(1);
                }
            };
            let (mut vm, tx) = VM::new(bytecode, None);

            // Simulate sending messages to the VM
            tokio::spawn(async move {
                if let Err(e) = tx.send(Value::Integer(42)).await {
                    eprintln!("Send error: {}", e);
                }
                if let Err(e) = tx.send(Value::Boolean(true)).await {
                    eprintln!("Send error: {}", e);
                }
            });

            if let Err(e) = vm.run().await {
                eprintln!("Execution error: {}", e);
                process::exit(1);
            }
        }
        Err(e) => handle_file_error(e),
    }
}

fn handle_file_error(e: std::io::Error) -> ! {
    eprintln!("File error: {}", e);
    process::exit(1);
}

async fn start_repl() {
    let stdin = io::stdin();
    let mut reader = io::BufReader::new(stdin);
    let mut input = String::new();
    let mut buffer = String::new();

    loop {
        if buffer.is_empty() {
            print!("raft> ");
        } else {
            print!("...> ");
        }
        std::io::stdout().flush().unwrap();
        input.clear();

        if let Err(e) = reader.read_line(&mut input).await {
            eprintln!("Read error: {}", e);
            continue;
        }

        if input.is_empty() {
            break;
        }

        if buffer.is_empty() && input.trim() == "exit" {
            break;
        }

        let blank_line = input.trim().is_empty();
        if blank_line && buffer.is_empty() {
            continue;
        }

        let line_continues = input.trim_end().ends_with('\\');
        if line_continues {
            let without_slash = input.trim_end_matches(['\n', '\r']).trim_end_matches('\\');
            buffer.push_str(without_slash);
            buffer.push('\n');
            continue;
        }

        buffer.push_str(&input);

        match Compiler::compile(&buffer) {
            Ok(bytecode) => {
                if bytecode.is_empty() {
                    buffer.clear();
                    continue;
                }

                let (mut vm, _tx) = VM::new(bytecode, None);
                match vm.run().await {
                    Ok(_) => println!("Success"),
                    Err(e) => eprintln!("Error: {}", e),
                }
                buffer.clear();
            }
            Err(e) if !blank_line && repl_input_is_incomplete(&buffer, &e) => {
                continue;
            }
            Err(e) => {
                let err: VmError = e.into();
                eprintln!("Error: {}", err);
                buffer.clear();
            }
        }
    }
}

fn repl_input_is_incomplete(source: &str, error: &CompilerError) -> bool {
    match error {
        CompilerError::InvalidAddress(message) => message.starts_with("expected "),
        CompilerError::InvalidToken(_)
        | CompilerError::ParseError(_)
        | CompilerError::ParseErrorAt { .. } => delimiters_are_unclosed(source),
    }
}

fn delimiters_are_unclosed(source: &str) -> bool {
    let mut parens = 0usize;
    let mut braces = 0usize;
    let mut brackets = 0usize;

    for ch in source.chars() {
        match ch {
            '(' => parens += 1,
            ')' => parens = parens.saturating_sub(1),
            '{' => braces += 1,
            '}' => braces = braces.saturating_sub(1),
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            _ => {}
        }
    }

    parens > 0 || braces > 0 || brackets > 0
}

// Removed unused utility functions that produced dead-code warnings.
