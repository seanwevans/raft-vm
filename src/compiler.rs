use crate::vm::opcodes::OpCode;
use crate::vm::value::Value;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum CompilerError {
    #[error("Invalid token: {0}")]
    InvalidToken(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Parse error at {line}:{column}: {message}")]
    ParseErrorAt {
        message: String,
        line: usize,
        column: usize,
    },
    #[error("Parse error: {0}")]
    ParseError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: SourceLocation,
    pub end: SourceLocation,
}

#[derive(Debug, Clone, Default)]
pub struct DebugInfo {
    instruction_spans: Vec<Option<SourceSpan>>,
}

impl DebugInfo {
    pub fn new(instruction_spans: Vec<Option<SourceSpan>>) -> Self {
        Self { instruction_spans }
    }

    pub fn span_for_instruction(&self, instruction: usize) -> Option<SourceSpan> {
        self.instruction_spans.get(instruction).copied().flatten()
    }

    pub fn location_for_instruction(&self, instruction: usize) -> Option<SourceLocation> {
        self.span_for_instruction(instruction)
            .map(|span| span.start)
    }

    pub fn instruction_spans(&self) -> &[Option<SourceSpan>] {
        &self.instruction_spans
    }
}

#[derive(Debug, Clone)]
pub struct CompiledProgram {
    pub bytecode: Vec<OpCode>,
    pub debug_info: DebugInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Label(String),
    Integer(i32),
    Float(f64),
    Boolean(bool),
    String(String),
    Symbol(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressOperand {
    Address(usize),
    Label(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    PushConst(Value),
    PushString(String),
    StoreVar(usize),
    LoadVar(usize),
    LoadGlobal(String),
    LoadIoPrint,
    GetExport(String),
    MakeArray(usize),
    ArrayGet,
    ArraySet,
    MakeString(String),
    StringConcat,
    MakeModule(Vec<String>),
    ModuleGet(String),
    ModuleSet(String),
    CallNative(usize),
    Pop,
    Dup,
    Swap,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,
    Exp,
    Jump(AddressOperand),
    JumpIfFalse(AddressOperand),
    Call(AddressOperand),
    Return,
    SpawnActor(AddressOperand),
    SendMessage,
    ReceiveMessage,
    SpawnSupervisor(AddressOperand),
    SetStrategy(usize),
    RestartChild(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum AstNode {
    Instruction {
        instruction: Instruction,
        span: SourceSpan,
    },
    Label {
        name: String,
        span: SourceSpan,
    },
}

pub struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    line: usize,
    column: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            chars: source.char_indices().peekable(),
            line: 1,
            column: 1,
        }
    }

    pub fn lex(mut self) -> Result<Vec<Token>, CompilerError> {
        let mut tokens = Vec::new();
        while let Some((_, ch)) = self.peek() {
            match ch {
                c if c.is_whitespace() => {
                    self.bump();
                }
                '#' => self.skip_line_comment(),
                '/' if self.peek_next_char() == Some('/') => self.skip_line_comment(),
                '"' => tokens.push(self.lex_string()?),
                // `-` immediately followed by a digit starts a negative
                // literal. Raft is postfix, so subtraction always follows its
                // two operands and is never written flush against a number:
                // `5 3 -` subtracts, `-3` is the literal.
                '-' if self.peek_next_char().is_some_and(|ch| ch.is_ascii_digit()) => {
                    tokens.push(self.lex_word()?)
                }
                '+' | '-' | '*' | '/' | '%' | '^' => tokens.push(self.lex_symbol()),
                _ => tokens.push(self.lex_word()?),
            };
        }
        Ok(tokens)
    }

    fn peek(&mut self) -> Option<(usize, char)> {
        self.chars.peek().copied()
    }

    fn peek_next_char(&self) -> Option<char> {
        let mut chars = self.chars.clone();
        chars.next();
        chars.next().map(|(_, ch)| ch)
    }

    fn bump(&mut self) -> Option<(usize, char)> {
        let next = self.chars.next()?;
        if next.1 == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(next)
    }

    fn location(&self) -> SourceLocation {
        SourceLocation {
            line: self.line,
            column: self.column,
        }
    }

    fn skip_line_comment(&mut self) {
        while let Some((_, ch)) = self.peek() {
            self.bump();
            if ch == '\n' {
                break;
            }
        }
    }

    fn lex_symbol(&mut self) -> Token {
        let start = self.location();
        let (_, ch) = self.bump().expect("symbol to exist");
        Token {
            kind: TokenKind::Symbol(ch.to_string()),
            span: SourceSpan {
                start,
                end: self.location(),
            },
        }
    }

    fn lex_string(&mut self) -> Result<Token, CompilerError> {
        let start = self.location();
        self.bump();
        let mut value = String::new();
        while let Some((_, ch)) = self.bump() {
            match ch {
                '"' => {
                    return Ok(Token {
                        kind: TokenKind::String(value),
                        span: SourceSpan {
                            start,
                            end: self.location(),
                        },
                    })
                }
                '\\' => {
                    let Some((_, escaped)) = self.bump() else {
                        return Err(self.error_at(start, "unterminated string escape"));
                    };
                    match escaped {
                        'n' => value.push('\n'),
                        'r' => value.push('\r'),
                        't' => value.push('\t'),
                        '"' => value.push('"'),
                        '\\' => value.push('\\'),
                        other => value.push(other),
                    }
                }
                other => value.push(other),
            }
        }
        Err(self.error_at(start, "unterminated string literal"))
    }

    fn lex_word(&mut self) -> Result<Token, CompilerError> {
        let start = self.location();
        let mut text = String::new();
        while let Some((_, ch)) = self.peek() {
            if ch.is_whitespace()
                || ch == '"'
                || ch == '#'
                || (ch == '/' && self.peek_next_char() == Some('/'))
            {
                break;
            }
            text.push(ch);
            self.bump();
        }

        if text.is_empty() {
            return Err(self.error_at(start, "unexpected character"));
        }

        let kind = if text.starts_with('.') {
            TokenKind::Label(text.trim_start_matches('.').to_string())
        } else if text == "true" || text == "false" {
            TokenKind::Boolean(text == "true")
        } else if let Ok(value) = text.parse::<i32>() {
            TokenKind::Integer(value)
        } else if text.contains('.') {
            match text.parse::<f64>() {
                Ok(value) => TokenKind::Float(value),
                Err(_) if text.chars().any(|ch| ch.is_ascii_digit()) => {
                    return Err(CompilerError::ParseError(format!("Invalid float: {text}")))
                }
                Err(_) => TokenKind::Identifier(text),
            }
        } else {
            TokenKind::Identifier(text)
        };

        Ok(Token {
            kind,
            span: SourceSpan {
                start,
                end: self.location(),
            },
        })
    }

    fn error_at(&self, location: SourceLocation, message: impl Into<String>) -> CompilerError {
        CompilerError::ParseErrorAt {
            message: message.into(),
            line: location.line,
            column: location.column,
        }
    }
}

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, current: 0 }
    }

    pub fn parse(mut self) -> Result<Vec<AstNode>, CompilerError> {
        let mut nodes = Vec::new();
        while !self.is_at_end() {
            nodes.push(self.parse_node()?);
        }
        Ok(nodes)
    }

    fn parse_node(&mut self) -> Result<AstNode, CompilerError> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Integer(value) => {
                Ok(self.instruction(Instruction::PushConst(Value::Integer(value)), token.span))
            }
            TokenKind::Float(value) => {
                Ok(self.instruction(Instruction::PushConst(Value::Float(value)), token.span))
            }
            TokenKind::Boolean(value) => {
                Ok(self.instruction(Instruction::PushConst(Value::Boolean(value)), token.span))
            }
            TokenKind::String(value) => {
                Ok(self.instruction(Instruction::PushString(value), token.span))
            }
            TokenKind::Symbol(symbol) => self.parse_symbol(&symbol, token.span),
            TokenKind::Label(name) => Ok(AstNode::Label {
                name,
                span: token.span,
            }),
            TokenKind::Identifier(identifier) => self.parse_identifier(&identifier, token.span),
        }
    }

    fn parse_symbol(&self, symbol: &str, span: SourceSpan) -> Result<AstNode, CompilerError> {
        let instruction = match symbol {
            "+" => Instruction::Add,
            "-" => Instruction::Sub,
            "*" => Instruction::Mul,
            "/" => Instruction::Div,
            "%" => Instruction::Mod,
            "^" => Instruction::Exp,
            _ => return Err(CompilerError::InvalidToken(symbol.to_string())),
        };
        Ok(self.instruction(instruction, span))
    }

    fn parse_identifier(
        &mut self,
        identifier: &str,
        span: SourceSpan,
    ) -> Result<AstNode, CompilerError> {
        let instruction = match identifier {
            "io.print" => {
                return Ok(self.instruction(Instruction::LoadIoPrint, span));
            }
            "StoreVar" => Instruction::StoreVar(self.expect_usize("StoreVar")?),
            "LoadVar" => Instruction::LoadVar(self.expect_usize("LoadVar")?),
            "LoadGlobal" => Instruction::LoadGlobal(self.expect_name("LoadGlobal")?),
            "GetExport" => Instruction::GetExport(self.expect_name("GetExport")?),
            "MakeArray" => Instruction::MakeArray(self.expect_usize("MakeArray")?),
            "ArrayGet" => Instruction::ArrayGet,
            "ArraySet" => Instruction::ArraySet,
            "MakeString" => Instruction::MakeString(self.expect_string_or_name("MakeString")?),
            "PushString" => Instruction::PushString(self.expect_string_or_name("PushString")?),
            "StringConcat" => Instruction::StringConcat,
            "MakeModule" => {
                let count = self.expect_usize("MakeModule")?;
                let mut exports = Vec::with_capacity(count);
                for _ in 0..count {
                    exports.push(self.expect_name("MakeModule")?);
                }
                Instruction::MakeModule(exports)
            }
            "ModuleGet" => Instruction::ModuleGet(self.expect_name("ModuleGet")?),
            "ModuleSet" => Instruction::ModuleSet(self.expect_name("ModuleSet")?),
            "CallNative" => Instruction::CallNative(self.expect_usize("CallNative")?),
            "Pop" => Instruction::Pop,
            "Dup" => Instruction::Dup,
            "Swap" => Instruction::Swap,
            "Add" => Instruction::Add,
            "Sub" => Instruction::Sub,
            "Mul" => Instruction::Mul,
            "Div" => Instruction::Div,
            "Mod" => Instruction::Mod,
            "Neg" => Instruction::Neg,
            "Exp" => Instruction::Exp,
            "Jump" => Instruction::Jump(self.expect_address("Jump")?),
            "JumpIfFalse" => Instruction::JumpIfFalse(self.expect_address("JumpIfFalse")?),
            "Call" => Instruction::Call(self.expect_address("Call")?),
            "Return" => Instruction::Return,
            "SpawnActor" => Instruction::SpawnActor(self.expect_address("SpawnActor")?),
            "SendMessage" => Instruction::SendMessage,
            "ReceiveMessage" => Instruction::ReceiveMessage,
            "SpawnSupervisor" => {
                Instruction::SpawnSupervisor(self.expect_address("SpawnSupervisor")?)
            }
            "SetStrategy" => Instruction::SetStrategy(self.expect_usize("SetStrategy")?),
            "RestartChild" => Instruction::RestartChild(self.expect_usize("RestartChild")?),
            _ => return Err(CompilerError::InvalidToken(identifier.to_string())),
        };
        Ok(self.instruction(instruction, span))
    }

    fn instruction(&self, instruction: Instruction, span: SourceSpan) -> AstNode {
        AstNode::Instruction { instruction, span }
    }

    fn expect_usize(&mut self, operation: &'static str) -> Result<usize, CompilerError> {
        let token = self.expect_operand(operation)?;
        match &token.kind {
            TokenKind::Integer(value) if *value >= 0 => Ok(*value as usize),
            TokenKind::Identifier(text) => text
                .parse::<usize>()
                .map_err(|_| CompilerError::InvalidAddress(text.clone())),
            _ => Err(CompilerError::InvalidAddress(token_text(token))),
        }
    }

    fn expect_address(&mut self, operation: &'static str) -> Result<AddressOperand, CompilerError> {
        let token = self.expect_operand(operation)?;
        match &token.kind {
            TokenKind::Integer(value) if *value >= 0 => {
                Ok(AddressOperand::Address(*value as usize))
            }
            TokenKind::Identifier(text) => text
                .parse::<usize>()
                .map(AddressOperand::Address)
                .map_err(|_| CompilerError::InvalidAddress(text.clone())),
            TokenKind::Label(name) => Ok(AddressOperand::Label(name.clone())),
            _ => Err(CompilerError::InvalidAddress(token_text(token))),
        }
    }

    fn expect_name(&mut self, operation: &'static str) -> Result<String, CompilerError> {
        let token = self.expect_operand(operation)?;
        match &token.kind {
            TokenKind::Identifier(text) | TokenKind::Label(text) => Ok(text.clone()),
            TokenKind::String(text) => Ok(text.clone()),
            _ => Err(CompilerError::ParseError(format!(
                "expected name after {operation}"
            ))),
        }
    }

    fn expect_string_or_name(&mut self, operation: &'static str) -> Result<String, CompilerError> {
        let token = self.expect_operand(operation)?;
        match &token.kind {
            TokenKind::String(text) | TokenKind::Identifier(text) | TokenKind::Label(text) => {
                Ok(text.clone())
            }
            _ => Err(CompilerError::ParseError(format!(
                "expected string token after {operation}"
            ))),
        }
    }

    fn expect_operand(&mut self, operation: &'static str) -> Result<&Token, CompilerError> {
        if self.is_at_end() {
            return Err(CompilerError::InvalidAddress(format!(
                "expected operand after {operation}"
            )));
        }
        Ok(self.advance())
    }

    fn is_at_end(&self) -> bool {
        self.current >= self.tokens.len()
    }

    fn advance(&mut self) -> &Token {
        let token = &self.tokens[self.current];
        self.current += 1;
        token
    }
}

