//! The checks CPython's `compile()` makes after parsing: `__future__`
//! imports, the symbol table pass (`symtable.rs`) and the code generator's
//! context checks.
//! Each reports the first error CPython would raise, with its message and
//! location (1-based line, offset = byte column + 1). Nothing is executed.
//!
//! Children are visited in CPython's field order, so the first error found
//! is the one CPython reports.

use crate::ast::{
    walk_expr, walk_stmt, Arg, Arguments, Comprehension, Expr, ExprContext, ExprKind, Keyword, MatchCase, Module, Pattern,
    PatternKind,
    Stmt, StmtKind, Visitor,
};
use crate::node::{Constant, Loc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError {
    pub line: u32,
    pub offset: u32,
    pub message: String,
}

fn error_at(loc: Loc, message: String) -> CompileError {
    CompileError { line: loc.line, offset: loc.col + 1, message }
}

type Check = Result<(), CompileError>;

/// Run all post-parse checks on a module, in CPython's order.
pub fn check(module: &Module) -> Option<CompileError> {
    let run = || -> Result<(), CompileError> {
        let future_line = check_future(&module.body)?;
        crate::symtable::check(module, future_annotations(&module.body))?;
        let mut codegen = Codegen { future_line, frames: vec![Frame::module()] };
        codegen.statements(&module.body)
    };
    run().err()
}

/// The post-parse checks, returning the module's symbol table when it
/// compiles (so callers need not build it again).
pub fn check_with_symbols(module: &Module) -> Result<crate::symtable::SymbolTable, CompileError> {
    let future_line = check_future(&module.body)?;
    let symbols = crate::symtable::build(module, future_annotations(&module.body))?;
    let mut codegen = Codegen { future_line, frames: vec![Frame::module()] };
    codegen.statements(&module.body)?;
    Ok(symbols)
}

// ----- tree helpers ----------------------------------------------------

fn is_comprehension(expr: &Expr) -> bool {
    matches!(expr.kind, ExprKind::ListComp { .. } | ExprKind::SetComp { .. } | ExprKind::DictComp { .. } | ExprKind::GeneratorExp { .. })
}

/// Every argument of an `arguments` node, in CPython's order.
fn all_args(arguments: &Arguments) -> impl Iterator<Item = &Arg> {
    arguments
        .posonlyargs
        .iter()
        .chain(&arguments.args)
        .chain(arguments.vararg.as_ref())
        .chain(&arguments.kwonlyargs)
        .chain(arguments.kwarg.as_ref())
}

/// Visit an expression's child expressions in field order (keywords as
/// their values, comprehensions as target, iter, ifs; a lambda's
/// arguments are left out).
fn expr_children<'a>(expr: &'a Expr, visit: &mut impl FnMut(&'a Expr) -> Check) -> Check {
    fn each<'a>(items: &'a [Expr], visit: &mut impl FnMut(&'a Expr) -> Check) -> Check {
        items.iter().try_for_each(|item| visit(item))
    }
    fn maybe<'a>(item: &'a Option<Box<Expr>>, visit: &mut impl FnMut(&'a Expr) -> Check) -> Check {
        item.as_deref().map_or(Ok(()), |item| visit(item))
    }
    fn generators<'a>(generators: &'a [Comprehension], visit: &mut impl FnMut(&'a Expr) -> Check) -> Check {
        for generator in generators {
            visit(&generator.target)?;
            visit(&generator.iter)?;
            each(&generator.ifs, visit)?;
        }
        Ok(())
    }
    match &expr.kind {
        ExprKind::BoolOp { values, .. } => each(values, visit),
        ExprKind::NamedExpr { target, value } => {
            visit(target)?;
            visit(value)
        }
        ExprKind::BinOp { left, right, .. } => {
            visit(left)?;
            visit(right)
        }
        ExprKind::UnaryOp { operand, .. } => visit(operand),
        ExprKind::Lambda { body, .. } => visit(body),
        ExprKind::IfExp { test, body, orelse } => {
            visit(test)?;
            visit(body)?;
            visit(orelse)
        }
        ExprKind::Dict { keys, values } => {
            for key in keys.iter().flatten() {
                visit(key)?;
            }
            each(values, visit)
        }
        ExprKind::Set { elts } | ExprKind::List { elts, .. } | ExprKind::Tuple { elts, .. } => each(elts, visit),
        ExprKind::ListComp { elt, generators: items } | ExprKind::SetComp { elt, generators: items } | ExprKind::GeneratorExp { elt, generators: items } => {
            visit(elt)?;
            generators(items, visit)
        }
        ExprKind::DictComp { key, value, generators: items } => {
            visit(key)?;
            visit(value)?;
            generators(items, visit)
        }
        ExprKind::Await { value } | ExprKind::YieldFrom { value } => visit(value),
        ExprKind::Yield { value } => maybe(value, visit),
        ExprKind::Compare { left, comparators, .. } => {
            visit(left)?;
            each(comparators, visit)
        }
        ExprKind::Call { func, args, keywords } => {
            visit(func)?;
            each(args, visit)?;
            keywords.iter().try_for_each(|keyword| visit(&keyword.value))
        }
        ExprKind::FormattedValue { value, format_spec, .. } => {
            visit(value)?;
            maybe(format_spec, visit)
        }
        ExprKind::JoinedStr { values } => each(values, visit),
        ExprKind::Constant { .. } | ExprKind::Name { .. } => Ok(()),
        ExprKind::Attribute { value, .. } | ExprKind::Starred { value, .. } => visit(value),
        ExprKind::Subscript { value, slice, .. } => {
            visit(value)?;
            visit(slice)
        }
        ExprKind::Slice { lower, upper, step } => {
            maybe(lower, visit)?;
            maybe(upper, visit)?;
            maybe(step, visit)
        }
    }
}

// ----- __future__ -------------------------------------------------------

const FUTURE_FEATURES: [&str; 10] = [
    "nested_scopes", "generators", "division", "absolute_import", "with_statement",
    "print_function", "unicode_literals", "barry_as_FLUFL", "generator_stop", "annotations",
];

/// future.c: validate the leading `from __future__` imports and return
/// the line of the last one (0 when there is none).
fn check_future(body: &[Stmt]) -> Result<u32, CompileError> {
    let (mut done, mut previous_line, mut future_line) = (false, 0, 0);
    for (index, statement) in body.iter().enumerate() {
        let loc = statement.loc;
        if done && loc.line > previous_line {
            return Ok(future_line);
        }
        previous_line = loc.line;
        match &statement.kind {
            StmtKind::ImportFrom { module: Some(module), names, .. } if &**module == "__future__" => {
                if done {
                    return Err(error_at(loc, "from __future__ imports must occur at the beginning of the file".into()));
                }
                for alias in names {
                    let name = &*alias.name;
                    if name == "braces" {
                        return Err(error_at(loc, "not a chance".into()));
                    }
                    if !FUTURE_FEATURES.contains(&name) {
                        return Err(error_at(loc, format!("future feature {name} is not defined")));
                    }
                }
                future_line = loc.line;
            }
            StmtKind::ImportFrom { .. } => done = true,
            _ if index == 0 && is_docstring(statement) => {}
            _ => done = true,
        }
    }
    Ok(future_line)
}

/// Whether the module's leading `from __future__` imports enable
/// `annotations` (PEP 563), as future.c collects features.
pub fn future_annotations(body: &[Stmt]) -> bool {
    let (mut done, mut previous_line, mut enabled) = (false, 0, false);
    for (index, statement) in body.iter().enumerate() {
        let line = statement.loc.line;
        if done && line > previous_line {
            break;
        }
        previous_line = line;
        match &statement.kind {
            StmtKind::ImportFrom { module: Some(module), names, .. } if &**module == "__future__" && !done => {
                enabled |= names.iter().any(|alias| &*alias.name == "annotations");
            }
            StmtKind::ImportFrom { .. } => done = true,
            _ if index == 0 && is_docstring(statement) => {}
            _ => done = true,
        }
    }
    enabled
}

fn is_docstring(statement: &Stmt) -> bool {
    matches!(&statement.kind, StmtKind::Expr { value } if matches!(value.kind, ExprKind::Constant { value: Constant::Str(_), .. }))
}

// ----- code generator checks -------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    Module,
    Class,
    Function { is_async: bool, async_generator: bool },
    Lambda,
    GeneratorExp,
}

