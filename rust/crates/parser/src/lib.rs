//! RefacTrail's own Python parser.
//!
//! Builds syntax trees that `ast.dump` exactly like CPython 3.12's
//! `ast.parse`, from RefacTrail's own tokenizer. Written from the
//! language reference and CPython's observable behaviour; no third-party
//! parser code is used.

pub mod ast;
pub mod compile;
pub mod fast_hash;
mod expr;
pub mod literals;
pub mod node;
pub mod nfkc;
mod nfkc_tables;
mod parser;
mod patterns;
pub mod small_str;
pub mod symtable;
mod strings;
mod unicode;

pub use ast::{dump_module, Module};
pub use node::{Constant, Loc};
pub use parser::ParseError;

/// Parse a module (source already decoded, without a BOM).
pub fn parse(source: &str) -> Result<Module, ParseError> {
    parser::Parser::new(source).parse_module()
}

/// Parse a module and also return the byte spans of its comments.
pub fn parse_with_comments(source: &str) -> Result<(Module, Vec<(usize, usize)>), ParseError> {
    let mut parser = parser::Parser::new(source);
    let comments = std::mem::take(&mut parser.comments);
    parser.parse_module().map(|tree| (tree, comments))
}

/// What `compile(source, path, "exec")` reports: None when it compiles,
/// else (line, offset, message) as SyntaxError's lineno, offset and msg.
pub fn compile_check(source: &str) -> Option<(u32, u32, String)> {
    if source.contains('\0') {
        return Some((1, 1, "source code string cannot contain null bytes".into()));
    }
    match parse(source) {
        Err(error) => Some(syntax_error_tuple(source, error)),
        Ok(tree) => compile::check(&tree).map(|error| (error.line, error.offset, error.message)),
    }
}

/// A parse error as CPython's SyntaxError (line, offset, message).
pub fn syntax_error_tuple(source: &str, error: ParseError) -> (u32, u32, String) {
    let offset = if error.offset > 0 { error.offset } else { char_offset(source, error.line, error.col) };
    (error.line, offset, error.message)
}

/// 1-based character offset of a byte column on a line.
fn char_offset(source: &str, line: u32, byte_col: u32) -> u32 {
    let text = source.split_inclusive(['\n']).nth(line.saturating_sub(1) as usize).unwrap_or("");
    let prefix = text.get(..(byte_col as usize).min(text.len())).unwrap_or(text);
    prefix.chars().count() as u32 + 1
}

/// `ast.dump(ast.parse(source), include_attributes=attributes)`, or
/// "ERROR line" when the source does not parse.
pub fn dump_source(source: &str, attributes: bool) -> String {
    match parse(source) {
        Ok(tree) => dump_module(&tree, attributes),
        Err(error) => format!("ERROR {}", error.line),
    }
}

/// Development only: tokenize and prepare the parser without parsing.
#[doc(hidden)]
pub fn prepare_only(source: &str) -> usize {
    parser::Parser::new(source).kinds.len()
}

/// Development: the symbol table dump compared with CPython's symtable by
/// scripts/symtable_parity.py, or "ERROR ..." when the source fails.
pub fn dump_symtable_source(source: &str) -> String {
    let tree = match parse(source) {
        Ok(tree) => tree,
        Err(error) => return format!("ERROR {}", error.line),
    };
    match symtable::build(&tree, compile::future_annotations(&tree.body)) {
        Ok(table) => symtable::dump(&table),
        Err(error) => format!("ERROR {} {}", error.line, error.message),
    }
}
