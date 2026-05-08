// src/vm/opcodes.rs

use crate::vm::error::VmError;
use crate::vm::execution::ExecutionContext;
use crate::vm::heap::{Heap, HeapObject};
use crate::vm::value::Value;
use crate::vm::vm::VM;
use tokio::sync::mpsc::Receiver;

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
    if let Some(object) = heap.get_mut(address) {
        object.decrement_ref();
        Ok(())
    } else {
        Err(VmError::InvalidReference)
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

#[derive(Debug, Clone)]
pub enum OpCode {
    // Variables
    StoreVar(usize),
    LoadVar(usize),
    LoadGlobal(String),
    GetExport(String),

    // Stack
    PushConst(Value),
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
        match self {
            OpCode::Add => binary_op(execution, heap, |a, b| a.add(b)),
            OpCode::Sub => binary_op(execution, heap, |a, b| a.sub(b)),
            OpCode::Mul => binary_op(execution, heap, |a, b| a.mul(b)),
            OpCode::Div => binary_op(execution, heap, |a, b| a.div(b)),
            OpCode::Neg => unary_op(execution, heap, |a| match a {
                Value::Integer(i) => Ok(Value::Integer(-i)),
                Value::Float(f) => Ok(Value::Float(-f)),
                _ => Err(VmError::TypeMismatch("Neg")),
            }),
            OpCode::PushConst(v) => push_value(execution, heap, *v),
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

            OpCode::CallNative(arity) => {
                let callable = pop_value(execution, heap)?;
                let Value::Reference(address) = callable else {
                    return Err(VmError::InvalidReference);
                };

                let native = match heap.get(address) {
                    Some(HeapObject::NativeFunction(native, _)) => native,
                    _ => return Err(VmError::InvalidReference),
                };

                if native.arity != *arity {
                    return Err(VmError::Message(format!(
                        "Native function `{}` expected {} argument(s), got {}",
                        native.name, native.arity, arity
                    )));
                }

                if execution.stack.len() < *arity {
                    return Err(VmError::StackUnderflowFor("CallNative"));
                }

                let function = native.function;
                let mut args = Vec::with_capacity(*arity);
                for _ in 0..*arity {
                    args.push(pop_value(execution, heap)?);
                }
                args.reverse();

                let result = function(args)?;
                push_value(execution, heap, result)
            }
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
                let (mut vm, tx) = VM::new(bytecode, None);
                if *addr >= execution.bytecode.len() {
                    log::error!(
                        "SpawnActor target {} out of bounds (bytecode length {})",
                        addr,
                        execution.bytecode.len()
                    );
                    return Err(VmError::ExecutionOutOfBounds);
                }
                vm.set_ip(*addr);
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
                let (mut vm, tx) = VM::new(bytecode, None);
                if *addr >= execution.bytecode.len() {
                    log::error!(
                        "SpawnSupervisor target {} out of bounds (bytecode length {})",
                        addr,
                        execution.bytecode.len()
                    );
                    return Err(VmError::ExecutionOutOfBounds);
                }
                vm.set_ip(*addr);
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
                    if let Some(HeapObject::Supervisor(vm, _, _)) = heap.get_mut(addr) {
                        vm.restart_child(*child);
                    } else {
                        return Err(VmError::InvalidReference);
                    }
                    push_value(execution, heap, Value::Reference(addr))
                } else {
                    Err(VmError::InvalidReference)
                }
            }
        }
    }
}
