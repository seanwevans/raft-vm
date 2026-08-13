//! Front-end edge cases: escapes, comments, operand handling, and the errors
//! each stage reports.

use raft::compiler::{
    AddressOperand, AstNode, CompilerError, Instruction, Lexer, Parser, TokenKind,
};
use raft::vm::opcodes::OpCode;
use raft::vm::value::Value;
use raft::Compiler;

fn kinds(source: &str) -> Vec<TokenKind> {
    Lexer::new(source)
        .lex()
        .unwrap_or_else(|error| panic!("{source:?} should lex: {error}"))
        .into_iter()
        .map(|token| token.kind)
        .collect()
}

fn lex_error(source: &str) -> CompilerError {
    Lexer::new(source)
        .lex()
        .expect_err("expected this source to fail lexing")
}

fn compile_error(source: &str) -> CompilerError {
    Compiler::compile(source).expect_err("expected this source to fail compiling")
}

// --- literals and words ----------------------------------------------------

#[test]
fn booleans_labels_and_identifiers_are_distinguished() {
    assert_eq!(
        kinds("true false .loop StoreVar"),
        vec![
            TokenKind::Boolean(true),
            TokenKind::Boolean(false),
            TokenKind::Label("loop".to_string()),
            TokenKind::Identifier("StoreVar".to_string()),
        ]
    );
}

