//! Safe fixes for Ruff-compatible lint findings (`lint --fix`): a port of
//! refactrail/compat_fixes.py with the same edits, the same verification
//! (the fixed text must compile to the original tree with exactly the
//! intended changes) and the same notes.
//!
//! Offsets follow the Python engine exactly: character offsets, line
//! starts from `str.splitlines(keepends=True)`, token positions in
//! characters and AST byte columns converted per split line.

use std::collections::{HashMap, HashSet};

use refactrail_lexer::{tokenize, Kind, Token};
use refactrail_parser::ast::{
    dump_module, walk_expr, walk_expr_mut, CmpOperator, Expr, ExprKind, Module, Stmt, StmtKind,
    UnaryOperator, Visitor, VisitorMut,
};
use refactrail_parser::node::{Constant, Loc};

use crate::lint::lint_file;
use crate::settings::Settings;
use crate::Row;

pub const FIXABLE_CODES: [&str; 6] = ["F401", "F541", "F632", "E703", "E713", "E714"];
const MAX_PASSES: usize = 4;
const DEFERRED_NOTE: &str = "overlapping fixes wait for the next pass";
const LEFT_OUT_NOTE: &str = "some fixes left out: the fixed code did not match the expected syntax tree";
const IMPORT_ERROR_NAMES: [&str; 4] = ["ImportError", "ModuleNotFoundError", "Exception", "BaseException"];

/// The fixes applied to one file (CompatFixOutcome).
#[derive(Debug, Clone, Default)]
pub struct FixOutcome {
    pub text: String,
    pub applied: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Clone)]
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
    finding: usize,
    also: Vec<usize>,
    group: u64,
}

impl Edit {
    fn key(&self) -> u64 {
        if self.group != 0 {
            self.group
        } else {
            (1u64 << 62) | self.finding as u64
        }
    }
}

#[derive(Clone, PartialEq)]
enum Change {
    CompareEq { compare: Loc },
    Negate { unary: Loc, compare: Loc },
    KeepAliases { statement: Loc, kept: Vec<Loc> },
    Remove { statement: Loc },
}

/// Python's str.isspace (strip() removes these).
fn is_python_space(character: char) -> bool {
    character.is_whitespace() || ('\x1c'..='\x1f').contains(&character)
}

fn python_strip(text: &str) -> &str {
    text.trim_matches(is_python_space)
}

/// A key that names one node by kind and position.
fn node_key(tag: u64, loc: &Loc) -> u64 {
    let mut value = tag.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    for part in [loc.line, loc.col, loc.end_line, loc.end_col] {
        value = (value ^ u64::from(part)).wrapping_mul(0x0100_0000_01B3);
    }
    value | 1
}

/// Who owns a list of statements.
#[derive(Clone, Copy, PartialEq)]
enum Owner {
    Module,
    Statement { loc: Loc, is_try: bool, field: u8 },
    Handler { try_loc: Loc },
    Case { match_loc: Loc },
}

struct Body {
    owner: Owner,
    statements: Vec<Loc>,
}

struct ImportInfo {
    statement: Loc,
    aliases: Vec<Loc>,
}

/// Offsets, tokens and nodes of one text (SourceIndex).
struct SourceIndex<'t> {
    text: &'t str,
    char_bytes: Vec<usize>,
    lines: Vec<&'t str>,
    starts: Vec<usize>,
    tree: Module,
    tokens: Vec<Token>,
    token_at: HashMap<usize, usize>,
    bodies: Vec<Body>,
    body_of: HashMap<Loc, usize>,
    try_guarded: HashSet<Loc>,
    imports: Vec<ImportInfo>,
}

/// str.splitlines(keepends=True).
fn split_lines_keep(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut characters = text.char_indices().peekable();
    while let Some((at, character)) = characters.next() {
        let end = match character {
            '\r' => {
                if let Some(&(_, '\n')) = characters.peek() {
                    characters.next();
                    at + 2
                } else {
                    at + 1
                }
            }
            '\n' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}' => {
                at + character.len_utf8()
            }
            _ => continue,
        };
        lines.push(&text[start..end]);
        start = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Parse and compile like parse_quietly_node; None on any error.
fn parse_checked(text: &str) -> Option<Module> {
    if text.contains('\0') {
        return None;
    }
    let (tree, _) = refactrail_parser::parse_with_comments(text).ok()?;
    refactrail_parser::compile::check_with_symbols(&tree).ok()?;
    Some(tree)
}