#[derive(Clone, Copy)]
struct Frame {
    kind: FrameKind,
    loops: u32,
}

impl Frame {
    fn module() -> Frame {
        Frame { kind: FrameKind::Module, loops: 0 }
    }
}

struct Codegen {
    future_line: u32,
    frames: Vec<Frame>,
}

/// Finds a `yield` in a function's own scope (nested functions, classes,
/// lambdas and generator expressions excluded).
struct YieldFinder {
    found: bool,
}

impl<'a> Visitor<'a> for YieldFinder {
    fn visit_stmt(&mut self, node: &'a Stmt) {
        if !self.found && !matches!(node.kind, StmtKind::FunctionDef { .. } | StmtKind::AsyncFunctionDef { .. } | StmtKind::ClassDef { .. }) {
            walk_stmt(self, node);
        }
    }

    fn visit_expr(&mut self, node: &'a Expr) {
        match node.kind {
            _ if self.found => {}
            ExprKind::Yield { .. } | ExprKind::YieldFrom { .. } => self.found = true,
            ExprKind::Lambda { .. } | ExprKind::GeneratorExp { .. } => {}
            _ => walk_expr(self, node),
        }
    }
}

/// Whether a function body yields (not counting nested scopes).
fn contains_yield(function: &Stmt) -> bool {
    let mut finder = YieldFinder { found: false };
    walk_stmt(&mut finder, function);
    finder.found
}

