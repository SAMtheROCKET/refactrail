//! Wrapping for the formatter (mirror of format_wrapping.py and
//! format_statements.py): split long lines at existing brackets, then lay
//! out still-long statements as a whole. Only gaps between existing
//! tokens change; widths count characters, as Python's len() does.

use crate::format::{build_token_index, line_tokens, normalize_newlines, protected_lines, split_line_ends, FormatError, Tok, TokenIndex};
use refactrail_lexer::Kind;
use refactrail_parser::fast_hash::{FastMap, FastSet};

const CONTINUATION: &str = "    ";
const HEADER_KEYWORDS: [&str; 11] = ["if", "elif", "while", "for", "with", "def", "class", "async", "except", "match", "case"];
const COMPARISONS: [&str; 9] = ["<", ">", "==", "!=", "<=", ">=", "in", "is", "not"];
const ARITHMETIC: [&str; 13] = ["+", "-", "*", "/", "//", "%", "@", "**", "|", "&", "^", "<<", ">>"];
const KEYWORDS: [&str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or",
    "pass", "raise", "return", "try", "while", "with", "yield",
];

fn width_of(text: &str) -> usize {
    if text.is_ascii() { text.len() } else { text.chars().count() }
}

/// Characters [start, end) of a line (character columns).
fn char_slice(line: &str, start: usize, end: usize) -> &str {
    if line.is_ascii() {
        let end = end.min(line.len());
        return &line[start.min(end)..end];
    }
    let byte = |column: usize| line.char_indices().nth(column).map_or(line.len(), |(at, _)| at);
    let (start, end) = (byte(start), byte(end));
    &line[start.min(end)..end]
}

fn strip(text: &str) -> &str {
    crate::source::python_strip(text)
}

fn leading(text: &str, characters: &[char]) -> usize {
    text.len() - text.trim_start_matches(characters).len()
}

fn longest(lines: &[String]) -> usize {
    lines.iter().map(|line| width_of(line)).max().unwrap_or(0)
}

fn is_open(text: &str) -> bool {
    matches!(text, "(" | "[" | "{")
}

fn is_close(text: &str) -> bool {
    matches!(text, ")" | "]" | "}")
}

// ----- bracket-group splitting (format_wrapping.py) -----------------------

struct Group {
    open: Tok,
    close: Tok,
    commas: Vec<Tok>,
    opaque: bool,
    depth: usize,
}

/// collect_groups_list: non-empty bracket groups on one line.
fn collect_groups(tokens: &[Tok]) -> Vec<Group> {
    let mut stack: Vec<(Tok, Vec<Tok>, bool)> = Vec::new();
    let mut groups = Vec::new();
    for token in tokens {
        let text = token.text.as_str();
        if is_open(text) {
            stack.push((token.clone(), Vec::new(), false));
        } else if is_close(text) && !stack.is_empty() {
            let (open, commas, opaque) = stack.pop().expect("an open bracket");
            if token.start.1 > open.end.1 {
                groups.push(Group { open, close: token.clone(), commas, opaque, depth: stack.len() });
            }
        } else if let Some(top) = stack.last_mut() {
            if text == "," {
                top.1.push(token.clone());
            }
            if text == "for" || text == "lambda" {
                top.2 = true;
            }
        }
    }
    groups
}

/// `\s*(?:async\s+)?(?:def|class)\b` at the start of a line.
fn is_definition_header(line: &str) -> bool {
    let rest = crate::source::python_strip_start(line);
    let rest = match rest.strip_prefix("async") {
        Some(after) if after.starts_with(crate::source::is_python_space) => crate::source::python_strip_start(after),
        _ => rest,
    };
    for word in ["def", "class"] {
        if let Some(after) = rest.strip_prefix(word) {
            if !after.chars().next().is_some_and(crate::lexical::is_word_char) {
                return true;
            }
        }
    }
    false
}

fn order_groups(mut groups: Vec<Group>, header: bool) -> Vec<Group> {
    let side: i64 = if header { 1 } else { -1 };
    groups.sort_by_key(|group| (group.depth, side * i64::from(group.open.start.1)));
    groups
}

