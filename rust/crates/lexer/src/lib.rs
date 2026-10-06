//! RefacTrail's own Python tokenizer.
//!
//! Produces the same token stream as CPython's `tokenize` module (Python
//! 3.12 grammar, PEP 701 f-strings), including its positions: 1-based
//! rows and code-point columns. Written from the language reference and
//! CPython's observable behaviour; no third-party parser code is used.

mod identifier_tables_312;
mod identifier_tables_313;
mod identifier_tables_314;

/// The Python version whose tokenizer is mirrored. Grammar and Unicode
/// data differ between versions (t-strings and prefix checks in 3.14,
/// format-spec newlines in 3.13, identifier characters in each).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Version {
    Py312,
    Py313,
    Py314,
}

/// The process's configured version (minor number), 3.12 until set.
static CONFIGURED_MINOR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(12);

/// `Version::default()` is the configured version: the running Python's
/// in the PyO3 module, the `--python-version` choice in the native CLI.
impl Default for Version {
    fn default() -> Version {
        Version::from_minor(CONFIGURED_MINOR.load(std::sync::atomic::Ordering::Relaxed))
    }
}

impl Version {
    /// "3.12", "3.13" or "3.14".
    pub fn parse(text: &str) -> Option<Version> {
        match text.trim() {
            "3.12" => Some(Version::Py312),
            "3.13" => Some(Version::Py313),
            "3.14" => Some(Version::Py314),
            _ => None,
        }
    }

    /// The minor version number (12, 13 or 14).
    pub fn minor(self) -> u32 {
        match self {
            Version::Py312 => 12,
            Version::Py313 => 13,
            Version::Py314 => 14,
        }
    }

    /// The newest supported version.
    pub const LATEST: Version = Version::Py314;

    /// The supported version for a Python 3 minor number: older ones use
    /// 3.12's grammar, newer ones the newest supported grammar.
    pub fn from_minor(minor: u32) -> Version {
        match minor {
            0..=12 => Version::Py312,
            13 => Version::Py313,
            _ => Version::Py314,
        }
    }

    /// Make `version` the default for this process (set once at start-up).
    pub fn configure(version: Version) {
        CONFIGURED_MINOR.store(version.minor(), std::sync::atomic::Ordering::Relaxed);
    }
}

struct IdentifierTables {
    start: &'static [(u32, u32)],
    cont: &'static [(u32, u32)],
    nonprintable: &'static [(u32, u32)],
}

fn identifier_tables(version: Version) -> IdentifierTables {
    match version {
        Version::Py312 => IdentifierTables {
            start: &identifier_tables_312::XID_START,
            cont: &identifier_tables_312::XID_CONTINUE,
            nonprintable: &identifier_tables_312::NONPRINTABLE,
        },
        Version::Py313 => IdentifierTables {
            start: &identifier_tables_313::XID_START,
            cont: &identifier_tables_313::XID_CONTINUE,
            nonprintable: &identifier_tables_313::NONPRINTABLE,
        },
        Version::Py314 => IdentifierTables {
            start: &identifier_tables_314::XID_START,
            cont: &identifier_tables_314::XID_CONTINUE,
            nonprintable: &identifier_tables_314::NONPRINTABLE,
        },
    }
}

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Name,
    Number,
    String,
    Op,
    Newline,
    Nl,
    Comment,
    Indent,
    Dedent,
    FStringStart,
    FStringMiddle,
    FStringEnd,
    TStringStart,
    TStringMiddle,
    TStringEnd,
    EndMarker,
}

impl Kind {
    /// The name CPython's `tokenize.tok_name` uses.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Name => "NAME",
            Kind::Number => "NUMBER",
            Kind::String => "STRING",
            Kind::Op => "OP",
            Kind::Newline => "NEWLINE",
            Kind::Nl => "NL",
            Kind::Comment => "COMMENT",
            Kind::Indent => "INDENT",
            Kind::Dedent => "DEDENT",
            Kind::FStringStart => "FSTRING_START",
            Kind::FStringMiddle => "FSTRING_MIDDLE",
            Kind::FStringEnd => "FSTRING_END",
            Kind::TStringStart => "TSTRING_START",
            Kind::TStringMiddle => "TSTRING_MIDDLE",
            Kind::TStringEnd => "TSTRING_END",
            Kind::EndMarker => "ENDMARKER",
        }
    }
}

/// A (row, column) position: 1-based row, code-point column.
pub type Position = (u32, u32);

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: Kind,
    /// Byte range in the source (empty for synthetic tokens).
    pub start: usize,
    pub end: usize,
    /// Token text when it is not the source slice (f-string middles with
    /// doubled braces, synthetic empty tokens).
    pub text: Option<String>,
    pub start_pos: Position,
    pub end_pos: Position,
}

impl Token {
    pub fn text<'a>(&'a self, source: &'a str) -> &'a str {
        self.text.as_deref().unwrap_or(&source[self.start..self.end])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LexError {
    pub line: u32,
    /// CPython's SyntaxError.offset: 1-based, in characters.
    pub offset: u32,
    pub message: String,
}

impl fmt::Display for LexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "line {}: {}", self.line, self.message)
    }
}

/// The state of one open f-string, mirroring CPython's tokenizer mode so
/// that malformed fields fail (or recover) exactly where CPython's do.
#[derive(Clone, Copy, Debug)]
struct FString {
    quote: u8,
    triple: bool,
    raw: bool,
    start_line: u32,
    /// Byte offsets of the f-string's start and of its line's start.
    start_offset: usize,
    start_line_start: usize,
    /// Brackets opened minus brackets closed inside this f-string's
    /// expressions, of any kind (CPython's `curly_bracket_depth`).
    depth: i32,
    /// The depth at which the current replacement field started, or -1
    /// outside any field (CPython's `curly_bracket_expr_start_depth`).
    expr_start: i32,
    /// Lexing an expression rather than literal text or a format spec.
    in_expression: bool,
    /// A top-level `:`, `!` or `}` ended the field's expression
    /// (CPython's `last_expr_end != -1`).
    expr_ended: bool,
    /// A t-string (3.14): TSTRING tokens and "t-string:" messages.
    template: bool,
}

