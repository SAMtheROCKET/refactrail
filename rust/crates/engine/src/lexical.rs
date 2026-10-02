//! Lexical evidence from the symbol tables (mirror of lexical.py and
//! lexical_scopes.py): each loaded name with the scope that resolves it,
//! each imported alias with usage evidence, and the scope summaries the
//! `scope` command reports.

use crate::source::SourceFile;
use crate::walk::{walk_expr, walk_stmt, Expr, ExprContext, ExprKind, Module, Stmt, StmtKind, Visitor};
use refactrail_parser::ast::{Alias, Arguments};
use refactrail_parser::fast_hash::{FastMap, FastSet};
use refactrail_parser::symtable::{
    BlockType, SymbolTable, CELL, DEF_BOUND, DEF_IMPORT, DEF_LOCAL, DEF_PARAM, FREE, LOCAL, SCOPE_MASK, SCOPE_OFFSET, USE,
};
use refactrail_parser::Loc;
use std::sync::LazyLock;

pub const FLOW_LIMIT: &str = "Lexical bindings do not prove initialization, branch reachability, \
exception cleanup, deletion effects or call-time availability.";

const IMPLICIT_NAMES: [&str; 10] = [
    "__name__", "__file__", "__package__", "__doc__", "__builtins__", "__spec__", "__loader__", "__cached__",
    "__annotations__", "__path__",
];
const DYNAMIC_NAMES: [&str; 5] = ["exec", "eval", "globals", "locals", "vars"];

static BUILTINS: LazyLock<FastSet<&'static str>> =
    LazyLock::new(|| include_str!("../data/builtins.txt").lines().filter(|line| !line.is_empty()).collect());

/// Code point ranges where Python's regular expression `\w` matches.
static WORD_RANGES: LazyLock<Vec<(u32, u32)>> = LazyLock::new(|| {
    include_str!("../data/word_chars.txt")
        .lines()
        .filter_map(|line| {
            let (start, end) = line.split_once(' ')?;
            Some((u32::from_str_radix(start, 16).ok()?, u32::from_str_radix(end, 16).ok()?))
        })
        .collect()
});

/// Python's `\w` for one character (`str.isalnum()` or `_`).
pub fn is_word_char(character: char) -> bool {
    let code = character as u32;
    if code < 0x80 {
        return character.is_ascii_alphanumeric() || character == '_';
    }
    let ranges = &*WORD_RANGES;
    let at = ranges.partition_point(|&(start, _)| start <= code);
    at > 0 && code <= ranges[at - 1].1
}

/// `re.findall(r"\w+", text)`.
pub fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|character: char| !is_word_char(character)).filter(|word| !word.is_empty())
}

/// One loaded name and how it resolves.
pub struct Read {
    pub name: String,
    pub line: usize,
    pub column: usize,
    pub scope: usize,
    pub resolution: &'static str,
    pub annotation: bool,
}

/// One imported alias with usage evidence.
pub struct ImportRecord {
    pub name: String,
    pub line: usize,
    pub column: usize,
    pub used: bool,
    pub exempt: bool,
}

/// One scope summary.
pub struct ScopeRecord {
    pub id: usize,
    pub name: String,
    pub line: u32,
    pub kind: &'static str,
    pub symbols: Vec<String>,
}

/// The lexical report (fields of analyze_lexical_dict).
pub struct LexicalReport {
    pub limitations: Vec<String>,
    pub reads: Vec<Read>,
    pub imports: Vec<ImportRecord>,
    pub scopes: Vec<ScopeRecord>,
    pub diagnostics_supported: bool,
}

