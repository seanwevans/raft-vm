// src/vm/opcodes.rs

use crate::vm::error::VmError;
use crate::vm::execution::{BlockingOperation, ExecutionContext, ExecutionState, ProcessContext};
use crate::vm::heap::{Heap, HeapObject, NativeFunction, ProcessHandle};
use crate::vm::supervision::ChildSpec;
use crate::vm::value::{MessageValue, Value};
use crate::vm::vm::VM;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::error::TrySendError;

fn unary_op<F>(execution: &mut ExecutionContext, heap: &mut Heap, f: F) -> Result<(), VmError>
where
    F: Fn(Value) -> Result<Value, VmError>,
{
    let v = pop_value(execution, heap)?;
    let result = f(v)?;
    execution.stack.push(result);
    Ok(())
}

fn binary_op<F>(execution: &mut ExecutionContext, heap: &mut Heap, f: F) -> Result<(), VmError>
where
    F: Fn(Value, Value) -> Result<Value, VmError>,
{
    if execution.stack.len() < 2 {
        log::error!("Stack underflow during binary operation");
        return Err(VmError::StackUnderflow);
    }
    let b = pop_value(execution, heap)?;
    let a = pop_value(execution, heap)?;
    let result = f(a, b)?;
    execution.stack.push(result);
    Ok(())
}

fn increment_reference(heap: &mut Heap, address: usize) -> Result<(), VmError> {
    if let Some(object) = heap.get_mut(address) {
        object.increment_ref();
        Ok(())
    } else {
        Err(VmError::InvalidReference)
    }
}

fn actor_start_ip(heap: &Heap, address: usize) -> Result<usize, VmError> {
    match heap.get(address) {
        Some(HeapObject::Actor(process, _, _)) => Ok(process.restart_ip()),
        _ => Err(VmError::InvalidReference),
    }
}

fn run_process(
    mut vm: VM,
    final_stack: Arc<Mutex<Vec<Value>>>,
) -> tokio::task::JoinHandle<Result<(), VmError>> {
    tokio::spawn(async move {
        let result = vm.run().await;
        if let Ok(mut stack) = final_stack.lock() {
            *stack = vm.stack().clone();
        }
        result
    })
}

fn spawn_child_vm(
    bytecode: Vec<OpCode>,
    debug_info: Option<crate::compiler::DebugInfo>,
    start_ip: usize,
    parent: Option<&ProcessContext>,
    trap_exits: bool,
    links: Vec<tokio::sync::mpsc::Sender<MessageValue>>,
) -> Result<(ProcessHandle, tokio::sync::mpsc::Sender<MessageValue>), VmError> {
    let (mut vm, tx) = VM::new_with_debug(bytecode, debug_info, None)?;
    vm.set_ip(start_ip);
    vm.set_restart_ip(start_ip);
    vm.set_trap_exits(trap_exits);
    for link in links {
        vm.link(link);
    }
    if let Some(process) = parent {
        vm.set_parent(process.process_id);
        vm.link(process.self_sender.clone());
        if process.trap_exits {
            vm.set_trap_exits(true);
        }
    }

    let handle = {
        let process_id = vm.process_id();
        let parent = vm.parent();
        let bytecode = vm.bytecode();
        let debug_info = vm.debug_info();
        let links = vm.links();
        let trap_exits = vm.trap_exits();
        let final_stack = Arc::new(Mutex::new(Vec::new()));
        let task = run_process(vm, final_stack.clone());
        ProcessHandle::new(
            process_id,
            parent,
            start_ip,
            bytecode,
            debug_info,
            links,
            trap_exits,
            task,
            final_stack,
        )
    };
    Ok((handle, tx))
}

