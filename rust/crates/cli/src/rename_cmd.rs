//! `rename PATH --function F --old O --new N`: a read-only proposal to
//! rename one local variable in one top-level function, byte-identical to
//! `refactrail rename` (refactrail/rename.py): the same refusals (RENAME001
//! to RENAME008), the same structural check and the same JSON document.

use std::path::Path;

use refactrail_engine::lint::syntax_error_text;
use refactrail_parser::ast::{
    dump_module, walk_excepthandler, walk_expr, walk_pattern, walk_stmt, write_stmt, Alias, Arg, ExceptHandler,
    Expr, ExprKind, Module, Pattern, PatternKind, Stmt, StmtKind, Visitor,
};
use refactrail_parser::node::{write_ident, Loc};
use refactrail_parser::symtable::{SymbolTable, CELL, DEF_PARAM, LOCAL, SCOPE_MASK, SCOPE_OFFSET};

use crate::diff::{split_lines_keep, unified_diff};
use crate::json::{dumps, sha256_hex, Json};

pub const RENAME_USAGE: &str = "usage: refactrail-native rename path --function FUNCTION --old OLD --new NEW";

/// Python's `keyword.kwlist` (identical in 3.12 to 3.14).
const KEYWORDS: [&str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def",
    "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];

struct RenameOptions {
    path: String,
    function: String,
    old: String,
    new: String,
}

/// The arguments of `rename`, as argparse reads them.
fn parse_rename_options(arguments: &[String]) -> Result<RenameOptions, String> {
    let (mut path, mut function, mut old, mut new) = (None, None, None, None);
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (argument, None),
        };
        let slot = match flag {
            "--function" => &mut function,
            "--old" => &mut old,
            "--new" => &mut new,
            _ if !argument.starts_with('-') && path.is_none() => {
                path = Some(argument.to_string());
                index += 1;
                continue;
            }
            _ => return Err(format!("unrecognized arguments: {argument}")),
        };
        let value = match inline {
            Some(value) => value,
            None => {
                index += 1;
                arguments.get(index).cloned().ok_or_else(|| format!("argument {flag}: expected one argument"))?
            }
        };
        *slot = Some(value);
        index += 1;
    }
    let missing: Vec<&str> = [("path", path.is_none()), ("--function", function.is_none()), ("--old", old.is_none()), ("--new", new.is_none())]
        .iter()
        .filter(|(_, absent)| *absent)
        .map(|(name, _)| *name)
        .collect();
    if !missing.is_empty() {
        return Err(format!("the following arguments are required: {}", missing.join(", ")));
    }
    Ok(RenameOptions { path: path.unwrap(), function: function.unwrap(), old: old.unwrap(), new: new.unwrap() })
}

/// `rename PATH --function F --old O --new N`: one JSON document.
pub fn run_rename(arguments: &[String]) -> Result<i32, String> {
    let options = parse_rename_options(arguments)?;
    print!("{}", rename_document(&options.path, &options.function, &options.old, &options.new)?);
    Ok(0)
}

/// A refusal in the Python engine's words: "CODE line N: message".
fn refusal(code: &str, line: u32, message: &str) -> String {
    format!("{code} line {line}: {message}")
}

/// Parse and compile like `parse_quietly_node`, or the SyntaxError text.
fn parse_checked(text: &str, path: &str) -> Result<(Module, SymbolTable), String> {
    if text.contains('\0') {
        return Err("source code string cannot contain null bytes".into());
    }
    let (tree, _) = refactrail_parser::parse_with_comments(text).map_err(|error| {
        let (line, _, message) = refactrail_parser::syntax_error_tuple(text, error);
        syntax_error_text(path, line as usize, &message)
    })?;
    let symbols = refactrail_parser::compile::check_with_symbols(&tree)
        .map_err(|error| syntax_error_text(path, error.line as usize, &error.message))?;
    Ok((tree, symbols))
}

/// The unique top-level (async) function named `name`, or None.
fn find_function<'a>(module: &'a Module, name: &str) -> Vec<&'a Stmt> {
    module
        .body
        .iter()
        .filter(|statement| match &statement.kind {
            StmtKind::FunctionDef { name: found, .. } | StmtKind::AsyncFunctionDef { name: found, .. } => &**found == name,
            _ => false,
        })
        .collect()
}