/// collect_scope_limits_list: constructs outside the diagnostic contract.
fn scope_limits(module: &Module) -> Vec<String> {
    struct Limits {
        found: FastSet<String>,
    }
    impl<'a> Visitor<'a> for Limits {
        fn visit_stmt(&mut self, node: &'a Stmt) {
            match &node.kind {
                StmtKind::ImportFrom { names, .. } if names.iter().any(|alias| &*alias.name == "*") => {
                    self.found.insert(format!("Wildcard import at line {}.", node.loc.line));
                }
                StmtKind::TypeAlias { .. } => {
                    self.found.insert(format!("Type parameter scope at line {}.", node.loc.line));
                }
                StmtKind::FunctionDef { type_params, .. }
                | StmtKind::AsyncFunctionDef { type_params, .. }
                | StmtKind::ClassDef { type_params, .. }
                    if !type_params.is_empty() =>
                {
                    self.found.insert(format!("Type parameter scope at line {}.", node.loc.line));
                }
                _ => {}
            }
            walk_stmt(self, node);
        }

        fn visit_expr(&mut self, node: &'a Expr) {
            if let ExprKind::Name { id, ctx: ExprContext::Load } = &node.kind {
                if DYNAMIC_NAMES.contains(&&**id) {
                    self.found.insert(format!("Dynamic namespace access at line {}.", node.loc.line));
                }
            }
            walk_expr(self, node);
        }
    }
    let mut limits = Limits { found: FastSet::default() };
    for statement in &module.body {
        limits.visit_stmt(statement);
    }
    let mut found: Vec<String> = limits.found.into_iter().collect();
    found.sort();
    found
}

/// A loaded name: (location, table, mangled name, annotation, isolated).
struct RawRead<'a> {
    loc: Loc,
    id: &'a str,
    table: usize,
    name: String,
    annotation: bool,
    isolated: bool,
}

/// ScopeCollector from lexical_scopes.py.
struct Collector<'a, 't> {
    symbols: &'t SymbolTable,
    stack: Vec<usize>,
    parents: FastMap<usize, usize>,
    /// Tables in the order the collector entered them (module first).
    order: Vec<usize>,
    used_tables: FastSet<usize>,
    reads: Vec<RawRead<'a>>,
    imports: Vec<(&'a Alias, usize, &'a str, bool)>,
    annotation: bool,
    class_name: &'a str,
    comprehension_names: Vec<FastSet<&'a str>>,
    walrus_globals: FastSet<&'a str>,
    error: Option<String>,
}

impl<'a> Collector<'a, '_> {
    fn current(&self) -> usize {
        *self.stack.last().expect("the module table")
    }

    fn collect_scope(&mut self, line: u32, name: &str, body: impl FnOnce(&mut Self)) {
        if self.error.is_some() {
            return;
        }
        let parent = self.current();
        let found = self.symbols.tables[parent].children.iter().copied().find(|&child| {
            let table = &self.symbols.tables[child];
            table.name.as_str() == name && table.lineno == line && !self.used_tables.contains(&child)
        });
        let Some(child) = found else {
            self.error = Some(format!("Unresolved compiler scope at line {line}"));
            return;
        };
        self.used_tables.insert(child);
        self.parents.insert(child, parent);
        self.order.push(child);
        self.stack.push(child);
        body(self);
        self.stack.pop();
    }

    fn annotation_expr(&mut self, node: Option<&'a Expr>) {
        if let Some(node) = node {
            let previous = self.annotation;
            self.annotation = true;
            self.visit_expr(node);
            self.annotation = previous;
        }
    }

    fn function(&mut self, line: u32, name: &str, decorators: &'a [Expr], args: &'a Arguments, returns: Option<&'a Expr>, body: FunctionBody<'a>) {
        for decorator in decorators {
            self.visit_expr(decorator);
        }
        for default in &args.defaults {
            self.visit_expr(default);
        }
        for default in args.kw_defaults.iter().flatten() {
            self.visit_expr(default);
        }
        let parameters = args.posonlyargs.iter().chain(&args.args).chain(&args.kwonlyargs).chain(args.vararg.as_ref()).chain(args.kwarg.as_ref());
        for parameter in parameters {
            self.annotation_expr(parameter.annotation.as_deref());
        }
        self.annotation_expr(returns);
        self.collect_scope(line, name, |collector| match body {
            FunctionBody::Statements(statements) => {
                for statement in statements {
                    collector.visit_stmt(statement);
                }
            }
            FunctionBody::Expression(expression) => collector.visit_expr(expression),
        });
    }

    fn comprehension(&mut self, node: &'a Expr) {
        let (generators, parts): (&'a [refactrail_parser::ast::Comprehension], Vec<&'a Expr>) = match &node.kind {
            ExprKind::ListComp { elt, generators } | ExprKind::SetComp { elt, generators } | ExprKind::GeneratorExp { elt, generators } => {
                (generators, vec![&**elt])
            }
            ExprKind::DictComp { key, value, generators } => (generators, vec![&**key, &**value]),
            _ => return,
        };
        let Some(first) = generators.first() else { return };
        self.visit_expr(&first.iter);
        let mut body: Vec<&'a Expr> = Vec::new();
        for (index, generator) in generators.iter().enumerate() {
            if index > 0 {
                body.push(&generator.iter);
            }
            body.extend(generator.ifs.iter());
        }
        body.extend(parts);
        if matches!(node.kind, ExprKind::GeneratorExp { .. }) {
            self.collect_scope(node.loc.line, "genexpr", |collector| {
                for expression in body {
                    collector.visit_expr(expression);
                }
            });
            return;
        }
        // Inlined comprehension: its target names are isolated.
        let mut names = FastSet::default();
        for generator in generators {
            collect_names(&generator.target, &mut names);
        }
        self.comprehension_names.push(names);
        let original = self.stack.clone();
        if self.symbols.tables[self.current()].block == BlockType::Class {
            self.stack.retain(|&table| self.symbols.tables[table].block != BlockType::Class);
        }
        for expression in body {
            self.visit_expr(expression);
        }
        self.stack = original;
        self.comprehension_names.pop();
    }
}

