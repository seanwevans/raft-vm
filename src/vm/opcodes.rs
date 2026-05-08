// src/vm/opcodes.rs

use crate::vm::error::VmError;
use crate::vm::execution::{BlockingOperation, ExecutionContext, ExecutionState, ProcessContext};
use crate::vm::heap::{Heap, HeapObject, NativeFunction};
use crate::vm::supervision::ChildSpec;
use crate::vm::value::Value;
use crate::vm::vm::VM;
use tokio::sync::mpsc::{self, error::TrySendError};

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
            *vm.mailbox_mut() = replacement_rx;
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
pub struct Bytecode {
    instructions: Vec<u8>,
    constants: Vec<BytecodeConstant>,
    offsets: Vec<usize>,
}

#[derive(Debug, Clone)]
pub enum BytecodeConstant {
    Value(Value),
    String(String),
    Strings(Vec<String>),
    NativeFunction(NativeFunction),
}

impl Bytecode {
    pub fn new(opcodes: Vec<OpCode>) -> Self {
        let mut bytecode = Self {
            instructions: Vec::new(),
            constants: Vec::new(),
            offsets: Vec::with_capacity(opcodes.len()),
        };
        for opcode in opcodes {
            bytecode.encode(opcode);
        }
        bytecode
    }

    pub fn instructions(&self) -> &[u8] {
        &self.instructions
    }

    pub fn constants(&self) -> &[BytecodeConstant] {
        &self.constants
    }