/// collect_single_lines_set: lines holding one complete logical line.
fn single_lines(index: &TokenIndex) -> FastSet<u32> {
    let mut lines = FastSet::default();
    let mut start: Option<u32> = None;
    for token in &index.tokens {
        if matches!(token.kind, Kind::Nl | Kind::Comment | Kind::Indent | Kind::Dedent) {
            continue;
        }
        let first = *start.get_or_insert(token.start.0);
        if token.kind == Kind::Newline {
            if token.start.0 == first {
                lines.insert(first);
            }
            start = None;
        }
    }
    lines
}

/// collect_open_lines_set: lines starting inside an earlier bracket.
fn open_lines(index: &TokenIndex) -> FastSet<u32> {
    let mut lines = FastSet::default();
    let (mut depth, mut line) = (0usize, 0u32);
    for token in &index.tokens {
        if token.start.0 != line {
            line = token.start.0;
            if depth > 0 {
                lines.insert(line);
            }
        }
        if token.kind == Kind::Op {
            if is_open(&token.text) {
                depth += 1;
            } else if is_close(&token.text) {
                depth = depth.saturating_sub(1);
            }
        }
    }
    lines
}

/// pack_commas_list: re-pack a continuation line at its own commas.
fn pack_commas(line: &str, tokens: &[Tok], width: usize) -> Vec<String> {
    let indent = &line[..leading(line, &[' '])];
    let mut depth: i64 = 0;
    let mut cuts = Vec::new();
    for token in tokens {
        if is_open(&token.text) {
            depth += 1;
        } else if is_close(&token.text) {
            depth -= 1;
            if depth < 0 {
                break;
            }
        } else if token.text == "," && depth == 0 {
            cuts.push(token.end.1 as usize);
        }
    }
    let total = width_of(line);
    let mut pieces = Vec::new();
    let mut start = width_of(indent);
    for cut in cuts.into_iter().chain(std::iter::once(total)) {
        let piece = strip(char_slice(line, start, cut));
        if !piece.is_empty() {
            pieces.push(piece);
        }
        start = cut;
    }
    let Some(first) = pieces.first() else { return vec![line.to_string()] };
    let mut lines = vec![format!("{indent}{first}")];
    for piece in &pieces[1..] {
        let last = lines.last_mut().expect("a line");
        if width_of(last) + 1 + width_of(piece) <= width {
            last.push(' ');
            last.push_str(piece);
        } else {
            lines.push(format!("{indent}{piece}"));
        }
    }
    if longest(&lines) < total { lines } else { vec![line.to_string()] }
}

/// split_group_list: head, content (or one element per line), tail.
fn split_group(line: &str, group: &Group, width: usize) -> Vec<String> {
    let indent = &line[..leading(line, &[' ', '\t'])];
    if indent.contains('\t') {
        return vec![line.to_string()];
    }
    let inner = format!("{indent}{CONTINUATION}");
    let (open_end, close_start) = (group.open.end.1 as usize, group.close.start.1 as usize);
    let head = char_slice(line, 0, open_end).trim_end_matches(crate::source::is_python_space).to_string();
    let tail = format!("{indent}{}", char_slice(line, close_start, usize::MAX));
    let content = format!("{inner}{}", strip(char_slice(line, open_end, close_start)));
    if width_of(&content) <= width || group.commas.is_empty() || group.opaque {
        return vec![head, content, tail];
    }
    let mut segments = vec![head];
    let mut start = open_end;
    for comma in &group.commas {
        let end = comma.end.1 as usize;
        segments.push(format!("{inner}{}", strip(char_slice(line, start, end))));
        start = end;
    }
    let remainder = strip(char_slice(line, start, close_start));
    if !remainder.is_empty() {
        segments.push(format!("{inner}{remainder}"));
    }
    segments.push(tail);
    segments
}

/// split_line_list: the first group split that shortens the line.
fn split_line(line: &str, groups: Vec<Group>, width: usize) -> Vec<String> {
    let header = is_definition_header(line);
    let length = width_of(line);
    for group in order_groups(groups, header) {
        let lines = split_group(line, &group, width);
        if longest(&lines) < length {
            return lines;
        }
    }
    vec![line.to_string()]
}

