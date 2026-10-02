//! Settings from the nearest pyproject.toml's [tool.refactrail] table and
//! the command line, validated as src/refactrail/config.py does.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const DEFAULT_EXCLUDE: [&str; 14] = [
    ".git", ".hg", ".venv", "venv", "__pycache__", ".tox", ".nox", ".mypy_cache", ".pytest_cache", ".ruff_cache",
    "build", "dist", "node_modules", ".refactrail_cache",
];
const KNOWN: [&str; 8] =
    ["profile", "line_length", "function_preferred_lines", "function_max_lines", "main_max_lines", "select", "ignore", "exclude"];

/// A TOML value of the kinds the settings use.
#[derive(Clone, Debug, PartialEq)]
pub enum Toml {
    Str(String),
    Int(i64),
    Bool(bool),
    Float,
    Array(Vec<Toml>),
    Table,
}

/// Run-time settings (the engine's plus discovery's exclusions).
pub struct Config {
    pub engine: refactrail_engine::Settings,
    pub exclude: Vec<String>,
}

/// Command-line values; None means "not given".
#[derive(Default)]
pub struct Overrides {
    pub profile: Option<String>,
    pub line_length: Option<i64>,
    pub select: Option<Vec<String>>,
    pub ignore: Option<Vec<String>>,
}

/// find_pyproject_path: nearest pyproject.toml at or above the start.
pub fn find_pyproject(start: &Path) -> Option<PathBuf> {
    let mut folder = std::fs::canonicalize(start).unwrap_or_else(|_| std::path::absolute(start).unwrap_or(start.to_path_buf()));
    if folder.is_file() {
        folder = folder.parent()?.to_path_buf();
    }
    folder.ancestors().map(|candidate| candidate.join("pyproject.toml")).find(|candidate| candidate.is_file())
}

/// Build validated settings (build_settings in config.py).
pub fn load(start: &Path, overrides: Overrides) -> Result<Config, String> {
    let mut table = match find_pyproject(start) {
        Some(path) => {
            let text = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            read_tool_table(&text).map_err(|error| format!("{}: invalid TOML: {error}", path.display()))?
        }
        None => BTreeMap::new(),
    };
    if let Some(profile) = overrides.profile {
        table.insert("profile".into(), Toml::Str(profile));
    }
    if let Some(length) = overrides.line_length {
        table.insert("line_length".into(), Toml::Int(length));
    }
    if let Some(select) = overrides.select {
        table.insert("select".into(), Toml::Array(select.into_iter().map(Toml::Str).collect()));
    }
    if let Some(ignore) = overrides.ignore {
        table.insert("ignore".into(), Toml::Array(ignore.into_iter().map(Toml::Str).collect()));
    }
    let unknown: Vec<&str> = table.keys().map(String::as_str).filter(|key| !KNOWN.contains(key)).collect();
    if !unknown.is_empty() {
        return Err(format!("Unknown setting(s): {}", unknown.join(", ")));
    }
    let profile = match table.get("profile") {
        None => "standard".to_string(),
        Some(Toml::Str(text)) if text == "standard" || text == "strict" => text.clone(),
        Some(_) => return Err("Profile must be one of ('standard', 'strict')".into()),
    };
    let integer = |key: &str, default: i64| -> Result<i64, String> {
        match table.get(key) {
            None => Ok(default),
            Some(Toml::Int(value)) if *value >= 1 => Ok(*value),
            Some(_) => Err(format!("{key} must be a positive integer")),
        }
    };
    let line_length = integer("line_length", 79)?;
    let preferred = integer("function_preferred_lines", 40)?;
    let maximum = integer("function_max_lines", 50)?;
    let main_max = integer("main_max_lines", 100)?;
    if !(40..=200).contains(&line_length) {
        return Err("line_length must be from 40 to 200".into());
    }
    if preferred > maximum {
        return Err("Preferred function size exceeds maximum".into());
    }
    let list = |key: &str, default: &[&str]| -> Result<Vec<String>, String> {
        match table.get(key) {
            None => Ok(default.iter().map(|text| text.to_string()).collect()),
            Some(Toml::Array(items)) => items
                .iter()
                .map(|item| match item {
                    Toml::Str(text) if !text.trim().is_empty() => Ok(text.clone()),
                    _ => Err(format!("{key} must be a list of strings")),
                })
                .collect(),
            Some(_) => Err(format!("{key} must be a list of strings")),
        }
    };
    Ok(Config {
        engine: refactrail_engine::Settings {
            profile,
            line_length: line_length as usize,
            function_preferred_lines: preferred as usize,
            function_max_lines: maximum as usize,
            main_max_lines: main_max as usize,
            select: list("select", &["RT"])?,
            ignore: list("ignore", &[])?,
        },
        exclude: list("exclude", &DEFAULT_EXCLUDE)?,
    })
}

// ----- a small TOML reader -------------------------------------------------

struct Reader<'a> {
    chars: Vec<char>,
    index: usize,
    _text: &'a str,
}