pub struct Compiler;

impl Compiler {
    pub fn compile(source: &str) -> Result<Vec<OpCode>, CompilerError> {
        Ok(Self::compile_with_debug(source)?.bytecode)
    }

    pub fn compile_with_debug(source: &str) -> Result<CompiledProgram, CompilerError> {
        let tokens = Lexer::new(source).lex()?;
        let ast = Parser::new(tokens).parse()?;
        emit(ast)
    }
}

fn token_text(token: &Token) -> String {
    match &token.kind {
        TokenKind::Identifier(text)
        | TokenKind::Label(text)
        | TokenKind::String(text)
        | TokenKind::Symbol(text) => text.clone(),
        TokenKind::Integer(value) => value.to_string(),
        TokenKind::Float(value) => value.to_string(),
        TokenKind::Boolean(value) => value.to_string(),
    }
}

fn emit(ast: Vec<AstNode>) -> Result<CompiledProgram, CompilerError> {
    let mut labels = HashMap::new();
    let mut offset = 0usize;

    for node in &ast {
        match node {
            AstNode::Label { name, .. } => {
                labels.insert(name.clone(), offset);
            }
            AstNode::Instruction { instruction, .. } => {
                offset += emitted_len(instruction);
            }
        }
    }

    let mut bytecode = Vec::new();
    let mut spans = Vec::new();
    for node in ast {
        if let AstNode::Instruction { instruction, span } = node {
            emit_instruction(instruction, span, &labels, &mut bytecode, &mut spans)?;
        }
    }

    Ok(CompiledProgram {
        bytecode,
        debug_info: DebugInfo::new(spans),
    })
}