/// collect_line_tokens_list: NAME and OP tokens of import names.
fn import_name_tokens(names: &str) -> Result<Vec<Tok>, FormatError> {
    let stripped = crate::source::python_strip_start(names);
    let offset = width_of(&names[..names.len() - stripped.len()]) as u32;
    let source = format!("{stripped}\n");
    let tokens = refactrail_lexer::tokenize(&source)
        .map_err(|error| FormatError::Syntax { line: error.line as usize, message: error.message })?;
    Ok(tokens
        .iter()
        .filter(|token| matches!(token.kind, Kind::Name | Kind::Op))
        .map(|token| Tok {
            kind: token.kind,
            text: token.text(&source).to_string(),
            start: (1, token.start_pos.1 + offset),
            end: (1, token.end_pos.1 + offset),
        })
        .collect())
}

/// parenthesize_import_list: wrap `from module import a, b` in brackets.
fn parenthesize_import(line: &str, tokens: &[Tok], width: usize) -> Result<Vec<String>, FormatError> {
    let spellings: Vec<&str> = tokens.iter().map(|token| token.text.as_str()).collect();
    if spellings.first() != Some(&"from") || !spellings.contains(&"import") || spellings.contains(&"(") || spellings.contains(&"*") {
        return Ok(vec![line.to_string()]);
    }
    let import = &tokens[spellings.iter().position(|text| *text == "import").expect("import")];
    let indent = &line[..leading(line, &[' '])];
    let names = format!("{indent}{CONTINUATION}{}", strip(char_slice(line, import.end.1 as usize, usize::MAX)));
    let packed = pack_commas(&names, &import_name_tokens(&names)?, width);
    let mut lines = vec![format!("{} (", char_slice(line, 0, import.end.1 as usize))];
    lines.extend(packed);
    lines.push(format!("{indent})"));
    Ok(lines)
}

/// wrap_once_str: split eligible long lines once.
fn wrap_once(text: &str, width: usize, hug: bool) -> Result<String, FormatError> {
    if !(40..=200).contains(&width) {
        return Err(FormatError::Value("Line length must be a whole number from 40 to 200".into()));
    }
    let normalized = normalize_newlines(text);
    let tree = refactrail_parser::parse(&normalized)
        .map_err(|error| FormatError::Syntax { line: error.line as usize, message: error.message })?;
    let index = build_token_index(&normalized)?;
    if crate::format::has_type_ignores(&index) {
        return Ok(text.to_string());
    }
    let protected = protected_lines(&index, &tree, true);
    let tokens = line_tokens(&index);
    let (single, open) = (single_lines(&index), open_lines(&index));
    let pieces = split_line_ends(text);
    let mut out = String::with_capacity(text.len() + 64);
    for (piece_index, piece) in pieces.iter().enumerate() {
        let number = (piece_index / 2 + 1) as u32;
        if piece_index % 2 == 1 || width_of(piece) <= width || protected.contains(&number) {
            out.push_str(piece);
            continue;
        }
        let ending = pieces.get(piece_index + 1).copied().unwrap_or("\n");
        let empty = Vec::new();
        let line_tokens = tokens.get(&number).unwrap_or(&empty);
        let lines = if single.contains(&number) {
            let lines = parenthesize_import(piece, line_tokens, width)?;
            if lines.len() == 1 && lines[0] == *piece && !hug {
                split_line(piece, collect_groups(line_tokens), width)
            } else {
                lines
            }
        } else if open.contains(&number) {
            pack_commas(piece, line_tokens, width)
        } else {
            vec![piece.to_string()]
        };
        out.push_str(&lines.join(ending));
    }
    Ok(out)
}

/// wrap_source_str: bracket splitting to a fixed point, then layout.
pub fn wrap_source(text: &str, width: usize, hug: bool) -> Result<String, FormatError> {
    let mut current = text.to_string();
    for _ in 0..200 {
        let output = wrap_once(&current, width, hug)?;
        if output == current {
            return layout_long_statements(&output, width, hug);
        }
        current = output;
    }
    Err(FormatError::Value("Bracket wrapping exceeded its 200-pass resource limit".into()))
}

