//! Adjacent string literals, f-strings (PEP 701 tokens) and t-strings
//! (PEP 750, Python 3.14) to `Constant` / `JoinedStr` / `TemplateStr`
//! nodes, with CPython's positions.

use refactrail_lexer::{Kind, Version};

use crate::ast::{Expr, ExprKind};
use crate::expr::{constant_node, expr};
use crate::literals;
use crate::node::{Constant, Loc};
use crate::parser::{ParseError, PResult, Parser};

enum Part {
    /// Text, its location, and whether it came from a `u"..."` literal (a
    /// folded run of text takes the kind of its first piece).
    Text(Vec<u32>, Loc, bool),
    Field(Expr),
}

impl<'a> Parser<'a> {
    pub fn strings(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let mut parts = Vec::new();
        let (mut any_fstring, mut any_bytes, mut any_str) = (false, false, false);
        let mut bytes_value = Vec::new();
        let mut unicode_kind = None;
        // (is a t-string, start token) of each literal, for the 3.14 check
        // that t-strings are not mixed with other literals.
        let mut items: Vec<(bool, usize)> = Vec::new();
        loop {
            items.push((self.kind() == Kind::TStringStart, self.pos));
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
                        let value = literals::decode_str_version(split.body, split.raw, self.version).map_err(|m| self.error_at_token(index, &m))?;
                        parts.push(Part::Text(value, self.loc(index), split.unicode_kind));
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
                Kind::TStringStart => {
                    let index = self.bump();
                    let raw = self.text_at(index).bytes().any(|byte| byte == b'r' || byte == b'R');
                    self.template_body(raw, &mut parts)?;
                    if self.kind() != Kind::TStringEnd {
                        return Err(self.fstring_error("t-string: expecting '}'"));
                    }
                    self.bump();
                }
                _ => {
                    items.pop();
                    break;
                }
            }
        }
        let loc = self.span(start);
        let any_template = items.iter().any(|(template, _)| *template);
        if any_template {
            if let Some(switch) = items.windows(2).position(|pair| pair[0].0 != pair[1].0) {
                let last = items[switch].1;
                return Err(self.demote(self.specific(self.loc(last), "cannot mix t-string literals with string or bytes literals")));
            }
            return Ok(expr(ExprKind::TemplateStr { values: merge_parts(parts, false) }, loc));
        }
        if any_bytes && (any_str || any_fstring) {
            return Err(self.error("cannot mix bytes and nonbytes literals"));
        }
        if any_bytes {
            return Ok(constant_node(Constant::Bytes(bytes_value), loc));
        }
        if !any_fstring {
            let mut parts = parts.into_iter();
            let mut value = match parts.next() {
                Some(Part::Text(text, _, _)) => text,
                _ => Vec::new(),
            };
            for part in parts {
                if let Part::Text(text, _, _) = part {
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

    /// A t-string's middles and interpolations up to TSTRING_END.
    fn template_body(&mut self, raw: bool, parts: &mut Vec<Part>) -> PResult<()> {
        self.string_body(raw, parts, false, true)
    }

    /// Parse middles and fields up to FSTRING_END (or, in a format spec,
    /// up to the closing `}` of the field), appending parts.
    fn fstring_body(&mut self, raw: bool, parts: &mut Vec<Part>, in_spec: bool) -> PResult<()> {
        self.string_body(raw, parts, in_spec, false)
    }

    fn string_body(&mut self, raw: bool, parts: &mut Vec<Part>, in_spec: bool, template: bool) -> PResult<()> {
        let prefix = if template { 't' } else { 'f' };
        loop {
            match self.kind() {
                Kind::FStringMiddle | Kind::TStringMiddle => {
                    let index = self.bump();
                    let text = self.text_at(index);
                    let value = literals::decode_str_version(text, raw, self.version).map_err(|m| self.error_at_token(index, &m))?;
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
                    parts.push(Part::Text(value, loc, false));
                }
                Kind::Op if self.code() == 5 => {
                    self.replacement_field(raw, parts, template)?;
                }
                Kind::Op if in_spec && self.code() == 6 => return Ok(()),
                Kind::FStringEnd | Kind::TStringEnd if !in_spec => return Ok(()),
                _ => return Err(self.fstring_error(&format!("{prefix}-string: expecting '}}'"))),
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

    fn replacement_field(&mut self, raw: bool, parts: &mut Vec<Part>, template: bool) -> PResult<()> {
        let prefix = if template { 't' } else { 'f' };
        let open = self.bump();
        if self.at_any_op(&["=", "!", ":", "}"]) {
            let message = format!("{prefix}-string: valid expression required before '{}'", self.text());
            return Err(self.fstring_error(&message));
        }
        let value = if self.at_kw("yield") { self.yield_expression()? } else { self.star_expressions()? };
        if !self.at_any_op(&["=", "!", ":", "}"]) {
            return Err(self.fstring_error(&format!("{prefix}-string: expecting '=', or '!', or ':', or '}}'")));
        }
        let mut debug = false;
        if self.at_op("=") {
            let equals = self.bump();
            debug = true;
            let text_start = self.raw_token(open).end;
            let text_end = self.raw_token(self.pos).start;
            let text = expression_text(&self.src[text_start..text_end]);
            let loc = Loc { line: self.loc(open).end_line, col: self.loc(open).end_col, ..self.loc(equals) };
            let loc = Loc { end_line: self.loc(self.pos).line, end_col: self.loc(self.pos).col, ..loc };
            parts.push(Part::Text(text.chars().map(|c| c as u32).collect(), loc, false));
            if !self.at_any_op(&["!", ":", "}"]) {
                return Err(self.fstring_error(&format!("{prefix}-string: expecting '!', or ':', or '}}'")));
            }
        }
        // A t-string keeps its expression's source text (CPython's token
        // metadata), without comments or trailing whitespace and '='.
        let expression_source = template.then(|| {
            let text_start = self.raw_token(open).end;
            let text_end = self.raw_token(self.pos).start;
            strip_interpolation_text(&expression_text(&self.src[text_start..text_end]))
        });
        let mut conversion: i64 = -1;
        if let Some(bang) = self.eat_op("!") {
            if self.at_any_op(&[":", "}"]) {
                return Err(self.fstring_error(&format!("{prefix}-string: missing conversion character")));
            }
            if self.kind() != Kind::Name {
                return Err(self.fstring_error(&format!("{prefix}-string: invalid conversion character")));
            }
            let index = self.bump();
            if self.loc(index).col != self.loc(bang).end_col || self.loc(index).line != self.loc(bang).line {
                let message = format!("{prefix}-string: conversion type must come right after the exclamanation mark");
                return Err(self.error_at_token(index, &message));
            }
            conversion = match self.text_at(index) {
                "r" => 114,
                "s" => 115,
                "a" => 97,
                name => {
                    let message = format!("{prefix}-string: invalid conversion character '{name}': expected 's', 'r', or 'a'");
                    return Err(self.error_at_token(index, &message));
                }
            };
            if !self.at_any_op(&[":", "}"]) {
                return Err(self.fstring_error(&format!("{prefix}-string: expecting ':' or '}}'")));
            }
        }
        let mut format_spec = None;
        if let Some(colon) = self.eat_op(":") {
            let mut spec_parts = Vec::new();
            self.fstring_body(raw, &mut spec_parts, true)?;
            let loc = self.span(colon);
            // Python 3.13 drops empty pieces and joins adjacent texts in a
            // format spec, as for the whole f-string.
            let mut values = merge_parts(spec_parts, self.version < Version::Py313);
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
            return Err(self.fstring_error(&format!("{prefix}-string: expecting '}}'")));
        }
        self.bump();
        let field = match expression_source {
            Some(text) => expr(
                ExprKind::Interpolation {
                    value: Box::new(value),
                    str_: Constant::Str(text.chars().map(|c| c as u32).collect()),
                    conversion,
                    format_spec,
                },
                self.span(open),
            ),
            None => expr(ExprKind::FormattedValue { value: Box::new(value), conversion, format_spec }, self.span(open)),
        };
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
                Part::Text(text, loc, _) => constant_node(Constant::Str(text), loc),
                Part::Field(node) => node,
            })
            .collect();
    }
    let mut values = Vec::new();
    // (text, location, kind of its first nonempty piece)
    let mut pending: Option<(Vec<u32>, Loc, Option<bool>)> = None;
    let flush = |pending: &mut Option<(Vec<u32>, Loc, Option<bool>)>, values: &mut Vec<Expr>| {
        if let Some((text, loc, unicode_kind)) = pending.take() {
            if !text.is_empty() {
                let kind = (unicode_kind == Some(true)).then(|| Box::<str>::from("u"));
                values.push(expr(ExprKind::Constant { value: Constant::Str(text), kind }, loc));
            }
        }
    };
    for part in parts {
        match part {
            Part::Text(text, loc, unicode_kind) => {
                let first_kind = (!text.is_empty()).then_some(unicode_kind);
                match pending.as_mut() {
                    Some((value, span, kind)) => {
                        if kind.is_none() {
                            *kind = first_kind;
                        }
                        value.extend(text);
                        *span = span.to(loc);
                    }
                    None => pending = Some((text, loc, first_kind)),
                }
            }
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

/// An f/t-string expression's source text as CPython's tokenizer records
/// it: newlines normalized, and `#` comments (outside string literals)
/// removed up to their line break.
fn expression_text(raw: &str) -> String {
    let text = literals::normalize_newlines(raw);
    if !text.contains('#') {
        return text.into_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut out, mut index, mut quote) = (String::with_capacity(text.len()), 0, None::<char>);
    while index < chars.len() {
        let c = chars[index];
        match c {
            '"' | '\'' => {
                quote = match quote {
                    None => Some(c),
                    Some(open) if open == c => None,
                    other => other,
                };
                out.push(c);
            }
            '#' if quote.is_none() => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
                if index < chars.len() {
                    out.push('\n');
                }
            }
            _ => out.push(c),
        }
        index += 1;
    }
    out
}

/// CPython's `_strip_interpolation_expr`: trailing whitespace and '='
/// removed.
fn strip_interpolation_text(text: &str) -> String {
    text.trim_end_matches(|c: char| c.is_whitespace() || c == '=').to_string()
}
