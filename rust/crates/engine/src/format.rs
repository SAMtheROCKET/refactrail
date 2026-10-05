//! The independent formatter (mirror of formatting.py, format_tokens.py
//! and format_operators.py): operator and comma spacing on unprotected
//! lines, optional wrapping (format_wrap.rs), and validation that only
//! whitespace changed. Columns are characters, as in Python's tokenize.

use refactrail_lexer::{Kind, Token};
use refactrail_parser::ast::{walk_expr, walk_stmt, CmpOperator, Expr, ExprKind, Module, Operator, Stmt, StmtKind, Visitor};
use refactrail_parser::fast_hash::FastSet;
use refactrail_parser::Loc;
use std::collections::{BTreeMap, BTreeSet};

/// A formatting failure: a SyntaxError (with line) or another refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    Syntax { line: usize, message: String },
    Value(String),
}

/// One tokenize token with character positions.
#[derive(Clone, Debug)]
pub struct Tok {
    pub kind: Kind,
    pub text: String,
    pub start: (u32, u32),
    pub end: (u32, u32),
}

/// TokenIndex: tokens, their starts, and the normalized lines.
pub struct TokenIndex<'a> {
    pub tokens: Vec<Tok>,
    pub starts: Vec<(u32, u32)>,
    pub lines: Vec<&'a str>,
}

/// Replace \r\n and \r with \n.
pub fn normalize_newlines(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Tokenize normalized source as tokenize.generate_tokens does.
pub fn build_token_index(normalized: &str) -> Result<TokenIndex<'_>, FormatError> {
    let tokens: Vec<Token> = refactrail_lexer::tokenize(normalized)
        .map_err(|error| FormatError::Syntax { line: error.line as usize, message: error.message })?;
    let tokens: Vec<Tok> = tokens
        .iter()
        .map(|token| Tok { kind: token.kind, text: token.text(normalized).to_string(), start: token.start_pos, end: token.end_pos })
        .collect();
    let starts = tokens.iter().map(|token| token.start).collect();
    Ok(TokenIndex { tokens, starts, lines: normalized.split('\n').collect() })
}

/// Parse (and compile-check) source as parse_quietly_node does.
pub fn parse_checked(normalized: &str) -> Result<Module, FormatError> {
    if normalized.contains('\0') {
        return Err(FormatError::Value("source code string cannot contain null bytes".into()));
    }
    let tree = refactrail_parser::parse(normalized).map_err(|error| {
        let (line, _, message) = refactrail_parser::syntax_error_tuple(normalized, error);
        FormatError::Syntax { line: line as usize, message }
    })?;
    if let Some(error) = refactrail_parser::compile::check(&tree) {
        return Err(FormatError::Syntax { line: error.line as usize, message: error.message });
    }
    Ok(tree)
}

fn is_ignored(kind: Kind) -> bool {
    matches!(kind, Kind::Indent | Kind::Dedent | Kind::Nl | Kind::Newline | Kind::EndMarker)
}

/// `#\s*fmt:\s*(off|on|skip)\b` searched in a comment.
fn fmt_directive(comment: &str) -> Option<&'static str> {
    let mut rest = comment;
    while let Some(at) = rest.find('#') {
        let after = crate::source::python_strip_start(&rest[at + 1..]);
        if let Some(tail) = after.strip_prefix("fmt:") {
            let tail = crate::source::python_strip_start(tail);
            for word in ["off", "on", "skip"] {
                if let Some(next) = tail.strip_prefix(word) {
                    if !next.chars().next().is_some_and(crate::lexical::is_word_char) {
                        return Some(match word {
                            "off" => "off",
                            "on" => "on",
                            _ => "skip",
                        });
                    }
                }
            }
        }
        rest = &rest[at + 1..];
    }
    None
}