impl<'t> SourceIndex<'t> {
    fn new(text: &'t str) -> Option<SourceIndex<'t>> {
        let tree = parse_checked(text)?;
        let tokens = tokenize(text).ok()?;
        let mut char_bytes: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
        char_bytes.push(text.len());
        let lines = split_lines_keep(text);
        let mut starts = vec![0];
        for line in &lines {
            starts.push(starts.last().unwrap() + line.chars().count());
        }
        let mut index = SourceIndex {
            text,
            char_bytes,
            lines,
            starts,
            tree,
            tokens,
            token_at: HashMap::new(),
            bodies: Vec::new(),
            body_of: HashMap::new(),
            try_guarded: HashSet::new(),
            imports: Vec::new(),
        };
        for (position, token) in index.tokens.iter().enumerate() {
            if let Some(offset) = index.offset(token.start_pos.0 as usize, token.start_pos.1 as usize) {
                index.token_at.entry(offset).or_insert(position);
            }
        }
        index.collect_structure();
        Some(index)
    }

    /// find_offset_int: a 1-based line and 0-based character column.
    fn offset(&self, line: usize, column: usize) -> Option<usize> {
        Some(self.starts.get(line.checked_sub(1)?)? + column)
    }

    /// find_byte_offset_int: a line and UTF-8 byte column.
    fn byte_offset(&self, line: usize, byte: usize) -> Option<usize> {
        let start = *self.starts.get(line.checked_sub(1)?)?;
        let line_text = self.lines.get(line - 1).copied().unwrap_or("");
        let count = line_text.char_indices().take_while(|(at, character)| at + character.len_utf8() <= byte).count();
        Some(start + count)
    }

    fn node_start(&self, loc: &Loc) -> Option<usize> {
        self.byte_offset(loc.line as usize, loc.col as usize)
    }

    fn node_end(&self, loc: &Loc) -> Option<usize> {
        self.byte_offset(loc.end_line as usize, loc.end_col as usize)
    }

    /// Text between two character offsets (clamped).
    fn slice(&self, start: usize, end: usize) -> &'t str {
        let last = self.char_bytes.len() - 1;
        let (start, end) = (start.min(last), end.min(last).max(start.min(last)));
        &self.text[self.char_bytes[start]..self.char_bytes[end]]
    }

    fn token_text(&self, position: usize) -> &str {
        let token = &self.tokens[position];
        token.text.as_deref().unwrap_or(&self.text[token.start..token.end])
    }

    fn token_start(&self, position: usize) -> Option<usize> {
        let token = &self.tokens[position];
        self.offset(token.start_pos.0 as usize, token.start_pos.1 as usize)
    }

    fn token_end(&self, position: usize) -> Option<usize> {
        let token = &self.tokens[position];
        self.offset(token.end_pos.0 as usize, token.end_pos.1 as usize)
    }

    fn collect_structure(&mut self) {
        let body = std::mem::take(&mut self.tree.body);
        let mut bodies = Vec::new();
        let mut guarded = HashSet::new();
        let mut imports = Vec::new();
        collect_body(&body, Owner::Module, &mut bodies, &mut guarded, &mut imports);
        self.tree.body = body;
        for (position, body) in bodies.iter().enumerate() {
            for statement in &body.statements {
                self.body_of.insert(*statement, position);
            }
        }
        self.bodies = bodies;
        self.try_guarded = guarded;
        self.imports = imports;
    }

    /// check_import_guarded_bool.
    fn is_guarded(&self, statement: Loc) -> bool {
        let mut current = statement;
        loop {
            let Some(&body) = self.body_of.get(&current) else { return false };
            match self.bodies[body].owner {
                Owner::Module => return false,
                Owner::Statement { loc, is_try, field } => {
                    if is_try && field == 0 && self.try_guarded.contains(&loc) {
                        return true;
                    }
                    current = loc;
                }
                Owner::Handler { try_loc } => current = try_loc,
                Owner::Case { match_loc } => current = match_loc,
            }
        }
    }

    /// check_own_lines_bool.
    fn own_lines(&self, statement: &Loc) -> bool {
        let (Some(start), Some(end)) = (self.node_start(statement), self.node_end(statement)) else { return false };
        let line_start = self.starts[statement.line as usize - 1];
        let Some(&line_end) = self.starts.get(statement.end_line as usize) else { return false };
        let before = self.slice(line_start, start);
        let after = python_strip(self.slice(end, line_end));
        python_strip(before).is_empty() && (after.is_empty() || after.starts_with('#'))
    }
}

fn handler_guards(type_: &Option<Box<Expr>>) -> bool {
    struct Names(bool);
    impl<'a> Visitor<'a> for Names {
        fn visit_expr(&mut self, node: &'a Expr) {
            if let ExprKind::Name { id, .. } = &node.kind {
                self.0 |= IMPORT_ERROR_NAMES.contains(&&**id);
            }
            walk_expr(self, node)
        }
    }
    match type_ {
        None => true,
        Some(expression) => {
            let mut names = Names(false);
            names.visit_expr(expression);
            names.0
        }
    }
}