/// Whether a comprehension awaits or iterates asynchronously in its own
/// scope (the outermost iterable belongs to the enclosing scope).
fn comprehension_is_async(expr: &Expr) -> bool {
    let generators = match &expr.kind {
        ExprKind::ListComp { generators, .. }
        | ExprKind::SetComp { generators, .. }
        | ExprKind::DictComp { generators, .. }
        | ExprKind::GeneratorExp { generators, .. } => generators,
        _ => return false,
    };
    if generators.iter().any(|generator| generator.is_async == 1) {
        return true;
    }
    for (index, generator) in generators.iter().enumerate() {
        if contains_await(&generator.target)
            || (index > 0 && contains_await(&generator.iter))
            || generator.ifs.iter().any(contains_await)
        {
            return true;
        }
    }
    match &expr.kind {
        ExprKind::DictComp { key, value, .. } => contains_await(key) || contains_await(value),
        ExprKind::ListComp { elt, .. } | ExprKind::SetComp { elt, .. } | ExprKind::GeneratorExp { elt, .. } => contains_await(elt),
        _ => false,
    }
}

/// Finds an `await` outside nested scopes.
struct AwaitFinder {
    found: bool,
}

impl<'a> Visitor<'a> for AwaitFinder {
    fn visit_stmt(&mut self, node: &'a Stmt) {
        if !self.found && !matches!(node.kind, StmtKind::FunctionDef { .. } | StmtKind::AsyncFunctionDef { .. } | StmtKind::ClassDef { .. }) {
            walk_stmt(self, node);
        }
    }

    fn visit_expr(&mut self, node: &'a Expr) {
        if self.found {
            return;
        }
        match &node.kind {
            ExprKind::Await { .. } => self.found = true,
            ExprKind::Lambda { .. } | ExprKind::GeneratorExp { .. } => {}
            ExprKind::ListComp { generators, .. } | ExprKind::SetComp { generators, .. } | ExprKind::DictComp { generators, .. } => {
                self.found = comprehension_is_async(node) || generators.first().is_some_and(|first| contains_await(&first.iter));
            }
            _ => walk_expr(self, node),
        }
    }
}

fn contains_await(item: &Expr) -> bool {
    let mut finder = AwaitFinder { found: false };
    finder.visit_expr(item);
    finder.found
}

/// Where an expression sits, for the starred-expression checks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Other,
    /// An assignment or deletion target.
    Assign,
    /// A call argument or class base.
    Call,
    /// An element of a list, tuple or set.
    Display,
    Annotation,
}

impl Codegen {
    fn frame(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("module frame")
    }

    fn function_like(&self) -> bool {
        !matches!(self.frames.last().map(|f| f.kind), Some(FrameKind::Module | FrameKind::Class))
    }

    fn in_async_function(&self) -> bool {
        matches!(self.frames.last().map(|f| f.kind), Some(FrameKind::Function { is_async: true, .. }))
    }

    fn statements(&mut self, body: &[Stmt]) -> Check {
        body.iter().try_for_each(|statement| self.statement(statement))
    }

    fn exprs(&mut self, items: &[Expr], place: Place) -> Check {
        items.iter().try_for_each(|item| self.expr(item, place))
    }

