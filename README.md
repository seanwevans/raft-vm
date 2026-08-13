# Raft - A Lightweight Virtual Machine for Concurrent Systems
<img width="256" alt="raft in muted colors" src="https://github.com/user-attachments/assets/7d26552d-c53a-4440-bb0b-e09b6353cd07" />

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
                     native standard-library bindings, and execution behaviors.

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

# store the result, load it twice, and clean up the duplicate
StoreVar 0
LoadVar 0
Dup
Swap
Pop

# push a boolean and a float
true 3.14
```

The compiler now uses a lexer/parser pipeline that supports integers, floats,
booleans (`true`/`false`), string literals with spaces (for example,
`"hello raft vm"`), `#` and `//` comments, textual labels such as `.loop`,
basic arithmetic like `+`, and stack/variable keywords such as `StoreVar`,
`LoadVar`, `Pop`, `Dup`, and `Swap`. Running the above file will leave `3`,
`true`, and `3.14` on the VM's stack.

Native standard-library functions are injected when a VM starts. For example,
`io.print` loads the standard `io` module's `print` export, and `CallNative 1`
invokes it with one stack argument:

```
42 io.print CallNative 1
```

The REPL keeps running after compiler or runtime errors. Enter a trailing `\`
or an incomplete instruction such as `StoreVar` to continue on the next line;
enter `exit` at a fresh prompt to quit.

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
The VM operates through its runtime, message-passing interfaces, and native
standard-library bindings. Host Rust functions can be registered as heap-backed
native functions and exposed through modules such as `io`, allowing platform I/O
without adding dedicated I/O bytecode instructions.

### Opcodes
Raft uses a custom bytecode instruction set that mirrors fundamental operations:
- **Arithmetic**: `Add`, `Sub`, `Mul`, `Div`, `Mod`, `Neg`, `Exp`
- **Comparison**: `Eq`, `Ne`, `Lt`, `Le`, `Gt`, `Ge`
- **Logic**: `Not`, `And`, `Or`
- **Stack**: `PushConst`, `Pop`, `Dup`, `Swap`
- **Modules/Globals**: `LoadGlobal`, `GetExport`, `CallNative`
- **Control Flow**: `Jump`, `JumpIfFalse`, `Call`, `CallNative`, `Return`
- **Actor Management**: `SpawnActor`, `SendMessage`, `ReceiveMessage`
- **Supervision**: `SpawnSupervisor`, `SetStrategy`, `RestartChild`

### Comparison and control flow
Comparisons push a boolean, which is what `JumpIfFalse` branches on. `Eq` and
`Ne` are defined for every pair of values -- comparing across types answers
`false` rather than failing, and two references are equal when they name the
same heap object. The ordering operators (`Lt`, `Le`, `Gt`, `Ge`) require two
integers or two floats, since ordering across types has no meaning.

`Not`, `And` and `Or` take booleans. They cannot short-circuit: both operands
are already on the stack by the time the operator runs.

Together these make a loop that exits on a computed value expressible:

```
0 StoreVar 0        # total
5 StoreVar 1        # counter

.loop
LoadVar 1 0 Gt      # counter > 0 ?
JumpIfFalse .done
  LoadVar 0 LoadVar 1 + StoreVar 0
  LoadVar 1 1 - StoreVar 1
Jump .loop

.done
LoadVar 0 io.print CallNative 1
```

See `examples/loop.raft`, which prints `15`.

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
