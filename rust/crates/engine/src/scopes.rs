//! Earliest binding of each name per scope (mirror of scopes.py).

use crate::source::SourceFile;
use crate::walk::{
    alias_binding, all_args, walk_expr, walk_stmt, Arguments, Expr, ExprContext, ExprKind, Stmt, StmtKind, Visitor,
};
use refactrail_parser::ast::{Comprehension, ExceptHandler};
use refactrail_parser::fast_hash::{FastMap, FastSet};
use refactrail_parser::Loc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Function,
    Class,
    Parameter,
    Variable,
    Import,
    Except,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScopeKind {
    Module,
    Class,
    Function,
    Lambda,
    Comprehension,
}

pub struct Binding<'a> {
    pub name: &'a str,
    pub kind: Kind,
    pub scope_kind: ScopeKind,
    pub line: usize,
    pub column: usize,
    pub value: Option<&'a Expr>,
    pub imported_name: &'a str,
}

struct Scope<'a> {
    kind: ScopeKind,
    excluded: FastSet<&'a str>,
    parent: Option<usize>,
    bindings: FastMap<&'a str, Binding<'a>>,
}

pub struct BindingCollector<'s, 'a> {
    source: &'s SourceFile<'s>,
    scopes: Vec<Scope<'a>>,
    current: usize,
}

/// collect_declared_set: global/nonlocal names in a function body (not
/// entering nested functions, classes or lambdas).
fn declared_names(body: &[Stmt]) -> FastSet<&str> {
    struct Declared<'a> {
        names: FastSet<&'a str>,
    }
    impl<'a> Visitor<'a> for Declared<'a> {
        fn visit_stmt(&mut self, node: &'a Stmt) {
            match &node.kind {
                StmtKind::Global { names } | StmtKind::Nonlocal { names } => self.names.extend(names.iter().map(|name| &**name)),
                StmtKind::FunctionDef { .. } | StmtKind::AsyncFunctionDef { .. } | StmtKind::ClassDef { .. } => {}
                _ => walk_stmt(self, node),
            }
        }

        fn visit_expr(&mut self, _node: &'a Expr) {}
    }
    let mut declared = Declared { names: FastSet::default() };
    for statement in body {
        declared.visit_stmt(statement);
    }
    declared.names
}

impl<'s, 'a> BindingCollector<'s, 'a> {
    pub fn collect(source: &'s SourceFile<'s>, body: &'a [Stmt]) -> Vec<Binding<'a>> {
        let mut collector = BindingCollector { source, scopes: Vec::new(), current: 0 };
        collector.current = collector.open_scope(ScopeKind::Module, FastSet::default(), None);
        for statement in body {
            collector.visit_stmt(statement);
        }
        let mut bindings: Vec<Binding<'a>> =
            collector.scopes.into_iter().flat_map(|scope| scope.bindings.into_values()).collect();
        bindings.sort_by(|left, right| (left.line, left.column, &left.name).cmp(&(right.line, right.column, &right.name)));
        bindings
    }

    fn open_scope(&mut self, kind: ScopeKind, excluded: FastSet<&'a str>, parent: Option<usize>) -> usize {
        self.scopes.push(Scope { kind, excluded, parent, bindings: FastMap::default() });
        self.scopes.len() - 1
    }

    fn record(&mut self, scope_index: usize, mut binding: Binding<'a>) {
        let scope = &mut self.scopes[scope_index];
        if scope.excluded.contains(binding.name) {
            return;
        }
        binding.scope_kind = scope.kind;
        match scope.bindings.get(binding.name) {
            Some(current) if (current.line, current.column) <= (binding.line, binding.column) => {}
            _ => {
                scope.bindings.insert(binding.name, binding);
            }
        }
    }

    fn binding(&self, name: &'a str, kind: Kind, loc: Loc, value: Option<&'a Expr>) -> Binding<'a> {
        let (line, column) = self.source.position(loc);
        Binding { name, kind, scope_kind: ScopeKind::Module, line, column, value, imported_name: "" }
    }

    fn walrus_scope(&self, mut index: usize) -> usize {
        while self.scopes[index].kind == ScopeKind::Comprehension {
            match self.scopes[index].parent {
                Some(parent) => index = parent,
                None => break,
            }
        }
        index
    }

    fn with_scope(&mut self, scope_index: usize, action: impl FnOnce(&mut Self)) {
        let saved = self.current;
        self.current = scope_index;
        action(self);
        self.current = saved;
    }

    fn record_parameters(&mut self, arguments: &'a Arguments) {
        for parameter in all_args(arguments) {
            let binding = self.binding(&parameter.arg, Kind::Parameter, parameter.loc, None);
            self.record(self.current, binding);
        }
    }

    fn visit_defaults(&mut self, arguments: &'a Arguments) {
        for default in &arguments.defaults {
            self.visit_expr(default);
        }
        for default in arguments.kw_defaults.iter().flatten() {
            self.visit_expr(default);
        }
    }

    fn visit_function(&mut self, function: &'a Stmt) {
        let Some(parts) = crate::walk::as_function(function) else { return };
        for decorator in parts.decorator_list {
            self.visit_expr(decorator);
        }
        self.visit_defaults(parts.args);
        let mut binding = self.binding(parts.name, Kind::Function, function.loc, None);
        (binding.line, binding.column) = self.source.definition_name(function.loc, parts.name);
        self.record(self.current, binding);
        let inner = self.open_scope(ScopeKind::Function, declared_names(parts.body), Some(self.current));
        self.with_scope(inner, |collector| {
            collector.record_parameters(parts.args);
            for statement in parts.body {
                collector.visit_stmt(statement);
            }
        });
    }

