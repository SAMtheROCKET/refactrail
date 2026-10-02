//! Adjacent string literals and f-strings (PEP 701 tokens) to
//! `Constant` / `JoinedStr` nodes, with CPython 3.12's positions.

use refactrail_lexer::Kind;

use crate::ast::{Expr, ExprKind};
use crate::expr::{constant_node, expr};
use crate::literals;
use crate::node::{Constant, Loc};
use crate::parser::{ParseError, PResult, Parser};

enum Part {
    Text(Vec<u32>, Loc),
    Field(Expr),
}

impl<'a> Parser<'a> {
    pub fn strings(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let mut parts = Vec::new();
        let (mut any_fstring, mut any_bytes, mut any_str) = (false, false, false);
        let mut bytes_value = Vec::new();
        let mut unicode_kind = None;
        loop {
            match self.kind() {
                Kind::String => {
                    let index = self.bump();
                    let split = literals::split_string(self.text_at(index));
                    if unicode_kind.is_none() {
                        unicode_kind = Some(split.unicode_kind);
                    }
                    if split.bytes {
                        any_bytes = true;
                        let value = literals::decode_bytes(split.body, split.raw).map_err(|m| self.error_at_token(index, &m))?;
                        bytes_value.extend(value);
                    } else {
                        any_str = true;
                        let value = literals::decode_str(split.body, split.raw).map_err(|m| self.error_at_token(index, &m))?;
                        parts.push(Part::Text(value, self.loc(index)));
                    }
                }
                Kind::FStringStart => {
                    any_fstring = true;
                    if unicode_kind.is_none() {
                        unicode_kind = Some(false);
                    }
                    let index = self.bump();
                    let raw = self.text_at(index).bytes().any(|byte| byte == b'r' || byte == b'R');
                    self.fstring_body(raw, &mut parts, false)?;
                    if self.kind() != Kind::FStringEnd {
                        return Err(self.fstring_error("f-string: expecting '}'"));
                    }
                    self.bump();
                }
                _ => break,
            }
        }
        let loc = self.span(start);
        if any_bytes && (any_str || any_fstring) {
            return Err(self.error("cannot mix bytes and nonbytes literals"));
        }
        if any_bytes {
            return Ok(constant_node(Constant::Bytes(bytes_value), loc));
        }
        if !any_fstring {
            let mut parts = parts.into_iter();
            let mut value = match parts.next() {
                Some(Part::Text(text, _)) => text,
                _ => Vec::new(),
            };
            for part in parts {
                if let Part::Text(text, _) = part {
                    value.extend(text);
                }
            }
            let kind = (unicode_kind == Some(true)).then(|| Box::<str>::from("u"));
            return Ok(expr(ExprKind::Constant { value: Constant::Str(value), kind }, loc));
        }
        Ok(expr(ExprKind::JoinedStr { values: merge_parts(parts, false) }, loc))
    }

    fn error_at_token(&self, index: usize, message: &str) -> ParseError {
        let loc = self.loc(index);
        ParseError { line: loc.line, col: loc.col, offset: 0, message: message.to_string() }
    }

    /// Parse middles and fields up to FSTRING_END (or, in a format spec,
    /// up to the closing `}` of the field), appending parts.
    fn fstring_body(&mut self, raw: bool, parts: &mut Vec<Part>, in_spec: bool) -> PResult<()> {
        loop {
            match self.kind() {
                Kind::FStringMiddle => {
                    let index = self.bump();
                    let text = self.text_at(index);
                    let value = literals::decode_str(text, raw).map_err(|m| self.error_at_token(index, &m))?;
                    let mut loc = self.loc(index);
                    // A middle cut at a doubled brace ends after both braces.
                    let token = self.raw_token(index);
                    let last = text.chars().last();
                    let named_escape_end = last == Some('}')
                        && !raw
                        && text.rfind("\\N{").is_some_and(|at| !text[at..text.len() - 1].contains('}'));
                    if !in_spec
                        && !named_escape_end
                        && matches!(last, Some('{') | Some('}'))
                        && self.src.as_bytes().get(token.end) == Some(&(last.unwrap_or(' ') as u8))
                        && token.end > token.start
                    {
                        loc.end_col += 1;
                    }
                    parts.push(Part::Text(value, loc));
                }
                Kind::Op if self.code() == 5 => {
                    self.replacement_field(raw, parts)?;
                }
                Kind::Op if in_spec && self.code() == 6 => return Ok(()),
                Kind::FStringEnd if !in_spec => return Ok(()),
                _ => return Err(self.fstring_error("f-string: expecting '}'")),
            }
        }
    }

    /// CPython raises f-string messages from its second-pass rules; the
    /// first pass fails with a generic error.
    fn fstring_error(&self, message: &str) -> ParseError {
        self.error(if self.invalid_mode { message } else { "invalid syntax" })
    }