fn function_parts(statement: &Stmt) -> (&[Stmt], &[Expr]) {
    match &statement.kind {
        StmtKind::FunctionDef { body, decorator_list, .. } | StmtKind::AsyncFunctionDef { body, decorator_list, .. } => {
            (body, decorator_list)
        }
        _ => (&[], &[]),
    }
}

/// Facts about a function body: nested scopes and `Name` spans of one id.
struct BodyFacts<'n> {
    old: &'n str,
    nested: bool,
    spans: Vec<Loc>,
}

impl<'a> Visitor<'a> for BodyFacts<'_> {
    fn visit_stmt(&mut self, node: &'a Stmt) {
        if matches!(node.kind, StmtKind::FunctionDef { .. } | StmtKind::AsyncFunctionDef { .. } | StmtKind::ClassDef { .. }) {
            self.nested = true;
        }
        walk_stmt(self, node)
    }
    fn visit_expr(&mut self, node: &'a Expr) {
        match &node.kind {
            ExprKind::Lambda { .. }
            | ExprKind::ListComp { .. }
            | ExprKind::SetComp { .. }
            | ExprKind::DictComp { .. }
            | ExprKind::GeneratorExp { .. } => self.nested = true,
            ExprKind::Name { id, .. } if &**id == self.old => self.spans.push(node.loc),
            _ => {}
        }
        walk_expr(self, node)
    }
}

fn body_facts<'n>(body: &[Stmt], old: &'n str) -> BodyFacts<'n> {
    let mut facts = BodyFacts { old, nested: false, spans: Vec::new() };
    for statement in body {
        facts.visit_stmt(statement);
    }
    facts
}

/// Whether the replacement name already appears in the module as a
/// `Name`, a parameter or a definition (validate_rename_collisions_none).
struct Collisions<'n> {
    new: &'n str,
    found: bool,
}

impl<'a> Visitor<'a> for Collisions<'_> {
    fn visit_stmt(&mut self, node: &'a Stmt) {
        match &node.kind {
            StmtKind::FunctionDef { name, .. } | StmtKind::AsyncFunctionDef { name, .. } | StmtKind::ClassDef { name, .. }
                if &**name == self.new =>
            {
                self.found = true
            }
            _ => {}
        }
        walk_stmt(self, node)
    }
    fn visit_expr(&mut self, node: &'a Expr) {
        if let ExprKind::Name { id, .. } = &node.kind {
            self.found |= &**id == self.new;
        }
        walk_expr(self, node)
    }
    fn visit_arg(&mut self, node: &'a Arg) {
        self.found |= &*node.arg == self.new;
        refactrail_parser::ast::walk_arg(self, node)
    }
}

/// Bindings kept outside `Name` nodes (is_opaque_binding_bool).
struct Opaque<'n> {
    old: &'n str,
    found: bool,
}

impl<'a> Visitor<'a> for Opaque<'_> {
    fn visit_excepthandler(&mut self, node: &'a ExceptHandler) {
        self.found |= node.name.as_deref() == Some(self.old);
        walk_excepthandler(self, node)
    }
    fn visit_pattern(&mut self, node: &'a Pattern) {
        match &node.kind {
            PatternKind::MatchAs { name, .. } | PatternKind::MatchStar { name } => {
                self.found |= name.as_deref() == Some(self.old)
            }
            PatternKind::MatchMapping { rest, .. } => self.found |= rest.as_deref() == Some(self.old),
            _ => {}
        }
        walk_pattern(self, node)
    }
    fn visit_alias(&mut self, node: &'a Alias) {
        let bound = node.asname.as_deref().unwrap_or_else(|| node.name.split('.').next().unwrap_or(""));
        self.found |= bound == self.old;
    }
}

/// `str.isidentifier()`, `keyword.iskeyword()` and NFKC stability.
fn is_normalized_identifier(name: &str) -> bool {
    refactrail_lexer::is_identifier(name, refactrail_lexer::Version::default())
        && !KEYWORDS.contains(&name)
        && refactrail_parser::nfkc::nfkc(name) == name
}

/// The selected function's symbol table (its name and line match).
fn function_table<'s>(symbols: &'s SymbolTable, name: &str, line: u32) -> Option<&'s refactrail_parser::symtable::Table> {
    symbols.tables[0]
        .children
        .iter()
        .map(|&index| &symbols.tables[index])
        .find(|table| &*table.name == name && table.lineno == line)
}