    fn visit_class(&mut self, class: &'a Stmt) {
        let StmtKind::ClassDef { name, bases, keywords, body, decorator_list, .. } = &class.kind else { return };
        for decorator in decorator_list {
            self.visit_expr(decorator);
        }
        for base in bases {
            self.visit_expr(base);
        }
        for keyword in keywords {
            self.visit_expr(&keyword.value);
        }
        let mut binding = self.binding(name, Kind::Class, class.loc, None);
        (binding.line, binding.column) = self.source.definition_name(class.loc, name);
        self.record(self.current, binding);
        let inner = self.open_scope(ScopeKind::Class, FastSet::default(), Some(self.current));
        self.with_scope(inner, |collector| {
            for statement in body {
                collector.visit_stmt(statement);
            }
        });
    }

    fn visit_assignment(&mut self, targets: &'a [Expr], value: Option<&'a Expr>, annotation: Option<&'a Expr>, walrus: bool) {
        if let Some(value) = value {
            self.visit_expr(value);
        }
        if let Some(annotation) = annotation {
            self.visit_expr(annotation);
        }
        let target_scope = if walrus { self.walrus_scope(self.current) } else { self.current };
        if let [target] = targets {
            if let ExprKind::Name { id, .. } = &target.kind {
                let binding = self.binding(id, Kind::Variable, target.loc, value);
                self.record(target_scope, binding);
                return;
            }
        }
        self.with_scope(target_scope, |collector| {
            for target in targets {
                collector.visit_expr(target);
            }
        });
    }

    fn record_imports(&mut self, names: &'a [refactrail_parser::ast::Alias]) {
        for alias in names {
            let full_name: &'a str = &alias.name;
            if full_name == "*" {
                continue;
            }
            let mut binding = self.binding(alias_binding(alias), Kind::Import, alias.loc, None);
            binding.imported_name = full_name.rsplit('.').next().unwrap_or("");
            self.record(self.current, binding);
        }
    }

    fn visit_comprehension(&mut self, expression: &'a Expr, generators: &'a [Comprehension]) {
        if let Some(first) = generators.first() {
            self.visit_expr(&first.iter);
        }
        let inner = self.open_scope(ScopeKind::Comprehension, FastSet::default(), Some(self.current));
        self.with_scope(inner, |collector| {
            for (index, generator) in generators.iter().enumerate() {
                if index > 0 {
                    collector.visit_expr(&generator.iter);
                }
                collector.visit_expr(&generator.target);
                for condition in &generator.ifs {
                    collector.visit_expr(condition);
                }
            }
            match &expression.kind {
                ExprKind::DictComp { key, value, .. } => {
                    collector.visit_expr(key);
                    collector.visit_expr(value);
                }
                ExprKind::ListComp { elt, .. } | ExprKind::SetComp { elt, .. } | ExprKind::GeneratorExp { elt, .. } => {
                    collector.visit_expr(elt);
                }
                _ => {}
            }
        });
    }
}

impl<'a> Visitor<'a> for BindingCollector<'_, 'a> {
    fn visit_stmt(&mut self, node: &'a Stmt) {
        match &node.kind {
            StmtKind::FunctionDef { .. } | StmtKind::AsyncFunctionDef { .. } => self.visit_function(node),
            StmtKind::ClassDef { .. } => self.visit_class(node),
            StmtKind::Assign { targets, value, .. } => self.visit_assignment(targets, Some(value), None, false),
            StmtKind::AnnAssign { target, annotation, value, .. } => {
                self.visit_assignment(std::slice::from_ref(&**target), value.as_deref(), Some(annotation), false);
            }
            StmtKind::Import { names } | StmtKind::ImportFrom { names, .. } => self.record_imports(names),
            _ => walk_stmt(self, node),
        }
    }

    fn visit_excepthandler(&mut self, node: &'a ExceptHandler) {
        if let Some(name) = &node.name {
            let binding = self.binding(name, Kind::Except, node.loc, None);
            self.record(self.current, binding);
        }
        if let Some(kind) = &node.type_ {
            self.visit_expr(kind);
        }
        for statement in &node.body {
            self.visit_stmt(statement);
        }
    }

    fn visit_expr(&mut self, node: &'a Expr) {
        match &node.kind {
            ExprKind::Name { id, ctx: ExprContext::Store } => {
                let binding = self.binding(id, Kind::Variable, node.loc, None);
                self.record(self.current, binding);
            }
            ExprKind::NamedExpr { target, value } => {
                self.visit_assignment(std::slice::from_ref(&**target), Some(value), None, true);
            }
            ExprKind::Lambda { args, body } => {
                self.visit_defaults(args);
                let inner = self.open_scope(ScopeKind::Lambda, FastSet::default(), Some(self.current));
                self.with_scope(inner, |collector| {
                    collector.record_parameters(args);
                    collector.visit_expr(body);
                });
            }
            ExprKind::ListComp { generators, .. }
            | ExprKind::SetComp { generators, .. }
            | ExprKind::DictComp { generators, .. }
            | ExprKind::GeneratorExp { generators, .. } => self.visit_comprehension(node, generators),
            _ => walk_expr(self, node),
        }
    }
}