/// Record every statement list, try guards and import statements.
fn collect_body(
    statements: &[Stmt],
    owner: Owner,
    bodies: &mut Vec<Body>,
    guarded: &mut HashSet<Loc>,
    imports: &mut Vec<ImportInfo>,
) {
    bodies.push(Body { owner, statements: statements.iter().map(|statement| statement.loc).collect() });
    for statement in statements {
        let here = |is_try: bool, field: u8| Owner::Statement { loc: statement.loc, is_try, field };
        match &statement.kind {
            StmtKind::Import { names } | StmtKind::ImportFrom { names, .. } => imports.push(ImportInfo {
                statement: statement.loc,
                aliases: names.iter().map(|alias| alias.loc).collect(),
            }),
            StmtKind::FunctionDef { body, .. }
            | StmtKind::AsyncFunctionDef { body, .. }
            | StmtKind::ClassDef { body, .. }
            | StmtKind::With { body, .. }
            | StmtKind::AsyncWith { body, .. } => collect_body(body, here(false, 0), bodies, guarded, imports),
            StmtKind::For { body, orelse, .. }
            | StmtKind::AsyncFor { body, orelse, .. }
            | StmtKind::While { body, orelse, .. }
            | StmtKind::If { body, orelse, .. } => {
                collect_body(body, here(false, 0), bodies, guarded, imports);
                collect_body(orelse, here(false, 1), bodies, guarded, imports);
            }
            StmtKind::Match { cases, .. } => {
                for case in cases {
                    collect_body(&case.body, Owner::Case { match_loc: statement.loc }, bodies, guarded, imports);
                }
            }
            StmtKind::Try { body, handlers, orelse, finalbody } | StmtKind::TryStar { body, handlers, orelse, finalbody } => {
                if handlers.iter().any(|handler| handler_guards(&handler.type_)) {
                    guarded.insert(statement.loc);
                }
                collect_body(body, here(true, 0), bodies, guarded, imports);
                for handler in handlers {
                    collect_body(&handler.body, Owner::Handler { try_loc: statement.loc }, bodies, guarded, imports);
                }
                collect_body(orelse, here(true, 1), bodies, guarded, imports);
                collect_body(finalbody, here(true, 2), bodies, guarded, imports);
            }
            _ => {}
        }
    }
}

/// A comparison and the `not` around it, if any.
struct CompareSite {
    loc: Loc,
    ops: Vec<CmpOperator>,
    left: Loc,
    right: Loc,
    not_parent: Option<Loc>,
}

fn collect_compares(tree: &Module) -> Vec<CompareSite> {
    struct Compares {
        out: Vec<CompareSite>,
        not_operand: Option<(Loc, Loc)>,
    }
    impl<'a> Visitor<'a> for Compares {
        fn visit_expr(&mut self, node: &'a Expr) {
            if let ExprKind::UnaryOp { op: UnaryOperator::Not, operand } = &node.kind {
                self.not_operand = Some((operand.loc, node.loc));
                self.visit_expr(operand);
                return;
            }
            if let ExprKind::Compare { left, ops, comparators } = &node.kind {
                let not_parent = self.not_operand.filter(|(operand, _)| *operand == node.loc).map(|(_, parent)| parent);
                self.out.push(CompareSite {
                    loc: node.loc,
                    ops: ops.clone(),
                    left: left.loc,
                    right: comparators.first().map(|item| item.loc).unwrap_or(left.loc),
                    not_parent,
                });
            }
            self.not_operand = None;
            walk_expr(self, node)
        }
    }
    let mut compares = Compares { out: Vec::new(), not_operand: None };
    for statement in &tree.body {
        compares.visit_stmt(statement);
    }
    compares.out
}

struct Finding<'r> {
    row: &'r Row,
}

impl Finding<'_> {
    fn line(&self) -> usize {
        self.row.1
    }
    fn column(&self) -> usize {
        self.row.2
    }
    fn code(&self) -> &str {
        &self.row.3
    }
}

/// The state of one pass.
struct Pass<'t, 'r> {
    index: SourceIndex<'t>,
    findings: Vec<Finding<'r>>,
    changes: Vec<(u64, Change)>,
    notes: Vec<String>,
    compares: Vec<CompareSite>,
}

impl Pass<'_, '_> {
    fn finding_offset(&self, finding: usize) -> Option<usize> {
        let finding = &self.findings[finding];
        self.index.offset(finding.line(), finding.column().checked_sub(1)?)
    }

    /// E703.
    fn edit_semicolon(&self, finding: usize) -> Vec<Edit> {
        let Some(offset) = self.finding_offset(finding) else { return vec![] };
        if self.index.slice(offset, offset + 1) != ";" {
            return vec![];
        }
        vec![Edit { start: offset, end: offset + 1, replacement: String::new(), finding, also: vec![], group: 0 }]
    }