#[test]
fn every_arithmetic_symbol_lexes() {
    assert_eq!(
        kinds("+ - * / % ^"),
        ["+", "-", "*", "/", "%", "^"]
            .into_iter()
            .map(|symbol| TokenKind::Symbol(symbol.to_string()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_word_with_a_dot_but_no_digits_is_an_identifier() {
    assert_eq!(
        kinds("io.print"),
        vec![TokenKind::Identifier("io.print".to_string())]
    );
}

#[test]
fn a_malformed_float_is_reported() {
    let error = lex_error("1.2.3");
    assert!(
        error.to_string().contains("Invalid float: 1.2.3"),
        "got: {error}"
    );
}

// --- strings ---------------------------------------------------------------

#[test]
fn string_escapes_are_decoded() {
    assert_eq!(
        kinds(r#""a\nb\rc\td\"e\\f""#),
        vec![TokenKind::String("a\nb\rc\td\"e\\f".to_string())]
    );
}

#[test]
fn an_unknown_escape_keeps_the_escaped_character() {
    assert_eq!(
        kinds(r#""a\qb""#),
        vec![TokenKind::String("aqb".to_string())]
    );
}

#[test]
fn a_string_may_contain_spaces_and_comment_markers() {
    assert_eq!(
        kinds(r#""hello raft vm # not a comment""#),
        vec![TokenKind::String(
            "hello raft vm # not a comment".to_string()
        )]
    );
}

#[test]
fn an_unterminated_string_is_reported() {
    let error = lex_error("\"never closed");
    assert!(
        error.to_string().contains("unterminated string literal"),
        "got: {error}"
    );
}

#[test]
fn an_unterminated_escape_is_reported() {
    let error = lex_error("\"trailing\\");
    assert!(
        error.to_string().contains("unterminated string escape"),
        "got: {error}"
    );
}

// --- comments --------------------------------------------------------------

#[test]
fn both_comment_styles_are_skipped() {
    assert_eq!(
        kinds("# hash comment\n1\n// slash comment\n2"),
        vec![TokenKind::Integer(1), TokenKind::Integer(2)]
    );
}

#[test]
fn a_comment_may_follow_a_token_without_whitespace() {
    assert_eq!(
        kinds("1# trailing\n2// trailing"),
        vec![TokenKind::Integer(1), TokenKind::Integer(2)]
    );
}

#[test]
fn a_source_of_only_comments_produces_no_tokens() {
    assert!(kinds("# nothing here\n// nor here\n").is_empty());
}

// --- spans -----------------------------------------------------------------

#[test]
fn tokens_carry_their_source_position() {
    let tokens = Lexer::new("1\n  22").lex().expect("source should lex");
    assert_eq!(tokens[0].span.start.line, 1);
    assert_eq!(tokens[0].span.start.column, 1);
    assert_eq!(tokens[1].span.start.line, 2);
    assert_eq!(tokens[1].span.start.column, 3);
}

// --- parsing ---------------------------------------------------------------

#[test]
fn a_label_becomes_a_label_node() {
    let nodes = Parser::new(Lexer::new(".here").lex().expect("lex"))
        .parse()
        .expect("parse");
    assert!(matches!(
        nodes.as_slice(),
        [AstNode::Label { name, .. }] if name == "here"
    ));
}

#[test]
fn addresses_accept_both_numbers_and_labels() {
    let nodes = Parser::new(Lexer::new("Jump 3 Jump .target").lex().expect("lex"))
        .parse()
        .expect("parse");
    let operands: Vec<&AddressOperand> = nodes
        .iter()
        .filter_map(|node| match node {
            AstNode::Instruction {
                instruction: Instruction::Jump(operand),
                ..
            } => Some(operand),
            _ => None,
        })
        .collect();
    assert_eq!(operands[0], &AddressOperand::Address(3));
    assert_eq!(operands[1], &AddressOperand::Label("target".to_string()));
}

#[test]
fn a_missing_operand_is_reported_with_the_instruction_name() {
    let error = compile_error("StoreVar");
    assert!(
        error
            .to_string()
            .contains("expected operand after StoreVar"),
        "got: {error}"
    );
}

#[test]
fn a_negative_operand_is_not_an_address() {
    let error = compile_error("Jump -1");
    assert!(
        matches!(error, CompilerError::InvalidAddress(_)),
        "got: {error}"
    );
}

#[test]
fn a_non_numeric_operand_is_not_an_address() {
    let error = compile_error("StoreVar true");
    assert!(
        matches!(error, CompilerError::InvalidAddress(_)),
        "got: {error}"
    );
}

#[test]
fn an_unknown_label_is_reported() {
    let error = compile_error("Jump .nowhere");
    assert!(
        error.to_string().contains("unknown label .nowhere"),
        "got: {error}"
    );
}

#[test]
fn an_unknown_word_is_an_invalid_token() {
    assert!(matches!(
        compile_error("Frobnicate"),
        CompilerError::InvalidToken(word) if word == "Frobnicate"
    ));
}

#[test]
fn a_name_operand_accepts_identifiers_labels_and_strings() {
    for source in [
        r#"LoadGlobal io"#,
        r#"LoadGlobal .io"#,
        r#"LoadGlobal "io""#,
    ] {
        assert!(
            matches!(
                Compiler::compile(source).expect("should compile").as_slice(),
                [OpCode::LoadGlobal(name)] if name == "io"
            ),
            "{source} should load the io global"
        );
    }
}

#[test]
fn a_name_operand_rejects_a_number() {
    let error = compile_error("LoadGlobal 3.5");
    assert!(
        error.to_string().contains("expected name after LoadGlobal"),
        "got: {error}"
    );
}

#[test]
fn a_string_operand_rejects_a_number() {
    let error = compile_error("MakeString 3.5");
    assert!(
        error
            .to_string()
            .contains("expected string token after MakeString"),
        "got: {error}"
    );
}

#[test]
fn make_module_reads_one_name_per_declared_export() {
    assert!(matches!(
        Compiler::compile("1 2 MakeModule 2 first second")
            .expect("should compile")
            .as_slice(),
        [
            OpCode::PushConst(Value::Integer(1)),
            OpCode::PushConst(Value::Integer(2)),
            OpCode::MakeModule(exports),
        ] if exports == &["first", "second"]
    ));
}

// --- emission --------------------------------------------------------------

#[test]
fn labels_account_for_multi_opcode_instructions() {
    // `io.print` emits two opcodes, so `.after` must resolve past both.
    let bytecode =
        Compiler::compile("io.print Jump .after 1 .after Return").expect("should compile");
    assert!(matches!(bytecode[0], OpCode::LoadGlobal(_)));
    assert!(matches!(bytecode[1], OpCode::GetExport(_)));
    assert!(
        matches!(bytecode[2], OpCode::Jump(4)),
        "expected the label to resolve to index 4, got {:?}",
        bytecode[2]
    );
}

#[test]
fn a_label_at_the_end_resolves_to_the_program_length() {
    let bytecode = Compiler::compile("Jump .end 1 .end").expect("should compile");
    assert!(
        matches!(bytecode[0], OpCode::Jump(2)),
        "got {:?}",
        bytecode[0]
    );
    assert_eq!(bytecode.len(), 2);
}

#[test]
fn debug_spans_are_emitted_one_per_opcode() {
    let program = Compiler::compile_with_debug("1\n2\n+").expect("should compile");
    assert_eq!(program.bytecode.len(), 3);
    assert_eq!(program.debug_info.instruction_spans().len(), 3);

    let second = program
        .debug_info
        .location_for_instruction(1)
        .expect("the second opcode should have a location");
    assert_eq!(second.line, 2);
}

#[test]
fn an_empty_source_compiles_to_an_empty_program() {
    assert!(Compiler::compile("").expect("should compile").is_empty());
}