impl FString {
    fn prefix(&self) -> char {
        if self.template { 't' } else { 'f' }
    }
}

/// Length of the three- or two-character operator starting with these
/// bytes (`**=` `//=` `>>=` `<<=` `...`; `**` `//` `<<` `>>` `<=` `>=`
/// `==` `!=` `->` `+=` `-=` `*=` `/=` `%=` `&=` `|=` `^=` `@=` `:=` `<>`),
/// else 1.
fn operator_length(first: u8, second: u8, third: u8) -> usize {
    match (first, second) {
        (b'*', b'*') | (b'/', b'/') | (b'<', b'<') | (b'>', b'>') => {
            if third == b'=' { 3 } else { 2 }
        }
        (b'.', b'.') if third == b'.' => 3,
        (b'<' | b'>' | b'=' | b'!' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'@' | b':', b'=')
        | (b'-', b'>')
        | (b'<', b'>') => 2,
        _ => 1,
    }
}
const ONE_CHAR_OPS: &[u8] = b"+-*/%@&|^~<>()[]{},:;.=!";

/// Tokenize Python source text (already decoded, without a BOM), as
/// CPython's `tokenize` module does.
pub fn tokenize(source: &str) -> Result<Vec<Token>, LexError> {
    tokenize_version(source, Version::default())
}

/// `tokenize` for a given Python version.
pub fn tokenize_version(source: &str, version: Version) -> Result<Vec<Token>, LexError> {
    let (tokens, error) = tokenize_into_version(source, false, version, Vec::with_capacity(source.len() / 6 + 16));
    match error {
        Some(error) => Err(error),
        None => Ok(tokens),
    }
}

/// Tokenize for the parser: the tokens produced before any error, and the
/// error. With `check_brackets`, brackets are checked as CPython's parser
/// tokenizer does ("unmatched ')'", "'(' was never closed", ...).
pub fn tokenize_partial(source: &str, check_brackets: bool) -> (Vec<Token>, Option<LexError>) {
    tokenize_into(source, check_brackets, Vec::with_capacity(source.len() / 6 + 16))
}

/// Receives each token as the lexer produces it.
pub trait TokenSink {
    fn push(&mut self, token: Token);
}

impl TokenSink for Vec<Token> {
    #[inline]
    fn push(&mut self, token: Token) {
        Vec::push(self, token);
    }
}

/// Tokenize into a sink (see `tokenize_partial`), returning the sink and
/// the error that stopped tokenizing, if any.
pub fn tokenize_into<S: TokenSink>(source: &str, check_brackets: bool, sink: S) -> (S, Option<LexError>) {
    tokenize_into_version(source, check_brackets, Version::default(), sink)
}

/// `tokenize_into` for a given Python version.
pub fn tokenize_into_version<S: TokenSink>(
    source: &str,
    check_brackets: bool,
    version: Version,
    sink: S,
) -> (S, Option<LexError>) {
    let mut lexer = Lexer {
        version,
        check_brackets,
        byte_columns: check_brackets,
        src: source,
        b: source.as_bytes(),
        i: 0,
        line: 1,
        line_start: 0,
        tokens: sink,
        indents: vec![0],
        alt_indents: vec![0],
        brackets: Vec::new(),
        modes: Vec::new(),
        at_line_start: true,
        line_has_tokens: false,
        column_cache: std::cell::Cell::new((0, 0)),
    };
    let error = lexer.run().err();
    (lexer.tokens, error)
}

struct Lexer<'a, S> {
    version: Version,
    check_brackets: bool,
    /// Report byte columns (the parser's mode) instead of tokenize's
    /// character columns.
    byte_columns: bool,
    src: &'a str,
    b: &'a [u8],
    i: usize,
    line: u32,
    line_start: usize,
    tokens: S,
    indents: Vec<u32>,
    /// Indentation with tabs counted as one column, for TabError checks.
    alt_indents: Vec<u32>,
    /// Open brackets: (bracket, line, its line's start, byte offset).
    brackets: Vec<(u8, u32, usize, usize)>,
    modes: Vec<FString>,
    at_line_start: bool,
    line_has_tokens: bool,
    /// (byte position, column) of the last here() call.
    column_cache: std::cell::Cell<(usize, u32)>,
}

fn is_id_start(c: char) -> bool {
    // CPython's tokenizer takes any non-ASCII character as part of a name,
    // even a space such as U+00A0; in parser mode `verify_identifier`
    // then rejects names that are not identifiers.
    c == '_' || c.is_ascii_alphabetic() || !c.is_ascii()
}

fn is_id_continue(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric() || !c.is_ascii()
}

