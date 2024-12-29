// src/main.rs

// Example usage:
//   $ raft run example.raft
//   $ raft repl
//   $ raft version
//   $ raft help [command]

use raft::{self, run};
use std::env;
use std::fs;
use std::process;

use raft::vm::value::Value;
use raft::vm::backend::Backend;
use raft::compiler::Compiler;
use raft::vm::VM;


#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage_and_exit();
    }

    match args[1].as_str() {
        "help" | "--help" | "-?" => print_help(),
        "version" | "--version" | "-v" => print_version(),
        "run" => handle_run(&args).await,
        "repl" => start_repl().await,
        cmd => unknown_command(cmd),
    }
}

fn print_help() {
    println!("HALP");
}

fn print_version() {
    println!("Raft version {}", raft::VERSION);
}

async fn handle_run(args: &[String]) {
    if let Some(filename) = args.get(2) {
        match fs::read_to_string(filename) {
            Ok(source) => {
                let bytecode = Compiler::compile(&source).unwrap();
                let (mut vm, tx) = VM::new(bytecode, None, Backend::default());

                // Simulate sending messages to the VM
                tokio::spawn(async move {
                    tx.send(Value::Integer(42)).await.unwrap();
                    tx.send(Value::Boolean(true)).await.unwrap();
                });

                if let Err(e) = vm.run().await {
                    eprintln!("Execution error: {}", e);
                    process::exit(1);
                }
            }
            Err(e) => handle_file_error(e),
        }
    } else {
        eprintln!("Error: Missing filename.");
        print_usage_and_exit();
    }
}


fn handle_file_error(e: std::io::Error) -> ! {
    eprintln!("File error: {}", e);
    process::exit(1);
}

async fn start_repl() {
    loop {
        print!("raft> ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
        if input.trim() == "exit" {
            break;
        }
        match run(&input).await {
            Ok(_) => println!("Success"),
            Err(e) => eprintln!("Error: {}", e),
        }
    }
}


fn unknown_command(cmd: &str) -> ! {
    eprintln!("Unknown command: {}
Usage: raft [run <filename>|repl|--version]", cmd);
    process::exit(1);
}

fn print_usage_and_exit() -> ! {
    eprintln!("Usage: raft [run <filename>|repl|--version]");
    process::exit(1);
}