fn restart_actor(heap: &mut Heap, child: ChildSpec) -> Result<(), VmError> {
    match heap.get_mut(child.reference) {
        Some(HeapObject::Actor(process, sender, _)) => {
            let (mut vm, replacement_tx) =
                match VM::new_with_debug(process.bytecode(), process.debug_info(), None) {
                    Ok(vm) => vm,
                    Err(error) => {
                        log::error!("Failed to restart actor VM: {}", error);
                        return Err(error);
                    }
                };
            vm.set_ip(child.start_ip);
            vm.set_restart_ip(child.start_ip);
            vm.set_trap_exits(process.trap_exits());
            if let Some(parent) = process.parent() {
                vm.set_parent(parent);
            }
            for link in process.links() {
                vm.link(link);
            }
            *sender = replacement_tx.clone();
            let final_stack = Arc::new(Mutex::new(Vec::new()));
            process.replace_runtime(run_process(vm, final_stack.clone()), child.start_ip);
            log::info!(
                "Restarted actor {} at ip {}",
                child.reference,
                child.start_ip
            );
            Ok(())
        }
        _ => Err(VmError::InvalidReference),
    }
}

fn push_value(
    execution: &mut ExecutionContext,
    heap: &mut Heap,
    value: Value,
) -> Result<(), VmError> {
    if let Value::Reference(address) = value {
        increment_reference(heap, address)?;
    }
    execution.stack.push(value);
    Ok(())
}

fn push_existing_value(execution: &mut ExecutionContext, value: Value) {
    execution.stack.push(value);
}

fn pop_value(execution: &mut ExecutionContext, heap: &mut Heap) -> Result<Value, VmError> {
    if let Some(value) = execution.stack.pop() {
        if let Value::Reference(address) = value {
            heap.release_reference(address)?;
        }
        Ok(value)
    } else {
        Err(VmError::StackUnderflow)
    }
}

fn pop_raw_value(execution: &mut ExecutionContext) -> Result<Value, VmError> {
    execution.stack.pop().ok_or(VmError::StackUnderflow)
}

fn expect_usize(value: Value, operation: &'static str) -> Result<usize, VmError> {
    match value {
        Value::Integer(index) if index >= 0 => Ok(index as usize),
        _ => Err(VmError::TypeMismatch(operation)),
    }
}

fn retain_value(heap: &mut Heap, value: &Value) -> Result<(), VmError> {
    if let Value::Reference(address) = value {
        increment_reference(heap, *address)?;
    }
    Ok(())
}

fn release_value(heap: &mut Heap, value: Value) -> Result<(), VmError> {
    if let Value::Reference(address) = value {
        heap.release_reference(address)?;
    }
    Ok(())
}

fn make_array(
    execution: &mut ExecutionContext,
    heap: &mut Heap,
    length: usize,
) -> Result<(), VmError> {
    if execution.stack.len() < length {
        return Err(VmError::StackUnderflowFor("MakeArray"));
    }

    let mut elements = Vec::with_capacity(length);
    for _ in 0..length {
        elements.push(pop_value(execution, heap)?);
    }
    elements.reverse();

    for value in &elements {
        retain_value(heap, &value)?;
    }

    let address = heap.allocate(HeapObject::Array(elements, 0));
    push_value(execution, heap, Value::Reference(address))
}

fn array_get(execution: &mut ExecutionContext, heap: &mut Heap) -> Result<(), VmError> {
    let index = expect_usize(pop_value(execution, heap)?, "ArrayGet")?;
    let array_ref = pop_raw_value(execution)?;
    let element = match array_ref {
        Value::Reference(address) => match heap.get(address) {
            Some(HeapObject::Array(elements, _)) => {
                elements
                    .get(index)
                    .cloned()
                    .ok_or(VmError::IndexOutOfBounds {
                        index,
                        length: elements.len(),
                    })?
            }
            _ => return Err(VmError::InvalidReference),
        },
        _ => return Err(VmError::TypeMismatch("ArrayGet")),
    };

    release_value(heap, array_ref)?;
    push_value(execution, heap, element)
}

fn array_set(execution: &mut ExecutionContext, heap: &mut Heap) -> Result<(), VmError> {
    let new_value = pop_value(execution, heap)?;
    let index = expect_usize(pop_value(execution, heap)?, "ArraySet")?;
    let array_ref = pop_raw_value(execution)?;
    let address = match array_ref {
        Value::Reference(address) => address,
        _ => return Err(VmError::TypeMismatch("ArraySet")),
    };

    retain_value(heap, &new_value)?;
    let old_value = match heap.get_mut(address) {
        Some(HeapObject::Array(elements, _)) => {
            if index >= elements.len() {
                let length = elements.len();
                release_value(heap, new_value)?;
                return Err(VmError::IndexOutOfBounds { index, length });
            }
            std::mem::replace(&mut elements[index], new_value)
        }
        _ => {
            release_value(heap, new_value)?;
            return Err(VmError::InvalidReference);
        }
    };
    release_value(heap, old_value)?;

    push_existing_value(execution, Value::Reference(address));
    Ok(())
}