/// collect_protected_lines_set: comments, multi-line strings, f-strings
/// (optional), fmt: off/on/skip regions and backslash continuations.
pub fn protected_lines(index: &TokenIndex, tree: &Module, fstrings: bool) -> FastSet<u32> {
    let mut protected = FastSet::default();
    let mut directives: BTreeMap<u32, &'static str> = BTreeMap::new();
    for token in &index.tokens {
        if token.kind == Kind::Comment {
            protected.insert(token.start.0);
            if let Some(directive) = fmt_directive(&token.text) {
                directives.insert(token.start.0, directive);
            }
        }
        if token.kind == Kind::String && token.start.0 != token.end.0 {
            protected.extend(token.start.0..=token.end.0);
        }
    }
    if fstrings {
        struct Fstrings<'p> {
            protected: &'p mut FastSet<u32>,
        }
        impl<'a> Visitor<'a> for Fstrings<'_> {
            fn visit_expr(&mut self, node: &'a Expr) {
                if matches!(node.kind, ExprKind::JoinedStr { .. } | ExprKind::TemplateStr { .. }) {
                    self.protected.extend(node.loc.line..=node.loc.end_line);
                }
                walk_expr(self, node);
            }
        }
        let mut visitor = Fstrings { protected: &mut protected };
        for statement in &tree.body {
            visitor.visit_stmt(statement);
        }
    }
    let mut disabled = false;
    for (index_number, line) in index.lines.iter().enumerate() {
        let number = index_number as u32 + 1;
        let directive = directives.get(&number).copied();
        if directive == Some("off") {
            disabled = true;
        }
        if disabled || directive == Some("skip") {
            protected.insert(number);
        }
        if directive == Some("on") {
            disabled = false;
        }
        if line.trim_end_matches(crate::source::is_python_space).ends_with('\\') {
            protected.insert(number);
            protected.insert(number + 1);
        }
    }
    protected
}

/// collect_line_tokens_dict: single-line meaningful tokens per row.
pub fn line_tokens(index: &TokenIndex) -> BTreeMap<u32, Vec<Tok>> {
    let mut grouped: BTreeMap<u32, Vec<Tok>> = BTreeMap::new();
    for token in &index.tokens {
        if !is_ignored(token.kind) && token.start.0 == token.end.0 {
            grouped.entry(token.start.0).or_default().push(token.clone());
        }
    }
    grouped
}

/// collect_literal_tokens_list: STRING and COMMENT spellings in order.
fn literal_tokens(index: &TokenIndex) -> Vec<(Kind, String)> {
    index
        .tokens
        .iter()
        .filter(|token| matches!(token.kind, Kind::String | Kind::Comment))
        .map(|token| (token.kind, token.text.clone()))
        .collect()
}

fn operator_text(op: Operator) -> &'static str {
    match op {
        Operator::Add => "+",
        Operator::Sub => "-",
        Operator::Mult => "*",
        Operator::MatMult => "@",
        Operator::Div => "/",
        Operator::FloorDiv => "//",
        Operator::Mod => "%",
        Operator::Pow => "**",
        Operator::LShift => "<<",
        Operator::RShift => ">>",
        Operator::BitOr => "|",
        Operator::BitXor => "^",
        Operator::BitAnd => "&",
    }
}

fn comparison_text(op: CmpOperator) -> &'static str {
    match op {
        CmpOperator::Eq => "==",
        CmpOperator::NotEq => "!=",
        CmpOperator::Lt => "<",
        CmpOperator::LtE => "<=",
        CmpOperator::Gt => ">",
        CmpOperator::GtE => ">=",
        CmpOperator::Is => "is",
        CmpOperator::IsNot => "is not",
        CmpOperator::In => "in",
        CmpOperator::NotIn => "not in",
    }
}

/// Character column of a byte column on a (1-based) line.
fn char_column(lines: &[&str], line: u32, byte_col: u32) -> u32 {
    let text = lines.get(line as usize - 1).copied().unwrap_or("");
    let end = (byte_col as usize).min(text.len());
    let prefix = text.get(..end).unwrap_or(text);
    if prefix.is_ascii() { prefix.len() as u32 } else { prefix.chars().count() as u32 }
}

