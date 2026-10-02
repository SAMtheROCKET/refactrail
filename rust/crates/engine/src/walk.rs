//! Access helpers over RefacTrail's own typed syntax tree, which has
//! CPython's `ast` shape (classes and field names as in Python's `ast`).

pub use refactrail_parser::ast::{
    walk_expr, walk_stmt, Alias, Arg, Arguments, CmpOperator, Expr, ExprContext, ExprKind, Module, Operator, Stmt,
    StmtKind, UnaryOperator, Visitor,
};
pub use refactrail_parser::Constant;

/// The parts of a (sync or async) function definition the rules read.
pub struct Function<'a> {
    pub name: &'a str,
    pub args: &'a Arguments,
    pub body: &'a [Stmt],
    pub decorator_list: &'a [Expr],
    pub returns: Option<&'a Expr>,
}

/// A function definition's parts, or None for other statements.
pub fn as_function(statement: &Stmt) -> Option<Function<'_>> {
    match &statement.kind {
        StmtKind::FunctionDef { name, args, body, decorator_list, returns, .. }
        | StmtKind::AsyncFunctionDef { name, args, body, decorator_list, returns, .. } => {
            Some(Function { name, args, body, decorator_list, returns: returns.as_deref() })
        }
        _ => None,
    }
}

/// The name a function or class definition binds ("" otherwise).
pub fn definition_name(statement: &Stmt) -> &str {
    match &statement.kind {
        StmtKind::FunctionDef { name, .. } | StmtKind::AsyncFunctionDef { name, .. } | StmtKind::ClassDef { name, .. } => name,
        _ => "",
    }
}

/// The value of an `ast.Constant`.
pub fn constant(expression: &Expr) -> Option<&Constant> {
    match &expression.kind {
        ExprKind::Constant { value, .. } => Some(value),
        _ => None,
    }
}

/// The text of a str constant.
pub fn str_constant(expression: &Expr) -> Option<String> {
    match constant(expression)? {
        Constant::Str(points) => Some(points.iter().map(|&point| char::from_u32(point).unwrap_or('\u{fffd}')).collect()),
        _ => None,
    }
}

/// Whether an expression is the str constant `text`.
pub fn is_str_constant(expression: &Expr, text: &str) -> bool {
    match constant(expression) {
        Some(Constant::Str(points)) => {
            let mut chars = text.chars();
            points.iter().all(|&point| chars.next() == char::from_u32(point)) && chars.next().is_none()
        }
        _ => false,
    }
}

/// The name of a Name node.
pub fn name_id(expression: &Expr) -> Option<&str> {
    match &expression.kind {
        ExprKind::Name { id, .. } => Some(id),
        _ => None,
    }
}

/// Every parameter of an `arguments` node, in CPython's order.
pub fn all_args(arguments: &Arguments) -> impl Iterator<Item = &Arg> {
    arguments
        .posonlyargs
        .iter()
        .chain(&arguments.args)
        .chain(arguments.vararg.as_ref())
        .chain(&arguments.kwonlyargs)
        .chain(arguments.kwarg.as_ref())
}

/// Positional parameters (posonly, then regular).
pub fn positional_args(arguments: &Arguments) -> Vec<&Arg> {
    arguments.posonlyargs.iter().chain(&arguments.args).collect()
}

/// The name an import alias binds (`asname`, else the first dotted part).
pub fn alias_binding(alias: &Alias) -> &str {
    match &alias.asname {
        Some(asname) => asname,
        None => alias.name.split('.').next().unwrap_or(""),
    }
}

/// Visit the statement blocks directly inside a compound statement.
pub fn for_each_child_block<'a>(statement: &'a Stmt, mut visit: impl FnMut(&'a [Stmt])) {
    match &statement.kind {
        StmtKind::If { body, orelse, .. }
        | StmtKind::For { body, orelse, .. }
        | StmtKind::AsyncFor { body, orelse, .. }
        | StmtKind::While { body, orelse, .. } => {
            visit(body);
            visit(orelse);
        }
        StmtKind::With { body, .. } | StmtKind::AsyncWith { body, .. } => visit(body),
        StmtKind::Try { body, handlers, orelse, finalbody } | StmtKind::TryStar { body, handlers, orelse, finalbody } => {
            visit(body);
            for handler in handlers {
                visit(&handler.body);
            }
            visit(orelse);
            visit(finalbody);
        }
        StmtKind::Match { cases, .. } => {
            for case in cases {
                visit(&case.body);
            }
        }
        _ => {}
    }
}

/// Whether `predicate` holds for an expression or any expression inside
/// it (lambdas and comprehensions included).
pub fn any_expr(expression: &Expr, predicate: &impl Fn(&Expr) -> bool) -> bool {
    struct Finder<'p, P> {
        predicate: &'p P,
        found: bool,
    }
    impl<'a, P: Fn(&Expr) -> bool> Visitor<'a> for Finder<'_, P> {
        fn visit_expr(&mut self, node: &'a Expr) {
            if self.found {
                return;
            }
            if (self.predicate)(node) {
                self.found = true;
                return;
            }
            walk_expr(self, node);
        }
    }
    let mut finder = Finder { predicate, found: false };
    finder.visit_expr(expression);
    finder.found
}