/// layout_long_statements_str: whole-statement layout per statement.
fn layout_long_statements(text: &str, width: usize, hug: bool) -> Result<String, FormatError> {
    let pieces = split_line_ends(text);
    let mut lines: Vec<String> = pieces.iter().step_by(2).map(|line| line.to_string()).collect();
    let mut endings: Vec<String> = pieces.iter().skip(1).step_by(2).map(|ending| ending.to_string()).collect();
    endings.push(String::new());
    let normalized = lines.join("\n");
    let tree = refactrail_parser::parse(&normalized)
        .map_err(|error| FormatError::Syntax { line: error.line as usize, message: error.message })?;
    let index = build_token_index(&normalized)?;
    if crate::format::has_type_ignores(&index) {
        return Ok(text.to_string());
    }
    let protected = protected_lines(&index, &tree, false);
    for (first, last, new) in plan_layouts(&index, &protected, width, hug) {
        let ending = if endings[first - 1].is_empty() { "\n".to_string() } else { endings[first - 1].clone() };
        let last_ending = endings[last - 1].clone();
        let count = new.len();
        lines.splice(first - 1..last, new);
        let mut new_endings = vec![ending; count - 1];
        new_endings.push(last_ending);
        endings.splice(first - 1..last, new_endings);
    }
    Ok(lines.iter().zip(&endings).map(|(line, ending)| format!("{line}{ending}")).collect())
}

// ----- whole-statement layout (format_statements.py) ----------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum AtomKind {
    Open,
    Close,
    Comma,
    Op,
    Word,
    Str,
}

#[derive(Clone, Debug)]
struct Atom {
    text: String,
    space: bool,
    kind: AtomKind,
    depth: i64,
}

fn classify(token: &Tok) -> AtomKind {
    if is_open(&token.text) {
        AtomKind::Open
    } else if is_close(&token.text) {
        AtomKind::Close
    } else if token.text == "," {
        AtomKind::Comma
    } else if token.kind == Kind::Op {
        AtomKind::Op
    } else if token.kind == Kind::String {
        AtomKind::Str
    } else {
        AtomKind::Word
    }
}

fn decide_space(previous: Option<&Atom>, kind: AtomKind, text: &str, gap: Option<i64>) -> bool {
    let Some(previous) = previous else { return false };
    if let Some(gap) = gap {
        return gap > 0;
    }
    !(previous.kind == AtomKind::Open
        || previous.text == "."
        || matches!(kind, AtomKind::Close | AtomKind::Comma)
        || text == "."
        || text == ":")
}

/// build_atoms_list: a statement's atoms, or None to leave it alone.
fn build_atoms(tokens: &[&Tok], lines: &[&str]) -> Option<Vec<Atom>> {
    let mut atoms: Vec<Atom> = Vec::new();
    let mut depth = 0i64;
    let mut previous_end: Option<(u32, u32)> = None;
    let mut fstring_start: Option<(u32, u32)> = None;
    let mut nesting = 0;
    for &token in tokens {
        if token.kind == Kind::Comment {
            return None;
        }
        if matches!(token.kind, Kind::Nl | Kind::Newline | Kind::Indent | Kind::Dedent | Kind::EndMarker) {
            continue;
        }
        if matches!(token.kind, Kind::FStringStart | Kind::TStringStart) {
            nesting += 1;
            fstring_start.get_or_insert(token.start);
            continue;
        }
        let combined;
        let token = if nesting > 0 {
            if matches!(token.kind, Kind::FStringEnd | Kind::TStringEnd) {
                nesting -= 1;
            }
            if nesting > 0 {
                continue;
            }
            let start = fstring_start.take().expect("an f-string start");
            if start.0 != token.end.0 {
                return None;
            }
            let text = char_slice(lines[start.0 as usize - 1], start.1 as usize, token.end.1 as usize).to_string();
            combined = Tok { kind: Kind::String, text, start, end: token.end };
            &combined
        } else {
            token
        };
        if token.start.0 != token.end.0 {
            return None;
        }
        let gap = match previous_end {
            Some(end) if end.0 == token.start.0 => Some(i64::from(token.start.1) - i64::from(end.1)),
            Some(_) if depth == 0 => return None,
            _ => None,
        };
        let kind = classify(token);
        let space = decide_space(atoms.last(), kind, &token.text, gap);
        atoms.push(Atom { text: token.text.clone(), space, kind, depth });
        depth += match kind {
            AtomKind::Open => 1,
            AtomKind::Close => -1,
            _ => 0,
        };
        previous_end = Some(token.end);
    }
    Some(atoms)
}