impl Reader<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.index += 1;
        }
    }

    /// Whitespace, newlines and comments (inside arrays).
    fn skip_blank(&mut self) {
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\n' | '\r') => self.index += 1,
                Some('#') => {
                    while !matches!(self.peek(), None | Some('\n')) {
                        self.index += 1;
                    }
                }
                _ => return,
            }
        }
    }

    fn skip_comment_and_newline(&mut self) -> Result<(), String> {
        self.skip_space();
        if self.peek() == Some('#') {
            while !matches!(self.peek(), None | Some('\n')) {
                self.index += 1;
            }
        }
        if self.peek() == Some('\r') {
            self.index += 1;
        }
        match self.peek() {
            None => Ok(()),
            Some('\n') => {
                self.index += 1;
                Ok(())
            }
            Some(other) => Err(format!("unexpected character {other:?}")),
        }
    }

    fn key_part(&mut self) -> Result<String, String> {
        self.skip_space();
        match self.peek() {
            Some('"' | '\'') => match self.value()? {
                Toml::Str(text) => Ok(text),
                _ => Err("invalid key".into()),
            },
            _ => {
                let start = self.index;
                while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                    self.index += 1;
                }
                if start == self.index {
                    return Err("expected a key".into());
                }
                Ok(self.chars[start..self.index].iter().collect())
            }
        }
    }

    fn dotted_key(&mut self) -> Result<Vec<String>, String> {
        let mut parts = vec![self.key_part()?];
        loop {
            self.skip_space();
            if self.peek() != Some('.') {
                return Ok(parts);
            }
            self.index += 1;
            parts.push(self.key_part()?);
        }
    }

    fn string(&mut self) -> Result<String, String> {
        let quote = self.peek().ok_or("expected a string")?;
        let triple = self.chars.get(self.index..self.index + 3) == Some(&[quote, quote, quote][..]);
        self.index += if triple { 3 } else { 1 };
        if triple && self.peek() == Some('\n') {
            self.index += 1;
        }
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or("unterminated string")?;
            if triple && self.chars.get(self.index..self.index + 3) == Some(&[quote, quote, quote][..]) {
                self.index += 3;
                return Ok(out);
            }
            if !triple && c == quote {
                self.index += 1;
                return Ok(out);
            }
            if !triple && c == '\n' {
                return Err("unterminated string".into());
            }
            self.index += 1;
            if c == '\\' && quote == '"' {
                let escaped = self.peek().ok_or("unterminated string")?;
                self.index += 1;
                match escaped {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    'u' | 'U' => {
                        let count = if escaped == 'u' { 4 } else { 8 };
                        let hex: String = self.chars.iter().skip(self.index).take(count).collect();
                        let point = u32::from_str_radix(&hex, 16).map_err(|_| "invalid escape")?;
                        out.push(char::from_u32(point).ok_or("invalid escape")?);
                        self.index += count;
                    }
                    '\n' if triple => self.skip_blank(),
                    _ => return Err("invalid escape".into()),
                }
            } else {
                out.push(c);
            }
        }
    }

    fn value(&mut self) -> Result<Toml, String> {
        self.skip_space();
        match self.peek() {
            Some('"' | '\'') => self.string().map(Toml::Str),
            Some('[') => {
                self.index += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_blank();
                    if self.peek() == Some(']') {
                        self.index += 1;
                        return Ok(Toml::Array(items));
                    }
                    items.push(self.value()?);
                    self.skip_blank();
                    match self.peek() {
                        Some(',') => self.index += 1,
                        Some(']') => {}
                        _ => return Err("expected ',' or ']'".into()),
                    }
                }
            }
            Some('{') => {
                let mut depth = 0;
                while let Some(c) = self.peek() {
                    self.index += 1;
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                return Ok(Toml::Table);
                            }
                        }
                        '"' | '\'' => {
                            self.index -= 1;
                            self.string()?;
                        }
                        _ => {}
                    }
                }
                Err("unterminated inline table".into())
            }
            _ => {
                let start = self.index;
                while matches!(self.peek(), Some(c) if !matches!(c, ',' | ']' | '}' | '#' | '\n' | '\r' | ' ' | '\t')) {
                    self.index += 1;
                }
                let word: String = self.chars[start..self.index].iter().collect::<String>().replace('_', "");
                match word.as_str() {
                    "true" => Ok(Toml::Bool(true)),
                    "false" => Ok(Toml::Bool(false)),
                    _ if word.parse::<i64>().is_ok() => Ok(Toml::Int(word.parse().unwrap_or(0))),
                    _ if word.parse::<f64>().is_ok() || word.contains(['-', ':']) => Ok(Toml::Float),
                    _ => Err(format!("invalid value {word:?}")),
                }
            }
        }
    }
}

/// The [tool.refactrail] table (dashes in keys become underscores).
pub fn read_tool_table(text: &str) -> Result<BTreeMap<String, Toml>, String> {
    let mut reader = Reader { chars: text.chars().collect(), index: 0, _text: text };
    let mut table: Vec<String> = Vec::new();
    let mut result = BTreeMap::new();
    loop {
        reader.skip_blank();
        let Some(c) = reader.peek() else { return Ok(result) };
        if c == '[' {
            reader.index += 1;
            let array = reader.peek() == Some('[');
            if array {
                reader.index += 1;
            }
            table = reader.dotted_key()?;
            reader.skip_space();
            for _ in 0..(1 + usize::from(array)) {
                if reader.peek() != Some(']') {
                    return Err("expected ']'".into());
                }
                reader.index += 1;
            }
            if array {
                table.push("[]".into());
            }
            reader.skip_comment_and_newline()?;
            continue;
        }
        let key = reader.dotted_key()?;
        reader.skip_space();
        if reader.peek() != Some('=') {
            return Err("expected '='".into());
        }
        reader.index += 1;
        let value = reader.value()?;
        reader.skip_comment_and_newline()?;
        let mut full: Vec<String> = table.clone();
        full.extend(key);
        if full.len() >= 3 && full[0] == "tool" && full[1] == "refactrail" {
            if full.len() == 3 {
                result.insert(full[2].replace('-', "_"), value);
            }
        } else if full.len() == 2 && full[0] == "tool" && full[1] == "refactrail" {
            return Err("tool.refactrail must be a table".into());
        }
    }
}