enum FunctionBody<'a> {
    Statements(&'a [Stmt]),
    Expression(&'a Expr),
}

/// Every Name id inside an expression (ast.walk over a target).
fn collect_names<'a>(expression: &'a Expr, names: &mut FastSet<&'a str>) {
    struct Names<'a, 'n> {
        names: &'n mut FastSet<&'a str>,
    }
    impl<'a> Visitor<'a> for Names<'a, '_> {
        fn visit_expr(&mut self, node: &'a Expr) {
            if let ExprKind::Name { id, .. } = &node.kind {
                self.names.insert(id);
            }
            walk_expr(self, node);
        }
    }
    Names { names }.visit_expr(expression);
}

impl<'a> Visitor<'a> for Collector<'a, '_> {
    fn visit_stmt(&mut self, node: &'a Stmt) {
        if self.error.is_some() {
            return;
        }
        match &node.kind {
            StmtKind::FunctionDef { name, args, body, decorator_list, returns, .. }
            | StmtKind::AsyncFunctionDef { name, args, body, decorator_list, returns, .. } => {
                self.function(node.loc.line, name, decorator_list, args, returns.as_deref(), FunctionBody::Statements(body));
            }
            StmtKind::ClassDef { name, bases, keywords, body, decorator_list, .. } => {
                for expression in decorator_list.iter().chain(bases).chain(keywords.iter().map(|keyword| &*keyword.value)) {
                    self.visit_expr(expression);
                }
                let previous = self.class_name;
                self.class_name = name;
                self.collect_scope(node.loc.line, name, |collector| {
                    for statement in body {
                        collector.visit_stmt(statement);
                    }
                });
                self.class_name = previous;
            }
            StmtKind::AnnAssign { target, annotation, value, .. } => {
                self.annotation_expr(Some(annotation));
                self.visit_expr(target);
                if let Some(value) = value {
                    self.visit_expr(value);
                }
            }
            StmtKind::Import { names } => self.import(names),
            StmtKind::ImportFrom { module, names, .. } => {
                if module.as_deref() != Some("__future__") {
                    self.import(names);
                }
            }
            _ => walk_stmt(self, node),
        }
    }

