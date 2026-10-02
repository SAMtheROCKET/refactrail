//! count_module_bindings_dict from facts.py: module-level binding sites.

use crate::walk::{alias_binding, walk_expr, walk_stmt, Expr, ExprContext, ExprKind, Stmt, StmtKind, Visitor};
use refactrail_parser::fast_hash::FastMap;

fn add<'a>(counts: &mut FastMap<&'a str, usize>, name: &'a str, amount: usize) {
    *counts.entry(name).or_insert(0) += amount;
}

/// Module-level walk: definitions and imports count their names, Store
/// names count, comprehensions only count their := targets, lambdas and
/// nested function/class bodies are not entered.
struct ModuleCounter<'a, 'c> {
    counts: &'c mut FastMap<&'a str, usize>,
}

impl<'a> Visitor<'a> for ModuleCounter<'a, '_> {
    fn visit_stmt(&mut self, node: &'a Stmt) {
        match &node.kind {
            StmtKind::FunctionDef { name, .. } | StmtKind::AsyncFunctionDef { name, .. } | StmtKind::ClassDef { name, .. } => {
                add(self.counts, name, 1);
            }
            StmtKind::Import { names } | StmtKind::ImportFrom { names, .. } => {
                for alias in names {
                    add(self.counts, alias_binding(alias), 1);
                }
            }
            _ => walk_stmt(self, node),
        }
    }

    fn visit_expr(&mut self, node: &'a Expr) {
        match &node.kind {
            ExprKind::Name { id, ctx: ExprContext::Store } => add(self.counts, id, 1),
            ExprKind::ListComp { .. } | ExprKind::SetComp { .. } | ExprKind::DictComp { .. } | ExprKind::GeneratorExp { .. } => {
                walk_expr(&mut WalrusCounter { counts: self.counts }, node);
            }
            ExprKind::Lambda { .. } => {}
            _ => walk_expr(self, node),
        }
    }
}

/// Every `:=` target name at any depth.
struct WalrusCounter<'a, 'c> {
    counts: &'c mut FastMap<&'a str, usize>,
}

impl<'a> Visitor<'a> for WalrusCounter<'a, '_> {
    fn visit_expr(&mut self, node: &'a Expr) {
        if let ExprKind::NamedExpr { target, .. } = &node.kind {
            if let ExprKind::Name { id, .. } = &target.kind {
                add(self.counts, id, 1);
            }
        }
        walk_expr(self, node);
    }
}

/// `global NAME` statements at any depth.
struct GlobalCounter<'a, 'c> {
    counts: &'c mut FastMap<&'a str, usize>,
}

impl<'a> Visitor<'a> for GlobalCounter<'a, '_> {
    fn visit_stmt(&mut self, node: &'a Stmt) {
        if let StmtKind::Global { names } = &node.kind {
            for name in names {
                add(self.counts, name, 2);
            }
        }
        walk_stmt(self, node);
    }

    fn visit_expr(&mut self, _node: &'a Expr) {}
}

/// Name -> number of module-level binding sites (see facts.py).
pub fn count_module_bindings(body: &[Stmt]) -> FastMap<&str, usize> {
    let mut counts = FastMap::default();
    let mut counter = ModuleCounter { counts: &mut counts };
    for statement in body {
        counter.visit_stmt(statement);
    }
    // `global NAME` anywhere adds 2.
    let mut globals = GlobalCounter { counts: &mut counts };
    for statement in body {
        globals.visit_stmt(statement);
    }
    counts
}