fn join_atoms(atoms: &[Atom]) -> String {
    let mut out = String::new();
    for (index, atom) in atoms.iter().enumerate() {
        if atom.space && index > 0 {
            out.push(' ');
        }
        out.push_str(&atom.text);
    }
    out
}

/// find_pairs_list: outermost bracket pairs (relative indexes).
fn find_pairs(atoms: &[Atom]) -> Vec<(usize, usize)> {
    let base = atoms.iter().map(|atom| atom.depth).min().unwrap_or(0);
    let mut pairs = Vec::new();
    let mut open = None;
    for (index, atom) in atoms.iter().enumerate() {
        if atom.kind == AtomKind::Open && atom.depth == base {
            open = Some(index);
        } else if atom.kind == AtomKind::Close && atom.depth == base + 1 {
            if let Some(start) = open.take() {
                pairs.push((start, index));
            }
        }
    }
    pairs
}

fn is_binary_operator(atoms: &[Atom], index: usize) -> bool {
    if index == 0 {
        return false;
    }
    let previous = &atoms[index - 1];
    matches!(previous.kind, AtomKind::Word | AtomKind::Str | AtomKind::Close)
        && (!KEYWORDS.contains(&previous.text.as_str()) || matches!(previous.text.as_str(), "True" | "False" | "None"))
}

fn rank_delimiter(atoms: &[Atom], index: usize, first_for: Option<usize>) -> u32 {
    let atom = &atoms[index];
    let text = atom.text.as_str();
    if atom.kind == AtomKind::Comma {
        return 18;
    }
    if index == 0 {
        return 0;
    }
    if text == "for" || (text == "async" && atoms.get(index + 1).is_some_and(|next| next.text == "for")) {
        return 20;
    }
    if text == "if" {
        return if first_for.is_some_and(|first| index > first) { 20 } else { 16 };
    }
    match text {
        "else" => return 16,
        "or" => return 14,
        "and" => return 13,
        _ => {}
    }
    if atom.kind == AtomKind::Str && atoms[index - 1].kind == AtomKind::Str {
        return 12;
    }
    if !is_binary_operator(atoms, index) {
        return 0;
    }
    if COMPARISONS.contains(&text) {
        return 10;
    }
    if ARITHMETIC.contains(&text) { 8 } else { 0 }
}

/// collect_delimiters_tuple: the highest-priority split points.
fn collect_delimiters(atoms: &[Atom]) -> (u32, Vec<usize>) {
    let base = atoms.iter().map(|atom| atom.depth).min().unwrap_or(0);
    let level: Vec<usize> = (0..atoms.len()).filter(|&index| atoms[index].depth == base).collect();
    let first_for = level.iter().copied().find(|&index| atoms[index].text == "for");
    let mut ranked: FastMap<u32, Vec<usize>> = FastMap::default();
    let mut in_lambda = false;
    for &index in &level {
        let text = atoms[index].text.as_str();
        if text == "lambda" {
            in_lambda = true;
        } else if text == ":" && in_lambda {
            in_lambda = false;
        } else if !(in_lambda && atoms[index].kind == AtomKind::Comma) {
            let rank = rank_delimiter(atoms, index, first_for);
            if rank > 0 {
                ranked.entry(rank).or_default().push(index);
            }
        }
    }
    let mut ranks: Vec<u32> = ranked.keys().copied().collect();
    ranks.sort_unstable_by(|left, right| right.cmp(left));
    for rank in ranks {
        let indexes = &ranked[&rank];
        if rank != 18 || indexes.as_slice() != [atoms.len() - 1] {
            return (rank, indexes.clone());
        }
    }
    (0, Vec::new())
}