    fn maybe_expr(&mut self, item: &Option<Box<Expr>>, place: Place) -> Check {
        item.as_deref().map_or(Ok(()), |item| self.expr(item, place))
    }

    fn with_frame(&mut self, frame: Frame, run: impl FnOnce(&mut Self) -> Check) -> Check {
        self.frames.push(frame);
        let result = run(self);
        self.frames.pop();
        result
    }

    fn arguments(&mut self, arguments: &Arguments) -> Check {
        for arg in all_args(arguments) {
            if &*arg.arg == "__debug__" {
                return Err(error_at(arg.loc, "cannot assign to __debug__".into()));
            }
        }
        self.exprs(&arguments.defaults, Place::Other)?;
        for default in arguments.kw_defaults.iter().flatten() {
            self.expr(default, Place::Other)?;
        }
        Ok(())
    }

    fn statement(&mut self, statement: &Stmt) -> Check {
        let loc = statement.loc;
        match &statement.kind {
            StmtKind::FunctionDef { args, body, decorator_list, returns, .. }
            | StmtKind::AsyncFunctionDef { args, body, decorator_list, returns, .. } => {
                self.exprs(decorator_list, Place::Other)?;
                self.arguments(args)?;
                for arg in all_args(args) {
                    self.maybe_expr(&arg.annotation, Place::Annotation)?;
                }
                self.maybe_expr(returns, Place::Other)?;
                let is_async = matches!(statement.kind, StmtKind::AsyncFunctionDef { .. });
                let async_generator = is_async && contains_yield(statement);
                let frame = Frame { kind: FrameKind::Function { is_async, async_generator }, loops: 0 };
                self.with_frame(frame, |this| this.statements(body))
            }
            StmtKind::ClassDef { bases, keywords, body, decorator_list, .. } => {
                self.exprs(decorator_list, Place::Other)?;
                self.with_frame(Frame { kind: FrameKind::Class, loops: 0 }, |this| this.statements(body))?;
                self.exprs(bases, Place::Call)?;
                keywords.iter().try_for_each(|keyword| self.keyword(keyword))
            }
            StmtKind::Return { value } => {
                if !self.function_like() {
                    return Err(error_at(loc, "'return' outside function".into()));
                }
                if value.is_some()
                    && matches!(self.frames.last().map(|f| f.kind), Some(FrameKind::Function { async_generator: true, .. }))
                {
                    return Err(error_at(loc, "'return' with value in async generator".into()));
                }
                self.maybe_expr(value, Place::Other)
            }
            StmtKind::Break => {
                if self.frames.last().map_or(0, |f| f.loops) == 0 {
                    return Err(error_at(loc, "'break' outside loop".into()));
                }
                Ok(())
            }
            StmtKind::Continue => {
                if self.frames.last().map_or(0, |f| f.loops) == 0 {
                    return Err(error_at(loc, "'continue' not properly in loop".into()));
                }
                Ok(())
            }
            StmtKind::For { target, iter, body, orelse, .. } | StmtKind::AsyncFor { target, iter, body, orelse, .. } => {
                if matches!(statement.kind, StmtKind::AsyncFor { .. }) && !self.in_async_function() {
                    return Err(error_at(loc, "'async for' outside async function".into()));
                }
                self.expr(iter, Place::Other)?;
                self.loop_body(Some(target), body)?;
                self.statements(orelse)
            }
            StmtKind::While { test, body, orelse } => {
                self.expr(test, Place::Other)?;
                self.loop_body(None, body)?;
                self.statements(orelse)
            }
            StmtKind::With { items, body, .. } | StmtKind::AsyncWith { items, body, .. } => {
                if matches!(statement.kind, StmtKind::AsyncWith { .. }) && !self.in_async_function() {
                    return Err(error_at(loc, "'async with' outside async function".into()));
                }
                for item in items {
                    self.expr(&item.context_expr, Place::Other)?;
                    self.maybe_expr(&item.optional_vars, Place::Assign)?;
                }
                self.statements(body)
            }
            StmtKind::Try { body, handlers, orelse, finalbody } | StmtKind::TryStar { body, handlers, orelse, finalbody } => {
                self.statements(body)?;
                for (index, handler) in handlers.iter().enumerate() {
                    if handler.type_.is_none() && index + 1 < handlers.len() {
                        return Err(error_at(handler.loc, "default 'except:' must be last".into()));
                    }
                    self.maybe_expr(&handler.type_, Place::Other)?;
                    if handler.name.as_deref() == Some("__debug__") {
                        return Err(error_at(handler.loc, "cannot assign to __debug__".into()));
                    }
                    self.statements(&handler.body)?;
                }
                self.statements(orelse)?;
                self.statements(finalbody)
            }
            StmtKind::ImportFrom { module, names, .. } => {
                if module.as_deref() == Some("__future__") && loc.line > self.future_line {
                    return Err(error_at(loc, "from __future__ imports must occur at the beginning of the file".into()));
                }
                self.aliases(names)
            }
            StmtKind::Import { names } => self.aliases(names),
            StmtKind::Match { subject, cases } => self.match_statement(subject, cases),
            StmtKind::Assign { targets, value, .. } => {
                self.expr(value, Place::Other)?;
                self.exprs(targets, Place::Assign)
            }
            StmtKind::AugAssign { target, value, .. } => {
                self.expr(target, Place::Assign)?;
                self.expr(value, Place::Other)
            }
            StmtKind::AnnAssign { target, annotation, value, .. } => {
                self.expr(target, Place::Assign)?;
                self.expr(annotation, Place::Annotation)?;
                self.maybe_expr(value, Place::Other)
            }
            StmtKind::Delete { targets } => self.exprs(targets, Place::Assign),
            StmtKind::TypeAlias { value, .. } => {
                self.with_frame(Frame { kind: FrameKind::Lambda, loops: 0 }, |this| this.expr(value, Place::Other))
            }
            StmtKind::If { test, body, orelse } => {
                self.expr(test, Place::Other)?;
                self.statements(body)?;
                self.statements(orelse)
            }
            StmtKind::Raise { exc, cause } => {
                self.maybe_expr(exc, Place::Other)?;
                self.maybe_expr(cause, Place::Other)
            }
            StmtKind::Assert { test, msg } => {
                self.expr(test, Place::Other)?;
                self.maybe_expr(msg, Place::Other)
            }
            StmtKind::Expr { value } => self.expr(value, Place::Other),
            StmtKind::Global { .. } | StmtKind::Nonlocal { .. } | StmtKind::Pass => Ok(()),
        }
    }

