// src/vm/opcodes.rs

use crate::vm::error::VmError;
use crate::vm::execution::{ExecutionContext, ProcessContext};
use crate::vm::heap::{Heap, HeapObject, NativeFunction};
use crate::vm::supervision::ChildSpec;
use crate::vm::value::Value;
use crate::vm::vm::VM;
use tokio::sync::mpsc::{self, Receiver};

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

fn decrement_reference(heap: &mut Heap, address: usize) -> Result<(), VmError> {
    let child_references = if let Some(object) = heap.get_mut(address) {
        let child_references = match object {
            HeapObject::Array(elements, rc) if *rc == 1 => elements.clone(),
            HeapObject::Module {
                exports, ref_count, ..
            } if *ref_count == 1 => exports.values().copied().collect(),
            _ => Vec::new(),
        };
        object.decrement_ref();
        child_references
    } else {
        return Err(VmError::InvalidReference);
    };

    for value in child_references {
        if let Value::Reference(child_address) = value {
            decrement_reference(heap, child_address)?;
        }
    }

    Ok(())
}

fn actor_start_ip(heap: &Heap, address: usize) -> Result<usize, VmError> {
    match heap.get(address) {
        Some(HeapObject::Actor(vm, _, _)) => Ok(vm.restart_ip()),
        _ => Err(VmError::InvalidReference),
    }
}

