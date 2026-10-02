//! Notebook formatting (mirror of notebooks.py and json_spans.py): format
//! Python code cells and replace only their `source` JSON values, keeping
//! every other byte of the notebook.

use crate::format::{format_source, FormatError};

/// A parsed JSON value; numbers keep their spelling.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(String),
    Str(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

struct Parser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    position: usize,
    /// Byte span of each `cells[i].source` value.
    sources: Vec<(usize, usize, usize)>,
}

impl Parser<'_> {
    /// json.JSONDecodeError's message: "msg: line L column C (char N)".
    fn error(&self, message: &str, at: usize) -> String {
        let prefix = &self.text[..at.min(self.text.len())];
        let char_position = prefix.chars().count();
        let line = prefix.matches('\n').count() + 1;
        let column = match prefix.rfind('\n') {
            Some(newline) => prefix[newline + 1..].chars().count() + 1,
            None => char_position + 1,
        };
        format!("{message}: line {line} column {column} (char {char_position})")
    }

    fn skip_space(&mut self) {
        while self.position < self.bytes.len() && matches!(self.bytes[self.position], b' ' | b'\t' | b'\n' | b'\r') {
            self.position += 1;
        }
    }

    fn value(&mut self, path: &[PathPart]) -> Result<JsonValue, String> {
        let start = self.position;
        let value = match self.bytes.get(self.position) {
            Some(b'{') => self.object(path)?,
            Some(b'[') => self.array(path)?,
            Some(b'"') => JsonValue::Str(self.string()?),
            Some(b'n') if self.text[self.position..].starts_with("null") => {
                self.position += 4;
                JsonValue::Null
            }
            Some(b't') if self.text[self.position..].starts_with("true") => {
                self.position += 4;
                JsonValue::Bool(true)
            }
            Some(b'f') if self.text[self.position..].starts_with("false") => {
                self.position += 5;
                JsonValue::Bool(false)
            }
            Some(b'N') if self.text[self.position..].starts_with("NaN") => {
                return Err("Unsupported notebook JSON constant: NaN".into());
            }
            Some(b'I') if self.text[self.position..].starts_with("Infinity") => {
                return Err("Unsupported notebook JSON constant: Infinity".into());
            }
            Some(b'-') if self.text[self.position..].starts_with("-Infinity") => {
                return Err("Unsupported notebook JSON constant: -Infinity".into());
            }
            Some(b'-' | b'0'..=b'9') => self.number()?,
            _ => return Err(self.error("Expecting value", self.position)),
        };
        if let [PathPart::Key(cells), PathPart::Index(index), PathPart::Key(source)] = path {
            if cells == "cells" && source == "source" {
                self.sources.push((*index, start, self.position));
            }
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<JsonValue, String> {
        let start = self.position;
        let bytes = self.bytes;
        let mut at = start;
        if bytes.get(at) == Some(&b'-') {
            at += 1;
        }
        match bytes.get(at) {
            Some(b'0') => at += 1,
            Some(b'1'..=b'9') => {
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
            }
            _ => return Err(self.error("Expecting value", start)),
        }
        if bytes.get(at) == Some(&b'.') && bytes.get(at + 1).is_some_and(u8::is_ascii_digit) {
            at += 1;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
        }
        if matches!(bytes.get(at), Some(b'e' | b'E')) {
            let mut exponent = at + 1;
            if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
                exponent += 1;
            }
            if bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
                at = exponent;
                while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
            }
        }
        self.position = at;
        Ok(JsonValue::Number(self.text[start..at].to_string()))
    }

    fn string(&mut self) -> Result<String, String> {
        let start = self.position;
        self.position += 1;
        let mut out = String::new();
        loop {
            let Some(&byte) = self.bytes.get(self.position) else {
                return Err(self.error("Unterminated string starting at", start));
            };
            match byte {
                b'"' => {
                    self.position += 1;
                    return Ok(out);
                }
                b'\\' => {
                    let escape = self.bytes.get(self.position + 1).copied();
                    let replacement = match escape {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            let code = self.hex4(self.position + 2)?;
                            self.position += 6;
                            if (0xd800..0xdc00).contains(&code) && self.text[self.position..].starts_with("\\u") {
                                if let Ok(low) = self.hex4(self.position + 2) {
                                    if (0xdc00..0xe000).contains(&low) {
                                        self.position += 6;
                                        let combined = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                                        out.push(char::from_u32(combined).unwrap_or('\u{fffd}'));
                                        continue;
                                    }
                                }
                            }
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                            continue;
                        }
                        _ => {
                            let shown: String = self.text[self.position + 1..].chars().take(1).collect();
                            return Err(self.error(&format!("Invalid \\escape: {shown:?}").replace('"', "'"), self.position));
                        }
                    };
                    out.push(replacement);
                    self.position += 2;
                }
                0..=0x1f => return Err(self.error("Invalid control character at", self.position)),
                _ => {
                    let rest = &self.text[self.position..];
                    let end = rest.find(['"', '\\']).unwrap_or(rest.len());
                    let chunk = &rest[..end];
                    if let Some(control) = chunk.bytes().position(|byte| byte < 0x20) {
                        return Err(self.error("Invalid control character at", self.position + control));
                    }
                    out.push_str(chunk);
                    self.position += end;
                }
            }
        }
    }

    fn hex4(&self, at: usize) -> Result<u32, String> {
        let digits = self.text.get(at..at + 4).filter(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()));
        match digits {
            Some(digits) => Ok(u32::from_str_radix(digits, 16).unwrap_or(0)),
            None => Err(self.error("Invalid \\uXXXX escape", at - 1)),
        }
    }

    fn array(&mut self, path: &[PathPart]) -> Result<JsonValue, String> {
        self.position += 1;
        self.skip_space();
        let mut items = Vec::new();
        if self.bytes.get(self.position) == Some(&b']') {
            self.position += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            let mut child = path.to_vec();
            child.push(PathPart::Index(items.len()));
            items.push(self.value(&child)?);
            self.skip_space();
            match self.bytes.get(self.position) {
                Some(b',') => {
                    self.position += 1;
                    self.skip_space();
                }
                Some(b']') => {
                    self.position += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err(self.error("Expecting ',' delimiter", self.position)),
            }
        }
    }

    fn object(&mut self, path: &[PathPart]) -> Result<JsonValue, String> {
        self.position += 1;
        self.skip_space();
        let mut members: Vec<(String, JsonValue)> = Vec::new();
        if self.bytes.get(self.position) == Some(&b'}') {
            self.position += 1;
            return Ok(JsonValue::Object(members));
        }
        loop {
            if self.bytes.get(self.position) != Some(&b'"') {
                return Err(self.error("Expecting property name enclosed in double quotes", self.position));
            }
            let key = self.string()?;
            self.skip_space();
            if self.bytes.get(self.position) != Some(&b':') {
                return Err(self.error("Expecting ':' delimiter", self.position));
            }
            self.position += 1;
            self.skip_space();
            let mut child = path.to_vec();
            child.push(PathPart::Key(key.clone()));
            let value = self.value(&child)?;
            members.push((key, value));
            self.skip_space();
            match self.bytes.get(self.position) {
                Some(b',') => {
                    self.position += 1;
                    self.skip_space();
                }
                Some(b'}') => {
                    self.position += 1;
                    let mut keys: Vec<&str> = members.iter().map(|(key, _)| key.as_str()).collect();
                    keys.sort_unstable();
                    keys.dedup();
                    if keys.len() != members.len() {
                        return Err("Notebook contains duplicate JSON keys".into());
                    }
                    return Ok(JsonValue::Object(members));
                }
                _ => return Err(self.error("Expecting ',' delimiter", self.position)),
            }
        }
    }
}

