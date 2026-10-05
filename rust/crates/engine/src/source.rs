//! Reading source files exactly as the Python engine does (RULES.md:
//! encodings, line splitting, character columns and `# noqa`).

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

static CODING_PATTERN: LazyLock<regex::bytes::Regex> = LazyLock::new(|| {
    regex::bytes::Regex::new(r"(?-u)^[ \t\x0c]*#.*?coding[:=][ \t]*([-\w.]+)")
        .expect("valid coding pattern")
});
static NOQA_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)#\s*noqa(?::\s*(?P<codes>[A-Za-z0-9]+(?:[\s,]+[A-Za-z0-9]+)*))?")
        .expect("valid noqa pattern")
});
static FILE_NOQA_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^#\s*(?:flake8|ruff)\s*:\s*noqa\b(?::\s*(?P<codes>[A-Za-z0-9]+(?:[\s,]+[A-Za-z0-9]+)*))?")
        .expect("valid file noqa pattern")
});
static CODE_SPLIT_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\s,]+").expect("valid split pattern"));
const UTF8_NAMES: [&str; 5] = ["utf-8", "utf8", "utf_8", "utf-8-sig", "utf8-sig"];

/// Suppressions for one line: `None` suppresses every code.
pub type Suppression = Option<Vec<String>>;

/// A decoded file with its physical lines and a byte-offset line index.
pub struct SourceFile<'a> {
    pub text: &'a str,
    pub lines: Vec<&'a str>,
    line_starts: Vec<usize>,
    pub noqa: HashMap<usize, Suppression>,
}

/// Python's str.isspace() for one character (Rust's is_whitespace plus
/// the four ASCII separators U+001C..U+001F that Python also strips).
pub fn is_python_space(character: char) -> bool {
    character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
}

/// Python's str.strip().
pub fn python_strip(text: &str) -> &str {
    text.trim_matches(is_python_space)
}

/// Python's str.lstrip().
pub fn python_strip_start(text: &str) -> &str {
    text.trim_start_matches(is_python_space)
}

/// Number of leading whitespace characters, as len(s) - len(s.lstrip()).
pub fn python_indent(text: &str) -> usize {
    text.chars().take_while(|character| is_python_space(*character)).count()
}

/// The first `count` lines of `raw`, split at \r\n, \r or \n like
/// bytes.splitlines().
fn first_byte_lines(raw: &[u8], count: usize) -> Vec<&[u8]> {
    let mut lines = Vec::with_capacity(count);
    let mut start = 0;
    while lines.len() < count && start < raw.len() {
        match memchr::memchr2(b'\n', b'\r', &raw[start..]) {
            Some(at) => {
                let end = start + at;
                lines.push(&raw[start..end]);
                start = if raw[end] == b'\r' && raw.get(end + 1) == Some(&b'\n') { end + 2 } else { end + 1 };
            }
            None => {
                lines.push(&raw[start..]);
                break;
            }
        }
    }
    lines
}

/// Decode UTF-8 source, or return None for other encodings (RT002).
pub fn decode_source(raw: &[u8]) -> Option<&str> {
    let raw = raw.strip_prefix(b"\xef\xbb\xbf").unwrap_or(raw);
    for line in first_byte_lines(raw, 2) {
        if let Some(captures) = CODING_PATTERN.captures(line) {
            let name = String::from_utf8_lossy(&captures[1]).to_lowercase();
            if !UTF8_NAMES.contains(&name.as_str()) {
                return None;
            }
        }
    }
    let text = std::str::from_utf8(raw).ok()?;
    Some(text.strip_prefix('\u{feff}').unwrap_or(text))
}

impl<'a> SourceFile<'a> {
    /// Split lines at \r\n, \r and \n and index their byte offsets;
    /// `comments` are the byte spans of the comment tokens.
    pub fn new(text: &'a str, comments: &[(usize, usize)]) -> Self {
        let bytes = text.as_bytes();
        let mut lines = Vec::with_capacity(bytes.len() / 32 + 1);
        let mut line_starts = Vec::with_capacity(bytes.len() / 32 + 1);
        line_starts.push(0);
        let mut start = 0;
        for index in memchr::memchr2_iter(b'\n', b'\r', bytes) {
            if index < start {
                continue; // the \n of a \r\n pair
            }
            lines.push(&text[start..index]);
            start = if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') { index + 2 } else { index + 1 };
            line_starts.push(start);
        }
        if start < bytes.len() {
            lines.push(&text[start..]);
        }
        let noqa = parse_noqa(text, &line_starts, comments);
        SourceFile { text, lines, line_starts, noqa }
    }