/// cut_pieces_list: (start, end) ranges of the pieces.
fn cut_pieces(length: usize, rank: u32, indexes: &[usize]) -> Vec<(usize, usize)> {
    let mut pieces = Vec::new();
    let mut start = 0;
    for cut in indexes.iter().map(|&index| if rank == 18 { index + 1 } else { index }).chain(std::iter::once(length)) {
        if cut > start {
            pieces.push((start, cut));
        }
        start = cut.max(start);
    }
    pieces
}

/// One statement's atoms, layout settings and memo.
struct Layout<'a> {
    atoms: &'a [Atom],
    width: usize,
    hug: bool,
    memo: FastMap<(u8, String, usize, usize), Option<Vec<String>>>,
}

impl Layout<'_> {
    fn body(&mut self, start: usize, end: usize, indent: &str) -> Vec<String> {
        let key = (0, indent.to_string(), start, end);
        if let Some(found) = self.memo.get(&key) {
            return found.clone().unwrap_or_default();
        }
        let lines = self.compute_body(start, end, indent);
        self.memo.insert(key, Some(lines.clone()));
        lines
    }

    fn compute_body(&mut self, start: usize, end: usize, indent: &str) -> Vec<String> {
        let atoms = &self.atoms[start..end];
        let line = format!("{indent}{}", join_atoms(atoms));
        if width_of(&line) <= self.width {
            return vec![line];
        }
        let (rank, indexes) = collect_delimiters(atoms);
        let pieces = cut_pieces(atoms.len(), rank, &indexes);
        if pieces.len() < 2 {
            return self.statement(start, end, indent, false).unwrap_or_else(|| vec![line]);
        }
        let mut lines: Vec<String> = Vec::new();
        for (piece_start, piece_end) in pieces {
            let piece_lines = self.body(start + piece_start, start + piece_end, indent);
            let joined = match lines.last() {
                Some(last) => format!("{last} {}", crate::source::python_strip_start(&piece_lines[0])),
                None => String::new(),
            };
            if self.hug && piece_lines.len() == 1 && !lines.is_empty() && width_of(&joined) <= self.width {
                *lines.last_mut().expect("a line") = joined;
            } else {
                lines.extend(piece_lines);
            }
        }
        lines
    }

    fn split_pair(&mut self, start: usize, end: usize, pair: (usize, usize), indent: &str, header: bool) -> Vec<String> {
        let (open, close) = pair;
        let extra = CONTINUATION.repeat(if header && self.hug { 2 } else { 1 });
        let head = self
            .statement(start, start + open + 1, indent, header)
            .unwrap_or_else(|| vec![format!("{indent}{}", join_atoms(&self.atoms[start..start + open + 1]))]);
        let body = self.body(start + open + 1, start + close, &format!("{indent}{extra}"));
        let tail = join_atoms(&self.atoms[start + close..end]);
        let last = body.last().cloned().unwrap_or_default();
        if self.hug && width_of(&last) + width_of(&tail) <= self.width {
            let mut lines = head;
            lines.extend(body[..body.len() - 1].iter().cloned());
            lines.push(format!("{last}{tail}"));
            return lines;
        }
        let tail_lines = self.statement(start + close, end, indent, header).unwrap_or_else(|| vec![format!("{indent}{tail}")]);
        let mut lines = head;
        lines.extend(body);
        lines.extend(tail_lines);
        lines
    }

    fn unwrap_single_pair(&self, start: usize, end: usize, pair: (usize, usize)) -> (usize, usize) {
        let atoms = &self.atoms[start..end];
        let (open, close) = pair;
        let mut inner_start = open + 1;
        while inner_start < close && atoms[inner_start].kind == AtomKind::Op {
            inner_start += 1;
        }
        if inner_start >= close || atoms[inner_start].kind != AtomKind::Open {
            return pair;
        }
        let inner = find_pairs(&atoms[inner_start..close]);
        if inner.len() != 1 || inner[0] != (0, close - inner_start - 1) {
            return pair;
        }
        if close - inner_start - 1 <= 1 {
            return pair;
        }
        self.unwrap_single_pair(start, end, (inner_start, close - 1))
    }

    fn statement(&mut self, start: usize, end: usize, indent: &str, header: bool) -> Option<Vec<String>> {
        let key = (if header { 2 } else { 1 }, indent.to_string(), start, end);
        if let Some(found) = self.memo.get(&key) {
            return found.clone();
        }
        let lines = self.compute_statement(start, end, indent, header);
        self.memo.insert(key, lines.clone());
        lines
    }

    fn compute_statement(&mut self, start: usize, end: usize, indent: &str, header: bool) -> Option<Vec<String>> {
        let line = format!("{indent}{}", join_atoms(&self.atoms[start..end]));
        if width_of(&line) <= self.width {
            return Some(vec![line]);
        }
        let mut pairs: Vec<(usize, usize)> =
            find_pairs(&self.atoms[start..end]).into_iter().filter(|&(open, close)| close > open + 1).collect();
        if self.hug {
            pairs = pairs.into_iter().map(|pair| self.unwrap_single_pair(start, end, pair)).collect();
        }
        if !header {
            pairs.reverse();
        }
        let layouts: Vec<Vec<String>> = pairs.into_iter().map(|pair| self.split_pair(start, end, pair, indent, header)).collect();
        let mut fitting: Option<&Vec<String>> = None;
        for layout in layouts.iter().filter(|layout| longest(layout) <= self.width) {
            if fitting.is_none_or(|best| layout.len() < best.len()) {
                fitting = Some(layout);
            }
        }
        if let Some(fitting) = fitting {
            return Some(fitting.clone());
        }
        let mut best: Option<&Vec<String>> = None;
        for layout in &layouts {
            if best.is_none_or(|current| longest(layout) < longest(current)) {
                best = Some(layout);
            }
        }
        best.filter(|best| longest(best) < width_of(&line)).cloned()
    }
}

