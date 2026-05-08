// src/compiler/compiler.rs

use std::collections::HashMap;

use crate::vm::opcodes::OpCode;
use crate::vm::value::Value;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum CompilerError {
    #[error("Invalid token: {0}")]
    InvalidToken(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Parse error: {0}")]
    ParseError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}

impl SourceLocation {
    pub fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: SourceLocation,
    pub end: SourceLocation,
}

impl SourceSpan {
    pub fn new(start: SourceLocation, end: SourceLocation) -> Self {
        Self { start, end }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionDebugInfo {
    pub instruction: usize,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DebugInfo {
    instructions: Vec<InstructionDebugInfo>,
}

impl DebugInfo {
    pub fn new(instructions: Vec<InstructionDebugInfo>) -> Self {
        Self { instructions }
    }

    pub fn location_for_ip(&self, ip: usize) -> Option<SourceLocation> {
        self.instructions
            .iter()
            .find(|entry| entry.instruction == ip)
            .map(|entry| entry.span.start)
    }

    pub fn span_for_ip(&self, ip: usize) -> Option<SourceSpan> {
        self.instructions
            .iter()
            .find(|entry| entry.instruction == ip)
            .map(|entry| entry.span)
    }

    pub fn instructions(&self) -> &[InstructionDebugInfo] {
        &self.instructions
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledProgram {
    pub bytecode: Vec<OpCode>,
    pub debug_info: DebugInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Word(String),
    Integer(i32),
    Float(f64),
    String(String),
    Boolean(bool),
    Label(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub span: SourceSpan,
}

pub struct Lexer {
    chars: Vec<char>,
    index: usize,
    line: usize,
    column: usize,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            index: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn lex(mut self) -> Result<Vec<Token>, CompilerError> {
        let mut tokens = Vec::new();

        while let Some(ch) = self.peek() {
            match ch {
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '\n' => {
                    self.advance();
                }
                '#' => self.skip_comment(),
                '"' => tokens.push(self.lex_string()?),
                _ => tokens.push(self.lex_atom()?),
            }
        }

        Ok(tokens)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += 1;
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn location(&self) -> SourceLocation {
        SourceLocation::new(self.line, self.column)
    }

    fn skip_comment(&mut self) {
        while let Some(ch) = self.peek() {
            self.advance();
            if ch == '\n' {
                break;
            }
        }
    }

    fn lex_string(&mut self) -> Result<Token, CompilerError> {
        let start = self.location();
        let mut lexeme = String::new();
        let mut value = String::new();

        lexeme.push(self.advance().expect("opening quote must exist"));

        while let Some(ch) = self.peek() {
            if ch == '"' {
                lexeme.push(self.advance().expect("closing quote must exist"));
                return Ok(Token {
                    kind: TokenKind::String(value),
                    lexeme,
                    span: SourceSpan::new(start, self.location()),
                });
            }

            if ch == '\\' {
                lexeme.push(self.advance().expect("escape slash must exist"));
                let escaped = self.peek().ok_or_else(|| {
                    CompilerError::ParseError(format!(
                        "Unterminated string literal at line {}, column {}",
                        start.line, start.column
                    ))
                })?;
                lexeme.push(self.advance().expect("escape target must exist"));
                match escaped {
                    'n' => value.push('\n'),
                    'r' => value.push('\r'),
                    't' => value.push('\t'),
                    '"' => value.push('"'),
                    '\\' => value.push('\\'),
                    other => value.push(other),
                }
            } else {
                lexeme.push(self.advance().expect("string char must exist"));
                value.push(ch);
            }
        }

        Err(CompilerError::ParseError(format!(
            "Unterminated string literal at line {}, column {}",
            start.line, start.column
        )))
    }

    fn lex_atom(&mut self) -> Result<Token, CompilerError> {
        let start = self.location();
        let mut lexeme = String::new();

        while let Some(ch) = self.peek() {
            if ch.is_whitespace() || ch == '#' || ch == '"' {
                break;
            }
            lexeme.push(self.advance().expect("atom char must exist"));
        }

        let kind = classify_atom(&lexeme)?;
        Ok(Token {
            kind,
            lexeme,
            span: SourceSpan::new(start, self.location()),
        })
    }
}

fn classify_atom(lexeme: &str) -> Result<TokenKind, CompilerError> {
    if lexeme == "true" {
        return Ok(TokenKind::Boolean(true));
    }
    if lexeme == "false" {
        return Ok(TokenKind::Boolean(false));
    }

    if is_label(lexeme) {
        return Ok(TokenKind::Label(
            lexeme.strip_suffix(':').unwrap_or(lexeme).to_string(),
        ));
    }

    if lexeme.contains('.') {
        return lexeme
            .parse::<f64>()
            .map(TokenKind::Float)
            .map_err(|_| CompilerError::ParseError(format!("Invalid float: {}", lexeme)));
    }

    if let Ok(num) = lexeme.parse::<i32>() {
        return Ok(TokenKind::Integer(num));
    }

    Ok(TokenKind::Word(lexeme.to_string()))
}

fn is_label(lexeme: &str) -> bool {
    let label = lexeme.strip_suffix(':').unwrap_or(lexeme);
    let mut chars = label.chars();
    matches!(chars.next(), Some('.'))
        && matches!(chars.next(), Some(ch) if ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[derive(Debug, Clone, PartialEq)]
pub enum AstNodeKind {
    Label(String),
    Instruction(Instruction),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AstNode {
    pub kind: AstNodeKind,
    pub span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    PushConst(Value),
    PushString(String),
    StoreVar(usize),
    LoadVar(usize),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressOperand {
    Absolute(usize),
    Label(String),
}

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, current: 0 }
    }

    pub fn parse(&mut self) -> Result<Vec<AstNode>, CompilerError> {
        let mut nodes = Vec::new();
        while let Some(token) = self.advance().cloned() {
            let node = match token.kind {
                TokenKind::Boolean(value) => AstNode {
                    kind: AstNodeKind::Instruction(Instruction::PushConst(Value::Boolean(value))),
                    span: token.span,
                },
                TokenKind::Integer(value) => AstNode {
                    kind: AstNodeKind::Instruction(Instruction::PushConst(Value::Integer(value))),
                    span: token.span,
                },
                TokenKind::Float(value) => AstNode {
                    kind: AstNodeKind::Instruction(Instruction::PushConst(Value::Float(value))),
                    span: token.span,
                },
                TokenKind::String(value) => AstNode {
                    kind: AstNodeKind::Instruction(Instruction::PushString(value)),
                    span: token.span,
                },
                TokenKind::Label(label) => AstNode {
                    kind: AstNodeKind::Label(label),
                    span: token.span,
                },
                TokenKind::Word(word) => self.parse_word(word, token.span)?,
            };
            nodes.push(node);
        }

        Ok(nodes)
    }

    fn advance(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.current);
        if token.is_some() {
            self.current += 1;
        }
        token
    }

    fn parse_word(&mut self, word: String, span: SourceSpan) -> Result<AstNode, CompilerError> {
        let instruction = match word.as_str() {
            "StoreVar" => Instruction::StoreVar(self.parse_index_operand("StoreVar")?),
            "LoadVar" => Instruction::LoadVar(self.parse_index_operand("LoadVar")?),
            "Pop" => Instruction::Pop,
            "Dup" => Instruction::Dup,
            "Swap" => Instruction::Swap,
            "+" | "Add" => Instruction::Add,
            "-" | "Sub" => Instruction::Sub,
            "*" | "Mul" => Instruction::Mul,
            "/" | "Div" => Instruction::Div,
            "%" | "Mod" => Instruction::Mod,
            "Neg" => Instruction::Neg,
            "Exp" | "^" => Instruction::Exp,
            "Jump" => Instruction::Jump(self.parse_address_operand("Jump")?),
            "JumpIfFalse" => Instruction::JumpIfFalse(self.parse_address_operand("JumpIfFalse")?),
            "Call" => Instruction::Call(self.parse_address_operand("Call")?),
            "Return" => Instruction::Return,
            "SpawnActor" => Instruction::SpawnActor(self.parse_address_operand("SpawnActor")?),
            "SendMessage" => Instruction::SendMessage,
            "ReceiveMessage" => Instruction::ReceiveMessage,
            "SpawnSupervisor" => {
                Instruction::SpawnSupervisor(self.parse_address_operand("SpawnSupervisor")?)
            }
            "SetStrategy" => Instruction::SetStrategy(self.parse_index_operand("SetStrategy")?),
            "RestartChild" => Instruction::RestartChild(self.parse_index_operand("RestartChild")?),
            _ => return Err(CompilerError::InvalidToken(word)),
        };

        Ok(AstNode {
            kind: AstNodeKind::Instruction(instruction),
            span,
        })
    }

    fn parse_index_operand(&mut self, instruction: &str) -> Result<usize, CompilerError> {
        let token = self.advance().ok_or_else(|| {
            CompilerError::InvalidAddress(format!("expected variable index after {}", instruction))
        })?;

        match &token.kind {
            TokenKind::Integer(value) if *value >= 0 => Ok(*value as usize),
            _ => Err(CompilerError::InvalidAddress(token.lexeme.clone())),
        }
    }

    fn parse_address_operand(
        &mut self,
        instruction: &str,
    ) -> Result<AddressOperand, CompilerError> {
        let token = self.advance().ok_or_else(|| {
            CompilerError::InvalidAddress(format!("expected address after {}", instruction))
        })?;

        match &token.kind {
            TokenKind::Integer(value) if *value >= 0 => {
                Ok(AddressOperand::Absolute(*value as usize))
            }
            TokenKind::Label(label) => Ok(AddressOperand::Label(label.clone())),
            _ => Err(CompilerError::InvalidAddress(token.lexeme.clone())),
        }
    }
}

pub struct Compiler;

impl Compiler {
    pub fn lex(source: &str) -> Result<Vec<Token>, CompilerError> {
        Lexer::new(source).lex()
    }

    pub fn parse(source: &str) -> Result<Vec<AstNode>, CompilerError> {
        let tokens = Self::lex(source)?;
        Parser::new(tokens).parse()
    }

    pub fn compile(source: &str) -> Result<Vec<OpCode>, CompilerError> {
        Ok(Self::compile_with_debug(source)?.bytecode)
    }

    pub fn compile_with_debug(source: &str) -> Result<CompiledProgram, CompilerError> {
        let ast = Self::parse(source)?;
        emit(ast)
    }
}

fn emit(ast: Vec<AstNode>) -> Result<CompiledProgram, CompilerError> {
    let mut labels = HashMap::new();
    let mut instruction_index = 0;

    for node in &ast {
        match &node.kind {
            AstNodeKind::Label(label) => {
                if labels.insert(label.clone(), instruction_index).is_some() {
                    return Err(CompilerError::ParseError(format!(
                        "Duplicate label: {}",
                        label
                    )));
                }
            }
            AstNodeKind::Instruction(_) => instruction_index += 1,
        }
    }

    let mut bytecode = Vec::new();
    let mut debug_entries = Vec::new();

    for node in ast {
        let instruction = match node.kind {
            AstNodeKind::Label(_) => continue,
            AstNodeKind::Instruction(instruction) => instruction,
        };

        let ip = bytecode.len();
        bytecode.push(emit_instruction(instruction, &labels)?);
        debug_entries.push(InstructionDebugInfo {
            instruction: ip,
            span: node.span,
        });
    }

    Ok(CompiledProgram {
        bytecode,
        debug_info: DebugInfo::new(debug_entries),
    })
}

fn emit_instruction(
    instruction: Instruction,
    labels: &HashMap<String, usize>,
) -> Result<OpCode, CompilerError> {
    Ok(match instruction {
        Instruction::PushConst(value) => OpCode::PushConst(value),
        Instruction::PushString(value) => OpCode::PushString(value),
        Instruction::StoreVar(index) => OpCode::StoreVar(index),
        Instruction::LoadVar(index) => OpCode::LoadVar(index),
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
        Instruction::Jump(target) => OpCode::Jump(resolve_address(target, labels)?),
        Instruction::JumpIfFalse(target) => OpCode::JumpIfFalse(resolve_address(target, labels)?),
        Instruction::Call(target) => OpCode::Call(resolve_address(target, labels)?),
        Instruction::Return => OpCode::Return,
        Instruction::SpawnActor(target) => OpCode::SpawnActor(resolve_address(target, labels)?),
        Instruction::SendMessage => OpCode::SendMessage,
        Instruction::ReceiveMessage => OpCode::ReceiveMessage,
        Instruction::SpawnSupervisor(target) => {
            OpCode::SpawnSupervisor(resolve_address(target, labels)?)
        }
        Instruction::SetStrategy(strategy) => OpCode::SetStrategy(strategy),
        Instruction::RestartChild(child) => OpCode::RestartChild(child),
    })
}

fn resolve_address(
    address: AddressOperand,
    labels: &HashMap<String, usize>,
) -> Result<usize, CompilerError> {
    match address {
        AddressOperand::Absolute(value) => Ok(value),
        AddressOperand::Label(label) => labels
            .get(&label)
            .copied()
            .ok_or(CompilerError::InvalidAddress(label)),
    }
}