    fn at_any_op(&self, ops: &[&str]) -> bool {
        self.kind() == Kind::Op && ops.contains(&self.text())
    }

    fn replacement_field(&mut self, raw: bool, parts: &mut Vec<Part>) -> PResult<()> {
        let open = self.bump();
        if self.at_any_op(&["=", "!", ":", "}"]) {
            let message = format!("f-string: valid expression required before '{}'", self.text());
            return Err(self.fstring_error(&message));
        }
        let value = if self.at_kw("yield") { self.yield_expression()? } else { self.star_expressions()? };
        if !self.at_any_op(&["=", "!", ":", "}"]) {
            return Err(self.fstring_error("f-string: expecting '=', or '!', or ':', or '}'"));
        }
        let mut debug = false;
        if self.at_op("=") {
            let equals = self.bump();
            debug = true;
            let text_start = self.raw_token(open).end;
            let text_end = self.raw_token(self.pos).start;
            let text = literals::normalize_newlines(&self.src[text_start..text_end]);
            let loc = Loc { line: self.loc(open).end_line, col: self.loc(open).end_col, ..self.loc(equals) };
            let loc = Loc { end_line: self.loc(self.pos).line, end_col: self.loc(self.pos).col, ..loc };
            parts.push(Part::Text(text.chars().map(|c| c as u32).collect(), loc));
            if !self.at_any_op(&["!", ":", "}"]) {
                return Err(self.fstring_error("f-string: expecting '!', or ':', or '}'"));
            }
        }
        let mut conversion: i64 = -1;
        if let Some(bang) = self.eat_op("!") {
            if self.at_any_op(&[":", "}"]) {
                return Err(self.fstring_error("f-string: missing conversion character"));
            }
            if self.kind() != Kind::Name {
                return Err(self.fstring_error("f-string: invalid conversion character"));
            }
            let index = self.bump();
            if self.loc(index).col != self.loc(bang).end_col || self.loc(index).line != self.loc(bang).line {
                return Err(self.error_at_token(index, "f-string: conversion type must come right after the exclamanation mark"));
            }
            conversion = match self.text_at(index) {
                "r" => 114,
                "s" => 115,
                "a" => 97,
                name => {
                    let message = format!("f-string: invalid conversion character '{name}': expected 's', 'r', or 'a'");
                    return Err(self.error_at_token(index, &message));
                }
            };
            if !self.at_any_op(&[":", "}"]) {
                return Err(self.fstring_error("f-string: expecting ':' or '}'"));
            }
        }
        let mut format_spec = None;
        if let Some(colon) = self.eat_op(":") {
            let mut spec_parts = Vec::new();
            self.fstring_body(raw, &mut spec_parts, true)?;
            let loc = self.span(colon);
            let mut values = merge_parts(spec_parts, true);
            // CPython keeps 3.11's shape: a spec that is only an empty
            // string is an empty JoinedStr.
            if values.len() == 1 && is_empty_constant(&values[0]) {
                values.clear();
            }
            format_spec = Some(Box::new(expr(ExprKind::JoinedStr { values }, loc)));
        }
        if debug && conversion == -1 && format_spec.is_none() {
            conversion = 114;
        }
        if !self.at_op("}") {
            return Err(self.fstring_error("f-string: expecting '}'"));
        }
        self.bump();
        let field = expr(ExprKind::FormattedValue { value: Box::new(value), conversion, format_spec }, self.span(open));
        parts.push(Part::Field(field));
        Ok(())
    }
}

/// Turn parts into values. At the top level adjacent texts merge into one
/// Constant and empty ones are dropped; inside a format spec (`in_spec`)
/// CPython keeps every text piece as its own Constant, empty ones too.
fn merge_parts(parts: Vec<Part>, in_spec: bool) -> Vec<Expr> {
    if in_spec {
        return parts
            .into_iter()
            .map(|part| match part {
                Part::Text(text, loc) => constant_node(Constant::Str(text), loc),
                Part::Field(node) => node,
            })
            .collect();
    }
    let mut values = Vec::new();
    let mut pending: Option<(Vec<u32>, Loc)> = None;
    let flush = |pending: &mut Option<(Vec<u32>, Loc)>, values: &mut Vec<Expr>| {
        if let Some((text, loc)) = pending.take() {
            if !text.is_empty() {
                values.push(constant_node(Constant::Str(text), loc));
            }
        }
    };
    for part in parts {
        match part {
            Part::Text(text, loc) => match pending.as_mut() {
                Some((value, span)) => {
                    value.extend(text);
                    *span = span.to(loc);
                }
                None => pending = Some((text, loc)),
            },
            Part::Field(node) => {
                flush(&mut pending, &mut values);
                values.push(node);
            }
        }
    }
    flush(&mut pending, &mut values);
    values
}

fn is_empty_constant(value: &Expr) -> bool {
    matches!(&value.kind, ExprKind::Constant { value: Constant::Str(text), .. } if text.is_empty())
}