    /// F541.
    fn edit_fstring(&self, finding: usize) -> Vec<Edit> {
        let Some(offset) = self.finding_offset(finding) else { return vec![] };
        let Some(&first) = self.index.token_at.get(&offset) else { return vec![] };
        let tokens = &self.index.tokens;
        if tokens[first].kind != Kind::FStringStart {
            return vec![];
        }
        let mut depth = 0i32;
        let mut last = None;
        for position in first..tokens.len() {
            match tokens[position].kind {
                Kind::FStringStart => depth += 1,
                Kind::FStringEnd => {
                    depth -= 1;
                    if depth == 0 {
                        last = Some(position);
                        break;
                    }
                }
                Kind::FStringMiddle => {}
                _ => return vec![],
            }
        }
        let Some(last) = last else { return vec![] };
        let start_text = self.index.token_text(first);
        let quote = self.index.token_text(last);
        let Some(end_offset) = self.index.token_end(last) else { return vec![] };
        let body_start = offset + start_text.chars().count();
        let body_end = end_offset.saturating_sub(quote.chars().count());
        let body = self.index.slice(body_start, body_end.max(body_start)).replace("{{", "{").replace("}}", "}");
        let prefix: String = start_text.chars().filter(|character| !matches!(character, 'f' | 'F')).collect();
        vec![Edit { start: offset, end: end_offset, replacement: prefix + &body + quote, finding, also: vec![], group: 0 }]
    }

    /// find_operator_span_tuple.
    fn operator_span(&self, left: &Loc, right: &Loc) -> Option<(usize, usize)> {
        let left_end = self.index.node_end(left)?;
        let right_start = self.index.node_start(right)?;
        let mut words = Vec::new();
        for position in 0..self.index.tokens.len() {
            if self.index.tokens[position].kind != Kind::Name {
                continue;
            }
            let (Some(start), Some(end)) = (self.index.token_start(position), self.index.token_end(position)) else {
                continue;
            };
            if left_end <= start && end <= right_start && matches!(self.index.token_text(position), "is" | "not" | "in") {
                words.push((start, end));
            }
        }
        Some((words.first()?.0, words.last()?.1))
    }

    fn compares_at(&self, offset: usize) -> Vec<usize> {
        (0..self.compares.len()).filter(|&position| self.index.node_start(&self.compares[position].loc) == Some(offset)).collect()
    }

    /// F632.
    fn edit_literal_identity(&mut self, finding: usize) -> Vec<Edit> {
        let Some(offset) = self.finding_offset(finding) else { return vec![] };
        for position in self.compares_at(offset) {
            let site = &self.compares[position];
            if site.ops.len() != 1 || !matches!(site.ops[0], CmpOperator::Is | CmpOperator::IsNot) {
                continue;
            }
            let Some((start, end)) = self.operator_span(&site.left, &site.right) else { return vec![] };
            let group = node_key(1, &site.loc);
            let replacement = if site.ops[0] == CmpOperator::Is { "==" } else { "!=" };
            let loc = site.loc;
            self.changes.retain(|(_, change)| *change != Change::CompareEq { compare: loc });
            self.changes.push((group, Change::CompareEq { compare: loc }));
            return vec![Edit { start, end, replacement: replacement.into(), finding, also: vec![], group }];
        }
        vec![]
    }

    /// E713 / E714.
    fn edit_negated_test(&mut self, finding: usize) -> Vec<Edit> {
        let Some(offset) = self.finding_offset(finding) else { return vec![] };
        let in_test = self.findings[finding].code() == "E713";
        for position in self.compares_at(offset) {
            let site = &self.compares[position];
            let wanted = if in_test { CmpOperator::In } else { CmpOperator::Is };
            let Some(parent) = site.not_parent else { continue };
            if site.ops.len() != 1 || site.ops[0] != wanted {
                continue;
            }
            let (compare, left, right) = (site.loc, site.left, site.right);
            let not_token = self.index.node_start(&parent).and_then(|start| self.index.token_at.get(&start).copied());
            let span = self.operator_span(&left, &right);
            let (Some(not_token), Some(span), Some(parent_start)) = (not_token, span, self.index.node_start(&parent)) else {
                return vec![];
            };
            let Some((not_end, close)) = self.not_cut(not_token, &parent) else { return vec![] };
            let group = node_key(2, &parent);
            self.changes.push((group, Change::Negate { unary: parent, compare }));
            let replacement = if in_test { "not in" } else { "is not" };
            let mut edits = vec![
                Edit { start: parent_start, end: not_end, replacement: String::new(), finding, also: vec![], group },
                Edit { start: span.0, end: span.1, replacement: replacement.into(), finding, also: vec![], group },
            ];
            if let Some(close) = close {
                let Some(inner_end) = self.index.node_end(&compare) else { return vec![] };
                edits.push(Edit { start: inner_end, end: close + 1, replacement: String::new(), finding, also: vec![], group });
            }
            return edits;
        }
        vec![]
    }

    /// find_not_cut_tuple.
    fn not_cut(&self, not_token: usize, parent: &Loc) -> Option<(usize, Option<usize>)> {
        let mut close = self.wrapping_close(not_token + 1, self.index.node_end(parent)?);
        let mut drop = not_token + if close.is_some() { 2 } else { 1 };
        while self.index.tokens.get(drop)?.kind == Kind::Nl {
            drop += 1;
        }
        if close.is_some() && self.index.tokens.get(drop)?.kind == Kind::Comment {
            close = None;
            drop = not_token + 1;
        }
        Some((self.index.token_start(drop)?, close))
    }