    /// 1-based line and 1-based character column of a byte offset.
    pub fn locate(&self, offset: usize) -> (usize, usize) {
        let line_index = match self.line_starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index - 1,
        };
        let line_start = self.line_starts[line_index];
        let column = self.text[line_start..offset].chars().count() + 1;
        (line_index + 1, column)
    }

    /// 1-based line and character column of a tree location
    /// (locate_node_tuple).
    pub fn position(&self, loc: refactrail_parser::Loc) -> (usize, usize) {
        let line = loc.line as usize;
        let start = self.line_starts.get(line.saturating_sub(1)).copied().unwrap_or(self.text.len());
        let offset = (start + loc.col as usize).min(self.text.len());
        let next = self.line_starts.get(line).copied().unwrap_or(usize::MAX);
        if line >= 1 && offset < next && start <= offset {
            let prefix = &self.text[start..offset];
            let column = if prefix.is_ascii() { prefix.len() } else { prefix.chars().count() };
            return (line, column + 1);
        }
        self.locate(offset)
    }

    /// locate_definition_name_tuple: the name after `def`/`class` when it
    /// is on the keyword's line, else the definition's own position.
    pub fn definition_name(&self, loc: refactrail_parser::Loc, name: &str) -> (usize, usize) {
        let (line, column) = self.position(loc);
        let Some(text) = self.lines.get(line - 1) else { return (line, column) };
        let tail = text.char_indices().nth(column - 1).map_or("", |(at, _)| &text[at..]);
        // `^(?:async\s+)?(?:def|class)\s+` followed by the name.
        let skip_space = |rest: &str| -> usize { rest.len() - rest.trim_start_matches(char::is_whitespace).len() };
        let mut keyword_end = 0;
        if let Some(rest) = tail.strip_prefix("async") {
            let space = skip_space(rest);
            if space > 0 {
                keyword_end = 5 + space;
            }
        }
        let rest = &tail[keyword_end..];
        let keyword = if rest.starts_with("def") { 3 } else if rest.starts_with("class") { 5 } else { return (line, column) };
        let space = skip_space(&rest[keyword..]);
        if space == 0 {
            return (line, column);
        }
        let end = keyword_end + keyword + space;
        if tail[end..].starts_with(name) {
            return (line, column + tail[..end].chars().count());
        }
        (line, column)
    }
}

/// Split a noqa code list into upper-case codes.
fn split_codes(codes: &str) -> Vec<String> {
    CODE_SPLIT_PATTERN.split(codes).filter(|code| !code.is_empty()).map(str::to_uppercase).collect()
}

/// Merge a file-wide exemption into line 0: a blanket one wins, listed
/// codes accumulate (the Python engine's record_file_noqa_none).
fn record_file_noqa(noqa: &mut HashMap<usize, Suppression>, codes: Option<&str>) {
    match (codes, noqa.get_mut(&0)) {
        (None, _) => {
            noqa.insert(0, None);
        }
        (Some(_), Some(None)) => {}
        (Some(listed), Some(Some(existing))) => {
            for code in split_codes(listed) {
                if !existing.contains(&code) {
                    existing.push(code);
                }
            }
        }
        (Some(listed), None) => {
            noqa.insert(0, Some(split_codes(listed)));
        }
    }
}

/// Whether a code is suppressed on a line (or for the whole file).
pub fn is_suppressed(noqa: &HashMap<usize, Suppression>, line: usize, code: &str) -> bool {
    [0, line].iter().any(|key| match noqa.get(key) {
        Some(None) => true,
        Some(Some(codes)) => codes.iter().any(|prefix| code.starts_with(prefix.as_str())),
        None => false,
    })
}

/// Map line numbers to suppressed codes (None = all codes); line 0 holds
/// file-wide exemptions ("# ruff: noqa", "# flake8: noqa").
fn parse_noqa(text: &str, line_starts: &[usize], comments: &[(usize, usize)]) -> HashMap<usize, Suppression> {
    let mut noqa = HashMap::new();
    for &(start, end) in comments {
        let comment = &text[start..end];
        // Cheap pre-check: the pattern needs "noqa" (any case).
        if !comment.as_bytes().windows(4).any(|window| window.eq_ignore_ascii_case(b"noqa")) {
            continue;
        }
        if let Some(captures) = FILE_NOQA_PATTERN.captures(comment) {
            record_file_noqa(&mut noqa, captures.name("codes").map(|codes| codes.as_str()));
            continue;
        }
        let Some(captures) = NOQA_PATTERN.captures(comment) else { continue; };
        let line = line_starts.partition_point(|position| *position <= start);
        let codes = captures.name("codes").map(|codes| split_codes(codes.as_str()));
        noqa.insert(line, codes);
    }
    noqa
}