#[derive(Clone, Debug)]
enum PathPart {
    Key(String),
    Index(usize),
}

/// json.loads plus the source-value spans of the cells.
fn parse_json(text: &str) -> Result<(JsonValue, Vec<(usize, usize, usize)>), String> {
    let mut parser = Parser { text, bytes: text.as_bytes(), position: 0, sources: Vec::new() };
    parser.skip_space();
    let value = parser.value(&[])?;
    parser.skip_space();
    if parser.position != text.len() {
        return Err(parser.error("Extra data", parser.position));
    }
    Ok((value, parser.sources))
}

fn member<'v>(object: &'v JsonValue, key: &str) -> Option<&'v JsonValue> {
    match object {
        JsonValue::Object(members) => members.iter().find(|(name, _)| name == key).map(|(_, value)| value),
        _ => None,
    }
}

/// read_cell_source_str.
fn cell_source(cell: &JsonValue, index: usize) -> Result<(String, bool), String> {
    match member(cell, "source") {
        Some(JsonValue::Str(text)) => Ok((text.clone(), false)),
        Some(JsonValue::Array(items)) if items.iter().all(|item| matches!(item, JsonValue::Str(_))) => Ok((
            items.iter().map(|item| if let JsonValue::Str(text) = item { text.as_str() } else { "" }).collect(),
            true,
        )),
        _ => Err(format!("Cell {} source must be a string or string array", index + 1)),
    }
}