fn string_concat(execution: &mut ExecutionContext, heap: &mut Heap) -> Result<(), VmError> {
    let rhs = pop_raw_value(execution)?;
    let lhs = pop_raw_value(execution)?;

    let mut concatenated = match lhs {
        Value::Reference(address) => match heap.get(address) {
            Some(HeapObject::String(value, _)) => value.clone(),
            _ => return Err(VmError::InvalidReference),
        },
        _ => return Err(VmError::TypeMismatch("StringConcat")),
    };

    match rhs {
        Value::Reference(address) => match heap.get(address) {
            Some(HeapObject::String(value, _)) => concatenated.push_str(value),
            _ => return Err(VmError::InvalidReference),
        },
        _ => return Err(VmError::TypeMismatch("StringConcat")),
    }

    release_value(heap, lhs)?;
    release_value(heap, rhs)?;

    let address = heap.allocate(HeapObject::String(concatenated, 0));
    push_value(execution, heap, Value::Reference(address))
}

fn make_module(
    execution: &mut ExecutionContext,
    heap: &mut Heap,
    names: &[String],
) -> Result<(), VmError> {
    if execution.stack.len() < names.len() {
        return Err(VmError::StackUnderflowFor("MakeModule"));
    }

    let mut values = Vec::with_capacity(names.len());
    for _ in names {
        values.push(pop_value(execution, heap)?);
    }
    values.reverse();

    let mut exports = std::collections::HashMap::with_capacity(names.len());
    for (name, value) in names.iter().cloned().zip(values.into_iter()) {
        retain_value(heap, &value)?;
        exports.insert(name, value);
    }

    let address = heap.allocate(HeapObject::Module {
        name: String::new(),
        exports,
        ref_count: 0,
    });
    push_value(execution, heap, Value::Reference(address))
}

fn module_get(
    execution: &mut ExecutionContext,
    heap: &mut Heap,
    name: &str,
) -> Result<(), VmError> {
    let module_ref = pop_raw_value(execution)?;
    let value = match module_ref {
        Value::Reference(address) => match heap.get(address) {
            Some(HeapObject::Module { exports, .. }) => exports
                .get(name)
                .cloned()
                .ok_or_else(|| VmError::Message(format!("Module export not found: {name}")))?,
            _ => return Err(VmError::InvalidReference),
        },
        _ => return Err(VmError::TypeMismatch("ModuleGet")),
    };
    release_value(heap, module_ref)?;
    push_value(execution, heap, value.clone())
}

fn module_set(
    execution: &mut ExecutionContext,
    heap: &mut Heap,
    name: &str,
) -> Result<(), VmError> {
    let new_value = pop_value(execution, heap)?;
    let module_ref = pop_raw_value(execution)?;
    let address = match module_ref {
        Value::Reference(address) => address,
        _ => return Err(VmError::TypeMismatch("ModuleSet")),
    };

    retain_value(heap, &new_value)?;
    let old_value = match heap.get_mut(address) {
        Some(HeapObject::Module { exports, .. }) => exports.insert(name.to_string(), new_value),
        _ => {
            release_value(heap, new_value)?;
            return Err(VmError::InvalidReference);
        }
    };
    if let Some(old_value) = old_value {
        release_value(heap, old_value)?;
    }

    push_value(execution, heap, Value::Reference(address))
}