    /// find_wrapping_close_int.
    fn wrapping_close(&self, open: usize, end: usize) -> Option<usize> {
        if open >= self.index.tokens.len() || self.index.token_text(open) != "(" {
            return None;
        }
        let mut depth = 0i32;
        for position in open..self.index.tokens.len() {
            if self.index.tokens[position].kind != Kind::Op {
                continue;
            }
            match self.index.token_text(position) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth -= 1;
                    if depth == 0 {
                        let close = self.index.token_start(position)?;
                        return (close + 1 == end).then_some(close);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// F401 (edit_imports_list).
    fn edit_imports(&mut self, import_findings: &[usize]) -> Vec<Edit> {
        let mut order: Vec<usize> = Vec::new();
        let mut grouped: HashMap<usize, (Vec<usize>, Vec<usize>)> = HashMap::new();
        for &finding in import_findings {
            let found = self.finding_offset(finding).and_then(|offset| self.alias_at(offset));
            let Some((import, alias)) = found.filter(|(import, _)| !self.index.is_guarded(self.index.imports[*import].statement))
            else {
                let finding = &self.findings[finding];
                self.notes.push(format!("{}:{} F401 not fixed: import is guarded or not found", finding.line(), finding.column()));
                continue;
            };
            let entry = grouped.entry(import).or_insert_with(|| {
                order.push(import);
                (Vec::new(), Vec::new())
            });
            entry.0.push(alias);
            entry.1.push(finding);
        }
        let mut edits = Vec::new();
        for import in order {
            let (unused, findings) = grouped.remove(&import).unwrap();
            edits.extend(self.edit_import_statement(import, &unused, &findings));
        }
        edits
    }

    /// find_alias_at: (import index, alias index) containing an offset.
    fn alias_at(&self, offset: usize) -> Option<(usize, usize)> {
        for (import, info) in self.index.imports.iter().enumerate() {
            for (alias, loc) in info.aliases.iter().enumerate() {
                if self.index.node_start(loc)? <= offset && offset < self.index.node_end(loc)? {
                    return Some((import, alias));
                }
            }
        }
        None
    }

    /// edit_import_statement_list.
    fn edit_import_statement(&mut self, import: usize, unused: &[usize], findings: &[usize]) -> Vec<Edit> {
        let info = &self.index.imports[import];
        let statement = info.statement;
        let kept: Vec<Loc> =
            (0..info.aliases.len()).filter(|alias| !unused.contains(alias)).map(|alias| info.aliases[alias]).collect();
        let edits = if kept.is_empty() { vec![] } else { self.edit_alias_runs(import, unused, findings) };
        if !kept.is_empty() && !edits.is_empty() {
            let group = node_key(3, &statement);
            self.changes.push((group, Change::KeepAliases { statement, kept }));
            return edits;
        }
        if !kept.is_empty() || !self.index.own_lines(&statement) {
            for &finding in findings {
                let finding = &self.findings[finding];
                self.notes.push(format!(
                    "{}:{} F401 not fixed: the statement shares its line or a comment would be removed",
                    finding.line(),
                    finding.column()
                ));
            }
            return vec![];
        }
        let group = node_key(3, &statement);
        self.changes.push((group, Change::Remove { statement }));
        let start = self.index.starts[statement.line as usize - 1];
        let end = self.index.starts[statement.end_line as usize];
        vec![Edit { start, end, replacement: String::new(), finding: findings[0], also: findings[1..].to_vec(), group }]
    }

    /// edit_alias_runs_list.
    fn edit_alias_runs(&self, import: usize, unused: &[usize], findings: &[usize]) -> Vec<Edit> {
        let info = &self.index.imports[import];
        let finding_of: HashMap<usize, usize> = unused.iter().copied().zip(findings.iter().copied()).collect();
        let names = &info.aliases;
        let group = node_key(3, &info.statement);
        let mut edits = Vec::new();
        let mut position = 0;
        while position < names.len() {
            if !finding_of.contains_key(&position) {
                position += 1;
                continue;
            }
            let mut run_end = position;
            while run_end + 1 < names.len() && finding_of.contains_key(&(run_end + 1)) {
                run_end += 1;
            }
            let span = if run_end + 1 < names.len() {
                (self.index.node_start(&names[position]), self.index.node_start(&names[run_end + 1]))
            } else {
                (position.checked_sub(1).and_then(|before| self.index.node_end(&names[before])), self.index.node_end(&names[run_end]))
            };
            let (Some(start), Some(end)) = span else { return vec![] };
            if self.index.slice(start, end.max(start)).contains('#') {
                return vec![];
            }
            let run: Vec<usize> = (position..=run_end).map(|alias| finding_of[&alias]).collect();
            edits.push(Edit { start, end, replacement: String::new(), finding: run[0], also: run[1..].to_vec(), group });
            position = run_end + 1;
        }
        edits
    }

    /// fill_empty_bodies_none.
    fn fill_empty_bodies(&self, edits: &mut [Edit]) {
        let removed: Vec<Loc> = self
            .changes
            .iter()
            .filter_map(|(_, change)| if let Change::Remove { statement } = change { Some(*statement) } else { None })
            .collect();
        let mut by_body: HashMap<usize, Vec<Loc>> = HashMap::new();
        for statement in removed {
            if let Some(&body) = self.index.body_of.get(&statement) {
                by_body.entry(body).or_default().push(statement);
            }
        }
        for (body, statements) in by_body {
            let body = &self.index.bodies[body];
            if body.owner == Owner::Module || statements.len() < body.statements.len() {
                continue;
            }
            let last = body.statements[body.statements.len() - 1];
            let line_start = self.index.starts[last.line as usize - 1];
            let line_end = self.index.starts[last.end_line as usize];
            let (Some(start), Some(end)) = (self.index.node_start(&last), self.index.node_end(&last)) else { continue };
            let replacement =
                format!("{}pass{}", self.index.slice(line_start, start), self.index.slice(end, line_end.max(end)));
            for edit in edits.iter_mut().filter(|edit| edit.start == line_start) {
                edit.replacement = replacement.clone();
            }
        }
    }

    /// build_edits_list.
    fn build_edits(&mut self) -> Vec<Edit> {
        let import_findings: Vec<usize> =
            (0..self.findings.len()).filter(|&finding| self.findings[finding].code() == "F401").collect();
        let mut edits = self.edit_imports(&import_findings);
        for finding in 0..self.findings.len() {
            match self.findings[finding].code() {
                "E703" => edits.extend(self.edit_semicolon(finding)),
                "F541" => edits.extend(self.edit_fstring(finding)),
                "F632" => edits.extend(self.edit_literal_identity(finding)),
                "E713" | "E714" => edits.extend(self.edit_negated_test(finding)),
                _ => {}
            }
        }
        self.fill_empty_bodies(&mut edits);
        let kept = self.drop_overlaps(&edits);
        if kept.len() < edits.len() {
            self.notes.push(DEFERRED_NOTE.into());
        }
        kept
    }

    /// drop_overlaps_list.
    fn drop_overlaps(&mut self, edits: &[Edit]) -> Vec<Edit> {
        let mut dropped: HashSet<u64> = HashSet::new();
        let kept = loop {
            let mut kept: Vec<Edit> = edits.iter().filter(|edit| !dropped.contains(&edit.key())).cloned().collect();
            kept.sort_by_key(|edit| (edit.start, edit.end));
            match kept.windows(2).find(|pair| pair[1].start < pair[0].end) {
                Some(pair) => {
                    dropped.insert(pair[1].key());
                }
                None => break kept,
            }
        };
        self.changes.retain(|(group, _)| !dropped.contains(group));
        kept
    }

    /// check_fixed_text_bool.
    fn check_text(&self, changes: &[(u64, Change)], text: &str) -> bool {
        let Some(mut new_tree) = parse_checked(text) else { return false };
        let mut expected = build_expected(&self.index.tree, changes);
        unify(&mut expected);
        unify(&mut new_tree);
        dump_module(&expected, false) == dump_module(&new_tree, false)
    }

    fn changes_for(&self, groups: &HashSet<u64>) -> Vec<(u64, Change)> {
        self.changes.iter().filter(|(group, _)| groups.contains(group)).cloned().collect()
    }

    /// keep_verified_edits_list.
    fn keep_verified(&self, edits: Vec<Edit>) -> Vec<Edit> {
        if edits.is_empty() || self.check_text(&self.changes, &apply_edits(&self.index, &edits)) {
            return edits;
        }
        let mut order: Vec<u64> = Vec::new();
        let mut groups: HashMap<u64, Vec<Edit>> = HashMap::new();
        for edit in &edits {
            groups.entry(edit.key()).or_insert_with(|| {
                order.push(edit.key());
                Vec::new()
            });
            groups.get_mut(&edit.key()).unwrap().push(edit.clone());
        }
        let mut passing: Vec<(u64, Vec<Edit>)> = Vec::new();
        for key in order {
            let group_edits = groups.remove(&key).unwrap();
            let part = self.changes_for(&HashSet::from([key]));
            if self.check_text(&part, &apply_edits(&self.index, &group_edits)) {
                passing.push((key, group_edits));
            }
        }
        let mut combined: Vec<Edit> = passing.iter().flat_map(|(_, group)| group.iter().cloned()).collect();
        combined.sort_by_key(|edit| (edit.start, edit.end));
        let keys: HashSet<u64> = passing.iter().map(|(key, _)| *key).collect();
        if !combined.is_empty() && self.check_text(&self.changes_for(&keys), &apply_edits(&self.index, &combined)) {
            return combined;
        }
        passing.into_iter().next().map(|(_, group)| group).unwrap_or_default()
    }
}

/// apply_edits_str: edits in text order, by character offsets.
fn apply_edits(index: &SourceIndex, edits: &[Edit]) -> String {
    let mut out = String::with_capacity(index.text.len());
    let mut at = 0;
    for edit in edits {
        out.push_str(index.slice(at, edit.start));
        out.push_str(&edit.replacement);
        at = edit.end.max(at);
    }
    out.push_str(index.slice(at, index.char_bytes.len() - 1));
    out
}

/// build_expected_tree: the original tree with the intended changes.
fn build_expected(tree: &Module, changes: &[(u64, Change)]) -> Module {
    struct Rewrite<'c> {
        equal: HashSet<Loc>,
        negate: HashSet<Loc>,
        keep: HashMap<Loc, &'c Vec<Loc>>,
        remove: HashSet<Loc>,
    }
    impl VisitorMut for Rewrite<'_> {
        fn visit_stmt_mut(&mut self, node: &mut Stmt) {
            if let Some(kept) = self.keep.get(&node.loc) {
                if let StmtKind::Import { names } | StmtKind::ImportFrom { names, .. } = &mut node.kind {
                    names.retain(|alias| kept.contains(&alias.loc));
                }
            }
            refactrail_parser::ast::walk_stmt_mut(self, node);
            for_each_body(node, |body, is_module| remove_from(body, &self.remove, is_module));
        }
        fn visit_expr_mut(&mut self, node: &mut Expr) {
            walk_expr_mut(self, node);
            if let ExprKind::Compare { ops, .. } = &mut node.kind {
                if self.equal.contains(&node.loc) && ops.len() == 1 {
                    ops[0] = if ops[0] == CmpOperator::Is { CmpOperator::Eq } else { CmpOperator::NotEq };
                }
            }
            if self.negate.contains(&node.loc) {
                if let ExprKind::UnaryOp { operand, .. } = &mut node.kind {
                    let mut inner = std::mem::replace(&mut **operand, Expr { kind: ExprKind::Constant { value: Constant::None, kind: None }, loc: Loc::default() });
                    if let ExprKind::Compare { ops, .. } = &mut inner.kind {
                        ops[0] = if ops[0] == CmpOperator::In { CmpOperator::NotIn } else { CmpOperator::IsNot };
                    }
                    *node = inner;
                }
            }
        }
    }
    let mut rewrite = Rewrite { equal: HashSet::new(), negate: HashSet::new(), keep: HashMap::new(), remove: HashSet::new() };
    for (_, change) in changes {
        match change {
            Change::CompareEq { compare } => {
                rewrite.equal.insert(*compare);
            }
            Change::Negate { unary, .. } => {
                rewrite.negate.insert(*unary);
            }
            Change::KeepAliases { statement, kept } => {
                rewrite.keep.insert(*statement, kept);
            }
            Change::Remove { statement } => {
                rewrite.remove.insert(*statement);
            }
        }
    }
    let mut clone = tree.clone();
    for statement in clone.body.iter_mut() {
        rewrite.visit_stmt_mut(statement);
    }
    remove_from(&mut clone.body, &rewrite.remove, true);
    clone
}

fn remove_from(body: &mut Vec<Stmt>, removed: &HashSet<Loc>, is_module: bool) {
    if removed.is_empty() || !body.iter().any(|statement| removed.contains(&statement.loc)) {
        return;
    }
    body.retain(|statement| !removed.contains(&statement.loc));
    if body.is_empty() && !is_module {
        body.push(Stmt { kind: StmtKind::Pass, loc: Loc::default() });
    }
}

/// Call `action` on each statement list directly inside a statement.
fn for_each_body(node: &mut Stmt, mut action: impl FnMut(&mut Vec<Stmt>, bool)) {
    match &mut node.kind {
        StmtKind::FunctionDef { body, .. }
        | StmtKind::AsyncFunctionDef { body, .. }
        | StmtKind::ClassDef { body, .. }
        | StmtKind::With { body, .. }
        | StmtKind::AsyncWith { body, .. } => action(body, false),
        StmtKind::For { body, orelse, .. }
        | StmtKind::AsyncFor { body, orelse, .. }
        | StmtKind::While { body, orelse, .. }
        | StmtKind::If { body, orelse, .. } => {
            action(body, false);
            action(orelse, false);
        }
        StmtKind::Match { cases, .. } => {
            for case in cases {
                action(&mut case.body, false);
            }
        }
        StmtKind::Try { body, handlers, orelse, finalbody } | StmtKind::TryStar { body, handlers, orelse, finalbody } => {
            action(body, false);
            for handler in handlers {
                action(&mut handler.body, false);
            }
            action(orelse, false);
            action(finalbody, false);
        }
        _ => {}
    }
}

/// UnifyTransformer: f-strings of constants become strings; no `u` kind.
fn unify(tree: &mut Module) {
    struct Unify;
    impl VisitorMut for Unify {
        fn visit_expr_mut(&mut self, node: &mut Expr) {
            walk_expr_mut(self, node);
            match &mut node.kind {
                ExprKind::Constant { kind, .. } => *kind = None,
                ExprKind::JoinedStr { values } => {
                    let mut joined: Vec<u32> = Vec::new();
                    for value in values.iter() {
                        match &value.kind {
                            ExprKind::Constant { value: Constant::Str(points), .. } => joined.extend(points),
                            _ => return,
                        }
                    }
                    node.kind = ExprKind::Constant { value: Constant::Str(joined), kind: None };
                }
                _ => {}
            }
        }
    }
    for statement in tree.body.iter_mut() {
        Unify.visit_stmt_mut(statement);
    }
}

/// filter_fixable_list.
fn fixable<'r>(rows: &'r [Row], path: &str) -> Vec<Finding<'r>> {
    let normalized = path.replace('\\', "/");
    let is_init = normalized.ends_with("__init__.py") || normalized.ends_with(".pyi");
    rows.iter()
        .filter(|row| FIXABLE_CODES.contains(&row.3.as_str()) && !(is_init && row.3 == "F401"))
        .map(|row| Finding { row })
        .collect()
}