fn emitted_len(instruction: &Instruction) -> usize {
    match instruction {
        Instruction::LoadIoPrint => 2,
        _ => 1,
    }
}

fn resolve_address(
    operand: AddressOperand,
    labels: &HashMap<String, usize>,
) -> Result<usize, CompilerError> {
    match operand {
        AddressOperand::Address(address) => Ok(address),
        AddressOperand::Label(label) => {
            labels
                .get(&label)
                .copied()
                .ok_or(CompilerError::InvalidAddress(format!(
                    "unknown label .{label}"
                )))
        }
    }
}

fn push(
    bytecode: &mut Vec<OpCode>,
    spans: &mut Vec<Option<SourceSpan>>,
    opcode: OpCode,
    span: SourceSpan,
) {
    bytecode.push(opcode);
    spans.push(Some(span));
}

fn emit_instruction(
    instruction: Instruction,
    span: SourceSpan,
    labels: &HashMap<String, usize>,
    bytecode: &mut Vec<OpCode>,
    spans: &mut Vec<Option<SourceSpan>>,
) -> Result<(), CompilerError> {
    let opcode = match instruction {
        Instruction::PushConst(value) => OpCode::PushConst(value),
        Instruction::PushString(value) => OpCode::PushString(value),
        Instruction::StoreVar(index) => OpCode::StoreVar(index),
        Instruction::LoadVar(index) => OpCode::LoadVar(index),
        Instruction::LoadGlobal(name) => OpCode::LoadGlobal(name),
        Instruction::LoadIoPrint => {
            push(bytecode, spans, OpCode::LoadGlobal("io".to_string()), span);
            push(
                bytecode,
                spans,
                OpCode::GetExport("print".to_string()),
                span,
            );
            return Ok(());
        }
        Instruction::GetExport(name) => OpCode::GetExport(name),
        Instruction::MakeArray(length) => OpCode::MakeArray(length),
        Instruction::ArrayGet => OpCode::ArrayGet,
        Instruction::ArraySet => OpCode::ArraySet,
        Instruction::MakeString(value) => OpCode::MakeString(value),
        Instruction::StringConcat => OpCode::StringConcat,
        Instruction::MakeModule(exports) => OpCode::MakeModule(exports),
        Instruction::ModuleGet(name) => OpCode::ModuleGet(name),
        Instruction::ModuleSet(name) => OpCode::ModuleSet(name),
        Instruction::CallNative(arity) => OpCode::CallNative(arity),
        Instruction::Pop => OpCode::Pop,
        Instruction::Dup => OpCode::Dup,
        Instruction::Swap => OpCode::Swap,
        Instruction::Add => OpCode::Add,
        Instruction::Sub => OpCode::Sub,
        Instruction::Mul => OpCode::Mul,
        Instruction::Div => OpCode::Div,
        Instruction::Mod => OpCode::Mod,
        Instruction::Neg => OpCode::Neg,
        Instruction::Exp => OpCode::Exp,
        Instruction::Jump(address) => OpCode::Jump(resolve_address(address, labels)?),
        Instruction::JumpIfFalse(address) => OpCode::JumpIfFalse(resolve_address(address, labels)?),
        Instruction::Call(address) => OpCode::Call(resolve_address(address, labels)?),
        Instruction::Return => OpCode::Return,
        Instruction::SpawnActor(address) => OpCode::SpawnActor(resolve_address(address, labels)?),
        Instruction::SendMessage => OpCode::SendMessage,
        Instruction::ReceiveMessage => OpCode::ReceiveMessage,
        Instruction::SpawnSupervisor(address) => {
            OpCode::SpawnSupervisor(resolve_address(address, labels)?)
        }
        Instruction::SetStrategy(strategy) => OpCode::SetStrategy(strategy),
        Instruction::RestartChild(child) => OpCode::RestartChild(child),
    };
    push(bytecode, spans, opcode, span);
    Ok(())
}