fn in_ranges(code: u32, ranges: &[(u32, u32)]) -> bool {
    ranges
        .binary_search_by(|&(start, end)| {
            if end < code {
                std::cmp::Ordering::Less
            } else if start > code {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// The index (in characters) and value of the first character that keeps
/// a name from being an identifier, as `_PyUnicode_ScanIdentifier` finds.
fn first_invalid_identifier_char(name: &str, tables: &IdentifierTables) -> Option<(usize, char)> {
    name.chars().enumerate().find(|&(index, c)| {
        if c.is_ascii() {
            !(c == '_' || c.is_ascii_alphabetic() || (index > 0 && c.is_ascii_digit()))
        } else if index == 0 {
            !in_ranges(c as u32, tables.start)
        } else {
            !in_ranges(c as u32, tables.cont)
        }
    })
}

/// `str.isidentifier()` under the Unicode tables of `version`'s CPython.
pub fn is_identifier(name: &str, version: Version) -> bool {
    !name.is_empty() && first_invalid_identifier_char(name, &identifier_tables(version)).is_none()
}

fn expand_bare_cr(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '\r' && chars.peek() != Some(&'\n') {
            out.push('\n');
        }
    }
    out
}

/// The kind of string a prefix starts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StringKind {
    Plain,
    Format,
    Template,
}

/// Python 3.14's prefix scan: each of b, u, r, f and t (any case) at most
/// once. Returns (saw_b, saw_r, saw_u, saw_f, saw_t), or None when the word
/// is not entirely such a prefix.
fn prefix_letters(word: &str) -> Option<[bool; 5]> {
    let mut seen = [false; 5];
    for byte in word.bytes() {
        let index = match byte.to_ascii_lowercase() {
            b'b' => 0,
            b'r' => 1,
            b'u' => 2,
            b'f' => 3,
            b't' => 4,
            _ => return None,
        };
        if seen[index] {
            return None;
        }
        seen[index] = true;
    }
    Some(seen)
}

/// Python 3.14's error for an incompatible prefix pair, in CPython's order.
fn incompatible_prefixes(seen: [bool; 5]) -> Option<(char, char)> {
    let [b, r, u, f, t] = seen;
    [(u && b, 'u', 'b'), (u && r, 'u', 'r'), (u && f, 'u', 'f'), (u && t, 'u', 't'), (b && f, 'b', 'f'), (b && t, 'b', 't'), (f && t, 'f', 't')]
        .into_iter()
        .find(|(clash, _, _)| *clash)
        .map(|(_, first, second)| (first, second))
}

fn string_prefix(word: &str) -> Option<(bool, bool)> {
    // (is f-string, is raw) for a valid prefix, compared case-insensitively.
    let lower = word.to_ascii_lowercase();
    match lower.as_str() {
        "" | "u" | "b" => Some((false, false)),
        "r" | "br" | "rb" => Some((false, true)),
        "f" => Some((true, false)),
        "fr" | "rf" => Some((true, true)),
        _ => None,
    }
}

impl<'a, S: TokenSink> Lexer<'a, S> {
    /// Error at the cursor: CPython's offset counts the characters up to
    /// and including the one being read.
    fn error<T>(&self, message: &str) -> Result<T, LexError> {
        self.error_offset(self.line, self.cursor_col() + 1, message)
    }

    fn error_offset<T>(&self, line: u32, offset: u32, message: &str) -> Result<T, LexError> {
        Err(LexError { line, offset, message: message.to_string() })
    }

    /// Characters from a line's start (byte offset) to a byte offset.
    fn char_column(&self, line_start: usize, at: usize) -> u32 {
        let segment = &self.src[line_start.min(at)..at];
        if segment.is_ascii() { segment.len() as u32 } else { segment.chars().count() as u32 }
    }

    /// Characters from the start of the line to the cursor.
    fn cursor_col(&self) -> u32 {
        let start = self.line_start.min(self.i);
        let segment = &self.src[start..self.i];
        if segment.is_ascii() { segment.len() as u32 } else { segment.chars().count() as u32 }
    }

    fn error_at<T>(&self, line: u32, message: &str) -> Result<T, LexError> {
        Err(LexError { line, offset: self.cursor_col() + 1, message: message.to_string() })
    }

    /// The row of the last character, as CPython reports EOF errors.
    fn last_content_line(&self) -> u32 {
        if self.src.ends_with('\n') || self.src.ends_with('\r') {
            self.line.saturating_sub(1).max(1)
        } else {
            self.line
        }
    }

    /// The cursor's (row, column); columns are counted incrementally from
    /// the previous call on the same line, so long lines stay linear.
    fn here(&self) -> Position {
        if self.byte_columns {
            return (self.line, (self.i - self.line_start) as u32);
        }
        let (cached_at, cached_column) = self.column_cache.get();
        let (from, base) =
            if cached_at >= self.line_start && cached_at <= self.i { (cached_at, cached_column) } else { (self.line_start, 0) };
        let segment = &self.src[from..self.i];
        let column = base + if segment.is_ascii() { segment.len() } else { segment.chars().count() } as u32;
        self.column_cache.set((self.i, column));
        (self.line, column)
    }

    fn peek(&self, offset: usize) -> u8 {
        *self.b.get(self.i + offset).unwrap_or(&0)
    }

    fn at_end(&self) -> bool {
        self.i >= self.b.len()
    }

    fn newline_len(&self, at: usize) -> usize {
        match self.b.get(at) {
            Some(b'\n') => 1,
            Some(b'\r') => {
                if self.b.get(at + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                }
            }
            _ => 0,
        }
    }

    fn current_char(&self) -> char {
        self.src[self.i..].chars().next().unwrap_or('\0')
    }

    /// Advance over one character, keeping row tracking for line breaks.
    fn advance_char(&mut self) {
        let newline = self.newline_len(self.i);
        if newline > 0 {
            self.i += newline;
            self.line += 1;
            self.line_start = self.i;
        } else {
            self.i += self.current_char().len_utf8().max(1);
        }
    }

    fn push(&mut self, kind: Kind, start: usize, start_pos: Position, text: Option<String>) {
        let end_pos = self.here();
        if !matches!(kind, Kind::Comment | Kind::Nl | Kind::Newline | Kind::Indent | Kind::Dedent) {
            self.line_has_tokens = true;
        }
        self.tokens.push(Token { kind, start, end: self.i, text, start_pos, end_pos });
    }

    fn push_synthetic(&mut self, kind: Kind, text: &str, start_pos: Position, end_pos: Position) {
        self.tokens.push(Token {
            kind,
            start: self.i,
            end: self.i,
            text: Some(text.to_string()),
            start_pos,
            end_pos,
        });
    }

    fn run(&mut self) -> Result<(), LexError> {
        loop {
            match self.modes.last().copied() {
                Some(fstring) if !fstring.in_expression => self.lex_fstring_middle(fstring)?,
                _ => {
                    if !self.lex_normal()? {
                        break;
                    }
                }
            }
        }
        self.finish()
    }

    fn finish(&mut self) -> Result<(), LexError> {
        if let Some(&(bracket, line, line_start, at)) = self.brackets.last() {
            let column = self.char_column(line_start, at);
            if self.check_brackets {
                return self.error_offset(line, column + 1, &format!("'{}' was never closed", bracket as char));
            }
            return self.error_at(self.last_content_line(), "unexpected EOF in multi-line statement");
        }
        let (row, column) = self.here();
        let ends_with_newline = self.src.ends_with('\n') || self.src.ends_with('\r');
        if self.line_has_tokens {
            self.push_synthetic(Kind::Newline, "", (row, column), (row, column + 1));
        } else if !ends_with_newline && !self.src.is_empty() {
            // A last line holding only whitespace or a comment.
            self.push_synthetic(Kind::Nl, "", (row, column), (row, column + 1));
        }
        let end_row = if self.src.is_empty() {
            1
        } else if ends_with_newline {
            self.line
        } else {
            self.line + 1
        };
        while self.indents.len() > 1 {
            self.indents.pop();
            self.alt_indents.pop();
            self.push_synthetic(Kind::Dedent, "", (end_row, 0), (end_row, 0));
        }
        self.push_synthetic(Kind::EndMarker, "", (end_row, 0), (end_row, 0));
        Ok(())
    }

    /// Handle indentation at the start of a logical line.
    fn indentation(&mut self) -> Result<(), LexError> {
        let mut column: u32 = 0;
        let mut alt_column: u32 = 0;
        // As in CPython, the column of the first backslash continuation
        // (when nonzero) sets the indentation; otherwise whitespace keeps
        // counting across the continued physical lines.
        let mut continuation_column: u32 = 0;
        while !self.at_end() {
            match self.b[self.i] {
                b'\\' => {
                    let after = self.newline_len(self.i + 1);
                    if after == 0 && self.check_brackets {
                        // CPython reads continuations while measuring the
                        // indentation, before checking it.
                        let backslash = self.cursor_col();
                        let message = if self.i + 1 >= self.b.len() {
                            "unexpected EOF while parsing"
                        } else {
                            "unexpected character after line continuation character"
                        };
                        return self.error_offset(self.line, backslash + 2, message);
                    }
                    if after == 0 {
                        break; // reported by lex_normal
                    }
                    if continuation_column == 0 {
                        continuation_column = column;
                    }
                    let backslash = self.cursor_col();
                    self.i += 1 + after;
                    self.line += 1;
                    self.line_start = self.i;
                    if self.at_end() {
                        return self.error_offset(self.last_content_line(), backslash + 2, "unexpected EOF while parsing");
                    }
                    continue;
                }
                b' ' => {
                    column += 1;
                    alt_column += 1;
                }
                b'\t' => {
                    column = (column / 8 + 1) * 8;
                    alt_column += 1;
                }
                0x0c => {
                    column = 0;
                    alt_column = 0;
                }
                _ => break,
            }
            self.i += 1;
        }
        if self.at_end() || self.b[self.i] == b'#' || self.newline_len(self.i) > 0 {
            return Ok(()); // blank or comment-only line: no INDENT/DEDENT
        }
        if continuation_column != 0 {
            column = continuation_column;
            alt_column = continuation_column;
        }
        let start = self.line_start;
        let width = self.here().1;
        let current = *self.indents.last().unwrap_or(&0);
        let alt_current = *self.alt_indents.last().unwrap_or(&0);
        if column == current && alt_column != alt_current {
            return self.error_offset(self.line, 1, "inconsistent use of tabs and spaces in indentation");
        }
        if column > current {
            if alt_column <= alt_current {
                return self.error_offset(self.line, 1, "inconsistent use of tabs and spaces in indentation");
            }
            self.indents.push(column);
            self.alt_indents.push(alt_column);
            self.tokens.push(Token {
                kind: Kind::Indent,
                start,
                end: self.i,
                text: None,
                start_pos: (self.line, 0),
                end_pos: (self.line, width),
            });
        } else if column < current {
            // CPython checks the target level before emitting any DEDENT.
            let keep = self.indents.iter().rposition(|&level| level <= column).unwrap_or(0);
            if column != self.indents[keep] {
                // Its cursor is then at the end of the line.
                let line_end = self.src[self.i..].find('\n').map_or(self.src.len(), |at| self.i + at + 1);
                let offset = self.src[self.line_start..line_end].chars().count() as u32;
                return self.error_offset(self.line, offset, "unindent does not match any outer indentation level");
            }
            if alt_column != self.alt_indents[keep] {
                return self.error_offset(self.line, 1, "inconsistent use of tabs and spaces in indentation");
            }
            while self.indents.len() > keep + 1 {
                self.indents.pop();
                self.alt_indents.pop();
                let position = (self.line, width);
                self.push_synthetic(Kind::Dedent, "", position, position);
            }
        }
        Ok(())
    }

    /// Lex one token in normal (or f-string replacement field) mode.
    /// Returns false at the end of input.
    fn lex_normal(&mut self) -> Result<bool, LexError> {
        if self.at_line_start {
            self.at_line_start = false;
            // As in CPython, stray closers inside an f-string expression
            // can bring the bracket level back to 0, and lines are then
            // indented normally.
            if self.brackets.is_empty() {
                self.indentation()?;
            }
        }
        while !self.at_end() && matches!(self.b[self.i], b' ' | b'\t' | 0x0c) {
            self.i += 1;
        }
        if self.at_end() {
            // Open brackets are reported by finish(); with none, CPython
            // ends normally even inside an f-string expression.
            return Ok(false);
        }
        let start = self.i;
        let start_pos = self.here();
        let byte = self.b[self.i];
        let newline = self.newline_len(self.i);
        if newline > 0 {
            let kind = if !self.brackets.is_empty() || !self.line_has_tokens {
                Kind::Nl
            } else {
                Kind::Newline
            };
            self.i += newline;
            // CPython reports a bare "\r" line end as two columns wide, and
            // an NL token ending that way with empty text.
            let bare_cr = byte == b'\r' && newline == 1;
            let width = if newline == 2 || bare_cr { 2 } else { 1 };
            let end_pos = (start_pos.0, start_pos.1 + width);
            let text = (bare_cr && kind == Kind::Nl).then(String::new);
            self.tokens.push(Token { kind, start, end: self.i, text, start_pos, end_pos });
            self.line += 1;
            self.line_start = self.i;
            if self.brackets.is_empty() {
                self.at_line_start = true;
                self.line_has_tokens = false;
            }
            return Ok(true);
        }
        match byte {
            b'#' => {
                self.i = self.b[self.i..].iter().position(|&byte| byte == b'\n' || byte == b'\r').map_or(self.b.len(), |at| self.i + at);
                self.push(Kind::Comment, start, start_pos, None);
            }
            b'\\' => {
                let after = self.newline_len(self.i + 1);
                let backslash = self.cursor_col();
                if after == 0 && self.i + 1 >= self.b.len() {
                    return self.error_offset(self.line, backslash + 2, "unexpected EOF while parsing");
                }
                if after == 0 {
                    return self.error_offset(self.line, backslash + 2, "unexpected character after line continuation character");
                }
                self.i += 1 + after;
                self.line += 1;
                self.line_start = self.i;
                if self.at_end() {
                    return self.error_offset(self.last_content_line(), backslash + 2, "unexpected EOF while parsing");
                }
            }
            b'0'..=b'9' => self.lex_number(start, start_pos)?,
            b'.' if self.peek(1).is_ascii_digit() => self.lex_number(start, start_pos)?,
            b'\'' | b'"' => self.lex_string(start, start_pos, StringKind::Plain, false)?,
            _ => {
                let is_name = if byte.is_ascii() { byte == b'_' || byte.is_ascii_alphabetic() } else { is_id_start(self.current_char()) };
                if is_name {
                    self.lex_name_or_string(start, start_pos)?;
                } else {
                    self.lex_operator(start, start_pos)?;
                }
            }
        }
        Ok(true)
    }

    fn lex_name_or_string(&mut self, start: usize, start_pos: Position) -> Result<(), LexError> {
        while self.i < self.b.len() && (self.b[self.i].is_ascii_alphanumeric() || self.b[self.i] == b'_') {
            self.i += 1;
        }
        // Non-ASCII identifier characters (an ASCII byte here ends the name).
        while !self.at_end() && !self.b[self.i].is_ascii() {
            let c = self.current_char();
            if !is_id_continue(c) {
                break;
            }
            self.i += c.len_utf8();
            while self.i < self.b.len() && (self.b[self.i].is_ascii_alphanumeric() || self.b[self.i] == b'_') {
                self.i += 1;
            }
        }
        let word = &self.src[start..self.i];
        if matches!(self.peek(0), b'\'' | b'"') {
            if self.version >= Version::Py314 {
                if let Some(seen) = prefix_letters(word) {
                    if let Some((first, second)) = incompatible_prefixes(seen) {
                        let column = self.char_column(self.line_start, start) + 1;
                        return self.error_offset(self.line, column, &format!("'{first}' and '{second}' prefixes are incompatible"));
                    }
                    let kind = if seen[4] {
                        StringKind::Template
                    } else if seen[3] {
                        StringKind::Format
                    } else {
                        StringKind::Plain
                    };
                    return self.lex_string(start, start_pos, kind, seen[1]);
                }
            } else if word.len() <= 2 {
                if let Some((is_fstring, raw)) = string_prefix(word) {
                    let kind = if is_fstring { StringKind::Format } else { StringKind::Plain };
                    return self.lex_string(start, start_pos, kind, raw);
                }
            }
        }
        if self.byte_columns && !word.is_ascii() {
            let tables = identifier_tables(self.version);
            if let Some((index, c)) = first_invalid_identifier_char(word, &tables) {
                // CPython reports the column just past the bad character.
                let offset = self.char_column(self.line_start, start) + index as u32 + 1;
                let message = if in_ranges(c as u32, tables.nonprintable) {
                    format!("invalid non-printable character U+{:04X}", c as u32)
                } else {
                    format!("invalid character '{c}' (U+{:04X})", c as u32)
                };
                return self.error_offset(self.line, offset, &message);
            }
        }
        self.push(Kind::Name, start, start_pos, None);
        Ok(())
    }

    /// Consume digits with single underscores between them. Returns how
    /// many digits were read; an underscore not followed by a digit is an
    /// error, as in CPython.
    fn digit_group(&mut self, accept: fn(u8) -> bool, kind: &str) -> Result<usize, LexError> {
        let mut count = 0;
        loop {
            let byte = self.peek(0);
            if accept(byte) {
                self.i += 1;
                count += 1;
            } else if byte == b'_' && count > 0 {
                if !accept(self.peek(1)) {
                    return self.error(&format!("invalid {kind} literal"));
                }
                self.i += 1;
            } else {
                return Ok(count);
            }
        }
    }

    fn lex_number(&mut self, start: usize, start_pos: Position) -> Result<(), LexError> {
        let prefix = self.peek(1).to_ascii_lowercase();
        if self.b[self.i] == b'0' && matches!(prefix, b'x' | b'o' | b'b') {
            let (accept, kind): (fn(u8) -> bool, &str) = match prefix {
                b'x' => (|byte| byte.is_ascii_hexdigit(), "hexadecimal"),
                b'o' => (|byte| (b'0'..=b'7').contains(&byte), "octal"),
                _ => (|byte| byte == b'0' || byte == b'1', "binary"),
            };
            self.i += 2;
            if self.peek(0) == b'_' {
                self.i += 1;
            }
            if self.digit_group(accept, kind)? == 0 {
                if prefix != b'x' && self.peek(0).is_ascii_digit() {
                    let digit = self.peek(0) as char;
                    return self.error(&format!("invalid digit '{digit}' in {kind} literal"));
                }
                return self.error_offset(self.line, self.cursor_col(), &format!("invalid {kind} literal"));
            }
            if prefix != b'x' && self.peek(0).is_ascii_digit() {
                let digit = self.peek(0) as char;
                return self.error(&format!("invalid digit '{digit}' in {kind} literal"));
            }
        } else {
            let decimal: fn(u8) -> bool = |byte| byte.is_ascii_digit();
            self.digit_group(decimal, "decimal")?;
            if self.peek(0) == b'.' {
                self.i += 1;
                if self.peek(0).is_ascii_digit() {
                    self.digit_group(decimal, "decimal")?;
                }
            }
            if matches!(self.peek(0), b'e' | b'E') {
                if self.peek(1).is_ascii_digit() {
                    self.i += 1;
                    self.digit_group(decimal, "decimal")?;
                } else if matches!(self.peek(1), b'+' | b'-') {
                    if !self.peek(2).is_ascii_digit() {
                        return self.error("invalid decimal literal");
                    }
                    self.i += 2;
                    self.digit_group(decimal, "decimal")?;
                }
            }
            if matches!(self.peek(0), b'j' | b'J') {
                self.i += 1;
            }
            // The parser's tokenizer rejects old-style octal such as 0123.
            let text = &self.src[start..self.i];
            if self.check_brackets
                && text.starts_with('0')
                && text.bytes().all(|byte| byte.is_ascii_digit() || byte == b'_')
                && text.bytes().any(|byte| (b'1'..=b'9').contains(&byte))
            {
                return self.error_offset(
                    self.line,
                    self.char_column(self.line_start, start) + 1,
                    "leading zeros in decimal integer literals are not permitted; use an 0o prefix for octal integers",
                );
            }
        }
        if self.check_brackets {
            self.verify_end_of_number(start)?;
        }
        self.push(Kind::Number, start, start_pos, None);
        Ok(())
    }

    /// CPython's parser tokenizer rejects a number followed directly by an
    /// identifier character, except a keyword valid after a number.
    fn verify_end_of_number(&self, start: usize) -> Result<(), LexError> {
        let next = self.peek(0);
        if !(next.is_ascii_alphanumeric() || next == b'_') {
            return Ok(());
        }
        let rest = &self.b[self.i + 1..];
        let keyword_follows = match next {
            b'a' => rest.starts_with(b"nd"),
            b'e' => rest.starts_with(b"lse"),
            b'f' => rest.starts_with(b"or"),
            b'i' => matches!(rest.first(), Some(b'f' | b'n' | b's')),
            b'o' => rest.starts_with(b"r"),
            b'n' => rest.starts_with(b"ot"),
            _ => false,
        };
        if keyword_follows {
            return Ok(());
        }
        let text = self.src[start..self.i].to_ascii_lowercase();
        let kind = if text.starts_with("0x") {
            "hexadecimal"
        } else if text.starts_with("0o") {
            "octal"
        } else if text.starts_with("0b") {
            "binary"
        } else if text.ends_with('j') {
            "imaginary"
        } else {
            "decimal"
        };
        self.error_offset(self.line, self.cursor_col(), &format!("invalid {kind} literal"))
    }

    fn lex_string(
        &mut self,
        start: usize,
        start_pos: Position,
        kind: StringKind,
        raw: bool,
    ) -> Result<(), LexError> {
        let quote = self.b[self.i];
        let triple = self.peek(1) == quote && self.peek(2) == quote;
        let start_line = self.line;
        let start_line_start = self.line_start;
        self.i += if triple { 3 } else { 1 };
        if kind != StringKind::Plain {
            let template = kind == StringKind::Template;
            self.push(if template { Kind::TStringStart } else { Kind::FStringStart }, start, start_pos, None);
            self.modes.push(FString {
                template,
                quote,
                triple,
                raw,
                start_line,
                start_offset: start,
                start_line_start: self.line_start,
                depth: 0,
                expr_start: -1,
                in_expression: false,
                expr_ended: false,
            });
            return Ok(());
        }
        let mut bare_cr = false;
        loop {
            // Bytes other than the quote, a backslash or a line break need
            // no attention (UTF-8 continuation bytes are never ASCII).
            while self.i < self.b.len() && !matches!(self.b[self.i], b'\\' | b'\n' | b'\r') && self.b[self.i] != quote {
                self.i += 1;
            }
            if self.at_end() {
                if self.closes_enclosing_fstring(quote, triple) {
                    let message = format!("{}-string: expecting '}}'", self.mode_prefix());
                    return self.error_offset(start_line, self.char_column(start_line_start, start) + 1, &message);
                }
                let (message, detected) = if triple {
                    ("unterminated triple-quoted string literal", self.last_content_line())
                } else {
                    ("unterminated string literal", self.line)
                };
                return self.error_offset(start_line, self.char_column(start_line_start, start) + 1, &format!("{message} (detected at line {detected})"));
            }
            if self.b[self.i] == b'\r' && self.peek(1) != b'\n' {
                bare_cr = true;
            }
            let byte = self.b[self.i];
            if byte == b'\\' {
                self.i += 1;
                if !self.at_end() {
                    if self.b[self.i] == b'\r' && self.peek(1) != b'\n' {
                        bare_cr = true;
                    }
                    self.advance_char();
                }
                continue;
            }
            if byte == quote {
                if !triple {
                    self.i += 1;
                    break;
                }
                if self.peek(1) == quote && self.peek(2) == quote {
                    self.i += 3;
                    break;
                }
            }
            if self.newline_len(self.i) > 0 && !triple {
                if self.closes_enclosing_fstring(quote, triple) {
                    let message = format!("{}-string: expecting '}}'", self.mode_prefix());
                    return self.error_offset(start_line, self.char_column(start_line_start, start) + 1, &message);
                }
                let message = format!("unterminated string literal (detected at line {})", self.line);
                return self.error_offset(start_line, self.char_column(start_line_start, start) + 1, &message);
            }
            self.advance_char();
        }
        // CPython's tokenize reports a bare "\r" inside a string as "\r\n".
        let text = bare_cr.then(|| expand_bare_cr(&self.src[start..self.i]));
        self.push(Kind::String, start, start_pos, text);
        Ok(())
    }

    fn lex_operator(&mut self, start: usize, start_pos: Position) -> Result<(), LexError> {
        let byte = self.b[self.i];
        if let Some(fstring) = self.modes.last_mut() {
            // CPython looks at these before multi-character operators, so
            // `{x:=1}` starts a format spec.
            if fstring.expr_start >= 0 && matches!(byte, b':' | b'}' | b'!' | b'{') {
                let cursor = fstring.depth - i32::from(byte != b'{');
                if cursor == 0 {
                    fstring.expr_ended = byte != b'{';
                }
                if byte == b':' && cursor == fstring.expr_start {
                    fstring.in_expression = false;
                    self.i += 1;
                    self.push(Kind::Op, start, start_pos, None);
                    return Ok(());
                }
            }
        }
        let length = operator_length(byte, self.peek(1), self.peek(2));
        let length = if length > 1 {
            length
        } else if ONE_CHAR_OPS.contains(&byte) || byte.is_ascii_punctuation() {
            // tokenize reports unknown punctuation such as $ ? ` as OP.
            1
        } else {
            return self.error("invalid character");
        };
        match byte {
            b'(' | b'[' | b'{' if length == 1 => {
                self.brackets.push((byte, self.line, self.line_start, self.i));
                if let Some(fstring) = self.modes.last_mut() {
                    fstring.depth += 1;
                }
            }
            // tokenize does not check bracket matching; the parser does.
            b')' | b']' | b'}' => {
                if byte == b'}' && self.modes.last().is_some_and(|fstring| fstring.depth == 0) {
                    return self.error(&format!("{}-string: single '}}' is not allowed", self.mode_prefix()));
                }
                let opened = self.brackets.pop();
                if self.check_brackets {
                    self.check_closer(byte, opened)?;
                }
                if let Some(fstring) = self.modes.last_mut() {
                    fstring.depth -= 1;
                    if byte == b'}' && fstring.depth == fstring.expr_start {
                        fstring.expr_start -= 1;
                        fstring.in_expression = false;
                    }
                }
            }
            _ => {}
        }
        self.i += length;
        self.push(Kind::Op, start, start_pos, None);
        Ok(())
    }

    /// 'f' or 't': the innermost f-string's prefix, for messages.
    fn mode_prefix(&self) -> char {
        self.modes.last().map_or('f', FString::prefix)
    }

    fn middle_kind(&self) -> Kind {
        if self.modes.last().is_some_and(|fstring| fstring.template) { Kind::TStringMiddle } else { Kind::FStringMiddle }
    }

    /// Whether a string's quote is the innermost f-string's own quote.
    fn closes_enclosing_fstring(&self, quote: u8, triple: bool) -> bool {
        self.modes.last().is_some_and(|fstring| fstring.quote == quote && fstring.triple == triple)
    }

    fn unterminated_fstring<T>(&self, fstring: &FString, detected: u32) -> Result<T, LexError> {
        let triple = if fstring.triple { "triple-quoted " } else { "" };
        let column = self.char_column(fstring.start_line_start, fstring.start_offset);
        let message = format!("unterminated {triple}{}-string literal (detected at line {detected})", fstring.prefix());
        self.error_offset(fstring.start_line, column + 1, &message)
    }

    /// CPython's parser-mode checks for a closing bracket.
    fn check_closer(&self, closer: u8, opened: Option<(u8, u32, usize, usize)>) -> Result<(), LexError> {
        let Some((opener, line, _, _)) = opened else {
            return self.error(&format!("unmatched '{}'", closer as char));
        };
        let expected = match opener {
            b'(' => b')',
            b'[' => b']',
            _ => b'}',
        };
        if closer == expected {
            return Ok(());
        }
        let mut message = format!(
            "closing parenthesis '{}' does not match opening parenthesis '{}'",
            closer as char, opener as char
        );
        if line != self.line {
            message.push_str(&format!(" on line {line}"));
        }
        self.error(&message)
    }

    fn closes_fstring(&self, quote: u8, triple: bool) -> bool {
        self.peek(0) == quote && (!triple || (self.peek(1) == quote && self.peek(2) == quote))
    }

    fn flush_middle(&mut self, start: usize, start_pos: Position, text: &mut String) {
        if !text.is_empty() {
            let mut value = std::mem::take(text);
            // As for strings, tokenize reports a bare "\r" as "\r\n".
            if value.contains('\r') {
                value = expand_bare_cr(&value);
            }
            let slice_matches = &self.src[start..self.i] == value.as_str();
            let kind = self.middle_kind();
            self.push(kind, start, start_pos, if slice_matches { None } else { Some(value) });
        }
    }

    /// Begin a replacement field at the current `{`, which is then lexed
    /// as an operator in expression mode. The expression-end flag is kept:
    /// after a nested field CPython is still inside the format spec.
    fn open_field(&mut self) {
        if let Some(fstring) = self.modes.last_mut() {
            fstring.expr_start += 1;
            fstring.in_expression = true;
        }
    }

    /// Emit the text scanned so far, as an empty token too when `always`.
    fn finish_middle(&mut self, start: usize, start_pos: Position, text: &mut String, always: bool) {
        if text.is_empty() && always {
            let position = self.here();
            let kind = self.middle_kind();
            self.push_synthetic(kind, "", position, position);
        }
        self.flush_middle(start, start_pos, text);
    }

    /// Scan f-string literal text or a format spec until a field, a brace
    /// handed to expression mode, the closing quote or an error, as
    /// CPython's f-string mode does.
    fn lex_fstring_middle(&mut self, fstring: FString) -> Result<(), LexError> {
        let FString { quote, triple, raw, .. } = fstring;
        if self.closes_fstring(quote, triple) {
            let end_start = self.i;
            let end_pos = self.here();
            self.i += if triple { 3 } else { 1 };
            self.push(if fstring.template { Kind::TStringEnd } else { Kind::FStringEnd }, end_start, end_pos, None);
            self.modes.pop();
            return Ok(());
        }
        // A field right at the start yields no middle token.
        if self.peek(0) == b'{' && self.peek(1) != b'{' {
            self.open_field();
            return Ok(());
        }
        let in_spec = fstring.expr_ended && fstring.expr_start >= 0;
        let mut start = self.i;
        let mut start_pos = self.here();
        let mut text = String::new();
        loop {
            if self.at_end() {
                return self.unterminated_fstring(&fstring, self.last_content_line());
            }
            if self.closes_fstring(quote, triple) {
                self.flush_middle(start, start_pos, &mut text);
                return Ok(());
            }
            let byte = self.b[self.i];
            match byte {
                b'{' | b'}' if !in_spec && self.peek(1) == byte => {
                    text.push(byte as char);
                    self.i += 1;
                    self.flush_middle(start, start_pos, &mut text);
                    self.i += 1;
                    start = self.i;
                    start_pos = self.here();
                }
                b'{' => {
                    self.finish_middle(start, start_pos, &mut text, in_spec);
                    self.open_field();
                    return Ok(());
                }
                b'}' => {
                    // Expression mode closes the field, or reports a
                    // single '}' in literal text.
                    self.finish_middle(start, start_pos, &mut text, true);
                    if let Some(open) = self.modes.last_mut() {
                        open.in_expression = true;
                    }
                    return Ok(());
                }
                b'\\' => {
                    // As in CPython, literal text and specs share escapes:
                    // a backslash takes the next character, except a brace.
                    text.push('\\');
                    self.i += 1;
                    if !raw && self.peek(0) == b'N' && self.peek(1) == b'{' {
                        while !self.at_end() && self.b[self.i] != b'}' {
                            text.push(self.current_char());
                            self.advance_char();
                        }
                        if !self.at_end() {
                            text.push('}');
                            self.i += 1;
                        }
                        // CPython ends the text token after a named escape.
                        self.flush_middle(start, start_pos, &mut text);
                        start = self.i;
                        start_pos = self.here();
                    } else if !self.at_end() && !matches!(self.b[self.i], b'{' | b'}') {
                        let next_start = self.i;
                        self.advance_char();
                        text.push_str(&self.src[next_start..self.i]);
                    }
                }
                b'\n' if in_spec && !triple => {
                    if self.version >= Version::Py313 {
                        let prefix = fstring.prefix();
                        return self.error(&format!(
                            "{prefix}-string: newlines are not allowed in format specifiers for single quoted {prefix}-strings"
                        ));
                    }
                    // CPython 3.12 ends a single-quoted spec at a newline
                    // and resumes the replacement field.
                    self.finish_middle(start, start_pos, &mut text, true);
                    if let Some(open) = self.modes.last_mut() {
                        open.in_expression = true;
                    }
                    return Ok(());
                }
                b'\r' if in_spec && !triple && self.peek(1) == b'\n' => {
                    // The `\r` stays spec text; the `\n` then ends the spec.
                    self.i += 1;
                    text.push('\r');
                }
                _ => {
                    if !in_spec && !triple && self.newline_len(self.i) > 0 {
                        // CPython reports the line where the f-string began.
                        return self.unterminated_fstring(&fstring, self.line);
                    }
                    let char_start = self.i;
                    self.advance_char();
                    text.push_str(&self.src[char_start..self.i]);
                }
            }
        }
    }
}

/// JSON string escaping identical to Python's `json.dumps(..., ensure_ascii=False)`.
pub fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Render tokens in the dump format of `scripts/dump_tokens.py`.
pub fn dump(source: &str) -> String {
    dump_version(source, Version::default())
}

/// `dump` for a given Python version.
pub fn dump_version(source: &str, version: Version) -> String {
    match tokenize_version(source, version) {
        Ok(tokens) => {
            let mut out = String::new();
            for token in &tokens {
                out.push_str(&format!(
                    "{} {},{}-{},{} {}\n",
                    token.kind.name(),
                    token.start_pos.0,
                    token.start_pos.1,
                    token.end_pos.0,
                    token.end_pos.1,
                    json_escape(token.text(source))
                ));
            }
            out
        }
        Err(error) => format!("ERROR {}\n", error.line),
    }
}