/// collect_operator_positions_set: starts of operator tokens to space.
pub fn operator_positions(tree: &Module, index: &TokenIndex) -> FastSet<(u32, u32)> {
    struct Gaps<'i, 'a> {
        index: &'i TokenIndex<'a>,
        positions: FastSet<(u32, u32)>,
    }
    impl Gaps<'_, '_> {
        fn gap(&mut self, left: Loc, right: Loc, operator: &str) {
            let start = (left.end_line, char_column(&self.index.lines, left.end_line, left.end_col));
            let end = (right.line, char_column(&self.index.lines, right.line, right.col));
            let first = self.index.starts.partition_point(|&position| position < start);
            let last = self.index.starts.partition_point(|&position| position < end);
            let names: Vec<&str> = operator.split(' ').collect();
            for token in &self.index.tokens[first..last.max(first)] {
                if names.contains(&token.text.as_str()) {
                    self.positions.insert(token.start);
                }
            }
        }
    }
    impl<'a> Visitor<'a> for Gaps<'_, '_> {
        fn visit_stmt(&mut self, node: &'a Stmt) {
            match &node.kind {
                StmtKind::Assign { targets, value, .. } => {
                    let operands: Vec<&Expr> = targets.iter().chain(std::iter::once(&**value)).collect();
                    for pair in operands.windows(2) {
                        self.gap(pair[0].loc, pair[1].loc, "=");
                    }
                }
                StmtKind::AnnAssign { annotation, value: Some(value), .. } => self.gap(annotation.loc, value.loc, "="),
                StmtKind::AugAssign { target, op, value } => {
                    let text = format!("{}=", operator_text(*op));
                    self.gap(target.loc, value.loc, &text);
                }
                _ => {}
            }
            walk_stmt(self, node);
        }

        fn visit_expr(&mut self, node: &'a Expr) {
            match &node.kind {
                ExprKind::BinOp { left, op, right } => self.gap(left.loc, right.loc, operator_text(*op)),
                ExprKind::Compare { left, ops, comparators } => {
                    let operands: Vec<&Expr> = std::iter::once(&**left).chain(comparators).collect();
                    for (index, op) in ops.iter().enumerate() {
                        self.gap(operands[index].loc, operands[index + 1].loc, comparison_text(*op));
                    }
                }
                ExprKind::NamedExpr { target, value } => self.gap(target.loc, value.loc, ":="),
                _ => {}
            }
            walk_expr(self, node);
        }
    }
    let mut gaps = Gaps { index, positions: FastSet::default() };
    for statement in &tree.body {
        gaps.visit_stmt(statement);
    }
    gaps.positions
}

fn gap_replacement(left: &Tok, right: &Tok, positions: &FastSet<(u32, u32)>) -> Option<&'static str> {
    if positions.contains(&left.start) || positions.contains(&right.start) {
        return Some(" ");
    }
    if right.text == "," {
        return Some("");
    }
    if left.text == "," {
        return Some(if matches!(right.text.as_str(), ")" | "]" | "}") { "" } else { " " });
    }
    None
}

/// format_line_str: edit whitespace gaps between tokens on one line.
fn format_line(line: &str, tokens: &[Tok], positions: &FastSet<(u32, u32)>) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut edits: Vec<(usize, usize, &str)> = Vec::new();
    for pair in tokens.windows(2) {
        let (start, end) = (pair[0].end.1 as usize, pair[1].start.1 as usize);
        let Some(replacement) = gap_replacement(&pair[0], &pair[1], positions) else { continue };
        let gap = chars.get(start.min(chars.len())..end.min(chars.len()).max(start.min(chars.len()))).unwrap_or(&[]);
        if gap.iter().all(|&character| character == ' ' || character == '\t') {
            edits.push((start, end, replacement));
        }
    }
    let mut chars = chars;
    for (start, end, replacement) in edits.into_iter().rev() {
        let start = start.min(chars.len());
        let end = end.min(chars.len()).max(start);
        chars.splice(start..end, replacement.chars());
    }
    let text: String = chars.into_iter().collect();
    text.trim_end_matches([' ', '\t']).to_string()
}