/// select_rename_function and validate_rename_names_none.
fn validate_request(tree: &Module, symbols: &SymbolTable, function: &str, old: &str, new: &str) -> Result<(), String> {
    let matches = find_function(tree, function);
    let [selected] = matches.as_slice() else {
        return Err(refusal("RENAME001", 1, "select a unique top-level function"));
    };
    let line = selected.loc.line;
    let (body, decorators) = function_parts(selected);
    if !decorators.is_empty() || body_facts(body, old).nested {
        return Err(refusal("RENAME002", line, "decorators and nested scopes need broader reference analysis"));
    }
    let limits = refactrail_engine::lexical::scope_limits(tree);
    if !limits.is_empty() {
        return Err(format!("RENAME003: {}", limits.join("; ")));
    }
    if !is_normalized_identifier(new) {
        return Err(refusal("RENAME004", 1, "replacement must be a normalized identifier"));
    }
    let table = function_table(symbols, function, line).ok_or_else(|| refusal("RENAME005", line, &format!("Unknown local {old}")))?;
    if !table.contains(old) {
        return Err(refusal("RENAME005", line, &format!("Unknown local {old}")));
    }
    let flags = table.get(old);
    let scope = (flags >> SCOPE_OFFSET) & SCOPE_MASK;
    if !(scope == LOCAL || scope == CELL) || flags & DEF_PARAM != 0 {
        return Err(refusal("RENAME005", line, "Only local non-parameter bindings are supported"));
    }
    let mut collisions = Collisions { new, found: false };
    for statement in &tree.body {
        collisions.visit_stmt(statement);
    }
    if new == old || collisions.found || table.contains(new) {
        return Err(refusal("RENAME006", line, "Replacement collides with a source name"));
    }
    let mut opaque = Opaque { old, found: false };
    opaque.visit_stmt(selected);
    if opaque.found {
        return Err(refusal("RENAME007", line, "Pattern, exception and import bindings are excluded"));
    }
    Ok(())
}

/// Byte offset of each line start, splitting at \r\n, \r and \n.
fn line_starts(bytes: &[u8]) -> Vec<usize> {
    let mut starts = vec![0];
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                index += 2;
                starts.push(index);
            }
            b'\r' | b'\n' => {
                index += 1;
                starts.push(index);
            }
            _ => index += 1,
        }
    }
    starts
}

/// replace_name_spans_str: swap each selected `Name` span's bytes.
fn replace_spans(text: &str, spans: &[Loc], new: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    let starts = line_starts(text.as_bytes());
    let mut edits: Vec<(usize, usize)> = spans
        .iter()
        .map(|loc| {
            (starts[loc.line as usize - 1] + loc.col as usize, starts[loc.end_line as usize - 1] + loc.end_col as usize)
        })
        .collect();
    edits.sort_unstable_by(|a, b| b.cmp(a));
    for (start, end) in edits {
        bytes.splice(start..end, new.bytes());
    }
    String::from_utf8(bytes).expect("identifier edits keep UTF-8")
}

/// The module's dump without the function's body, and the body's
/// statement dumps (with `old` renamed to `new` when given).
fn split_dumps(tree: &Module, function: &str, rename: Option<(&str, &str)>) -> (String, Vec<String>) {
    let mut stripped = tree.clone();
    let mut body_dumps = Vec::new();
    for statement in stripped.body.iter_mut() {
        let body = match &mut statement.kind {
            StmtKind::FunctionDef { name, body, .. } | StmtKind::AsyncFunctionDef { name, body, .. } if &**name == function => body,
            _ => continue,
        };
        for item in body.iter() {
            let mut text = String::new();
            write_stmt(&mut text, item, false);
            if let Some((old, new)) = rename {
                text = text.replace(&name_dump(old), &name_dump(new));
            }
            body_dumps.push(text);
        }
        body.clear();
        break;
    }
    (dump_module(&stripped, false), body_dumps)
}

/// How `ast.dump` writes the start of a `Name` node with this id.
fn name_dump(id: &str) -> String {
    let mut text = String::from("Name(id=");
    write_ident(&mut text, id);
    text.push_str(", ctx=");
    text
}

