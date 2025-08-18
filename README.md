# Raft - A Lightweight Virtual Machine for Concurrent Systems

Raft is a lightweight, interpreted virtual machine (VM) designed to provide 
robust concurrency, fault tolerance, and actor-based message-passing models. 
Inspired by Erlang’s concurrency model and Rust’s safety principles, Raft 
focuses on enabling parallel, resilient execution environments while maintaining 
a simple and extensible design.

## Key Features
- **Actor Model**: Supports spawning actors, sending messages, and managing 
                   isolated execution contexts.
- **Supervisor Trees**: Implements supervision strategies to ensure fault 
                        tolerance by restarting failed processes.
- **Stack-Based Execution**: Operates on a stack-based virtual machine with a 
                             custom bytecode instruction set.
- **Concurrent Execution**: Built with asynchronous, non-blocking paradigms 
                            using Tokio.
- **Dynamic Heap Management**: Allocates and manages memory with garbage 
                               collection and safe reference counting.
- **Extensibility**: Designed for modular expansion of opcodes, heap structures,
                     and execution behaviors.

---

## Table of Contents
- [Getting Started](#getting-started)
- [Installation](#installation)
- [Usage](#usage)
- [Architecture](#architecture)
- [Opcode Reference](#opcode-reference)
- [Testing](#testing)
- [Contributing](#contributing)
- [License](#license)

---

## Getting Started

### Prerequisites
- Rust (1.65+)
- Tokio (for asynchronous runtime)
- Clang (for native function compilation, optional)

---

## Installation
Clone the repository and build the project using Cargo:

```bash
git clone https://github.com/user/raft-vm.git
cd raft-vm
cargo build --release
```

---

## Usage
Run a `.raft` script or start an interactive REPL:

```
# Execute a Raft script
cargo run -- run script.raft

# Start the REPL
cargo run -- repl

# Display version
cargo run -- --version
```

Logging is controlled via the `RUST_LOG` environment variable. Enable info-level
output like so:

```
RUST_LOG=info cargo run -- run script.raft
```

Example `.raft` file:
```
# push 1 and 2 on the stack and add them
1 2 +
```

The current compiler only tokenizes whitespace separated integers and
the `+` operator. Running the above file will leave `3` on the VM's
stack.

---

## Architecture

### Components
- **VM (Virtual Machine)**: Manages bytecode execution, stack, heap, and message
                            passing.
- **Execution Context**: Maintains the state of the current program, including
                         the instruction pointer and call stack.
- **Heap**: Allocates and manages dynamic memory for arrays, strings, and
            modules.
- **Opcodes**: Define the core instruction set for the VM, such as arithmetic,
               stack manipulation, and control flow.
 
### Platform Integration
The previous codebase included a placeholder `Backend` abstraction intended for
platform-specific hooks. The VM no longer depends on this layer and operates
solely through its runtime and message-passing interfaces. Future integrations
can be added by extending opcodes or runtime components as needed.

### Opcodes
Raft uses a custom bytecode instruction set that mirrors fundamental operations:
- **Arithmetic**: `Add`, `Sub`, `Mul`, `Div`, `Mod`, `Neg`, `Exp`
- **Stack**: `PushConst`, `Pop`, `Dup`, `Swap`
- **Control Flow**: `Jump`, `JumpIfFalse`, `Call`, `Return`
- **Actor Management**: `SpawnActor`, `SendMessage`, `ReceiveMessage`
- **Supervision**: `SpawnSupervisor`, `SetStrategy`, `RestartChild`

---


## Testing
Run the test suite with Cargo:

```bash
cargo test
```

Build the project in release mode:

```bash
cargo build --release
```

## Contributing
Contributions are welcome! Please open an issue or submit a pull request.

## License
Distributed under the MIT License. See [LICENSE](LICENSE) for details.