/// read_notebook_dict: validate the container; return the cells.
fn read_notebook(text: &str) -> Result<(Vec<JsonValue>, Vec<(usize, usize, usize)>), String> {
    let (notebook, spans) = parse_json(text)?;
    if !matches!(&notebook, JsonValue::Object(_)) || member(&notebook, "nbformat") != Some(&JsonValue::Number("4".into())) {
        return Err("Expected a notebook with nbformat 4".into());
    }
    let metadata = member(&notebook, "metadata");
    if metadata.is_some_and(|metadata| !matches!(metadata, JsonValue::Object(_))) {
        return Err("Notebook metadata must be an object".into());
    }
    for (key, field) in [("language_info", "name"), ("kernelspec", "language")] {
        let entry = metadata.and_then(|metadata| member(metadata, key));
        let language = match entry {
            None => Some("python".to_string()),
            Some(JsonValue::Object(_)) => match member(entry.expect("entry"), field) {
                None => Some("python".to_string()),
                Some(JsonValue::Str(text)) => Some(text.clone()),
                Some(_) => None,
            },
            Some(_) => None,
        };
        if !language.is_some_and(|language| matches!(language.to_lowercase().as_str(), "python" | "python3")) {
            return Err("Only Python notebooks are supported".into());
        }
    }
    let Some(JsonValue::Array(cells)) = member(&notebook, "cells") else {
        return Err("Notebook cells must be an array".into());
    };
    for (index, cell) in cells.iter().enumerate() {
        let kind = member(cell, "cell_type");
        let valid = matches!(kind, Some(JsonValue::Str(text)) if matches!(text.as_str(), "code" | "markdown" | "raw"));
        if !matches!(cell, JsonValue::Object(_)) || !valid {
            return Err(format!("Invalid notebook cell {}", index + 1));
        }
        cell_source(cell, index)?;
    }
    Ok((cells.clone(), spans))
}

/// json.dumps(text, ensure_ascii=False).
fn dump_string(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            control if (control as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", control as u32)),
            other => out.push(other),
        }
    }
    out.push('"');
}

/// re.findall(r"[^\r\n]*(?:\r\n|\r|\n|$)", text)[:-1].
fn source_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let end = match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => index + 2,
            b'\r' | b'\n' => index + 1,
            _ => {
                index += 1;
                continue;
            }
        };
        lines.push(&text[start..end]);
        start = end;
        index = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// The label of a cell in messages: "path#cell=N".
fn cell_label(path: &str, index: usize) -> String {
    format!("{path}#cell={}", index + 1)
}

/// format_notebook_str: the notebook with formatted code cells. Errors
/// are messages without the outer "Cannot format PATH: " prefix.
pub fn format_notebook(text: &str, path: &str, width: Option<usize>, hug: bool) -> Result<String, String> {
    let (cells, spans) = read_notebook(text)?;
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (index, cell) in cells.iter().enumerate() {
        if member(cell, "cell_type") != Some(&JsonValue::Str("code".into())) {
            continue;
        }
        let (source, is_list) = cell_source(cell, index)?;
        let label = cell_label(path, index);
        let output = format_source(&source, width, hug).map_err(|error| {
            let detail = match error {
                FormatError::Syntax { line, message } => crate::lint::syntax_error_text(&label, line, &message),
                FormatError::Value(message) => message,
            };
            format!("Cannot format {label}: {detail}")
        })?;
        if output == source {
            continue;
        }
        let mut replacement = String::new();
        if is_list {
            replacement.push('[');
            for (line_index, line) in source_lines(&output).iter().enumerate() {
                if line_index > 0 {
                    replacement.push_str(", ");
                }
                dump_string(line, &mut replacement);
            }
            replacement.push(']');
        } else {
            dump_string(&output, &mut replacement);
        }
        let Some(&(_, start, end)) = spans.iter().find(|(cell_index, _, _)| *cell_index == index) else { continue };
        edits.push((start, end, replacement));
    }
    let mut out = text.to_string();
    for (start, end, replacement) in edits.into_iter().rev() {
        out.replace_range(start..end, &replacement);
    }
    Ok(out)
}
