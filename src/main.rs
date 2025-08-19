// src/main.rs

// Example usage:
//   $ raft run example.raft
//   $ raft repl
//   $ raft version
//   $ raft help [command]

use clap::{CommandFactory, Parser, Subcommand};
use raft::{self, run};
use std::fs;
use std::process;

use raft::compiler::Compiler;
use raft::vm::value::Value;
use raft::vm::{VmError, VM};

use std::io::Write;
use tokio::io::{self, AsyncBufReadExt};

#[derive(Parser)]
#[command(name = "raft",author, version, about, long_about = None)]

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

    loop {
        print!("raft> ");
        std::io::stdout().flush().unwrap();
        input.clear();

        if reader.read_line(&mut input).await.unwrap() == 0 {
            break;
        }

        if input.trim() == "exit" {
            break;
        }
        match run(&input).await {
            Ok(_) => println!("Success"),
            Err(e) => eprintln!("Error: {}", e),
        }
    }
}

// Removed unused utility functions that produced dead-code warnings.