    fn loop_body(&mut self, target: Option<&Expr>, body: &[Stmt]) -> Check {
        self.frame().loops += 1;
        let result = target.map_or(Ok(()), |target| self.expr(target, Place::Assign)).and_then(|_| self.statements(body));
        self.frame().loops -= 1;
        result
    }

    fn aliases(&mut self, names: &[crate::ast::Alias]) -> Check {
        for alias in names {
            let stored = alias.asname.as_deref().unwrap_or(&alias.name);
            if stored == "__debug__" {
                return Err(error_at(alias.loc, "cannot assign to __debug__".into()));
            }
        }
        Ok(())
    }

    fn keyword(&mut self, keyword: &Keyword) -> Check {
        if keyword.arg.as_deref() == Some("__debug__") {
            return Err(error_at(keyword.loc, "cannot assign to __debug__".into()));
        }
        self.expr(&keyword.value, Place::Other)
    }

    fn expr(&mut self, expr: &Expr, place: Place) -> Check {
        let loc = expr.loc;
        match &expr.kind {
            ExprKind::Name { id, ctx } => {
                if &**id == "__debug__" && *ctx != ExprContext::Load {
                    let verb = if *ctx == ExprContext::Del { "delete" } else { "assign to" };
                    return Err(error_at(loc, format!("cannot {verb} __debug__")));
                }
                Ok(())
            }
            ExprKind::Starred { value, ctx } => {
                if *ctx == ExprContext::Store && place != Place::Display {
                    return Err(error_at(loc, "starred assignment target must be in a list or tuple".into()));
                }
                if *ctx == ExprContext::Load && !matches!(place, Place::Display | Place::Call | Place::Annotation) {
                    return Err(error_at(loc, "can't use starred expression here".into()));
                }
                self.expr(value, Place::Other)
            }
            ExprKind::List { elts, ctx } | ExprKind::Tuple { elts, ctx } => {
                if *ctx == ExprContext::Store {
                    let starred = elts.iter().filter(|elt| matches!(elt.kind, ExprKind::Starred { .. })).count();
                    if starred > 1 {
                        return Err(error_at(loc, "multiple starred expressions in assignment".into()));
                    }
                }
                self.exprs(elts, Place::Display)
            }
            ExprKind::Set { elts } => self.exprs(elts, Place::Display),
            ExprKind::Call { func, args, keywords } => {
                for (index, keyword) in keywords.iter().enumerate() {
                    if let Some(name) = &keyword.arg {
                        if let Some(repeated) = keywords[index + 1..].iter().find(|later| later.arg.as_ref() == Some(name)) {
                            return Err(error_at(repeated.loc, format!("keyword argument repeated: {name}")));
                        }
                    }
                }
                self.expr(func, Place::Other)?;
                self.exprs(args, Place::Call)?;
                keywords.iter().try_for_each(|keyword| self.keyword(keyword))
            }
            ExprKind::Yield { .. } | ExprKind::YieldFrom { .. } => {
                if !self.function_like() {
                    return Err(error_at(loc, "'yield' outside function".into()));
                }
                if matches!(expr.kind, ExprKind::YieldFrom { .. }) && self.in_async_function() {
                    return Err(error_at(loc, "'yield from' inside async function".into()));
                }
                expr_children(expr, &mut |child| self.expr(child, Place::Other))
            }
            ExprKind::Await { value } => {
                if !self.function_like() {
                    return Err(error_at(loc, "'await' outside function".into()));
                }
                if !self.in_async_function() && !matches!(self.frames.last().map(|f| f.kind), Some(FrameKind::GeneratorExp)) {
                    return Err(error_at(loc, "'await' outside async function".into()));
                }
                self.expr(value, Place::Other)
            }
            ExprKind::Lambda { args, body } => {
                self.arguments(args)?;
                self.with_frame(Frame { kind: FrameKind::Lambda, loops: 0 }, |this| this.expr(body, Place::Other))
            }
            _ if is_comprehension(expr) => self.comprehension(expr),
            _ => expr_children(expr, &mut |child| self.expr(child, Place::Other)),
        }
    }