/// fix_compat_text: apply verified passes of safe fixes to one text.
pub fn fix_compat_text(text: &str, path: &str, settings: &Settings, known: Option<&[Row]>) -> FixOutcome {
    let mut outcome = FixOutcome { text: text.to_string(), ..FixOutcome::default() };
    for pass in 0..MAX_PASSES {
        let rows: Vec<Row> = match (pass, known) {
            (0, Some(rows)) => rows.to_vec(),
            _ => {
                let mut rows = lint_file(path, outcome.text.as_bytes(), settings);
                rows.sort();
                rows.dedup();
                rows
            }
        };
        let findings = fixable(&rows, path);
        if findings.is_empty() || !run_fix_pass(&mut outcome, findings) {
            break;
        }
    }
    outcome
}

/// run_fix_pass_bool.
fn run_fix_pass(outcome: &mut FixOutcome, findings: Vec<Finding>) -> bool {
    let current = outcome.text.clone();
    let Some(index) = SourceIndex::new(&current) else { return false };
    let compares = collect_compares(&index.tree);
    let mut pass = Pass { index, findings, changes: Vec::new(), notes: Vec::new(), compares };
    let edits = pass.build_edits();
    outcome.notes = pass.notes.clone();
    let edit_count = edits.len();
    let verified = pass.keep_verified(edits);
    if verified.len() < edit_count {
        outcome.notes.push(LEFT_OUT_NOTE.into());
    }
    if verified.is_empty() {
        return false;
    }
    let fixed_text = apply_edits(&pass.index, &verified);
    let mut fixed: Vec<&Row> =
        verified.iter().flat_map(|edit| std::iter::once(edit.finding).chain(edit.also.iter().copied())).map(|finding| pass.findings[finding].row).collect();
    fixed.sort();
    fixed.dedup();
    outcome.applied.extend(fixed.iter().map(|row| format!("{}:{} {} {}", row.1, row.2, row.3, row.5)));
    let more = pass.notes.iter().any(|note| note == DEFERRED_NOTE) || fixed.iter().any(|row| row.3 == "F401");
    outcome.text = fixed_text;
    more
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lint::lint_settings;

    fn fix(text: &str) -> FixOutcome {
        let settings = lint_settings(&["E4".into(), "E7".into(), "F".into()], &[]);
        fix_compat_text(text, "example.py", &settings, None)
    }

    #[test]
    fn removes_unused_imports_and_keeps_used_ones() {
        let outcome = fix("import os\nimport sys, json\nprint(sys.argv)\n");
        assert_eq!(outcome.text, "import sys\nprint(sys.argv)\n");
        assert_eq!(outcome.applied.len(), 2);
    }

    #[test]
    fn fixes_the_small_rules() {
        let outcome = fix("x = 1;\nprint(f\"plain {{x}}\")\nif not x in [1]:\n    pass\nif x is 1:\n    pass\n");
        assert_eq!(outcome.text, "x = 1\nprint(\"plain {x}\")\nif x not in [1]:\n    pass\nif x == 1:\n    pass\n");
    }

    #[test]
    fn guarded_imports_stay_with_a_note() {
        let outcome = fix("try:\n    import yaml\nexcept ImportError:\n    yaml = None\n");
        assert!(outcome.text.contains("import yaml"));
    }

    #[test]
    fn emptied_bodies_get_pass() {
        let outcome = fix("if True:\n    import os\n");
        assert_eq!(outcome.text, "if True:\n    pass\n");
    }
}