    fn visit_expr(&mut self, node: &'a Expr) {
        if self.error.is_some() {
            return;
        }
        match &node.kind {
            ExprKind::Name { id, ctx } => {
                if *ctx == ExprContext::Load {
                    let mut name = id.to_string();
                    if id.starts_with("__") && !id.ends_with("__") {
                        let prefix = self.class_name.trim_start_matches('_');
                        if !prefix.is_empty() {
                            name = format!("_{prefix}{id}");
                        }
                    }
                    let isolated = self.comprehension_names.iter().any(|names| names.contains(&**id));
                    self.reads.push(RawRead { loc: node.loc, id, table: self.current(), name, annotation: self.annotation, isolated });
                }
            }
            ExprKind::Lambda { args, body } => {
                self.function(node.loc.line, "lambda", &[], args, None, FunctionBody::Expression(body));
            }
            ExprKind::ListComp { .. } | ExprKind::SetComp { .. } | ExprKind::DictComp { .. } | ExprKind::GeneratorExp { .. } => {
                self.comprehension(node);
            }
            ExprKind::NamedExpr { target, value } => {
                if let ExprKind::Name { id, .. } = &target.kind {
                    let table = &self.symbols.tables[self.current()];
                    let is_global = table.contains(id) && symbol_is_global(table, id);
                    if self.current() == 0 || is_global {
                        self.walrus_globals.insert(id);
                    }
                }
                self.visit_expr(value);
            }
            _ => walk_expr(self, node),
        }
    }
}

impl<'a> Collector<'a, '_> {
    fn import(&mut self, names: &'a [Alias]) {
        for alias in names {
            let name: &'a str = match &alias.asname {
                Some(asname) => asname,
                None => alias.name.split('.').next().unwrap_or(""),
            };
            let export = alias.asname.as_deref() == Some(&*alias.name);
            self.imports.push((alias, self.current(), name, export));
        }
    }
}

fn scope_bits(flags: u32) -> u32 {
    (flags >> SCOPE_OFFSET) & SCOPE_MASK
}

/// Symbol.is_global(): global scope, or a bound name of a table named "top".
fn symbol_is_global(table: &refactrail_parser::symtable::Table, name: &str) -> bool {
    let flags = table.get(name);
    matches!(scope_bits(flags), 2 | 3) || (table.name.as_str() == "top" && flags & DEF_BOUND != 0)
}

/// Symbol.is_local().
fn symbol_is_local(table: &refactrail_parser::symtable::Table, name: &str) -> bool {
    let flags = table.get(name);
    matches!(scope_bits(flags), LOCAL | CELL) || (table.name.as_str() == "top" && flags & DEF_BOUND != 0)
}

/// is_bound_bool: assigned, imported, parameter or namespace.
fn symbol_is_bound(symbols: &SymbolTable, table: usize, name: &str) -> bool {
    let entry = &symbols.tables[table];
    let flags = entry.get(name);
    flags & (DEF_LOCAL | DEF_IMPORT | DEF_PARAM) != 0
        || entry.children.iter().any(|&child| symbols.tables[child].name.as_str() == name)
}

/// resolve_binding_tuple: (category, owner table).
fn resolve(collector: &Collector, table: usize, name: &str) -> (&'static str, Option<usize>) {
    let symbols = collector.symbols;
    let entry = &symbols.tables[table];
    if entry.contains(name) {
        if symbol_is_local(entry, name) && symbol_is_bound(symbols, table, name) {
            return ("local", Some(table));
        }
        if scope_bits(entry.get(name)) == FREE {
            let mut parent = collector.parents.get(&table).copied();
            while let Some(candidate) = parent {
                let candidate_table = &symbols.tables[candidate];
                if candidate_table.block == BlockType::Function
                    && candidate_table.contains(name)
                    && symbol_is_local(candidate_table, name)
                {
                    return ("closure", Some(candidate));
                }
                parent = collector.parents.get(&candidate).copied();
            }
            if name == "__class__" {
                return ("implicit_class", None);
            }
        }
    }
    if collector.walrus_globals.contains(name) {
        return ("module", Some(0));
    }
    if symbols.tables[0].contains(name) && symbol_is_bound(symbols, 0, name) {
        return ("module", Some(0));
    }
    if BUILTINS.contains(name) || IMPLICIT_NAMES.contains(&name) {
        return ("builtin_or_implicit", None);
    }
    ("unresolved", None)
}