    pub fn offsets(&self) -> &[usize] {
        &self.offsets
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    pub fn decode(&self, ip: usize) -> Result<OpCode, VmError> {
        let offset = *self.offsets.get(ip).ok_or(VmError::ExecutionOutOfBounds)?;
        let opcode = *self
            .instructions
            .get(offset)
            .ok_or(VmError::ExecutionOutOfBounds)?;
        let operand = || self.read_u32(offset + 1).map(|value| value as usize);
        match opcode {
            OP_STORE_VAR => Ok(OpCode::StoreVar(operand()?)),
            OP_LOAD_VAR => Ok(OpCode::LoadVar(operand()?)),
            OP_LOAD_GLOBAL => Ok(OpCode::LoadGlobal(
                self.string_constant(operand()?)?.to_string(),
            )),
            OP_GET_EXPORT => Ok(OpCode::GetExport(
                self.string_constant(operand()?)?.to_string(),
            )),
            OP_PUSH_CONST => Ok(OpCode::PushConst(self.value_constant(operand()?)?)),
            OP_MAKE_ARRAY => Ok(OpCode::MakeArray(operand()?)),
            OP_POP => Ok(OpCode::Pop),
            OP_DUP => Ok(OpCode::Dup),
            OP_SWAP => Ok(OpCode::Swap),
            OP_ADD => Ok(OpCode::Add),
            OP_SUB => Ok(OpCode::Sub),
            OP_MUL => Ok(OpCode::Mul),
            OP_DIV => Ok(OpCode::Div),
            OP_MOD => Ok(OpCode::Mod),
            OP_NEG => Ok(OpCode::Neg),
            OP_EXP => Ok(OpCode::Exp),
            OP_ARRAY_GET => Ok(OpCode::ArrayGet),
            OP_ARRAY_SET => Ok(OpCode::ArraySet),
            OP_MAKE_STRING => Ok(OpCode::MakeString(
                self.string_constant(operand()?)?.to_string(),
            )),
            OP_STRING_CONCAT => Ok(OpCode::StringConcat),
            OP_MAKE_MODULE => Ok(OpCode::MakeModule(
                self.strings_constant(operand()?)?.to_vec(),
            )),
            OP_MODULE_GET => Ok(OpCode::ModuleGet(
                self.string_constant(operand()?)?.to_string(),
            )),
            OP_MODULE_SET => Ok(OpCode::ModuleSet(
                self.string_constant(operand()?)?.to_string(),
            )),
            OP_MAKE_NATIVE_FUNCTION => Ok(OpCode::MakeNativeFunction(
                self.native_constant(operand()?)?.clone(),
            )),
            OP_CALL_NATIVE => Ok(OpCode::CallNative(operand()?)),
            OP_JUMP => Ok(OpCode::Jump(operand()?)),
            OP_JUMP_IF_FALSE => Ok(OpCode::JumpIfFalse(operand()?)),
            OP_CALL => Ok(OpCode::Call(operand()?)),
            OP_RETURN => Ok(OpCode::Return),
            OP_SPAWN_ACTOR => Ok(OpCode::SpawnActor(operand()?)),
            OP_SEND_MESSAGE => Ok(OpCode::SendMessage),
            OP_RECEIVE_MESSAGE => Ok(OpCode::ReceiveMessage),
            OP_SPAWN_SUPERVISOR => Ok(OpCode::SpawnSupervisor(operand()?)),
            OP_SET_STRATEGY => Ok(OpCode::SetStrategy(operand()?)),
            OP_RESTART_CHILD => Ok(OpCode::RestartChild(operand()?)),
            _ => Err(VmError::ExecutionOutOfBounds),
        }
    }

    fn encode(&mut self, opcode: OpCode) {
        self.offsets.push(self.instructions.len());
        match opcode {
            OpCode::StoreVar(value) => self.emit_operand(OP_STORE_VAR, value),
            OpCode::LoadVar(value) => self.emit_operand(OP_LOAD_VAR, value),
            OpCode::LoadGlobal(value) => {
                let index = self.push_constant(BytecodeConstant::String(value));
                self.emit_operand(OP_LOAD_GLOBAL, index);
            }
            OpCode::GetExport(value) => {
                let index = self.push_constant(BytecodeConstant::String(value));
                self.emit_operand(OP_GET_EXPORT, index);
            }
            OpCode::PushConst(value) => {
                let index = self.push_constant(BytecodeConstant::Value(value));
                self.emit_operand(OP_PUSH_CONST, index);
            }
            OpCode::MakeArray(value) => self.emit_operand(OP_MAKE_ARRAY, value),
            OpCode::Pop => self.emit(OP_POP),
            OpCode::Dup => self.emit(OP_DUP),
            OpCode::Swap => self.emit(OP_SWAP),
            OpCode::Add => self.emit(OP_ADD),
            OpCode::Sub => self.emit(OP_SUB),
            OpCode::Mul => self.emit(OP_MUL),
            OpCode::Div => self.emit(OP_DIV),
            OpCode::Mod => self.emit(OP_MOD),
            OpCode::Neg => self.emit(OP_NEG),
            OpCode::Exp => self.emit(OP_EXP),
            OpCode::ArrayGet => self.emit(OP_ARRAY_GET),
            OpCode::ArraySet => self.emit(OP_ARRAY_SET),
            OpCode::MakeString(value) => {
                let index = self.push_constant(BytecodeConstant::String(value));
                self.emit_operand(OP_MAKE_STRING, index);
            }
            OpCode::StringConcat => self.emit(OP_STRING_CONCAT),
            OpCode::MakeModule(value) => {
                let index = self.push_constant(BytecodeConstant::Strings(value));
                self.emit_operand(OP_MAKE_MODULE, index);
            }
            OpCode::ModuleGet(value) => {
                let index = self.push_constant(BytecodeConstant::String(value));
                self.emit_operand(OP_MODULE_GET, index);
            }
            OpCode::ModuleSet(value) => {
                let index = self.push_constant(BytecodeConstant::String(value));
                self.emit_operand(OP_MODULE_SET, index);
            }
            OpCode::MakeNativeFunction(value) => {
                let index = self.push_constant(BytecodeConstant::NativeFunction(value));
                self.emit_operand(OP_MAKE_NATIVE_FUNCTION, index);
            }
            OpCode::CallNative(value) => self.emit_operand(OP_CALL_NATIVE, value),
            OpCode::Jump(value) => self.emit_operand(OP_JUMP, value),
            OpCode::JumpIfFalse(value) => self.emit_operand(OP_JUMP_IF_FALSE, value),
            OpCode::Call(value) => self.emit_operand(OP_CALL, value),
            OpCode::Return => self.emit(OP_RETURN),
            OpCode::SpawnActor(value) => self.emit_operand(OP_SPAWN_ACTOR, value),
            OpCode::SendMessage => self.emit(OP_SEND_MESSAGE),
            OpCode::ReceiveMessage => self.emit(OP_RECEIVE_MESSAGE),
            OpCode::SpawnSupervisor(value) => self.emit_operand(OP_SPAWN_SUPERVISOR, value),
            OpCode::SetStrategy(value) => self.emit_operand(OP_SET_STRATEGY, value),
            OpCode::RestartChild(value) => self.emit_operand(OP_RESTART_CHILD, value),
        }
    }

    fn emit(&mut self, opcode: u8) {
        self.instructions.push(opcode);
    }

    fn emit_operand(&mut self, opcode: u8, operand: usize) {
        self.instructions.push(opcode);
        self.instructions
            .extend_from_slice(&(operand as u32).to_le_bytes());
    }

    fn push_constant(&mut self, constant: BytecodeConstant) -> usize {
        let index = self.constants.len();
        self.constants.push(constant);
        index
    }

    fn read_u32(&self, offset: usize) -> Result<u32, VmError> {
        let bytes: [u8; 4] = self
            .instructions
            .get(offset..offset + 4)
            .ok_or(VmError::ExecutionOutOfBounds)?
            .try_into()
            .map_err(|_| VmError::ExecutionOutOfBounds)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn value_constant(&self, index: usize) -> Result<Value, VmError> {
        match self.constants.get(index) {
            Some(BytecodeConstant::Value(value)) => Ok(*value),
            _ => Err(VmError::ExecutionOutOfBounds),
        }
    }

    fn string_constant(&self, index: usize) -> Result<&str, VmError> {
        match self.constants.get(index) {
            Some(BytecodeConstant::String(value)) => Ok(value),
            _ => Err(VmError::ExecutionOutOfBounds),
        }
    }

    fn strings_constant(&self, index: usize) -> Result<&[String], VmError> {
        match self.constants.get(index) {
            Some(BytecodeConstant::Strings(value)) => Ok(value),
            _ => Err(VmError::ExecutionOutOfBounds),
        }
    }

    fn native_constant(&self, index: usize) -> Result<&NativeFunction, VmError> {
        match self.constants.get(index) {
            Some(BytecodeConstant::NativeFunction(value)) => Ok(value),
            _ => Err(VmError::ExecutionOutOfBounds),
        }
    }
}

impl From<Vec<OpCode>> for Bytecode {
    fn from(value: Vec<OpCode>) -> Self {
        Bytecode::new(value)
    }
}

const OP_STORE_VAR: u8 = 0x01;
const OP_LOAD_VAR: u8 = 0x02;
const OP_LOAD_GLOBAL: u8 = 0x03;
const OP_GET_EXPORT: u8 = 0x04;
const OP_PUSH_CONST: u8 = 0x05;
const OP_MAKE_ARRAY: u8 = 0x06;
const OP_POP: u8 = 0x07;
const OP_DUP: u8 = 0x08;
const OP_SWAP: u8 = 0x09;
const OP_ADD: u8 = 0x10;
const OP_SUB: u8 = 0x11;
const OP_MUL: u8 = 0x12;
const OP_DIV: u8 = 0x13;
const OP_MOD: u8 = 0x14;
const OP_NEG: u8 = 0x15;
const OP_EXP: u8 = 0x16;
const OP_ARRAY_GET: u8 = 0x20;
const OP_ARRAY_SET: u8 = 0x21;
const OP_MAKE_STRING: u8 = 0x22;
const OP_STRING_CONCAT: u8 = 0x23;
const OP_MAKE_MODULE: u8 = 0x24;
const OP_MODULE_GET: u8 = 0x25;
const OP_MODULE_SET: u8 = 0x26;
const OP_MAKE_NATIVE_FUNCTION: u8 = 0x27;
const OP_CALL_NATIVE: u8 = 0x28;
const OP_JUMP: u8 = 0x30;
const OP_JUMP_IF_FALSE: u8 = 0x31;
const OP_CALL: u8 = 0x32;
const OP_RETURN: u8 = 0x33;
const OP_SPAWN_ACTOR: u8 = 0x40;
const OP_SEND_MESSAGE: u8 = 0x41;
const OP_RECEIVE_MESSAGE: u8 = 0x42;
const OP_SPAWN_SUPERVISOR: u8 = 0x50;
const OP_SET_STRATEGY: u8 = 0x51;
const OP_RESTART_CHILD: u8 = 0x52;

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
                    if let Value::Reference(address) = message {
                        decrement_reference(heap, address)?;
                    }
                    push_value(execution, heap, message)
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    return Ok(ExecutionState::Yield(BlockingOperation::ReceiveMessage));
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    log::warn!("Mailbox is closed");
                    Err(VmError::MailboxEmpty)
                }
            },

            OpCode::SpawnActor(addr) => {
                let bytecode = execution.bytecode.clone();
                let (mut vm, tx) = VM::new_with_debug(bytecode, execution.debug_info.clone(), None);
                if *addr >= execution.bytecode.len() {
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
                    match sender.try_send(message) {
                        Ok(()) => push_value(execution, heap, Value::Reference(address)),
                        Err(TrySendError::Full(message)) => {
                            return Ok(ExecutionState::Yield(BlockingOperation::SendMessage {
                                sender,
                                actor_address: address,
                                message,
                            }));
                        }
                        Err(TrySendError::Closed(message)) => {
                            push_value(execution, heap, Value::Reference(address))?;
                            release_value(heap, message)?;
                            Err(VmError::ChannelSend {
                                error: "channel closed".to_string(),
                                value: message,
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
                if *addr >= execution.bytecode.len() {
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

        let array_addr = match execution.stack.last().copied() {
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
}
