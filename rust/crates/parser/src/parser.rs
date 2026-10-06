//! Recursive-descent parser over RefacTrail's own tokens, producing trees
//! that `ast.dump` identically to CPython 3.12's `ast.parse`. Statements
//! live here; expressions, f-strings and match patterns in sibling files.

use std::cell::Cell;

use refactrail_lexer::{Kind, LexError, Token, TokenSink, Version};

use crate::ast::{
    Alias, ExceptHandler, Expr, ExprContext, ExprKind, Id, Module, Operator, Stmt, StmtKind, TypeParam, TypeParamKind,
    WithItem,
};
use crate::expr::{expr, name_node};
use crate::node::{Constant, Loc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: u32,
    /// Byte column of the error.
    pub col: u32,
    /// CPython's 1-based character offset when known directly (tokenizer
    /// errors); 0 means "derive it from `col`".
    pub offset: u32,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "line {}: {}", self.line, self.message)
    }
}

pub type PResult<T> = Result<T, ParseError>;

/// A token's byte range in the source.
#[derive(Clone, Copy, Debug)]
pub struct TokenSpan {
    pub start: usize,
    pub end: usize,
}

/// A small code for every operator and (soft) keyword, so token checks
/// compare one byte instead of strings; 0 for anything else. Switches on
/// the length first, and folds to a constant for literal arguments.
#[inline(always)]
pub fn symbol_code(text: &str) -> u8 {
    let bytes = text.as_bytes();
    match bytes.len() {
        1 => match bytes {
            b"(" => 1,
            b")" => 2,
            b"[" => 3,
            b"]" => 4,
            b"{" => 5,
            b"}" => 6,
            b":" => 7,
            b"," => 8,
            b";" => 9,
            b"+" => 10,
            b"-" => 11,
            b"*" => 12,
            b"/" => 13,
            b"|" => 14,
            b"&" => 15,
            b"<" => 16,
            b">" => 17,
            b"=" => 18,
            b"." => 19,
            b"%" => 20,
            b"~" => 25,
            b"^" => 26,
            b"@" => 43,
            b"!" => 48,
            b"_" => 88,
            _ => 0,
        },
        2 => match bytes[0] {
            b'!' => match bytes {
                b"!=" => 22,
                _ => 0,
            },
            b'%' => match bytes {
                b"%=" => 34,
                _ => 0,
            },
            b'&' => match bytes {
                b"&=" => 35,
                _ => 0,
            },
            b'*' => match bytes {
                b"**" => 29,
                b"*=" => 32,
                _ => 0,
            },
            b'+' => match bytes {
                b"+=" => 30,
                _ => 0,
            },
            b'-' => match bytes {
                b"-=" => 31,
                b"->" => 45,
                _ => 0,
            },
            b'/' => match bytes {
                b"/=" => 33,
                b"//" => 41,
                _ => 0,
            },
            b':' => match bytes {
                b":=" => 47,
                _ => 0,
            },
            b'<' => match bytes {
                b"<=" => 23,
                b"<<" => 27,
                b"<>" => 49,
                _ => 0,
            },
            b'=' => match bytes {
                b"==" => 21,
                _ => 0,
            },
            b'>' => match bytes {
                b">=" => 24,
                b">>" => 28,
                _ => 0,
            },
            b'@' => match bytes {
                b"@=" => 44,
                _ => 0,
            },
            b'^' => match bytes {
                b"^=" => 37,
                _ => 0,
            },
            b'a' => match bytes {
                b"as" => 54,
                _ => 0,
            },
            b'i' => match bytes {
                b"if" => 70,
                b"in" => 72,
                b"is" => 73,
                _ => 0,
            },
            b'o' => match bytes {
                b"or" => 77,
                _ => 0,
            },
            b'|' => match bytes {
                b"|=" => 36,
                _ => 0,
            },
            _ => 0,
        },
        3 => match bytes[0] {
            b'*' => match bytes {
                b"**=" => 40,
                _ => 0,
            },
            b'.' => match bytes {
                b"..." => 46,
                _ => 0,
            },
            b'/' => match bytes {
                b"//=" => 42,
                _ => 0,
            },
            b'<' => match bytes {
                b"<<=" => 38,
                _ => 0,
            },
            b'>' => match bytes {
                b">>=" => 39,
                _ => 0,
            },
            b'a' => match bytes {
                b"and" => 53,
                _ => 0,
            },
            b'd' => match bytes {
                b"def" => 61,
                b"del" => 62,
                _ => 0,
            },
            b'f' => match bytes {
                b"for" => 67,
                _ => 0,
            },
            b'n' => match bytes {
                b"not" => 76,
                _ => 0,
            },
            b't' => match bytes {
                b"try" => 81,
                _ => 0,
            },
            _ => 0,
        },
        4 => match bytes[0] {
            b'N' => match bytes {
                b"None" => 51,
                _ => 0,
            },
            b'T' => match bytes {
                b"True" => 52,
                _ => 0,
            },
            b'c' => match bytes {
                b"case" => 86,
                _ => 0,
            },
            b'e' => match bytes {
                b"elif" => 63,
                b"else" => 64,
                _ => 0,
            },
            b'f' => match bytes {
                b"from" => 68,
                _ => 0,
            },
            b'p' => match bytes {
                b"pass" => 78,
                _ => 0,
            },
            b't' => match bytes {
                b"type" => 87,
                _ => 0,
            },
            b'w' => match bytes {
                b"with" => 83,
                _ => 0,
            },
            _ => 0,
        },
        5 => match bytes[0] {
            b'F' => match bytes {
                b"False" => 50,
                _ => 0,
            },
            b'a' => match bytes {
                b"async" => 56,
                b"await" => 57,
                _ => 0,
            },
            b'b' => match bytes {
                b"break" => 58,
                _ => 0,
            },
            b'c' => match bytes {
                b"class" => 59,
                _ => 0,
            },
            b'm' => match bytes {
                b"match" => 85,
                _ => 0,
            },
            b'r' => match bytes {
                b"raise" => 79,
                _ => 0,
            },
            b'w' => match bytes {
                b"while" => 82,
                _ => 0,
            },
            b'y' => match bytes {
                b"yield" => 84,
                _ => 0,
            },
            _ => 0,
        },
        6 => match bytes[0] {
            b'a' => match bytes {
                b"assert" => 55,
                _ => 0,
            },
            b'e' => match bytes {
                b"except" => 65,
                _ => 0,
            },
            b'g' => match bytes {
                b"global" => 69,
                _ => 0,
            },
            b'i' => match bytes {
                b"import" => 71,
                _ => 0,
            },
            b'l' => match bytes {
                b"lambda" => 74,
                _ => 0,
            },
            b'r' => match bytes {
                b"return" => 80,
                _ => 0,
            },
            _ => 0,
        },
        7 => match bytes[0] {
            b'f' => match bytes {
                b"finally" => 66,
                _ => 0,
            },
            _ => 0,
        },
        8 => match bytes[0] {
            b'c' => match bytes {
                b"continue" => 60,
                _ => 0,
            },
            b'n' => match bytes {
                b"nonlocal" => 75,
                _ => 0,
            },
            _ => 0,
        },
        _ => 0,
    }
}