/// validate_rename_none: the proposal differs only in the renamed names.
fn validate_proposal(tree: &Module, output: &str, function: &str, old: &str, new: &str, line: u32) -> Result<(), String> {
    let (proposed, _) = parse_checked(output, "<rename proposal>")?;
    if split_dumps(tree, function, Some((old, new))) != split_dumps(&proposed, function, None) {
        return Err(refusal("RENAME008", line, "Proposal changed unexpected syntax"));
    }
    Ok(())
}

/// plan_rename_dict as the JSON text `refactrail rename` prints.
pub fn rename_document(path: &str, function: &str, old: &str, new: &str) -> Result<String, String> {
    let file = Path::new(path);
    let linked = std::fs::symlink_metadata(file).map(|meta| meta.file_type().is_symlink()).unwrap_or(false);
    if linked {
        return Err("Linked rename inputs are unsupported".into());
    }
    let raw = std::fs::read(file).map_err(|error| format!("{path}: {error}"))?;
    let Some(text) = refactrail_engine::source::decode_source(&raw) else {
        return Err("Rename input must be UTF-8 Python source".into());
    };
    let (tree, symbols) = parse_checked(text, path)?;
    validate_request(&tree, &symbols, function, old, new)?;
    let selected = find_function(&tree, function)[0];
    let line = selected.loc.line;
    let spans = body_facts(function_parts(selected).0, old).spans;
    if spans.is_empty() {
        return Err(refusal("RENAME005", line, "No supported name spans"));
    }
    let output = replace_spans(text, &spans, new);
    validate_proposal(&tree, &output, function, old, new, line)?;
    let diff = unified_diff(&split_lines_keep(text), &split_lines_keep(&output), path, path);
    let limitations = [
        "Strings, comments and external references are unchanged.",
        "Tracing, frame inspection and local names remain observable.",
        "Recheck the source hash and run regressions before manual use.",
    ];
    let document = Json::Object(vec![
        ("schema_version".into(), Json::Str("refactrail-rename-1".into())),
        ("path".into(), Json::Str(path.into())),
        ("source_sha256".into(), Json::Str(sha256_hex(&raw))),
        ("status".into(), Json::Str("candidate_for_review".into())),
        ("can_apply".into(), Json::Bool(false)),
        ("behavior_verified".into(), Json::Bool(false)),
        ("function".into(), Json::Str(function.into())),
        ("old_name".into(), Json::Str(old.into())),
        ("new_name".into(), Json::Str(new.into())),
        ("diff".into(), Json::Str(diff)),
        ("proposed_source".into(), Json::Str(output)),
        ("limitations".into(), Json::List(limitations.iter().map(|text| Json::Str((*text).into())).collect())),
    ]);
    Ok(dumps(&document) + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(name: &str, text: &str) -> String {
        let path = std::env::temp_dir().join(format!("refactrail-rename-{}-{name}", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn renames_only_the_local_names() {
        let path = write_temp("ok.py", "def f(a):\n    x = a + 1\n    return x * 2  # x\n");
        let document = rename_document(&path, "f", "x", "total").unwrap();
        assert!(document.contains("\"proposed_source\": \"def f(a):\\n    total = a + 1\\n    return total * 2  # x\\n\""));
        assert!(document.contains("\"status\": \"candidate_for_review\""));
    }

    #[test]
    fn refusals_match_the_python_codes() {
        let cases = [
            ("def f():\n    x = 1\n", "g", "x", "y", "RENAME001 line 1"),
            ("def f():\n    x = [i for i in ()]\n", "f", "x", "y", "RENAME002 line 1"),
            ("def f():\n    x = 1\n", "f", "x", "class", "RENAME004 line 1"),
            ("def f(a):\n    x = a\n", "f", "a", "b", "RENAME005 line 1"),
            ("def f():\n    x = 1\n    y = 2\n", "f", "x", "y", "RENAME006 line 1"),
            ("def f():\n    import x\n    x.y = 1\n", "f", "x", "z", "RENAME007 line 1"),
        ];
        for (index, (source, function, old, new, expected)) in cases.iter().enumerate() {
            let path = write_temp(&format!("r{index}.py"), source);
            let error = rename_document(&path, function, old, new).unwrap_err();
            assert!(error.starts_with(expected), "{error}");
        }
    }

    #[test]
    fn line_starts_follow_universal_newlines() {
        assert_eq!(line_starts(b"a\r\nb\rc\nd"), vec![0, 3, 5, 7]);
    }
}