/// collect_statements_list: tokens of each logical line.
fn collect_statements(tokens: &[Tok]) -> Vec<Vec<&Tok>> {
    let mut statements = Vec::new();
    let mut current: Vec<&Tok> = Vec::new();
    for token in tokens {
        if current.is_empty()
            && matches!(token.kind, Kind::Nl | Kind::Newline | Kind::Indent | Kind::Dedent | Kind::EndMarker | Kind::Comment)
        {
            continue;
        }
        current.push(token);
        if token.kind == Kind::Newline {
            statements.push(std::mem::take(&mut current));
        }
    }
    statements
}

/// layout_statement_list: new lines for one over-long statement.
fn layout_statement(tokens: &[&Tok], lines: &[&str], width: usize, hug: bool) -> Option<Vec<String>> {
    let (first, last) = (tokens[0].start.0 as usize, tokens[tokens.len() - 1].start.0 as usize);
    let old: Vec<String> = lines[first - 1..last].iter().map(|line| line.to_string()).collect();
    if longest(&old) <= width {
        return None;
    }
    let first_line = &old[0];
    let indent = &first_line[..leading(first_line, &[' ', '\t'])];
    let atoms = build_atoms(tokens, lines)?;
    if atoms.is_empty() || indent.contains('\t') {
        return None;
    }
    let header = HEADER_KEYWORDS.contains(&atoms[0].text.as_str());
    let mut layout = Layout { atoms: &atoms, width, hug, memo: FastMap::default() };
    let new = layout.statement(0, atoms.len(), indent, header)?;
    if new == old || longest(&new) >= longest(&old) {
        return None;
    }
    Some(new)
}

/// plan_layouts_list: (first line, last line, new lines), bottom up.
fn plan_layouts(index: &TokenIndex, protected: &FastSet<u32>, width: usize, hug: bool) -> Vec<(usize, usize, Vec<String>)> {
    let mut plans = Vec::new();
    for statement in collect_statements(&index.tokens).into_iter().rev() {
        let first = statement[0].start.0;
        let last = statement[statement.len() - 1].start.0;
        if (first..=last).any(|line| protected.contains(&line)) {
            continue;
        }
        if let Some(new) = layout_statement(&statement, &index.lines, width, hug) {
            plans.push((first as usize, last as usize, new));
        }
    }
    plans
}