/// Split text at line ends, keeping them: [line, ending, line, ...].
pub fn split_line_ends(text: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                pieces.push(&text[start..index]);
                pieces.push(&text[index..index + 2]);
                index += 2;
                start = index;
            }
            b'\r' | b'\n' => {
                pieces.push(&text[start..index]);
                pieces.push(&text[index..index + 1]);
                index += 1;
                start = index;
            }
            _ => index += 1,
        }
    }
    pieces.push(&text[start..]);
    pieces
}

/// format_spacing_str: operator and comma spacing on unprotected lines.
pub fn format_spacing(text: &str) -> Result<String, FormatError> {
    let normalized = normalize_newlines(text);
    let tree = parse_checked(&normalized)?;
    let index = build_token_index(&normalized)?;
    let protected = protected_lines(&index, &tree, true);
    let positions = operator_positions(&tree, &index);
    let tokens = line_tokens(&index);
    let pieces = split_line_ends(text);
    let mut out = String::with_capacity(text.len() + 16);
    for (piece_index, piece) in pieces.iter().enumerate() {
        if piece_index % 2 == 1 {
            out.push_str(piece);
            continue;
        }
        let number = (piece_index / 2 + 1) as u32;
        if protected.contains(&number) {
            out.push_str(piece);
        } else {
            let empty = Vec::new();
            out.push_str(&format_line(piece, tokens.get(&number).unwrap_or(&empty), &positions));
        }
    }
    if !out.is_empty() && !out.ends_with(['\r', '\n']) {
        out.push_str(if pieces.len() > 1 { pieces[1] } else { "\n" });
    }
    Ok(out)
}

/// validate_formatted_none: same tree, same literal and comment tokens.
pub fn validate(original: &str, formatted: &str) -> Result<(), FormatError> {
    let original = normalize_newlines(original);
    let formatted = normalize_newlines(formatted);
    let formatted_tree = parse_checked(&formatted)?;
    let original_tree = refactrail_parser::parse(&original)
        .map_err(|error| FormatError::Syntax { line: error.line as usize, message: error.message })?;
    if refactrail_parser::dump_module(&original_tree, false) != refactrail_parser::dump_module(&formatted_tree, false) {
        return Err(FormatError::Value("Formatting would change the source AST".into()));
    }
    if literal_tokens(&build_token_index(&original)?) != literal_tokens(&build_token_index(&formatted)?) {
        return Err(FormatError::Value("Formatting would change literal or comment tokens".into()));
    }
    Ok(())
}

/// Whether the source has `# type: ignore` comments (ast type_ignores),
/// using CPython's tokenizer rule for type comments.
pub fn has_type_ignores(index: &TokenIndex) -> bool {
    index.tokens.iter().any(|token| token.kind == Kind::Comment && type_comment_kind(&token.text) == Some(true))
}

/// None for an ordinary comment; Some(true) for `# type: ignore...`,
/// Some(false) for another type comment.
pub fn type_comment_kind(comment: &str) -> Option<bool> {
    let bytes = comment.as_bytes();
    let prefix = b"# type: ";
    let mut at = 0;
    for &expected in prefix {
        if expected == b' ' {
            while at < bytes.len() && (bytes[at] == b' ' || bytes[at] == b'\t') {
                at += 1;
            }
        } else if at < bytes.len() && bytes[at] == expected {
            at += 1;
        } else {
            return None;
        }
    }
    let rest = &bytes[at..];
    let ignore = rest.starts_with(b"ignore") && rest.get(6).is_none_or(|&next| next < 128 && !next.is_ascii_alphanumeric());
    Some(ignore)
}

/// format_source_str: spacing, optional wrapping, then validation.
pub fn format_source(text: &str, width: Option<usize>, hug: bool) -> Result<String, FormatError> {
    let normalized = normalize_newlines(text);
    let mut formatted = format_spacing(text)?;
    if let Some(width) = width {
        formatted = format_spacing(&crate::format_wrap::wrap_source(&formatted, width, hug)?)?;
    }
    validate(&normalized, &normalize_newlines(&formatted))?;
    Ok(formatted)
}

/// Lines a set covers, sorted (for tests and debugging).
pub fn sorted_lines(lines: &FastSet<u32>) -> Vec<u32> {
    lines.iter().copied().collect::<BTreeSet<u32>>().into_iter().collect()
}