const FIRST_KEYWORD: u8 = 50;
const LAST_KEYWORD: u8 = 84;

/// Whether a symbol code is a hard keyword's.
#[inline(always)]
/// A NAME token's identifier: CPython NFKC-normalizes non-ASCII names
/// (PEP 3131), so `\u{fb01}le` is the name `file`.
pub fn identifier(text: &str) -> Id {
    crate::nfkc::nfkc(text).as_ref().into()
}

pub fn is_keyword_code(code: u8) -> bool {
    (FIRST_KEYWORD..=LAST_KEYWORD).contains(&code)
}

// Symbol codes of the keywords statements dispatch on.
const ASSERT: u8 = 55;
const ASYNC: u8 = 56;
const BREAK: u8 = 58;
const CLASS: u8 = 59;
const CONTINUE: u8 = 60;
const DEF: u8 = 61;
const DEL: u8 = 62;
const FOR: u8 = 67;
const FROM: u8 = 68;
const GLOBAL: u8 = 69;
const IF: u8 = 70;
const IMPORT: u8 = 71;
const NONLOCAL: u8 = 75;
const PASS: u8 = 78;
const RAISE: u8 = 79;
const RETURN: u8 = 80;
const TRY: u8 = 81;
const WHILE: u8 = 82;
const WITH: u8 = 83;
const MATCH: u8 = 85;
const TYPE: u8 = 87;

pub(crate) struct Parser<'a> {
    pub src: &'a str,
    /// The Python version whose grammar is parsed.
    pub version: Version,
    /// Byte span of each token.
    spans: Vec<TokenSpan>,
    /// (token index, text) of the few tokens whose text is not their
    /// source slice (strings with bare CRs, f-string middles with doubled
    /// braces), in index order.
    texts: Vec<(usize, String)>,
    /// Byte spans of the comments.
    pub comments: Vec<(usize, usize)>,
    pub kinds: Vec<Kind>,
    /// symbol_code of each operator or name token.
    codes: Vec<u8>,
    pub locs: Vec<Loc>,
    pub pos: usize,
    /// Furthest token index fetched: CPython reports a generic syntax
    /// error at the last token its parser read.
    furthest: Cell<usize>,
    /// The tokenizer error after the last token, raised only if parsing
    /// gets that far (CPython tokenizes lazily).
    lex_error: Option<LexError>,
    /// Bracket nesting level of each token.
    levels: Vec<u32>,
    /// Second pass: check CPython's invalid_* rules for specific errors.
    pub invalid_mode: bool,
    /// Token positions whose missing-comma check already passed (CPython
    /// memoizes its rules; without this the check is exponential).
    pub comma_checked: std::collections::HashSet<usize>,
}

/// Byte offset of the start of each line, splitting as the lexer does.
fn line_starts(src: &str) -> Vec<usize> {
    let bytes = src.as_bytes();
    let mut starts = Vec::with_capacity(bytes.len() / 32 + 1);
    starts.push(0);
    let mut skip_to = 0;
    for index in memchr::memchr2_iter(b'\n', b'\r', bytes) {
        if index < skip_to {
            continue; // the \n of a \r\n pair
        }
        let end = if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') { index + 2 } else { index + 1 };
        starts.push(end);
        skip_to = end;
    }
    starts
}

/// Builds the parser's token arrays as the lexer produces tokens: drops
/// comments and NL tokens (keeping comment spans) and records each kept
/// token's kind, position, symbol code, bracket level and span.
struct Prepare<'a> {
    src: &'a str,
    starts: Vec<usize>,
    eof_line: u32,
    eof_col: u32,
    /// The lexer stopped with an error (set before the final ENDMARKER).
    lex_error: bool,
    kinds: Vec<Kind>,
    locs: Vec<Loc>,
    codes: Vec<u8>,
    levels: Vec<u32>,
    spans: Vec<TokenSpan>,
    texts: Vec<(usize, String)>,
    comments: Vec<(usize, usize)>,
    level: u32,
    comment_start: Option<(u32, u32)>,
}

impl Prepare<'_> {
    #[inline]
    fn column(&self, row: u32, byte: usize) -> u32 {
        let line_start = self.starts.get(row as usize - 1).copied().unwrap_or(byte);
        byte.saturating_sub(line_start) as u32
    }
}

impl TokenSink for Prepare<'_> {
    #[inline]
    fn push(&mut self, mut token: Token) {
        if token.kind == Kind::Comment {
            self.comment_start = Some((token.start_pos.0, self.column(token.start_pos.0, token.start)));
            self.comments.push((token.start, token.end));
            return;
        }
        if token.kind == Kind::Nl {
            self.comment_start = None;
            return;
        }
        let after_comment = self.comment_start.take();
        let (line, end_line) = (token.start_pos.0, token.end_pos.0);
        let synthetic = token.start == token.end && token.text.is_some();
        let at_eof = token.start >= self.src.len() && matches!(token.kind, Kind::EndMarker | Kind::Dedent);
        let loc = if at_eof && !self.lex_error {
            Loc { line: self.eof_line, col: self.eof_col, end_line: self.eof_line, end_col: self.eof_col }
        } else if let (Kind::Newline, Some((comment_line, comment_col))) = (token.kind, after_comment) {
            Loc { line: comment_line, col: comment_col, end_line, end_col: self.column(end_line, token.end) }
        } else if synthetic {
            Loc { line, col: token.start_pos.1, end_line, end_col: token.end_pos.1 }
        } else {
            Loc { line, col: self.column(line, token.start), end_line, end_col: self.column(end_line, token.end) }
        };
        let code = if matches!(token.kind, Kind::Op | Kind::Name) { symbol_code(token.text(self.src)) } else { 0 };
        if token.kind == Kind::Op {
            match code {
                1 | 3 | 5 => self.level += 1,
                2 | 4 | 6 => self.level = self.level.saturating_sub(1),
                _ => {}
            }
        }
        if !synthetic {
            if let Some(text) = token.text.take() {
                self.texts.push((self.kinds.len(), text));
            }
        }
        self.kinds.push(token.kind);
        self.locs.push(loc);
        self.codes.push(code);
        self.levels.push(self.level);
        self.spans.push(TokenSpan { start: token.start, end: token.end });
    }
}