/// Words of string constants and of `# type:` comments, which keep
/// imports from being reported as unused.
fn mentioned_words<'a>(module: &'a Module, source: &'a SourceFile, comments: &[(usize, usize)]) -> FastSet<String> {
    struct Strings {
        words: FastSet<String>,
    }
    impl<'a> Visitor<'a> for Strings {
        fn visit_expr(&mut self, node: &'a Expr) {
            if let Some(text) = crate::walk::str_constant(node) {
                self.words.extend(words(&text).map(str::to_string));
            }
            walk_expr(self, node);
        }
    }
    let mut strings = Strings { words: FastSet::default() };
    for statement in &module.body {
        strings.visit_stmt(statement);
    }
    for &(start, end) in comments {
        let comment = &source.text[start..end];
        let after_hash = crate::source::python_strip_start(&comment[1..]);
        if after_hash.starts_with("type:") {
            strings.words.extend(words(comment).map(str::to_string));
        }
    }
    strings.words
}

/// analyze_lexical_dict for a compiled module (collect_lexical_dict).
pub fn analyze(module: &Module, source: &SourceFile, comments: &[(usize, usize)], symbols: &SymbolTable) -> LexicalReport {
    let mut report = LexicalReport {
        limitations: vec![FLOW_LIMIT.to_string()],
        reads: Vec::new(),
        imports: Vec::new(),
        scopes: Vec::new(),
        diagnostics_supported: false,
    };
    let limits = scope_limits(module);
    if !limits.is_empty() {
        report.limitations.extend(limits);
        return report;
    }
    let mut collector = Collector {
        symbols,
        stack: vec![0],
        parents: FastMap::default(),
        order: vec![0],
        used_tables: FastSet::default(),
        reads: Vec::new(),
        imports: Vec::new(),
        annotation: false,
        class_name: "",
        comprehension_names: Vec::new(),
        walrus_globals: FastSet::default(),
        error: None,
    };
    for statement in &module.body {
        collector.visit_stmt(statement);
    }
    if let Some(error) = collector.error.take() {
        report.limitations.push(error);
        return report;
    }
    let identities: FastMap<usize, usize> = collector.order.iter().enumerate().map(|(index, &table)| (table, index)).collect();
    let mut used: FastSet<(usize, &str)> = FastSet::default();
    for read in &collector.reads {
        let (mut resolution, mut owner) = resolve(&collector, read.table, &read.name);
        if read.isolated {
            (resolution, owner) = ("comprehension", None);
        }
        if let Some(owner) = owner {
            used.insert((owner, read.name.as_str()));
        }
        let (line, column) = source.position(read.loc);
        report.reads.push(Read {
            name: read.id.to_string(),
            line,
            column,
            scope: identities[&read.table],
            resolution,
            annotation: read.annotation,
        });
    }
    report.scopes = collector
        .order
        .iter()
        .map(|&table| {
            let entry = &symbols.tables[table];
            let mut names: Vec<String> = entry.symbols().map(|(name, _)| name.to_string()).collect();
            names.sort();
            ScopeRecord { id: identities[&table], name: entry.name.to_string(), line: entry.lineno, kind: entry.block.name(), symbols: names }
        })
        .collect();
    let mentioned = mentioned_words(module, source, comments);
    for &(alias, table, name, export) in &collector.imports {
        let entry = &symbols.tables[table];
        if !entry.contains(name) {
            continue;
        }
        let flags = entry.get(name);
        let (line, column) = source.position(alias.loc);
        report.imports.push(ImportRecord {
            name: name.to_string(),
            line,
            column,
            used: used.contains(&(table, name)) || flags & USE != 0,
            exempt: export || mentioned.contains(name) || flags & DEF_LOCAL != 0,
        });
    }
    report.diagnostics_supported = true;
    report
}