fn native_function_at(
    execution: &ExecutionContext,
    heap: &Heap,
    stack_index: usize,
) -> Option<NativeFunction> {
    match execution.stack.get(stack_index) {
        Some(Value::Reference(address)) => match heap.get(*address) {
            Some(HeapObject::NativeFunction(function, _)) => Some(function.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn call_native(
    execution: &mut ExecutionContext,
    heap: &mut Heap,
    arity: usize,
) -> Result<(), VmError> {
    if execution.stack.len() < arity + 1 {
        return Err(VmError::StackUnderflowFor("CallNative"));
    }

    let top_index = execution.stack.len() - 1;
    let function_index = execution.stack.len() - arity - 1;
    let function = native_function_at(execution, heap, top_index)
        .or_else(|| native_function_at(execution, heap, function_index))
        .ok_or(VmError::InvalidReference)?;

    if function.arity != arity {
        return Err(VmError::NativeArityMismatch {
            expected: function.arity,
            actual: arity,
        });
    }

    let callable_on_top = native_function_at(execution, heap, top_index).is_some();
    let mut args = Vec::with_capacity(arity);
    if callable_on_top {
        pop_value(execution, heap)?;
        for _ in 0..arity {
            args.push(pop_value(execution, heap)?);
        }
        args.reverse();
    } else {
        for _ in 0..arity {
            args.push(pop_value(execution, heap)?);
        }
        args.reverse();
        pop_value(execution, heap)?;
    }

    let result = (function.function)(args)?;
    push_value(execution, heap, result)
}

#[derive(Debug, Clone)]
pub struct Bytecode {
    opcodes: Vec<Arc<OpCode>>,
}

impl Bytecode {
    pub fn new(opcodes: Vec<OpCode>) -> Self {
        Self {
            opcodes: opcodes.into_iter().map(Arc::new).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.opcodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.opcodes.is_empty()
    }

    pub fn decode(&self, ip: usize) -> Result<Arc<OpCode>, VmError> {
        self.opcodes
            .get(ip)
            .cloned()
            .ok_or(VmError::ExecutionOutOfBounds)
    }

    pub fn opcodes(&self) -> Vec<OpCode> {
        self.opcodes
            .iter()
            .map(|opcode| opcode.as_ref().clone())
            .collect()
    }
}

impl From<Vec<OpCode>> for Bytecode {
    fn from(value: Vec<OpCode>) -> Self {
        Bytecode::new(value)
    }
}

#[derive(Debug, Clone)]
pub enum OpCode {
    // Variables
    StoreVar(usize),
    LoadVar(usize),
    LoadGlobal(String),
    GetExport(String),

    // Stack
    PushConst(Value),
    MakeArray(usize),
    Pop,
    Dup,
    Swap,

    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,
    Exp,

    // Heap values
    ArrayGet,
    ArraySet,
    MakeString(String),
    PushString(String),
    StringConcat,
    MakeModule(Vec<String>),
    ModuleGet(String),
    ModuleSet(String),
    MakeNativeFunction(NativeFunction),
    CallNative(usize),

    // Control Flow
    Jump(usize),
    JumpIfFalse(usize),
    Call(usize),
    Return,

    // Actors
    SpawnActor(usize),
    SendMessage,
    ReceiveMessage,

    // Supervisor
    SpawnSupervisor(usize),
    SetStrategy(usize),
    RestartChild(usize),
}

impl OpCode {
    pub fn execute(
        &self,
        execution: &mut ExecutionContext,
        heap: &mut Heap,
    ) -> Result<ExecutionState, VmError> {
        self.execute_with_process(execution, heap, None)
    }

    pub fn execute_with_process(
        &self,
        execution: &mut ExecutionContext,
        heap: &mut Heap,
        process: Option<ProcessContext>,
    ) -> Result<ExecutionState, VmError> {
        let result: Result<(), VmError> = match self {
            OpCode::Add => binary_op(execution, heap, |a, b| a.checked_add(b)),
            OpCode::Sub => binary_op(execution, heap, |a, b| a.checked_sub(b)),
            OpCode::Mul => binary_op(execution, heap, |a, b| a.checked_mul(b)),
            OpCode::Div => binary_op(execution, heap, |a, b| a.checked_div(b)),
            OpCode::Neg => unary_op(execution, heap, |a| match a {
                Value::Integer(i) => i32::checked_neg(i)
                    .map(Value::Integer)
                    .ok_or(VmError::IntegerOverflow),
                Value::Float(f) => Ok(Value::Float(-f)),
                _ => Err(VmError::TypeMismatch("Neg")),
            }),
            OpCode::PushConst(v) => push_value(execution, heap, v.clone()),
            OpCode::MakeArray(length) => make_array(execution, heap, *length),
            OpCode::Pop => {
                pop_value(execution, heap)?;
                Ok(())
            }
            OpCode::Dup => {
                if let Some(value) = execution.stack.last() {
                    push_value(execution, heap, value.clone())
                } else {
                    Err(VmError::StackUnderflow)
                }
            }
            OpCode::Swap => {
                if execution.stack.len() < 2 {
                    return Err(VmError::StackUnderflowFor("Swap"));
                }
                let len = execution.stack.len();
                execution.stack.swap(len - 1, len - 2);
                Ok(())
            }
            OpCode::StoreVar(index) => {
                let value = pop_raw_value(execution)?;

                if let Some(previous) = execution.locals.insert(*index, value) {
                    release_value(heap, previous)?;
                }

                Ok(())
            }
            OpCode::LoadVar(index) => {
                if let Some(value) = execution.locals.get(index) {
                    push_value(execution, heap, value.clone())
                } else {
                    Err(VmError::VariableNotFound(*index))
                }
            }
            OpCode::LoadGlobal(name) => {
                let value = execution
                    .globals
                    .get(name)
                    .cloned()
                    .ok_or_else(|| VmError::GlobalNotFound(name.clone()))?;
                push_value(execution, heap, value.clone())
            }
            OpCode::GetExport(export) => {
                let module_ref = pop_value(execution, heap)?;
                let Value::Reference(address) = module_ref else {
                    return Err(VmError::InvalidReference);
                };

                let (module_name, value) = match heap.get(address) {
                    Some(HeapObject::Module { name, exports, .. }) => {
                        let value = exports.get(export).cloned().ok_or_else(|| {
                            VmError::ExportNotFound {
                                module: name.clone(),
                                export: export.clone(),
                            }
                        })?;
                        (name.clone(), value)
                    }
                    _ => return Err(VmError::InvalidReference),
                };

                log::info!("Loaded export {} from module {}", export, module_name);
                push_value(execution, heap, value.clone())
            }
            OpCode::Mod => binary_op(execution, heap, |a, b| match (a, b) {
                (Value::Integer(x), Value::Integer(y)) => {
                    if y == 0 {
                        Err(VmError::DivisionByZero)
                    } else {
                        Ok(Value::Integer(x % y))
                    }
                }
                _ => Err(VmError::TypeMismatch("Mod")),
            }),
            OpCode::Exp => binary_op(execution, heap, |a, b| match (a, b) {
                (Value::Integer(x), Value::Integer(y)) => {
                    if y < 0 {
                        Ok(Value::Float((x as f64).powi(y)))
                    } else {
                        i32::checked_pow(x, y as u32)
                            .map(Value::Integer)
                            .ok_or(VmError::IntegerOverflow)
                    }
                }
                (Value::Float(x), Value::Float(y)) => Ok(Value::Float(x.powf(y))),
                _ => Err(VmError::TypeMismatch("Exp")),
            }),
            OpCode::ArrayGet => array_get(execution, heap),
            OpCode::ArraySet => array_set(execution, heap),
            OpCode::MakeString(value) | OpCode::PushString(value) => {
                let address = heap.allocate(HeapObject::String(value.clone(), 0));
                push_value(execution, heap, Value::Reference(address))
            }
            OpCode::StringConcat => string_concat(execution, heap),
            OpCode::MakeModule(names) => make_module(execution, heap, names),
            OpCode::ModuleGet(name) => module_get(execution, heap, name),
            OpCode::ModuleSet(name) => module_set(execution, heap, name),
            OpCode::MakeNativeFunction(function) => {
                let address = heap.allocate(HeapObject::NativeFunction(function.clone(), 0));
                push_value(execution, heap, Value::Reference(address))
            }
            OpCode::Jump(target) => {
                if *target > execution.bytecode.len() {
                    log::error!(
                        "Jump target {} out of bounds (bytecode length {})",
                        target,
                        execution.bytecode.len()
                    );
                    return Err(VmError::ExecutionOutOfBounds);
                }

                execution.ip = *target;
                Ok(())
            }

            OpCode::JumpIfFalse(target) => {
                let value = pop_value(execution, heap)?;
                match value {
                    Value::Boolean(false) => {
                        if *target > execution.bytecode.len() {
                            log::error!(
                                "JumpIfFalse target {} out of bounds (bytecode length {})",
                                target,
                                execution.bytecode.len()
                            );
                            return Err(VmError::ExecutionOutOfBounds);
                        }
                        execution.ip = *target;
                        Ok(())
                    }
                    Value::Boolean(true) => Ok(()),
                    _ => Err(VmError::TypeMismatch("JumpIfFalse")),
                }
            }
            OpCode::Call(addr) => {
                if *addr >= execution.bytecode.len() {
                    log::error!(
                        "Call target {} out of bounds (bytecode length {})",
                        addr,
                        execution.bytecode.len()
                    );
                    return Err(VmError::ExecutionOutOfBounds);
                }

                execution.call_stack.push(execution.ip);
                execution.ip = *addr;
                Ok(())
            }

            OpCode::CallNative(arity) => call_native(execution, heap, *arity),
            OpCode::Return => {
                if let Some(return_addr) = execution.call_stack.pop() {
                    execution.ip = return_addr;
                    Ok(())
                } else {
                    execution.ip = execution.bytecode.len();
                    Ok(())
                }
            }
            OpCode::ReceiveMessage => match execution.mailbox_mut().try_recv() {
                Ok(message) => {
                    log::info!("Received message: {:?}", message);
                    let value = heap.message_to_value(message)?;
                    push_value(execution, heap, value.clone())
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    return Ok(ExecutionState::Yield(BlockingOperation::ReceiveMessage));
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    log::warn!("Mailbox is closed");
                    Err(VmError::MailboxDisconnected)
                }
            },

            OpCode::SpawnActor(addr) => {
                if *addr >= execution.bytecode.len() {
                    log::error!(
                        "SpawnActor target {} out of bounds (bytecode length {})",
                        addr,
                        execution.bytecode.len()
                    );
                    return Err(VmError::ExecutionOutOfBounds);
                }
                let (handle, tx) = spawn_child_vm(
                    execution.bytecode.opcodes(),
                    execution.debug_info.clone(),
                    *addr,
                    process.as_ref(),
                    false,
                    Vec::new(),
                )?;
                let address = heap.allocate(HeapObject::Actor(handle, tx, 0));
                push_value(execution, heap, Value::Reference(address))
            }
            OpCode::SendMessage => {
                // SendMessage consumes the message and leaves the actor reference
                // on the stack (or restores it on failure/yield).
                let actor_ref = pop_raw_value(execution)?;
                let message = pop_value(execution, heap)?;
                if let Value::Reference(address) = actor_ref {
                    let sender = match heap.get(address) {
                        Some(HeapObject::Actor(_, sender, _)) => sender.clone(),
                        _ => return Err(VmError::InvalidReference),
                    };
                    if let Value::Reference(message_address) = message {
                        increment_reference(heap, message_address)?;
                    }
                    let message_for_channel = match heap.value_to_message(message.clone()) {
                        Ok(message_for_channel) => message_for_channel,
                        // A value the mailbox cannot represent (cyclic, too
                        // deeply nested, or an unsendable handle) fails the
                        // send without stranding the reference retained above.
                        Err(error) => {
                            push_existing_value(execution, Value::Reference(address));
                            release_value(heap, message)?;
                            return Err(error);
                        }
                    };
                    match sender.try_send(message_for_channel) {
                        Ok(()) => {
                            release_value(heap, message)?;
                            push_existing_value(execution, Value::Reference(address));
                            Ok(())
                        }
                        Err(TrySendError::Full(message_for_channel)) => {
                            release_value(heap, message)?;
                            release_value(heap, actor_ref)?;
                            return Ok(ExecutionState::Yield(BlockingOperation::SendMessage {
                                sender,
                                actor_address: address,
                                message: message_for_channel,
                            }));
                        }
                        Err(TrySendError::Closed(message_for_channel)) => {
                            push_existing_value(execution, Value::Reference(address));
                            release_value(heap, message)?;
                            Err(VmError::ChannelSend {
                                error: "channel closed".to_string(),
                                value: heap
                                    .message_to_value(message_for_channel)
                                    .unwrap_or(Value::Null),
                            })
                        }
                    }
                } else {
                    Err(VmError::InvalidReference)
                }
            }
            OpCode::SpawnSupervisor(addr) => {
                if *addr >= execution.bytecode.len() {
                    log::error!(
                        "SpawnSupervisor target {} out of bounds (bytecode length {})",
                        addr,
                        execution.bytecode.len()
                    );
                    return Err(VmError::ExecutionOutOfBounds);
                }
                let (handle, tx) = spawn_child_vm(
                    execution.bytecode.opcodes(),
                    execution.debug_info.clone(),
                    *addr,
                    process.as_ref(),
                    true,
                    Vec::new(),
                )?;
                let address = heap.allocate(HeapObject::Supervisor(handle, tx, 0));
                push_value(execution, heap, Value::Reference(address))
            }
            OpCode::SetStrategy(strategy) => {
                let sup_ref = pop_value(execution, heap)?;
                if let Value::Reference(addr) = sup_ref {
                    if let Some(HeapObject::Supervisor(vm, _, _)) = heap.get_mut(addr) {
                        vm.set_strategy(*strategy);
                    } else {
                        return Err(VmError::InvalidReference);
                    }
                    push_value(execution, heap, Value::Reference(addr))
                } else {
                    Err(VmError::InvalidReference)
                }
            }
            OpCode::RestartChild(child) => {
                let sup_ref = pop_value(execution, heap)?;
                if let Value::Reference(addr) = sup_ref {
                    let start_ip = actor_start_ip(heap, *child)?;
                    let targets = if let Some(HeapObject::Supervisor(vm, _, _)) = heap.get_mut(addr)
                    {
                        vm.restart_targets(ChildSpec {
                            reference: *child,
                            start_ip,
                        })
                    } else {
                        return Err(VmError::InvalidReference);
                    };

                    for target in targets {
                        restart_actor(heap, target)?;
                    }

                    push_value(execution, heap, Value::Reference(addr))
                } else {
                    Err(VmError::InvalidReference)
                }
            }
        };

        result.map(|()| ExecutionState::Continue)
    }
}

#[cfg(test)]
mod heap_opcode_tests {
    use super::*;

    fn add_native(args: Vec<Value>) -> Result<Value, VmError> {
        match args.as_slice() {
            [Value::Integer(a), Value::Integer(b)] => Ok(Value::Integer(a + b)),
            _ => Err(VmError::TypeMismatch("native add")),
        }
    }

    #[tokio::test]
    async fn make_array_get_and_set_values() {
        let mut execution = ExecutionContext::new(vec![OpCode::Return]);
        let mut heap = Heap::new();
        for value in [Value::Integer(10), Value::Integer(20), Value::Integer(30)] {
            OpCode::PushConst(value)
                .execute(&mut execution, &mut heap)
                .unwrap();
        }
        OpCode::MakeArray(3)
            .execute(&mut execution, &mut heap)
            .unwrap();

        let array_addr = match execution.stack.last().cloned() {
            Some(Value::Reference(address)) => address,
            other => panic!("expected array reference, got {other:?}"),
        };

        OpCode::Dup.execute(&mut execution, &mut heap).unwrap();
        OpCode::PushConst(Value::Integer(1))
            .execute(&mut execution, &mut heap)
            .unwrap();
        OpCode::ArrayGet.execute(&mut execution, &mut heap).unwrap();
        assert_eq!(execution.stack.pop(), Some(Value::Integer(20)));

        OpCode::PushConst(Value::Integer(2))
            .execute(&mut execution, &mut heap)
            .unwrap();
        OpCode::PushConst(Value::Integer(99))
            .execute(&mut execution, &mut heap)
            .unwrap();
        OpCode::ArraySet.execute(&mut execution, &mut heap).unwrap();

        match heap.get(array_addr) {
            Some(HeapObject::Array(elements, rc)) => {
                assert_eq!(
                    elements,
                    &[Value::Integer(10), Value::Integer(20), Value::Integer(99)]
                );
                assert_eq!(*rc, 1);
            }
            other => panic!("expected array object, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn string_concat_allocates_combined_string() {
        let mut execution = ExecutionContext::new(vec![OpCode::Return]);
        let mut heap = Heap::new();
        OpCode::MakeString("raft".to_string())
            .execute(&mut execution, &mut heap)
            .unwrap();
        OpCode::MakeString(" vm".to_string())
            .execute(&mut execution, &mut heap)
            .unwrap();
        OpCode::StringConcat
            .execute(&mut execution, &mut heap)
            .unwrap();

        let address = match execution.stack.last().cloned() {
            Some(Value::Reference(address)) => address,
            other => panic!("expected string reference, got {other:?}"),
        };
        match heap.get(address) {
            Some(HeapObject::String(value, rc)) => {
                assert_eq!(value, "raft vm");
                assert_eq!(*rc, 1);
            }
            other => panic!("expected string object, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn module_get_set_and_native_calls_work() {
        let mut execution = ExecutionContext::new(vec![OpCode::Return]);
        let mut heap = Heap::new();
        OpCode::PushConst(Value::Integer(7))
            .execute(&mut execution, &mut heap)
            .unwrap();
        OpCode::MakeModule(vec!["answer".to_string()])
            .execute(&mut execution, &mut heap)
            .unwrap();
        OpCode::Dup.execute(&mut execution, &mut heap).unwrap();
        OpCode::ModuleGet("answer".to_string())
            .execute(&mut execution, &mut heap)
            .unwrap();
        assert_eq!(execution.stack.pop(), Some(Value::Integer(7)));

        OpCode::PushConst(Value::Integer(8))
            .execute(&mut execution, &mut heap)
            .unwrap();
        OpCode::ModuleSet("answer".to_string())
            .execute(&mut execution, &mut heap)
            .unwrap();
        OpCode::Dup.execute(&mut execution, &mut heap).unwrap();
        OpCode::ModuleGet("answer".to_string())
            .execute(&mut execution, &mut heap)
            .unwrap();
        assert_eq!(execution.stack.pop(), Some(Value::Integer(8)));

        OpCode::PushConst(Value::Integer(2))
            .execute(&mut execution, &mut heap)
            .unwrap();
        OpCode::PushConst(Value::Integer(3))
            .execute(&mut execution, &mut heap)
            .unwrap();
        OpCode::MakeNativeFunction(NativeFunction {
            name: "add".to_string(),
            arity: 2,
            function: add_native,
        })
        .execute(&mut execution, &mut heap)
        .unwrap();
        OpCode::CallNative(2)
            .execute(&mut execution, &mut heap)
            .unwrap();
        assert_eq!(execution.stack.pop(), Some(Value::Integer(5)));
    }

    #[tokio::test]
    async fn store_var_preserves_child_reference_counts() {
        let mut execution = ExecutionContext::new(vec![OpCode::Return]);
        let mut heap = Heap::new();

        OpCode::MakeString("child".to_string())
            .execute(&mut execution, &mut heap)
            .unwrap();
        let child_address = match execution.stack.last().cloned() {
            Some(Value::Reference(address)) => address,
            other => panic!("expected child string reference on stack, got {other:?}"),
        };

        OpCode::MakeArray(1).execute(&mut execution, &mut heap).unwrap();
        let parent_address = match execution.stack.last().cloned() {
            Some(Value::Reference(address)) => address,
            other => panic!("expected parent array reference on stack, got {other:?}"),
        };

        OpCode::StoreVar(0).execute(&mut execution, &mut heap).unwrap();

        match heap.get(parent_address) {
            Some(HeapObject::Array(_, rc)) => assert_eq!(*rc, 1, "stored parent should be retained"),
            other => panic!("expected stored parent array in heap, got {other:?}"),
        }
        match heap.get(child_address) {
            Some(HeapObject::String(_, rc)) => assert_eq!(*rc, 1, "child should remain retained by parent"),
            other => panic!("expected child string in heap, got {other:?}"),
        }
    }
}
