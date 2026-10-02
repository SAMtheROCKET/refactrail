//! Per-file evidence for the project index (mirror of project_index.py's
//! collect_symbols_list and collect_imports_list): named definitions with
//! their lexical parents, and static import requests.

use refactrail_parser::ast::{walk_expr, walk_stmt, Expr, Module, Stmt, StmtKind, Visitor};

/// A class or function definition.
pub struct SymbolRecord {
    pub name: String,
    pub kind: &'static str,
    pub line: u32,
    pub end_line: u32,
}

/// A static import request.
pub struct ImportRecord {
    pub module: String,
    pub level: i64,
    pub names: Vec<String>,
    pub line: u32,
}

/// collect_symbols_list: definitions with dotted parent names, sorted by
/// (line, name).
pub fn symbols(module: &Module) -> Vec<SymbolRecord> {
    struct Symbols {
        parents: Vec<String>,
        records: Vec<SymbolRecord>,
    }
    impl<'a> Visitor<'a> for Symbols {
        fn visit_stmt(&mut self, node: &'a Stmt) {
            let (name, kind) = match &node.kind {
                StmtKind::FunctionDef { name, .. } => (name, "FunctionDef"),
                StmtKind::AsyncFunctionDef { name, .. } => (name, "AsyncFunctionDef"),
                StmtKind::ClassDef { name, .. } => (name, "ClassDef"),
                _ => return walk_stmt(self, node),
            };
            let parent = self.parents.last().cloned().unwrap_or_default();
            let dotted = if parent.is_empty() { name.to_string() } else { format!("{parent}.{name}") };
            self.records.push(SymbolRecord { name: dotted.clone(), kind, line: node.loc.line, end_line: node.loc.end_line });
            self.parents.push(dotted);
            walk_stmt(self, node);
            self.parents.pop();
        }

        fn visit_expr(&mut self, node: &'a Expr) {
            walk_expr(self, node);
        }
    }
    let mut collector = Symbols { parents: Vec::new(), records: Vec::new() };
    for statement in &module.body {
        collector.visit_stmt(statement);
    }
    let mut records = collector.records;
    records.sort_by(|left, right| (left.line, &left.name).cmp(&(right.line, &right.name)));
    records
}

/// collect_imports_list: import requests sorted by (line, module).
pub fn imports(module: &Module) -> Vec<ImportRecord> {
    struct Imports {
        records: Vec<ImportRecord>,
    }
    impl<'a> Visitor<'a> for Imports {
        fn visit_stmt(&mut self, node: &'a Stmt) {
            match &node.kind {
                StmtKind::Import { names } => {
                    for alias in names {
                        self.records.push(ImportRecord { module: alias.name.to_string(), level: 0, names: Vec::new(), line: node.loc.line });
                    }
                }
                StmtKind::ImportFrom { module, names, level } => self.records.push(ImportRecord {
                    module: module.as_deref().unwrap_or("").to_string(),
                    level: *level,
                    names: names.iter().map(|alias| alias.name.to_string()).collect(),
                    line: node.loc.line,
                }),
                _ => {}
            }
            walk_stmt(self, node);
        }
    }
    let mut collector = Imports { records: Vec::new() };
    for statement in &module.body {
        collector.visit_stmt(statement);
    }
    let mut records = collector.records;
    records.sort_by(|left, right| (left.line, &left.module).cmp(&(right.line, &right.module)));
    records
}