    fn comprehension(&mut self, expr: &Expr) -> Check {
        let generators = match &expr.kind {
            ExprKind::ListComp { generators, .. }
            | ExprKind::SetComp { generators, .. }
            | ExprKind::DictComp { generators, .. }
            | ExprKind::GeneratorExp { generators, .. } => generators,
            _ => return Ok(()),
        };
        if let Some(generator) = generators.first() {
            self.expr(&generator.iter, Place::Other)?;
        }
        let generator_expression = matches!(expr.kind, ExprKind::GeneratorExp { .. });
        let in_genexp = matches!(self.frames.last().map(|f| f.kind), Some(FrameKind::GeneratorExp));
        if !generator_expression && comprehension_is_async(expr) && !self.in_async_function() && !in_genexp {
            return Err(error_at(loc_of(expr), "asynchronous comprehension outside of an asynchronous function".into()));
        }
        let run = |this: &mut Self| -> Check {
            for (index, generator) in generators.iter().enumerate() {
                this.expr(&generator.target, Place::Assign)?;
                if index > 0 {
                    this.expr(&generator.iter, Place::Other)?;
                }
                this.exprs(&generator.ifs, Place::Other)?;
            }
            match &expr.kind {
                ExprKind::DictComp { key, value, .. } => {
                    this.expr(key, Place::Other)?;
                    this.expr(value, Place::Other)
                }
                ExprKind::ListComp { elt, .. } | ExprKind::SetComp { elt, .. } | ExprKind::GeneratorExp { elt, .. } => {
                    this.expr(elt, Place::Other)
                }
                _ => Ok(()),
            }
        };
        if generator_expression {
            self.with_frame(Frame { kind: FrameKind::GeneratorExp, loops: 0 }, run)
        } else {
            // 3.12 inlines list, set and dict comprehensions (PEP 709).
            run(self)
        }
    }

    // ----- match statements ---------------------------------------------

    fn match_statement(&mut self, subject: &Expr, cases: &[MatchCase]) -> Check {
        self.expr(subject, Place::Other)?;
        let count = cases.len();
        let has_default = count > 1 && cases.last().is_some_and(|case| is_wildcard(&case.pattern));
        for (index, case) in cases.iter().enumerate() {
            let last = index + 1 == count;
            if !(has_default && last) {
                let allow_irrefutable = case.guard.is_some() || last;
                let mut stores = Vec::new();
                self.pattern(&case.pattern, allow_irrefutable, &mut stores)?;
            }
            self.maybe_expr(&case.guard, Place::Other)?;
            self.statements(&case.body)?;
        }
        Ok(())
    }