impl<'a> Parser<'a> {
    /// Tokenize `src` (in the parser tokenizer's mode) and prepare to parse.
    pub fn new(src: &'a str) -> Parser<'a> {
        Self::new_version(src, Version::default())
    }

    /// `new` for a given Python version's grammar.
    pub fn new_version(src: &'a str, version: Version) -> Parser<'a> {
        let starts = line_starts(src);
        // CPython's parser tokenizer puts end-of-file tokens on the last
        // physical line.
        let eof_line = if src.is_empty() {
            1
        } else if src.ends_with(['\n', '\r']) {
            (starts.len() - 1) as u32
        } else {
            starts.len() as u32
        };
        // ... at the end of that line's content.
        let eof_col = {
            let last_start = if src.ends_with(['\n', '\r']) {
                // The last line is the one the final terminator ends.
                starts.get(starts.len().saturating_sub(2)).copied().unwrap_or(0)
            } else {
                starts.last().copied().unwrap_or(0)
            };
            let line_end = src[last_start..].find(['\n', '\r']).map_or(src.len(), |at| last_start + at);
            (line_end - last_start.min(line_end)) as u32
        };
        let capacity = src.len() / 6 + 16;
        let prepare = Prepare {
            src,
            starts,
            eof_line,
            eof_col,
            lex_error: false,
            kinds: Vec::with_capacity(capacity),
            locs: Vec::with_capacity(capacity),
            codes: Vec::with_capacity(capacity),
            levels: Vec::with_capacity(capacity),
            spans: Vec::with_capacity(capacity),
            texts: Vec::new(),
            comments: Vec::new(),
            level: 0,
            comment_start: None,
        };
        let (mut prepare, lex_error) = refactrail_lexer::tokenize_into_version(src, true, version, prepare);
        if let Some(error) = &lex_error {
            prepare.lex_error = true;
            prepare.push(Token {
                kind: Kind::EndMarker,
                start: src.len(),
                end: src.len(),
                text: Some(String::new()),
                start_pos: (error.line, 0),
                end_pos: (error.line, 0),
            });
        }
        let Prepare { kinds, locs, codes, levels, spans, texts, comments, .. } = prepare;
        Parser {
            src,
            version,
            spans,
            texts,
            comments,
            kinds,
            codes,
            locs,
            pos: 0,
            furthest: Cell::new(0),
            lex_error,
            levels,
            invalid_mode: false,
            comma_checked: Default::default(),
        }
    }

    /// The token stream (kinds and byte spans) for callers of
    /// parse_with_tokens.
    pub fn token_stream(&self) -> Vec<crate::LexedToken> {
        self.kinds
            .iter()
            .zip(&self.spans)
            .map(|(&kind, span)| crate::LexedToken { kind, start: span.start, end: span.end })
            .collect()
    }

    // ----- token helpers -------------------------------------------------

    fn touch(&self, index: usize) {
        if index > self.furthest.get() {
            self.furthest.set(index.min(self.kinds.len() - 1));
        }
    }

    pub fn kind(&self) -> Kind {
        self.touch(self.pos);
        self.kinds[self.pos]
    }

    pub fn kind_at(&self, index: usize) -> Kind {
        self.touch(index);
        self.kinds.get(index).copied().unwrap_or(Kind::EndMarker)
    }

    pub fn text_at(&self, index: usize) -> &str {
        self.touch(index);
        let Some(span) = self.spans.get(index) else { return "" };
        if !self.texts.is_empty() {
            if let Ok(found) = self.texts.binary_search_by_key(&index, |(at, _)| *at) {
                return &self.texts[found].1;
            }
        }
        &self.src[span.start..span.end]
    }

    pub fn text(&self) -> &str {
        self.text_at(self.pos)
    }

    pub fn raw_token(&self, index: usize) -> TokenSpan {
        self.spans[index]
    }

    /// Symbol code of the current token (0 unless an operator or name).
    #[inline(always)]
    pub fn code(&self) -> u8 {
        self.touch(self.pos);
        self.code_at(self.pos)
    }

    #[inline(always)]
    fn code_at(&self, index: usize) -> u8 {
        self.codes.get(index).copied().unwrap_or(0)
    }

    #[inline(always)]
    pub fn at_op(&self, op: &str) -> bool {
        let code = symbol_code(op);
        debug_assert!(code != 0, "unknown operator {op}");
        self.kind() == Kind::Op && self.code_at(self.pos) == code
    }

    #[inline(always)]
    pub fn op_at(&self, index: usize, op: &str) -> bool {
        let code = symbol_code(op);
        self.kind_at(index) == Kind::Op && self.code_at(index) == code
    }

    #[inline(always)]
    pub fn at_kw(&self, word: &str) -> bool {
        let code = symbol_code(word);
        debug_assert!(code != 0, "unknown keyword {word}");
        self.kind() == Kind::Name && self.code_at(self.pos) == code
    }

    #[inline(always)]
    pub fn kw_at(&self, index: usize, word: &str) -> bool {
        let code = symbol_code(word);
        self.kind_at(index) == Kind::Name && self.code_at(index) == code
    }

    /// A NAME token that is not a hard keyword.
    #[inline(always)]
    pub fn at_identifier(&self) -> bool {
        self.kind() == Kind::Name && !(FIRST_KEYWORD..=LAST_KEYWORD).contains(&self.code_at(self.pos))
    }

    pub fn bump(&mut self) -> usize {
        let index = self.pos;
        if self.kinds[index] != Kind::EndMarker {
            self.pos += 1;
        }
        index
    }

    pub fn eat_op(&mut self, op: &str) -> Option<usize> {
        self.at_op(op).then(|| self.bump())
    }

    pub fn eat_kw(&mut self, word: &str) -> Option<usize> {
        self.at_kw(word).then(|| self.bump())
    }

    /// The ':' ending a compound statement header. `forced` mirrors the
    /// grammar's `&&':'`: an immediate error at any other token; otherwise
    /// the specific message needs a NEWLINE there (second pass only).
    pub fn expect_colon(&mut self, forced: bool) -> PResult<usize> {
        if let Some(index) = self.eat_op(":") {
            return Ok(index);
        }
        if forced || (self.invalid_mode && self.kind() == Kind::Newline) {
            return Err(self.error("expected ':'"));
        }
        Err(self.error("invalid syntax"))
    }

    pub fn expect_op(&mut self, op: &str) -> PResult<usize> {
        // A missing token is CPython's generic error.
        self.eat_op(op).ok_or_else(|| self.error("invalid syntax"))
    }

    pub fn expect_kw(&mut self, word: &str) -> PResult<usize> {
        self.eat_kw(word).ok_or_else(|| self.error("invalid syntax"))
    }

    pub fn expect_name(&mut self) -> PResult<(Id, usize)> {
        if self.at_identifier() {
            let index = self.bump();
            return Ok((identifier(self.text_at(index)), index));
        }
        Err(self.error("invalid syntax"))
    }

    pub fn error(&self, message: &str) -> ParseError {
        let loc = self.locs[self.pos];
        ParseError { line: loc.line, col: loc.col, offset: 0, message: message.to_string() }
    }

    /// CPython raises this error only from its second-pass rules; in the
    /// first pass the parse just fails (a generic error at this token).
    pub fn demote(&self, error: ParseError) -> ParseError {
        if self.invalid_mode {
            error
        } else {
            self.error("invalid syntax")
        }
    }

    /// An error with a specific message at a known location.
    pub fn specific(&self, loc: Loc, message: &str) -> ParseError {
        ParseError { line: loc.line, col: loc.col, offset: 0, message: message.to_string() }
    }

    pub fn loc(&self, index: usize) -> Loc {
        self.locs[index]
    }

    /// Span from a start token to the last consumed token.
    pub fn span(&self, start: usize) -> Loc {
        self.locs[start].to(self.locs[self.pos.saturating_sub(1).max(start)])
    }

    // ----- module and blocks --------------------------------------------

    /// Parse the module and settle which error CPython would report:
    /// a tokenizer error only when parsing reached it, and a syntax error
    /// on the line of the furthest token read.
    pub fn parse_module(&mut self) -> PResult<Module> {
        let result = self.module();
        let furthest = self.furthest.get();
        let reached_end = furthest + 1 >= self.kinds.len();
        let lex_error = self.lex_error.clone().map(|error| ParseError { line: error.line, col: 0, offset: error.offset, message: error.message });
        let error = match (result, lex_error.clone()) {
            (Ok(tree), None) => return Ok(tree),
            (Ok(_), Some(lex)) => return Err(lex),
            (Err(_), Some(lex)) if reached_end => return Err(lex),
            (Err(error), _) => error,
        };
        let error = if error.message != "invalid syntax" {
            error
        } else {
            // CPython's second pass tries its "invalid_*" rules for a more
            // specific message.
            let generic_loc = self.locs[furthest];
            self.pos = 0;
            self.invalid_mode = true;
            let second = self.module();
            if let Some(lex) = &lex_error {
                if self.furthest.get() + 1 >= self.kinds.len() {
                    return Err(lex.clone());
                }
            }
            match second {
                Err(second) if second.message != "invalid syntax" => second,
                _ => {
                    let message = match self.kinds[furthest] {
                        Kind::Indent => "unexpected indent",
                        Kind::Dedent => "unexpected unindent",
                        _ => "invalid syntax",
                    };
                    let offset = if self.kinds[furthest] == Kind::Indent { generic_loc.end_col } else { 0 };
                    ParseError { line: generic_loc.line, col: generic_loc.col, offset, message: message.into() }
                }
            }
        };
        // CPython then tokenizes the rest of the source: a tokenizer error
        // raised as an exception replaces the syntax error, and so does a
        // bracket left open before the error line; indentation errors and
        // the parser's own unindent reports never do.
        if let Some(lex) = lex_error {
            let indentation = matches!(
                lex.message.as_str(),
                "unindent does not match any outer indentation level"
                    | "inconsistent use of tabs and spaces in indentation"
                    | "unexpected EOF while parsing"
                    | "unexpected character after line continuation character"
                    | "too many indentation levels"
            );
            let never_closed = lex.message.ends_with("was never closed");
            let parser_indentation = matches!(error.message.as_str(), "unexpected indent" | "unexpected unindent");
            if !parser_indentation && ((never_closed && self.locs[self.furthest.get()].line > lex.line) || (!never_closed && !indentation)) {
                return Err(lex);
            }
        }
        Err(error)
    }

    /// Token bracket level (CPython's `token->level`).
    pub fn level_at(&self, index: usize) -> u32 {
        self.levels.get(index).copied().unwrap_or(0)
    }

    fn module(&mut self) -> PResult<Module> {
        let mut body = Vec::new();
        while self.kind() != Kind::EndMarker {
            if self.kind() == Kind::Newline {
                self.bump();
                continue;
            }
            self.statement(&mut body)?;
        }
        Ok(Module { body })
    }

    /// Parse one statement line (or compound statement) into `body`.
    fn statement(&mut self, body: &mut Vec<Stmt>) -> PResult<()> {
        if self.kind() == Kind::Indent {
            let loc = self.loc(self.pos);
            return Err(ParseError { line: loc.line, col: loc.col, offset: loc.end_col, message: "unexpected indent".into() });
        }
        if let Some(node) = self.compound_statement()? {
            body.push(node);
            return Ok(());
        }
        self.simple_statements(body)
    }

    /// A statement's block; `what` and the keyword token name the header
    /// in CPython's "expected an indented block after ..." message.
    pub fn block(&mut self, what: &str, keyword: usize) -> PResult<Vec<Stmt>> {
        let mut body = Vec::new();
        if self.kind() != Kind::Newline {
            self.simple_statements(&mut body)?;
            return Ok(body);
        }
        self.bump();
        if self.kind() != Kind::Indent {
            let line = self.loc(keyword).line;
            return Err(self.demote(self.error(&format!("expected an indented block after {what} on line {line}"))));
        }
        self.bump();
        while !matches!(self.kind(), Kind::Dedent | Kind::EndMarker) {
            self.statement(&mut body)?;
        }
        if self.kind() == Kind::Dedent {
            self.bump();
        }
        Ok(body)
    }

    fn simple_statements(&mut self, statements: &mut Vec<Stmt>) -> PResult<()> {
        statements.push(self.simple_statement()?);
        while self.eat_op(";").is_some() {
            if matches!(self.kind(), Kind::Newline | Kind::EndMarker) {
                break;
            }
            statements.push(self.simple_statement()?);
        }
        match self.kind() {
            Kind::Newline => {
                self.bump();
            }
            Kind::EndMarker => {}
            _ => return Err(self.error("invalid syntax")),
        }
        Ok(())
    }

    // ----- simple statements --------------------------------------------

    fn simple_statement(&mut self) -> PResult<Stmt> {
        let start = self.pos;
        if self.kind() == Kind::Name {
            match self.code() {
                code @ (PASS | BREAK | CONTINUE) => {
                    self.bump();
                    let kind = match code {
                        PASS => StmtKind::Pass,
                        BREAK => StmtKind::Break,
                        _ => StmtKind::Continue,
                    };
                    return Ok(stmt(kind, self.span(start)));
                }
                RETURN => {
                    self.bump();
                    let value = if self.starts_expression() { Some(Box::new(self.star_expressions()?)) } else { None };
                    return Ok(stmt(StmtKind::Return { value }, self.span(start)));
                }
                RAISE => return self.raise_statement(),
                code @ (GLOBAL | NONLOCAL) => {
                    self.bump();
                    let mut names = vec![self.expect_name()?.0];
                    while self.eat_op(",").is_some() {
                        names.push(self.expect_name()?.0);
                    }
                    let kind = if code == GLOBAL { StmtKind::Global { names } } else { StmtKind::Nonlocal { names } };
                    return Ok(stmt(kind, self.span(start)));
                }
                DEL => return self.del_statement(),
                ASSERT => {
                    self.bump();
                    let test = Box::new(self.expression()?);
                    let msg = if self.eat_op(",").is_some() { Some(Box::new(self.expression()?)) } else { None };
                    return Ok(stmt(StmtKind::Assert { test, msg }, self.span(start)));
                }
                IMPORT => return self.import_statement(),
                FROM => return self.from_statement(),
                TYPE if self.kind_at(self.pos + 1) == Kind::Name
                    && (self.op_at(self.pos + 2, "=") || self.op_at(self.pos + 2, "[")) =>
                {
                    return self.type_alias();
                }
                _ => {}
            }
        }
        self.expression_statement()
    }

    fn raise_statement(&mut self) -> PResult<Stmt> {
        let start = self.bump();
        let (mut exc, mut cause) = (None, None);
        if self.starts_expression() {
            exc = Some(Box::new(self.expression()?));
            if self.eat_kw("from").is_some() {
                cause = Some(Box::new(self.expression()?));
            }
        }
        Ok(stmt(StmtKind::Raise { exc, cause }, self.span(start)))
    }

    fn del_statement(&mut self) -> PResult<Stmt> {
        let start = self.bump();
        let mut targets = Vec::new();
        loop {
            let mut target = self.bitwise_or()?;
            set_context(&mut target, ExprContext::Del).map_err(|error| self.demote(error))?;
            targets.push(target);
            if self.eat_op(",").is_none() || !self.starts_expression() {
                break;
            }
        }
        Ok(stmt(StmtKind::Delete { targets }, self.span(start)))
    }

    fn dotted_name(&mut self) -> PResult<Id> {
        let first = self.expect_name()?.0;
        if !(self.at_op(".") && self.kind_at(self.pos + 1) == Kind::Name) {
            return Ok(first);
        }
        let mut name = String::from(first.as_str());
        while self.at_op(".") && self.kind_at(self.pos + 1) == Kind::Name {
            self.bump();
            name.push('.');
            name.push_str(&self.expect_name()?.0);
        }
        Ok(name.into())
    }

    fn import_statement(&mut self) -> PResult<Stmt> {
        let start = self.bump();
        let mut names = Vec::new();
        loop {
            let alias_start = self.pos;
            let name = self.dotted_name()?;
            let asname = if self.eat_kw("as").is_some() { Some(self.expect_name()?.0) } else { None };
            names.push(Alias { name, asname, loc: self.span(alias_start) });
            if self.eat_op(",").is_none() {
                break;
            }
        }
        Ok(stmt(StmtKind::Import { names }, self.span(start)))
    }

    fn from_statement(&mut self) -> PResult<Stmt> {
        let start = self.bump();
        let mut level = 0;
        loop {
            if self.eat_op(".").is_some() {
                level += 1;
            } else if self.eat_op("...").is_some() {
                level += 3;
            } else {
                break;
            }
        }
        let module = if self.at_kw("import") && level > 0 { None } else { Some(self.dotted_name()?) };
        self.expect_kw("import")?;
        let mut names = Vec::new();
        if let Some(star) = self.eat_op("*") {
            names.push(Alias { name: "*".into(), asname: None, loc: self.loc(star) });
        } else {
            let parenthesized = self.eat_op("(").is_some();
            loop {
                let alias_start = self.pos;
                let name = self.expect_name()?.0;
                let asname = if self.eat_kw("as").is_some() { Some(self.expect_name()?.0) } else { None };
                names.push(Alias { name, asname, loc: self.span(alias_start) });
                if self.eat_op(",").is_none() {
                    break;
                }
                if parenthesized && self.at_op(")") {
                    break;
                }
                if !parenthesized && matches!(self.kind(), Kind::Newline | Kind::EndMarker) {
                    return Err(self.demote(self.error("trailing comma not allowed without surrounding parentheses")));
                }
            }
            if parenthesized {
                self.expect_op(")")?;
            }
        }
        Ok(stmt(StmtKind::ImportFrom { module, names, level }, self.span(start)))
    }

    fn type_alias(&mut self) -> PResult<Stmt> {
        let start = self.bump();
        let (name, name_index) = self.expect_name()?;
        let name = Box::new(name_node(name, ExprContext::Store, self.loc(name_index)));
        let type_params = self.optional_type_params()?;
        self.expect_op("=")?;
        let value = Box::new(self.expression()?);
        Ok(stmt(StmtKind::TypeAlias { name, type_params, value }, self.span(start)))
    }

    fn assignment_value(&mut self) -> PResult<Expr> {
        if self.at_kw("yield") {
            self.yield_expression()
        } else {
            self.star_expressions()
        }
    }

    fn expression_statement(&mut self) -> PResult<Stmt> {
        let start = self.pos;
        if self.invalid_mode {
            self.check_named_expression(true)?;
        }
        let first = self.assignment_value()?;
        if self.at_op(":") {
            return self.annotated_assignment(start, first);
        }
        if self.kind() == Kind::Op {
            if let Some(op) = augmented_operator(self.code()) {
                let mut target = first;
                if !matches!(target.kind, ExprKind::Name { .. } | ExprKind::Attribute { .. } | ExprKind::Subscript { .. }) {
                    let message = format!("'{}' is an illegal expression for augmented assignment", expr_name(&target));
                    return Err(self.demote(self.specific(target.loc, &message)));
                }
                set_context(&mut target, ExprContext::Store).map_err(|error| self.demote(error))?;
                self.bump();
                let value = Box::new(self.assignment_value()?);
                return Ok(stmt(StmtKind::AugAssign { target: Box::new(target), op, value }, self.span(start)));
            }
        }
        if self.at_op("=") {
            let mut targets = vec![first];
            while self.eat_op("=").is_some() {
                targets.push(self.assignment_value()?);
            }
            let value = Box::new(targets.pop().expect("an assignment has a value"));
            for target in targets.iter_mut() {
                set_context(target, ExprContext::Store).map_err(|error| self.demote(error))?;
            }
            return Ok(stmt(StmtKind::Assign { targets, value, type_comment: None }, self.span(start)));
        }
        Ok(stmt(StmtKind::Expr { value: Box::new(first) }, self.span(start)))
    }

    fn annotated_assignment(&mut self, start: usize, mut target: Expr) -> PResult<Stmt> {
        let parenthesized = self.op_at(start, "(");
        let simple = i64::from(matches!(target.kind, ExprKind::Name { .. }) && !parenthesized);
        if !matches!(target.kind, ExprKind::Name { .. } | ExprKind::Attribute { .. } | ExprKind::Subscript { .. }) {
            let message = match target.kind {
                ExprKind::Tuple { .. } => "only single target (not tuple) can be annotated",
                ExprKind::List { .. } => "only single target (not list) can be annotated",
                _ => "illegal target for annotation",
            };
            // CPython's (second-pass) rule needs an annotation after the
            // ':'; its first pass fails at the ':' itself.
            if !self.invalid_mode {
                return Err(self.error("invalid syntax"));
            }
            let colon = self.pos;
            self.bump();
            let has_annotation = self.expression().is_ok();
            self.pos = colon;
            if !has_annotation {
                return Err(self.error("invalid syntax"));
            }
            return Err(self.demote(self.specific(self.loc(start), message)));
        }
        set_context(&mut target, ExprContext::Store).map_err(|error| self.demote(error))?;
        self.bump();
        let annotation = Box::new(self.expression()?);
        let value = if self.eat_op("=").is_some() { Some(Box::new(self.assignment_value()?)) } else { None };
        Ok(stmt(StmtKind::AnnAssign { target: Box::new(target), annotation, value, simple }, self.span(start)))
    }

    // ----- compound statements ------------------------------------------

    fn compound_statement(&mut self) -> PResult<Option<Stmt>> {
        if self.at_op("@") {
            return self.decorated().map(Some);
        }
        if self.kind() != Kind::Name {
            return Ok(None);
        }
        let node = match self.code() {
            IF => self.if_statement()?,
            WHILE => self.while_statement()?,
            FOR => self.for_statement(self.pos, false)?,
            WITH => self.with_statement(self.pos, false)?,
            TRY => self.try_statement()?,
            DEF => self.function_def(self.pos, false, Vec::new())?,
            CLASS => self.class_def(Vec::new())?,
            ASYNC => {
                let start = self.bump();
                match if self.kind() == Kind::Name { self.code() } else { 0 } {
                    DEF => self.function_def(start, true, Vec::new())?,
                    FOR => self.for_statement(start, true)?,
                    WITH => self.with_statement(start, true)?,
                    _ => return Err(self.error("invalid syntax")),
                }
            }
            MATCH => match self.try_match_statement()? {
                Some(node) => node,
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        Ok(Some(node))
    }

    fn decorated(&mut self) -> PResult<Stmt> {
        let mut decorators = Vec::new();
        while self.eat_op("@").is_some() {
            decorators.push(self.named_expression()?);
            if self.kind() != Kind::Newline {
                return Err(self.error("invalid syntax"));
            }
            self.bump();
        }
        if self.at_kw("class") {
            return self.class_def(decorators);
        }
        let start = self.pos;
        let is_async = self.eat_kw("async").is_some();
        if !self.at_kw("def") {
            return Err(self.error("invalid syntax"));
        }
        self.function_def(start, is_async, decorators)
    }

    /// End of the last real token consumed: a compound statement ends
    /// where its final block's last line ends (a trailing ';' included).
    pub fn last_content_end(&self) -> Loc {
        let mut index = self.pos;
        while index > 0 {
            index -= 1;
            if !matches!(self.kinds[index], Kind::Newline | Kind::Indent | Kind::Dedent) {
                return self.locs[index];
            }
        }
        Loc::default()
    }

    fn if_statement(&mut self) -> PResult<Stmt> {
        let start = self.bump();
        let what = if self.kw_at(start, "elif") { "'elif' statement" } else { "'if' statement" };
        let test = Box::new(self.named_expression()?);
        self.expect_colon(false)?;
        let body = self.block(what, start)?;
        let orelse = self.else_chain()?;
        let end = self.last_content_end();
        Ok(stmt(StmtKind::If { test, body, orelse }, self.loc(start).to(end)))
    }

    fn else_chain(&mut self) -> PResult<Vec<Stmt>> {
        if self.at_kw("elif") {
            return Ok(vec![self.if_statement()?]);
        }
        self.else_block()
    }

    fn else_block(&mut self) -> PResult<Vec<Stmt>> {
        if let Some(keyword) = self.eat_kw("else") {
            self.expect_colon(true)?;
            return self.block("'else' statement", keyword);
        }
        Ok(Vec::new())
    }

    fn while_statement(&mut self) -> PResult<Stmt> {
        let start = self.bump();
        let test = Box::new(self.named_expression()?);
        self.expect_colon(false)?;
        let body = self.block("'while' statement", start)?;
        let orelse = self.else_block()?;
        let end = self.last_content_end();
        Ok(stmt(StmtKind::While { test, body, orelse }, self.loc(start).to(end)))
    }

    fn for_statement(&mut self, start: usize, is_async: bool) -> PResult<Stmt> {
        let keyword = self.expect_kw("for")?;
        let target = Box::new(self.star_targets()?);
        self.expect_kw("in")?;
        let iter = Box::new(self.star_expressions()?);
        self.expect_colon(false)?;
        let body = self.block("'for' statement", keyword)?;
        let orelse = self.else_block()?;
        let end = self.last_content_end();
        let kind = if is_async {
            StmtKind::AsyncFor { target, iter, body, orelse, type_comment: None }
        } else {
            StmtKind::For { target, iter, body, orelse, type_comment: None }
        };
        Ok(stmt(kind, self.loc(start).to(end)))
    }

    fn with_item(&mut self) -> PResult<WithItem> {
        let context_expr = Box::new(self.expression()?);
        let optional_vars = if self.eat_kw("as").is_some() { Some(Box::new(self.star_target()?)) } else { None };
        Ok(WithItem { context_expr, optional_vars })
    }

    fn with_statement(&mut self, start: usize, is_async: bool) -> PResult<Stmt> {
        let keyword = self.expect_kw("with")?;
        let mut items = None;
        if self.at_op("(") {
            let saved = self.pos;
            items = self.parenthesized_with_items().ok().flatten();
            if items.is_none() {
                self.pos = saved;
            }
        }
        let items = match items {
            Some(items) => items,
            None => {
                let mut items = vec![self.with_item()?];
                while self.eat_op(",").is_some() {
                    items.push(self.with_item()?);
                }
                items
            }
        };
        self.expect_colon(false)?;
        let body = self.block("'with' statement", keyword)?;
        let end = self.last_content_end();
        let kind = if is_async {
            StmtKind::AsyncWith { items, body, type_comment: None }
        } else {
            StmtKind::With { items, body, type_comment: None }
        };
        Ok(stmt(kind, self.loc(start).to(end)))
    }

    fn parenthesized_with_items(&mut self) -> PResult<Option<Vec<WithItem>>> {
        self.bump();
        let mut items = vec![self.with_item()?];
        while self.eat_op(",").is_some() {
            if self.at_op(")") {
                break;
            }
            items.push(self.with_item()?);
        }
        if self.eat_op(")").is_none() || !self.at_op(":") {
            return Ok(None);
        }
        Ok(Some(items))
    }

    fn try_statement(&mut self) -> PResult<Stmt> {
        let start = self.bump();
        self.expect_colon(true)?;
        let body = self.block("'try' statement", start)?;
        let mut handlers = Vec::new();
        let mut star = false;
        while self.at_kw("except") {
            let handler_start = self.bump();
            let mut what = "'except' statement";
            if self.eat_op("*").is_some() {
                star = true;
                what = "'except*' statement";
            }
            let (mut type_, mut name) = (None, None);
            if !self.at_op(":") {
                let first_start = self.pos;
                let first = self.expression()?;
                if self.at_op(",") {
                    if self.version >= Version::Py314 && !self.except_types_need_as() {
                        type_ = Some(Box::new(self.except_types_tuple(first_start, first)?));
                    } else {
                        return Err(self.multiple_except_types_error(&first));
                    }
                } else {
                    type_ = Some(Box::new(first));
                    if let Some(keyword) = self.eat_kw("as") {
                        if self.version >= Version::Py314 && !(self.kind() == Kind::Name && self.kind_at(self.pos + 1) == Kind::Op && self.text_at(self.pos + 1) == ":") {
                            return Err(self.except_target_error(keyword, star));
                        }
                        name = Some(self.expect_name()?.0);
                    }
                }
            }
            self.expect_colon(false)?;
            let handler_body = self.block(what, handler_start)?;
            let loc = self.loc(handler_start).to(self.last_content_end());
            handlers.push(ExceptHandler { type_, name, body: handler_body, loc });
        }
        let orelse = self.else_block()?;
        let finalbody = if let Some(keyword) = self.eat_kw("finally") {
            self.expect_colon(true)?;
            self.block("'finally' statement", keyword)?
        } else {
            Vec::new()
        };
        if handlers.is_empty() && finalbody.is_empty() {
            return Err(self.demote(self.error("expected 'except' or 'finally' block")));
        }
        let end = self.last_content_end();
        let kind = if star {
            StmtKind::TryStar { body, handlers, orelse, finalbody }
        } else {
            StmtKind::Try { body, handlers, orelse, finalbody }
        };
        Ok(stmt(kind, self.loc(start).to(end)))
    }

    /// Whether `, more ... as NAME :` follows an except type: 3.14 accepts
    /// unparenthesized types only without `as`.
    fn except_types_need_as(&self) -> bool {
        let mut index = self.pos;
        let mut level = 0u32;
        while index < self.kinds.len() {
            match self.kinds[index] {
                Kind::Newline | Kind::EndMarker => return false,
                Kind::Op => match self.text_at(index) {
                    "(" | "[" | "{" => level += 1,
                    ")" | "]" | "}" => level = level.saturating_sub(1),
                    ":" if level == 0 => return false,
                    _ => {}
                },
                Kind::Name if level == 0 && self.text_at(index) == "as" => return true,
                _ => {}
            }
            index += 1;
        }
        false
    }

    /// Python 3.14's `expressions` after an except: a Tuple of the types.
    fn except_types_tuple(&mut self, start: usize, first: Expr) -> PResult<Expr> {
        let mut elts = vec![first];
        while self.eat_op(",").is_some() {
            if self.at_op(":") {
                break;
            }
            elts.push(self.expression()?);
        }
        Ok(expr(ExprKind::Tuple { elts, ctx: ExprContext::Load }, self.span(start)))
    }

    /// CPython's invalid_except_stmt: `except A, B [as NAME]:` before 3.14,
    /// and `except A, B as NAME:` in 3.14, report unparenthesized types.
    fn multiple_except_types_error(&mut self, first: &Expr) -> ParseError {
        let generic = self.error("invalid syntax");
        if !self.invalid_mode {
            return generic;
        }
        let saved = self.pos;
        let mut matched = self.eat_op(",").is_some() && self.expressions_for_error();
        let with_as = matched && self.eat_kw("as").is_some();
        if with_as {
            matched = self.kind() == Kind::Name && { self.bump(); true };
        } else if self.version >= Version::Py314 {
            matched = false;
        }
        matched = matched && self.at_op(":");
        self.pos = saved;
        if !matched {
            return generic;
        }
        let message = if self.version >= Version::Py314 {
            "multiple exception types must be parenthesized when using 'as'"
        } else {
            "multiple exception types must be parenthesized"
        };
        self.specific(first.loc, message)
    }

    /// `expressions` for an error check: whether one or more expressions
    /// (with an optional trailing comma) parse here.
    fn expressions_for_error(&mut self) -> bool {
        if self.expression().is_err() {
            return false;
        }
        while self.at_op(",") {
            self.bump();
            if self.at_op(":") || self.at_kw("as") {
                break;
            }
            if self.expression().is_err() {
                return false;
            }
        }
        true
    }

    /// Python 3.14's `except E as <expression>:` with a target that is not a
    /// plain name.
    fn except_target_error(&mut self, keyword: usize, star: bool) -> ParseError {
        let generic = self.error("invalid syntax");
        if !self.invalid_mode {
            return generic;
        }
        let saved = self.pos;
        let target = self.expression();
        let matched = target.is_ok() && self.at_op(":");
        self.pos = saved;
        let _ = keyword;
        match target {
            Ok(target) if matched => {
                let what = if star { "except*" } else { "except" };
                self.specific(target.loc, &format!("cannot use {what} statement with {}", expr_name(&target)))
            }
            _ => generic,
        }
    }

    /// CPython's optional `[type_params]`: a generic failure backtracks to
    /// no type parameters (the following token then reports the error, as
    /// 3.12's forced `(` does); specific errors are reported as they are.
    pub fn optional_type_params(&mut self) -> PResult<Vec<TypeParam>> {
        let start = self.pos;
        match self.type_params() {
            Err(error) if error.message == "invalid syntax" => {
                self.pos = start;
                Ok(Vec::new())
            }
            result => result,
        }
    }

    pub fn type_params(&mut self) -> PResult<Vec<TypeParam>> {
        let mut params = Vec::new();
        if self.eat_op("[").is_none() {
            return Ok(params);
        }
        loop {
            let start = self.pos;
            let kind = if self.eat_op("*").is_some() {
                let name = self.expect_name()?.0;
                TypeParamKind::TypeVarTuple { name, default_value: self.type_param_default(true)? }
            } else if self.eat_op("**").is_some() {
                let name = self.expect_name()?.0;
                TypeParamKind::ParamSpec { name, default_value: self.type_param_default(false)? }
            } else {
                let name = self.expect_name()?.0;
                let bound = if self.eat_op(":").is_some() { Some(Box::new(self.expression()?)) } else { None };
                TypeParamKind::TypeVar { name, bound, default_value: self.type_param_default(false)? }
            };
            params.push(TypeParam { kind, loc: self.span(start) });
            if self.eat_op(",").is_none() || self.at_op("]") {
                break;
            }
        }
        self.expect_op("]")?;
        Ok(params)
    }

    /// A type parameter default (`= expression`, `= *expression` for a
    /// TypeVarTuple), part of the grammar since Python 3.13.
    fn type_param_default(&mut self, starred: bool) -> PResult<Option<Box<Expr>>> {
        if self.version < Version::Py313 || self.eat_op("=").is_none() {
            return Ok(None);
        }
        let value = if starred { self.star_expression()? } else { self.expression()? };
        Ok(Some(Box::new(value)))
    }

    fn function_def(&mut self, start: usize, is_async: bool, decorator_list: Vec<Expr>) -> PResult<Stmt> {
        let keyword = self.expect_kw("def")?;
        let name = self.expect_name()?.0;
        let type_params = self.optional_type_params()?;
        if !self.at_op("(") {
            return Err(self.error("expected '('"));
        }
        self.bump();
        let args = Box::new(self.parameters(false)?);
        self.expect_op(")")?;
        let mut returns = None;
        if self.at_op("->") {
            let arrow = self.pos;
            self.bump();
            match self.expression() {
                Ok(annotation) => returns = Some(Box::new(annotation)),
                Err(error) if error.message != "invalid syntax" => return Err(error),
                Err(_) => self.pos = arrow,
            }
        }
        self.expect_colon(true)?;
        let body = self.block("function definition", keyword)?;
        let end = self.last_content_end();
        let kind = if is_async {
            StmtKind::AsyncFunctionDef { name, args, body, decorator_list, returns, type_comment: None, type_params }
        } else {
            StmtKind::FunctionDef { name, args, body, decorator_list, returns, type_comment: None, type_params }
        };
        Ok(stmt(kind, self.loc(start).to(end)))
    }

    fn class_def(&mut self, decorator_list: Vec<Expr>) -> PResult<Stmt> {
        let start = self.bump();
        let name = self.expect_name()?.0;
        let type_params = self.optional_type_params()?;
        let (mut bases, mut keywords) = (Vec::new(), Vec::new());
        if self.at_op("(") {
            let (args, kws) = self.call_arguments()?;
            bases = args;
            keywords = kws;
        }
        self.expect_colon(false)?;
        let body = self.block("class definition", start)?;
        let end = self.last_content_end();
        Ok(stmt(StmtKind::ClassDef { name, bases, keywords, body, decorator_list, type_params }, self.loc(start).to(end)))
    }
}

#[inline(always)]
pub fn stmt(kind: StmtKind, loc: Loc) -> Stmt {
    Stmt { kind, loc }
}

/// The name CPython's messages use for an expression kind.
pub fn expr_name(node: &Expr) -> &'static str {
    match &node.kind {
        ExprKind::Attribute { .. } => "attribute",
        ExprKind::Subscript { .. } => "subscript",
        ExprKind::Starred { .. } => "starred",
        ExprKind::Name { .. } => "name",
        ExprKind::List { .. } => "list",
        ExprKind::Tuple { .. } => "tuple",
        ExprKind::Lambda { .. } => "lambda",
        ExprKind::Call { .. } => "function call",
        ExprKind::BoolOp { .. } | ExprKind::BinOp { .. } | ExprKind::UnaryOp { .. } => "expression",
        ExprKind::GeneratorExp { .. } => "generator expression",
        ExprKind::Yield { .. } | ExprKind::YieldFrom { .. } => "yield expression",
        ExprKind::Await { .. } => "await expression",
        ExprKind::ListComp { .. } => "list comprehension",
        ExprKind::SetComp { .. } => "set comprehension",
        ExprKind::DictComp { .. } => "dict comprehension",
        ExprKind::Dict { .. } => "dict literal",
        ExprKind::Set { .. } => "set display",
        ExprKind::JoinedStr { .. } | ExprKind::FormattedValue { .. } => "f-string expression",
        ExprKind::TemplateStr { .. } | ExprKind::Interpolation { .. } => "t-string expression",
        ExprKind::Compare { .. } => "comparison",
        ExprKind::IfExp { .. } => "conditional expression",
        ExprKind::NamedExpr { .. } => "named expression",
        ExprKind::Constant { value, .. } => match value {
            Constant::None => "None",
            Constant::True => "True",
            Constant::False => "False",
            Constant::Ellipsis => "ellipsis",
            _ => "literal",
        },
        _ => "expression",
    }
}

fn augmented_operator(code: u8) -> Option<Operator> {
    Some(match code {
        30 => Operator::Add,
        31 => Operator::Sub,
        32 => Operator::Mult,
        44 => Operator::MatMult,
        33 => Operator::Div,
        34 => Operator::Mod,
        40 => Operator::Pow,
        38 => Operator::LShift,
        39 => Operator::RShift,
        36 => Operator::BitOr,
        37 => Operator::BitXor,
        35 => Operator::BitAnd,
        42 => Operator::FloorDiv,
        _ => return None,
    })
}

/// Give an expression Store or Del context, as an assignment target.
pub fn set_context(node: &mut Expr, context: ExprContext) -> PResult<()> {
    match &mut node.kind {
        ExprKind::Name { ctx, .. } | ExprKind::Attribute { ctx, .. } | ExprKind::Subscript { ctx, .. } => *ctx = context,
        ExprKind::Starred { value, ctx } => {
            set_context(value, context)?;
            *ctx = context;
        }
        ExprKind::List { elts, ctx } | ExprKind::Tuple { elts, ctx } => {
            for item in elts.iter_mut() {
                set_context(item, context)?;
            }
            *ctx = context;
        }
        _ => {
            let loc = node.loc;
            let verb = if context == ExprContext::Del { "delete" } else { "assign to" };
            return Err(ParseError { line: loc.line, col: loc.col, offset: 0, message: format!("cannot {verb} {}", expr_name(node)) });
        }
    }
    Ok(())
}