fn restart_actor(heap: &mut Heap, child: ChildSpec) -> Result<(), VmError> {
    match heap.get_mut(child.reference) {
        Some(HeapObject::Actor(vm, sender, _)) => {
            vm.reset_for_restart(child.start_ip);
            let (replacement_tx, replacement_rx) = mpsc::channel(100);
            vm.mailbox = replacement_rx;
            *sender = replacement_tx.clone();
            // Keep the VM's self sender aligned with the mailbox sender stored in the heap.
            vm.replace_sender(replacement_tx);
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

fn pop_value(execution: &mut ExecutionContext, heap: &mut Heap) -> Result<Value, VmError> {
    if let Some(value) = execution.stack.pop() {
        if let Value::Reference(address) = value {
            decrement_reference(heap, address)?;
        }
        Ok(value)
    } else {
        Err(VmError::StackUnderflow)
    }
}

fn expect_usize(value: Value, operation: &'static str) -> Result<usize, VmError> {
    match value {
        Value::Integer(index) if index >= 0 => Ok(index as usize),
        _ => Err(VmError::TypeMismatch(operation)),
    }
}

fn retain_value(heap: &mut Heap, value: Value) -> Result<(), VmError> {
    if let Value::Reference(address) = value {
        increment_reference(heap, address)?;
    }
    Ok(())
}

fn release_value(heap: &mut Heap, value: Value) -> Result<(), VmError> {
    if let Value::Reference(address) = value {
        decrement_reference(heap, address)?;
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
        retain_value(heap, *value)?;
    }

    let address = heap.allocate(HeapObject::Array(elements, 0));
    push_value(execution, heap, Value::Reference(address))
}

fn array_get(execution: &mut ExecutionContext, heap: &mut Heap) -> Result<(), VmError> {
    let index = expect_usize(pop_value(execution, heap)?, "ArrayGet")?;
    let array_ref = pop_value(execution, heap)?;
    let element = match array_ref {
        Value::Reference(address) => match heap.get(address) {
            Some(HeapObject::Array(elements, _)) => {
                elements
                    .get(index)
                    .copied()
                    .ok_or(VmError::IndexOutOfBounds {
                        index,
                        length: elements.len(),
                    })?
            }
            _ => return Err(VmError::InvalidReference),
        },
        _ => return Err(VmError::TypeMismatch("ArrayGet")),
    };

    push_value(execution, heap, element)
}

fn array_set(execution: &mut ExecutionContext, heap: &mut Heap) -> Result<(), VmError> {
    let new_value = pop_value(execution, heap)?;
    let index = expect_usize(pop_value(execution, heap)?, "ArraySet")?;
    let array_ref = pop_value(execution, heap)?;
    let address = match array_ref {
        Value::Reference(address) => address,
        _ => return Err(VmError::TypeMismatch("ArraySet")),
    };

    retain_value(heap, new_value)?;
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

    push_value(execution, heap, Value::Reference(address))
}

fn string_concat(execution: &mut ExecutionContext, heap: &mut Heap) -> Result<(), VmError> {
    let rhs = pop_value(execution, heap)?;
    let lhs = pop_value(execution, heap)?;

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
        retain_value(heap, value)?;
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
    let module_ref = pop_value(execution, heap)?;
    let value = match module_ref {
        Value::Reference(address) => match heap.get(address) {
            Some(HeapObject::Module { exports, .. }) => exports
                .get(name)
                .copied()
                .ok_or_else(|| VmError::Message(format!("Module export not found: {name}")))?,
            _ => return Err(VmError::InvalidReference),
        },
        _ => return Err(VmError::TypeMismatch("ModuleGet")),
    };
    push_value(execution, heap, value)
}

fn module_set(
    execution: &mut ExecutionContext,
    heap: &mut Heap,
    name: &str,
) -> Result<(), VmError> {
    let new_value = pop_value(execution, heap)?;
    let module_ref = pop_value(execution, heap)?;
    let address = match module_ref {
        Value::Reference(address) => address,
        _ => return Err(VmError::TypeMismatch("ModuleSet")),
    };

    retain_value(heap, new_value)?;
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

    // Control Flow
    Jump(usize),
    JumpIfFalse(usize),
    Call(usize),
    CallNative(usize),
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
    pub async fn execute(
        &self,
        execution: &mut ExecutionContext,
        heap: &mut Heap,
        mailbox: &mut Receiver<Value>,
    ) -> Result<(), VmError> {
        self.execute_with_process(execution, heap, mailbox, None)
            .await
    }

    pub async fn execute_with_process(
        &self,
        execution: &mut ExecutionContext,
        heap: &mut Heap,
        mailbox: &mut Receiver<Value>,
        process: Option<ProcessContext>,
    ) -> Result<(), VmError> {
        match self {
            OpCode::Add => binary_op(execution, heap, |a, b| a.checked_add(b)),
            OpCode::Sub => binary_op(execution, heap, |a, b| a.checked_sub(b)),
            OpCode::Mul => binary_op(execution, heap, |a, b| a.checked_mul(b)),
            OpCode::Div => binary_op(execution, heap, |a, b| a.checked_div(b)),
            OpCode::Neg => unary_op(execution, heap, |a| match a {
                Value::Integer(i) => Ok(Value::Integer(-i)),
                Value::Float(f) => Ok(Value::Float(-f)),
                _ => Err(VmError::TypeMismatch("Neg")),
            }),
            OpCode::PushConst(v) => push_value(execution, heap, *v),
            OpCode::MakeArray(length) => make_array(execution, heap, *length),
            OpCode::Pop => {
                pop_value(execution, heap)?;
                Ok(())
            }
            OpCode::Dup => {
                if let Some(&value) = execution.stack.last() {
                    push_value(execution, heap, value)
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
                let value = pop_value(execution, heap)?;

                if let Some(Value::Reference(address)) = execution.locals.insert(*index, value) {
                    decrement_reference(heap, address)?;
                }

                if let Value::Reference(address) = value {
                    increment_reference(heap, address)?;
                }

                Ok(())
            }
            OpCode::LoadVar(index) => {
                if let Some(value) = execution.locals.get(index) {
                    push_value(execution, heap, *value)
                } else {
                    Err(VmError::VariableNotFound(*index))
                }
            }
            OpCode::LoadGlobal(name) => {
                let value = execution
                    .globals
                    .get(name)
                    .copied()
                    .ok_or_else(|| VmError::GlobalNotFound(name.clone()))?;
                push_value(execution, heap, value)
            }
            OpCode::GetExport(export) => {
                let module_ref = pop_value(execution, heap)?;
                let Value::Reference(address) = module_ref else {
                    return Err(VmError::InvalidReference);
                };

                let (module_name, value) = match heap.get(address) {
                    Some(HeapObject::Module { name, exports, .. }) => {
                        let value = exports.get(export).copied().ok_or_else(|| {
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
                push_value(execution, heap, value)
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
                        Ok(Value::Integer(x.pow(y as u32)))
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
            OpCode::ReceiveMessage => {
                if let Some(message) = mailbox.recv().await {
                    log::info!("Received message: {:?}", message);
                    if let Value::Reference(address) = message {
                        decrement_reference(heap, address)?;
                    }
                    push_value(execution, heap, message)
                } else {
                    log::warn!("Mailbox is empty or closed");
                    Err(VmError::MailboxEmpty)
                }
            }

            OpCode::SpawnActor(addr) => {
                let bytecode = execution.bytecode.clone();
                let (mut vm, tx) = VM::new_with_debug(bytecode, execution.debug_info.clone(), None);
                if *addr > execution.bytecode.len() {
                    log::error!(
                        "SpawnActor target {} out of bounds (bytecode length {})",
                        addr,
                        execution.bytecode.len()
                    );
                    return Err(VmError::ExecutionOutOfBounds);
                }
                vm.set_ip(*addr);
                vm.set_restart_ip(*addr);
                if let Some(process) = &process {
                    vm.set_parent(process.process_id);
                    vm.link(process.self_sender.clone());
                    if process.trap_exits {
                        vm.set_trap_exits(true);
                    }
                }
                let address = heap.allocate(HeapObject::Actor(vm, tx, 0));
                push_value(execution, heap, Value::Reference(address))
            }
            OpCode::SendMessage => {
                // SendMessage has stack-stable behavior for actor references:
                // the actor reference is present on the stack after the opcode
                // finishes, regardless of success or failure.
                let actor_ref = pop_value(execution, heap)?;
                let message = pop_value(execution, heap)?;
                if let Value::Reference(address) = actor_ref {
                    let sender = match heap.get(address) {
                        Some(HeapObject::Actor(_actor_vm, sender, _)) => sender.clone(),
                        _ => return Err(VmError::InvalidReference),
                    };
                    if let Value::Reference(message_address) = message {
                        increment_reference(heap, message_address)?;
                    }
                    match sender.send(message).await {
                        Ok(()) => push_value(execution, heap, Value::Reference(address)),
                        Err(err) => {
                            let error = err.to_string();
                            let failed_message = err.0;
                            push_value(execution, heap, Value::Reference(address))?;
                            // Keep the recovered message alive so that callers can
                            // safely inspect or resend it from the returned error.
                            // The send attempt already incremented the reference
                            // count to transfer ownership to the channel, so we
                            // intentionally skip the corresponding decrement here.
                            Err(VmError::ChannelSend {
                                error,
                                value: failed_message,
                            })
                        }
                    }
                } else {
                    Err(VmError::InvalidReference)
                }
            }
            OpCode::SpawnSupervisor(addr) => {
                let bytecode = execution.bytecode.clone();
                let (mut vm, tx) = VM::new_with_debug(bytecode, execution.debug_info.clone(), None);
                if *addr > execution.bytecode.len() {
                    log::error!(
                        "SpawnSupervisor target {} out of bounds (bytecode length {})",
                        addr,
                        execution.bytecode.len()
                    );
                    return Err(VmError::ExecutionOutOfBounds);
                }
                vm.set_ip(*addr);
                vm.set_restart_ip(*addr);
                vm.set_trap_exits(true);
                if let Some(process) = &process {
                    vm.set_parent(process.process_id);
                    vm.link(process.self_sender.clone());
                }
                let address = heap.allocate(HeapObject::Supervisor(vm, tx, 0));
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
        }
    }
}

#[cfg(test)]
mod heap_opcode_tests {
    use super::*;
    use tokio::sync::mpsc::channel;

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
        let (_tx, mut mailbox) = channel(1);

        for value in [Value::Integer(10), Value::Integer(20), Value::Integer(30)] {
            OpCode::PushConst(value)
                .execute(&mut execution, &mut heap, &mut mailbox)
                .await
                .unwrap();
        }
        OpCode::MakeArray(3)
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();

        let array_addr = match execution.stack.last().copied() {
            Some(Value::Reference(address)) => address,
            other => panic!("expected array reference, got {other:?}"),
        };

        OpCode::Dup
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::PushConst(Value::Integer(1))
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::ArrayGet
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        assert_eq!(execution.stack.pop(), Some(Value::Integer(20)));

        OpCode::PushConst(Value::Integer(2))
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::PushConst(Value::Integer(99))
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::ArraySet
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();

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
        let (_tx, mut mailbox) = channel(1);

        OpCode::MakeString("raft".to_string())
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::MakeString(" vm".to_string())
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::StringConcat
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();

        let address = match execution.stack.last().copied() {
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
        let (_tx, mut mailbox) = channel(1);

        OpCode::PushConst(Value::Integer(7))
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::MakeModule(vec!["answer".to_string()])
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::Dup
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::ModuleGet("answer".to_string())
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        assert_eq!(execution.stack.pop(), Some(Value::Integer(7)));

        OpCode::PushConst(Value::Integer(8))
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::ModuleSet("answer".to_string())
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::Dup
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::ModuleGet("answer".to_string())
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        assert_eq!(execution.stack.pop(), Some(Value::Integer(8)));

        OpCode::MakeNativeFunction(NativeFunction {
            name: "add".to_string(),
            arity: 2,
            function: add_native,
        })
        .execute(&mut execution, &mut heap, &mut mailbox)
        .await
        .unwrap();
        OpCode::PushConst(Value::Integer(2))
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::PushConst(Value::Integer(3))
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        OpCode::CallNative(2)
            .execute(&mut execution, &mut heap, &mut mailbox)
            .await
            .unwrap();
        assert_eq!(execution.stack.pop(), Some(Value::Integer(5)));
    }
}