    fn store_name<'a>(&self, name: &'a str, loc: Loc, stores: &mut Vec<&'a str>) -> Check {
        if name == "__debug__" {
            return Err(error_at(loc, "cannot assign to __debug__".into()));
        }
        if stores.contains(&name) {
            return Err(error_at(loc, format!("multiple assignments to name '{name}' in pattern")));
        }
        stores.push(name);
        Ok(())
    }

    fn pattern<'a>(&mut self, pattern: &'a Pattern, allow_irrefutable: bool, stores: &mut Vec<&'a str>) -> Check {
        let loc = pattern.loc;
        match &pattern.kind {
            PatternKind::MatchAs { pattern: sub, name } => {
                match sub {
                    None => {
                        if !allow_irrefutable {
                            return Err(error_at(loc, match name {
                                Some(name) => format!("name capture '{name}' makes remaining patterns unreachable"),
                                None => "wildcard makes remaining patterns unreachable".into(),
                            }));
                        }
                    }
                    Some(sub) => self.pattern(sub, allow_irrefutable, stores)?,
                }
                name.as_deref().map_or(Ok(()), |name| self.store_name(name, loc, stores))
            }
            PatternKind::MatchStar { name } => name.as_deref().map_or(Ok(()), |name| self.store_name(name, loc, stores)),
            PatternKind::MatchSequence { patterns } => {
                if patterns.iter().filter(|item| matches!(item.kind, PatternKind::MatchStar { .. })).count() > 1 {
                    return Err(error_at(loc, "multiple starred names in sequence pattern".into()));
                }
                patterns.iter().try_for_each(|item| self.pattern(item, true, stores))
            }
            PatternKind::MatchMapping { keys, patterns, rest } => {
                let mut seen: Vec<String> = Vec::new();
                for key in keys {
                    match &key.kind {
                        ExprKind::Constant { value, .. } => {
                            let mut text = String::new();
                            crate::node::write_constant(&mut text, value);
                            if seen.contains(&text) {
                                return Err(error_at(loc, format!("mapping pattern checks duplicate key ({text})")));
                            }
                            seen.push(text);
                        }
                        ExprKind::Attribute { .. } => {}
                        _ => return Err(error_at(key.loc, "mapping pattern keys may only match literals and attribute lookups".into())),
                    }
                }
                for item in patterns {
                    self.pattern(item, true, stores)?;
                }
                rest.as_deref().map_or(Ok(()), |rest| self.store_name(rest, loc, stores))
            }
            PatternKind::MatchClass { patterns, kwd_attrs, kwd_patterns, .. } => {
                for (index, attr) in kwd_attrs.iter().enumerate() {
                    if kwd_attrs[..index].contains(attr) {
                        let at = kwd_patterns.get(index).map_or(loc, |item| item.loc);
                        return Err(error_at(at, format!("attribute name repeated in class pattern: {attr}")));
                    }
                }
                for item in patterns.iter().chain(kwd_patterns) {
                    self.pattern(item, true, stores)?;
                }
                Ok(())
            }
            PatternKind::MatchOr { patterns } => {
                let mut first_names: Option<Vec<&'a str>> = None;
                let mut combined = stores.clone();
                for (index, alternative) in patterns.iter().enumerate() {
                    let last = index + 1 == patterns.len();
                    let mut names = Vec::new();
                    self.pattern(alternative, last && allow_irrefutable, &mut names)?;
                    let mut sorted = names.clone();
                    sorted.sort_unstable();
                    match &first_names {
                        None => {
                            for name in &names {
                                if combined.contains(name) {
                                    return Err(error_at(alternative.loc, format!("multiple assignments to name '{name}' in pattern")));
                                }
                                combined.push(*name);
                            }
                            first_names = Some(sorted);
                        }
                        Some(first) if *first != sorted => {
                            return Err(error_at(alternative.loc, "alternative patterns bind different names".into()));
                        }
                        _ => {}
                    }
                }
                *stores = combined;
                Ok(())
            }
            PatternKind::MatchValue { .. } | PatternKind::MatchSingleton { .. } => Ok(()),
        }
    }
}

fn loc_of(expr: &Expr) -> Loc {
    expr.loc
}

fn is_wildcard(pattern: &Pattern) -> bool {
    matches!(pattern.kind, PatternKind::MatchAs { pattern: None, name: None })
}
