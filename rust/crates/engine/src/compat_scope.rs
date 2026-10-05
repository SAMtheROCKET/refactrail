//! The flow-ordered checker behind the Pyflakes-compatible scope codes
//! F401, F402, F403, F405, F406, F811, F821, F822, F823, F841 and F842
//! (mirror of compat_bindings.py and compat_scope_checker.py). Findings
//! must match the Python engine.

use crate::compat_pycodestyle::{identifier_position, CompatFinding};
use crate::source::{python_strip, SourceFile};
use crate::walk::{walk_expr, walk_stmt, Constant, Expr, ExprContext, ExprKind, Module, Operator, Stmt, StmtKind, Visitor};
use refactrail_parser::ast::{Arg, Arguments, ExceptHandler, Pattern, PatternKind, TypeParam, TypeParamKind};
use refactrail_parser::fast_hash::{FastMap, FastSet};
use refactrail_parser::Loc;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::LazyLock;

pub const SCOPE_CODES: [&str; 11] = ["F401", "F402", "F403", "F405", "F406", "F811", "F821", "F822", "F823", "F841", "F842"];
const TRACEBACK_NAMES: [&str; 4] = ["__tracebackhide__", "__traceback_info__", "__traceback_supplement__", "__debuggerskip__"];
const MODULE_NAMES: [&str; 13] = [
    "__file__", "__name__", "__doc__", "__package__", "__spec__", "__loader__", "__builtins__", "__annotations__", "__path__", "__cached__", "__dict__",
    "__debug__", "WindowsError",
];
static BUILTINS: LazyLock<FastSet<&'static str>> =
    LazyLock::new(|| include_str!("../data/builtins.txt").lines().filter(|line| !line.is_empty()).chain(MODULE_NAMES).collect());

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Import,
    Future,
    Definition,
    Class,
    Assignment,
    Annotation,
    Argument,
    Loop,
    Handler,
    Declaration,
    Unpacked,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Module,
    Class,
    Function,
    Comprehension,
    Type,
}

/// How a binding's position is reported (locate_report_tuple).
#[derive(Clone)]
enum Place {
    Plain(Loc),
    Definition(Loc, String),
    Handler(Loc, String),
    Alias(Loc, Option<String>),
}

impl Place {
    fn line(&self) -> u32 {
        match self {
            Place::Plain(loc) | Place::Definition(loc, _) | Place::Handler(loc, _) | Place::Alias(loc, _) => loc.line,
        }
    }
}

struct Binding {
    name: String,
    kind: Kind,
    place: Place,
    branch: Vec<(usize, usize)>,
    full_name: String,
    used: bool,
    reexport: bool,
    from_import: bool,
    has_asname: bool,
    statement_line: u32,
    scope: usize,
    typing_only: bool,
    /// Decorated with @overload.
    overload: bool,
    /// Decorated with @<its own name>.<attribute> (property setters).
    property_pair: bool,
}

struct Scope {
    kind: ScopeKind,
    bindings: FastMap<String, usize>,
    history: Vec<usize>,
    globals: FastSet<String>,
    stars: usize,
    uses_locals: bool,
    nonlocals: FastMap<String, usize>,
    locals: FastSet<String>,
}

impl Scope {
    fn new(kind: ScopeKind) -> Self {
        Scope {
            kind,
            bindings: FastMap::default(),
            history: Vec::new(),
            globals: FastSet::default(),
            stars: 0,
            uses_locals: false,
            nonlocals: FastMap::default(),
            locals: FastSet::default(),
        }
    }
}

#[derive(Clone, Copy)]
enum Body<'a> {
    Statements(&'a [Stmt]),
    Expression(&'a Expr),
}

enum Work<'a> {
    Function(&'a Arguments, Body<'a>),
    TypeExpression(&'a Expr),
    Expression(&'a Expr),
    /// A parsed string annotation with its (line, column base).
    StringAnnotation(Rc<Module>, u32, u32),
}

/// Context saved with deferred work: scope stack, branch and shift.
type Saved = (Vec<usize>, Vec<(usize, usize)>, Option<(u32, u32)>);

pub struct ScopeChecker<'a, 's> {
    source: &'s SourceFile<'s>,
    bindings: Vec<Binding>,
    scopes: Vec<Scope>,
    stack: Vec<usize>,
    branch: Vec<(usize, usize)>,
    deferred: VecDeque<(Work<'a>, Saved)>,
    finished: Vec<usize>,
    redefinitions: Vec<(usize, usize)>,
    in_type_checking: bool,
    handled: Vec<bool>,
    future_annotations: bool,
    exports: Vec<(String, Loc, u32)>,
    loaded: FastSet<&'a str>,
    is_init: bool,
    folder: Option<std::path::PathBuf>,
    /// While a parsed string annotation is read: (line, column base).
    shift: Option<(u32, u32)>,
    pub out: Vec<CompatFinding>,
}

/// DUMMY_NAME_PATTERN: ^(_+|(_+[a-zA-Z0-9_]*[a-zA-Z0-9]+?))$
fn is_dummy(name: &str) -> bool {
    let underscores = name.bytes().take_while(|&byte| byte == b'_').count();
    if underscores == 0 {
        return false;
    }
    let rest = &name[underscores..];
    rest.is_empty()
        || (rest.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') && rest.bytes().last().is_some_and(|byte| byte.is_ascii_alphanumeric()))
}

fn name_id(expression: &Expr) -> Option<&str> {
    match &expression.kind {
        ExprKind::Name { id, .. } => Some(id),
        _ => None,
    }
}

fn is_type_checking_test(test: &Expr) -> bool {
    match &test.kind {
        ExprKind::Name { id, .. } => &**id == "TYPE_CHECKING",
        ExprKind::Attribute { attr, .. } => &**attr == "TYPE_CHECKING",
        _ => false,
    }
}

fn is_name_error_handler(handler: &ExceptHandler) -> bool {
    let Some(caught) = &handler.type_ else { return false };
    match &caught.kind {
        ExprKind::Tuple { elts, .. } => elts.iter().any(|name| name_id(name) == Some("NameError")),
        _ => name_id(caught) == Some("NameError"),
    }
}

fn arguments_of(arguments: &Arguments) -> impl Iterator<Item = &Arg> {
    arguments
        .posonlyargs
        .iter()
        .chain(&arguments.args)
        .chain(arguments.vararg.as_ref())
        .chain(&arguments.kwonlyargs)
        .chain(arguments.kwarg.as_ref())
}

fn first_part(text: &str) -> &str {
    text.split('.').next().unwrap_or("")
}

fn is_submodule_import(binding: &Binding) -> bool {
    !binding.from_import && binding.full_name.contains('.') && binding.name == first_part(&binding.full_name)
}

fn is_submodule_pair(existing: &Binding, binding: &Binding) -> bool {
    existing.kind == Kind::Import
        && binding.kind == Kind::Import
        && existing.full_name != binding.full_name
        && (existing.full_name.contains('.') || binding.full_name.contains('.'))
        && first_part(&existing.full_name) == first_part(&binding.full_name)
        && first_part(&binding.full_name) == binding.name
}

/// `\bas\s+NAME\b` searched in a line from a character column.
fn find_as_name(line: &str, from: usize, name: &str) -> Option<usize> {
    let characters: Vec<char> = line.chars().collect();
    let name_characters: Vec<char> = name.chars().collect();
    let is_word = |character: char| crate::lexical::is_word_char(character);
    let mut index = from;
    while index + 2 <= characters.len() {
        if characters[index] == 'a' && characters[index + 1] == 's' && (index == 0 || !is_word(characters[index - 1])) {
            let mut after = index + 2;
            while after < characters.len() && characters[after].is_whitespace() {
                after += 1;
            }
            if after > index + 2
                && characters.len() >= after + name_characters.len()
                && characters[after..after + name_characters.len()] == name_characters[..]
                && characters.get(after + name_characters.len()).is_none_or(|&next| !is_word(next))
            {
                return Some(after);
            }
        }
        index += 1;
    }
    None
}

/// collect_local_names_set: names a function body assigns.
fn collect_local_names(body: Body<'_>) -> FastSet<String> {
    struct Locals {
        names: FastSet<String>,
        declared: FastSet<String>,
    }
    impl<'t> Visitor<'t> for Locals {
        fn visit_stmt(&mut self, node: &'t Stmt) {
            match &node.kind {
                StmtKind::Global { names } | StmtKind::Nonlocal { names } => {
                    self.declared.extend(names.iter().map(|name| name.to_string()));
                }
                StmtKind::FunctionDef { name, decorator_list, .. }
                | StmtKind::AsyncFunctionDef { name, decorator_list, .. }
                | StmtKind::ClassDef { name, decorator_list, .. } => {
                    self.names.insert(name.to_string());
                    for decorator in decorator_list {
                        self.visit_expr(decorator);
                    }
                    return;
                }
                StmtKind::Import { names } | StmtKind::ImportFrom { names, .. } => {
                    for alias in names {
                        let bound: &str = alias.asname.as_deref().unwrap_or_else(|| first_part(&alias.name));
                        self.names.insert(bound.to_string());
                    }
                }
                _ => {}
            }
            walk_stmt(self, node);
        }
        fn visit_expr(&mut self, node: &'t Expr) {
            match &node.kind {
                ExprKind::Name { id, ctx } if *ctx != ExprContext::Load => {
                    self.names.insert(id.to_string());
                }
                ExprKind::Lambda { .. } | ExprKind::ListComp { .. } | ExprKind::SetComp { .. } | ExprKind::DictComp { .. } | ExprKind::GeneratorExp { .. } => {
                    return;
                }
                _ => {}
            }
            walk_expr(self, node);
        }
    }
    let mut locals = Locals { names: FastSet::default(), declared: FastSet::default() };
    match body {
        Body::Statements(statements) => {
            for statement in statements {
                locals.visit_stmt(statement);
            }
        }
        Body::Expression(expression) => locals.visit_expr(expression),
    }
    let Locals { mut names, declared } = locals;
    names.retain(|name| !declared.contains(name));
    names
}

/// Every name read anywhere (loaded_names_set) and every global statement.
fn scan_module(tree: &Module) -> (FastSet<&str>, Vec<(&str, Loc)>) {
    struct Scan<'t> {
        loaded: FastSet<&'t str>,
        globals: Vec<(&'t str, Loc)>,
    }
    impl<'t> Visitor<'t> for Scan<'t> {
        fn visit_stmt(&mut self, node: &'t Stmt) {
            if let StmtKind::Global { names } = &node.kind {
                for name in names {
                    self.globals.push((name, node.loc));
                }
            }
            walk_stmt(self, node);
        }
        fn visit_expr(&mut self, node: &'t Expr) {
            if let ExprKind::Name { id, ctx: ExprContext::Load } = &node.kind {
                self.loaded.insert(id);
            }
            walk_expr(self, node);
        }
    }
    let mut scan = Scan { loaded: FastSet::default(), globals: Vec::new() };
    for statement in &tree.body {
        scan.visit_stmt(statement);
    }
    (scan.loaded, scan.globals)
}

/// parse_string_annotation_info: a string annotation's expression.
fn parse_string_annotation(text: &str) -> Option<Rc<Module>> {
    let stripped = python_strip(text);
    if stripped.is_empty() {
        return None;
    }
    let module = refactrail_parser::parse(stripped).ok()?;
    (module.body.len() == 1 && matches!(module.body[0].kind, StmtKind::Expr { .. })).then(|| Rc::new(module))
}

fn str_text(expression: &Expr) -> Option<String> {
    match &expression.kind {
        ExprKind::Constant { value: Constant::Str(points), .. } => Some(points.iter().map(|&point| char::from_u32(point).unwrap_or('\u{fffd}')).collect()),
        _ => None,
    }
}

/// Every direct sub-expression, in field order (ast.iter_child_nodes).
fn children(node: &Expr) -> Vec<&Expr> {
    struct Children<'e> {
        found: Vec<&'e Expr>,
        root: *const Expr,
    }
    impl<'e> Visitor<'e> for Children<'e> {
        fn visit_expr(&mut self, node: &'e Expr) {
            if std::ptr::eq(node, self.root) {
                walk_expr(self, node);
            } else {
                self.found.push(node);
            }
        }
    }
    let mut collector = Children { found: Vec::new(), root: node };
    collector.visit_expr(node);
    collector.found
}

fn is_overload(decorators: &[Expr]) -> bool {
    decorators.iter().any(|decorator| match &decorator.kind {
        ExprKind::Name { id, .. } => &**id == "overload",
        ExprKind::Attribute { attr, .. } => &**attr == "overload",
        _ => false,
    })
}

fn is_property_pair(decorators: &[Expr], name: &str) -> bool {
    decorators.iter().any(|decorator| matches!(&decorator.kind, ExprKind::Attribute { value, .. } if name_id(value) == Some(name)))
}

impl<'a, 's> ScopeChecker<'a, 's> {
    pub fn new(tree: &'a Module, source: &'s SourceFile<'s>, path: &str) -> Self {
        let file = std::path::Path::new(path);
        let (loaded, globals) = scan_module(tree);
        let future_annotations = tree.body.iter().any(|statement| {
            matches!(&statement.kind, StmtKind::ImportFrom { module: Some(module), names, .. }
                if &**module == "__future__" && names.iter().any(|alias| &*alias.name == "annotations"))
        });
        let mut checker = ScopeChecker {
            source,
            bindings: Vec::new(),
            scopes: vec![Scope::new(ScopeKind::Module)],
            stack: vec![0],
            branch: Vec::new(),
            deferred: VecDeque::new(),
            finished: Vec::new(),
            redefinitions: Vec::new(),
            in_type_checking: false,
            handled: Vec::new(),
            future_annotations,
            exports: Vec::new(),
            loaded,
            is_init: file.file_name().is_some_and(|name| name == "__init__.py"),
            folder: file.parent().map(|parent| parent.to_path_buf()),
            shift: None,
            out: Vec::new(),
        };
        for (name, loc) in globals {
            if !checker.scopes[0].bindings.contains_key(name) {
                let id = checker.new_binding(name, Kind::Declaration, Place::Plain(loc));
                checker.bindings[id].used = true;
                checker.scopes[0].bindings.insert(name.to_string(), id);
            }
        }
        checker
    }

    fn new_binding(&mut self, name: &str, kind: Kind, place: Place) -> usize {
        self.bindings.push(Binding {
            name: name.to_string(),
            kind,
            place,
            branch: Vec::new(),
            full_name: String::new(),
            used: false,
            reexport: false,
            from_import: false,
            has_asname: false,
            statement_line: 0,
            scope: 0,
            typing_only: false,
            overload: false,
            property_pair: false,
        });
        self.bindings.len() - 1
    }

    fn shifted(&self, loc: Loc) -> Loc {
        match self.shift {
            Some((line, base)) => Loc { line, col: base + loc.col, end_line: line, end_col: loc.end_col },
            None => loc,
        }
    }

    fn position(&self, place: &Place) -> (usize, usize) {
        match place {
            Place::Plain(loc) => self.source.position(*loc),
            Place::Definition(loc, name) => self.source.definition_name(*loc, name),
            Place::Handler(loc, name) => identifier_position(self.source, *loc, name),
            Place::Alias(loc, asname) => {
                let (line, column) = self.source.position(*loc);
                if let Some(asname) = asname {
                    let text = self.source.lines.get(line - 1).copied().unwrap_or("");
                    if let Some(found) = find_as_name(text, column - 1, asname) {
                        return (line, found + 1);
                    }
                }
                (line, column)
            }
        }
    }

    fn report(&mut self, code: &'static str, place: &Place, message: String, parent_line: u32) {
        let position = self.position(place);
        self.out.push((code, position, message, parent_line as usize));
    }

    fn scope(&self) -> usize {
        *self.stack.last().expect("module scope")
    }

    fn schedule(&mut self, work: Work<'a>) {
        let saved = (self.stack.clone(), self.branch.clone(), self.shift);
        self.deferred.push_back((work, saved));
    }

    // ----- running -----

    pub fn run(&mut self, tree: &'a Module) {
        self.statements(&tree.body);
        while let Some((work, (stack, branch, shift))) = self.deferred.pop_front() {
            let saved_stack = std::mem::replace(&mut self.stack, stack);
            let saved_branch = std::mem::replace(&mut self.branch, branch);
            let saved_shift = std::mem::replace(&mut self.shift, shift);
            match work {
                Work::Function(arguments, body) => self.run_function(arguments, body),
                Work::TypeExpression(expression) => self.type_expression(expression),
                Work::Expression(expression) => self.expression(expression),
                Work::StringAnnotation(module, line, base) => {
                    self.shift = Some((line, base));
                    if let StmtKind::Expr { value } = &module.body[0].kind {
                        self.parsed_type_expression(value);
                    }
                }
            }
            self.stack = saved_stack;
            self.branch = saved_branch;
            self.shift = saved_shift;
        }
        for scope in std::mem::take(&mut self.finished) {
            self.check_function_scope(scope);
        }
        for (shadowed, binding) in std::mem::take(&mut self.redefinitions) {
            if !self.bindings[shadowed].used {
                let message = format!("Redefinition of unused `{}` from line {}.", self.bindings[binding].name, self.bindings[shadowed].place.line());
                let place = self.bindings[binding].place.clone();
                let parent = self.bindings[shadowed].place.line();
                self.report("F811", &place, message, parent);
            }
        }
        self.check_exports();
        self.check_unused_imports(0);
    }

    // ----- bindings -----

    fn add_binding(&mut self, id: usize) {
        let mut scope = self.scope();
        let mut redirected = false;
        let name = self.bindings[id].name.clone();
        if let Some(&owner) = self.scopes[scope].nonlocals.get(&name) {
            scope = owner;
            redirected = true;
            self.bindings[id].used = true;
        } else if self.scopes[scope].globals.contains(&name) {
            scope = 0;
            self.bindings[id].used = self.loaded.contains(name.as_str()) || self.bindings[id].kind == Kind::Import;
            redirected = true;
        }
        self.bindings[id].branch = self.branch.clone();
        self.bindings[id].scope = scope;
        self.bindings[id].typing_only = self.in_type_checking;
        let existing = self.scopes[scope].bindings.get(&name).copied();
        self.scopes[scope].history.push(id);
        let Some(existing) = existing.filter(|&existing| self.bindings[existing].kind != Kind::Declaration) else {
            if !redirected {
                self.check_outer_shadowing(id);
            }
            self.scopes[scope].bindings.insert(name, id);
            return;
        };
        if self.bindings[id].kind == Kind::Annotation {
            return;
        }
        if is_submodule_pair(&self.bindings[existing], &self.bindings[id]) {
            self.scopes[scope].bindings.insert(name, id);
            return;
        }
        if self.bindings[existing].branch == self.bindings[id].branch && !redirected {
            self.check_redefinition(existing, id);
        }
        self.bindings[id].used = self.bindings[id].used || self.bindings[existing].used;
        self.scopes[scope].bindings.insert(name, id);
    }

    fn bind(&mut self, name: &str, kind: Kind, place: Place) -> usize {
        let id = self.new_binding(name, kind, place);
        self.add_binding(id);
        id
    }

    /// check_outer_shadowing_none.
    fn check_outer_shadowing(&mut self, id: usize) {
        if self.scopes[self.scope()].kind != ScopeKind::Function {
            return;
        }
        let Some(shadowed) = self.find_binding(&self.bindings[id].name.clone()) else { return };
        if self.bindings[shadowed].kind != Kind::Import {
            return;
        }
        let binding = &self.bindings[id];
        if binding.kind == Kind::Loop {
            let message = format!("Import `{}` from line {} shadowed by loop variable.", binding.name, self.bindings[shadowed].place.line());
            let place = binding.place.clone();
            self.report("F402", &place, message, 0);
            return;
        }
        if is_dummy(&binding.name)
            || matches!(binding.kind, Kind::Annotation | Kind::Handler)
            || is_submodule_pair(&self.bindings[shadowed], binding)
            || self.bindings[shadowed].typing_only
        {
            return;
        }
        self.redefinitions.push((shadowed, id));
    }

    /// check_redefinition_none.
    fn check_redefinition(&mut self, existing: usize, id: usize) {
        let (old, new) = (&self.bindings[existing], &self.bindings[id]);
        if old.kind == Kind::Import && new.kind == Kind::Loop {
            let message = format!("Import `{}` from line {} shadowed by loop variable.", new.name, old.place.line());
            let place = new.place.clone();
            self.report("F402", &place, message, 0);
            return;
        }
        let is_definition = matches!(new.kind, Kind::Definition | Kind::Class);
        if old.used || !(matches!(old.kind, Kind::Definition | Kind::Class | Kind::Import) || (is_definition && old.kind == Kind::Assignment)) {
            return;
        }
        if is_dummy(&new.name) || (old.typing_only && !new.typing_only) {
            return;
        }
        if matches!(new.kind, Kind::Annotation | Kind::Declaration | Kind::Handler) {
            return;
        }
        if old.overload || new.property_pair {
            return;
        }
        self.redefinitions.push((existing, id));
    }

    // ----- reads -----

    fn find_binding(&self, name: &str) -> Option<usize> {
        let last = self.stack.len() - 1;
        for (index, &scope) in self.stack.iter().enumerate().rev() {
            if self.scopes[scope].kind == ScopeKind::Class && index != last {
                continue;
            }
            if let Some(&id) = self.scopes[scope].bindings.get(name) {
                return Some(id);
            }
        }
        None
    }

    /// mark_used_none.
    fn mark_used(&mut self, id: usize) {
        self.bindings[id].used = true;
        if self.bindings[id].kind != Kind::Import {
            return;
        }
        let scope = self.bindings[id].scope;
        let submodule = is_submodule_import(&self.bindings[id]);
        for index in 0..self.scopes[scope].history.len() {
            let member = self.scopes[scope].history[index];
            if self.bindings[member].kind == Kind::Import
                && self.bindings[member].name == self.bindings[id].name
                && (is_submodule_import(&self.bindings[member]) || submodule)
            {
                self.bindings[member].used = true;
            }
        }
    }

    /// handle_load_none.
    fn handle_load(&mut self, name: &str, loc: Loc) {
        let place = Place::Plain(self.shifted(loc));
        let scope = self.scope();
        if name == "locals" {
            self.scopes[scope].uses_locals = true;
        }
        let current = &self.scopes[scope];
        if current.kind == ScopeKind::Function
            && current.locals.contains(name)
            && !current.bindings.contains_key(name)
            && !current.globals.contains(name)
            && !current.nonlocals.contains_key(name)
            && self.find_binding(name).is_some()
        {
            self.report("F823", &place, format!("Local variable `{name}` referenced before assignment."), 0);
            return;
        }
        if let Some(id) = self.find_binding(name) {
            self.mark_used(id);
            return;
        }
        let in_class = self.scopes[scope].kind == ScopeKind::Class;
        let in_method = self.stack[..self.stack.len() - 1].iter().any(|&outer| self.scopes[outer].kind == ScopeKind::Class);
        if BUILTINS.contains(name) || (in_class && (name == "__module__" || name == "__qualname__")) || (name == "__class__" && in_method) {
            return;
        }
        if self.stack.iter().any(|&outer| self.scopes[outer].stars > 0) {
            self.report("F405", &place, format!("`{name}` may be undefined, or defined from star imports."), 0);
            return;
        }
        if self.handled.iter().any(|&guarded| guarded) {
            return;
        }
        self.report("F821", &place, format!("Undefined name `{name}`."), 0);
    }

    // ----- statements -----

    fn statements(&mut self, statements: &'a [Stmt]) {
        for statement in statements {
            self.statement(statement);
        }
    }

    fn arms(&mut self, node: &'a Stmt, arms: &[&'a [Stmt]], first_arm: usize) {
        let saved = self.branch.clone();
        for (index, arm) in arms.iter().enumerate() {
            self.branch = saved.clone();
            self.branch.push((node as *const Stmt as usize, first_arm + index));
            self.statements(arm);
        }
        self.branch = saved;
    }

    fn statement(&mut self, node: &'a Stmt) {
        match &node.kind {
            StmtKind::FunctionDef { name, args, body, decorator_list, returns, type_params, .. }
            | StmtKind::AsyncFunctionDef { name, args, body, decorator_list, returns, type_params, .. } => {
                for decorator in decorator_list {
                    self.expression(decorator);
                }
                for default in args.defaults.iter().chain(args.kw_defaults.iter().flatten()) {
                    self.expression(default);
                }
                let generic = self.enter_type_scope(type_params);
                for argument in arguments_of(args) {
                    if let Some(annotation) = &argument.annotation {
                        self.annotation(annotation);
                    }
                }
                if let Some(returns) = returns {
                    self.annotation(returns);
                }
                self.schedule(Work::Function(args, Body::Statements(body)));
                if generic {
                    self.stack.pop();
                }
                let id = self.new_binding(name, Kind::Definition, Place::Definition(node.loc, name.to_string()));
                self.bindings[id].overload = is_overload(decorator_list);
                self.bindings[id].property_pair = is_property_pair(decorator_list, name);
                self.add_binding(id);
            }
            StmtKind::ClassDef { name, bases, keywords, body, decorator_list, type_params } => {
                for decorator in decorator_list {
                    self.expression(decorator);
                }
                let generic = self.enter_type_scope(type_params);
                for expression in bases.iter().chain(keywords.iter().map(|keyword| &*keyword.value)) {
                    self.expression(expression);
                }
                self.scopes.push(Scope::new(ScopeKind::Class));
                self.stack.push(self.scopes.len() - 1);
                self.statements(body);
                self.stack.pop();
                if generic {
                    self.stack.pop();
                }
                self.bind(name, Kind::Class, Place::Definition(node.loc, name.to_string()));
            }
            StmtKind::TypeAlias { name, type_params, value } => {
                let generic = self.enter_type_scope(type_params);
                self.schedule(Work::Expression(value));
                if generic {
                    self.stack.pop();
                }
                if let Some(id) = name_id(name) {
                    self.bind(id, Kind::Assignment, Place::Plain(name.loc));
                }
            }
            StmtKind::Assign { targets, value, .. } => self.assign(targets, value),
            StmtKind::Expr { value } => self.expression_statement(value),
            StmtKind::AugAssign { target, value, .. } => {
                self.expression(value);
                if let Some(id) = name_id(target) {
                    self.handle_load(id, target.loc);
                    let binding = self.new_binding(id, Kind::Assignment, Place::Plain(target.loc));
                    self.bindings[binding].used = true;
                    self.add_binding(binding);
                    self.collect_exports(&[&**target], value);
                } else {
                    self.expression(target);
                }
            }
            StmtKind::AnnAssign { target, annotation, value, .. } => {
                self.annotation(annotation);
                if let Some(value) = value {
                    self.expression(value);
                }
                let Some(id) = name_id(target) else {
                    self.expression(target);
                    return;
                };
                let kind = if value.is_some() { Kind::Assignment } else { Kind::Annotation };
                self.bind(id, kind, Place::Plain(target.loc));
                if let Some(value) = value {
                    self.collect_exports(&[&**target], value);
                }
            }
            StmtKind::For { target, iter, body, orelse, .. } | StmtKind::AsyncFor { target, iter, body, orelse, .. } => {
                self.expression(iter);
                self.bind_target(target, Kind::Loop);
                self.statements(body);
                self.statements(orelse);
            }
            StmtKind::If { test, body, orelse } => {
                self.expression(test);
                if is_type_checking_test(test) {
                    let saved = std::mem::replace(&mut self.in_type_checking, true);
                    self.arms(node, &[body], 0);
                    self.in_type_checking = saved;
                    self.arms(node, &[orelse], 1);
                    return;
                }
                self.arms(node, &[body, orelse], 0);
            }
            StmtKind::With { items, body, .. } | StmtKind::AsyncWith { items, body, .. } => {
                for item in items {
                    self.expression(&item.context_expr);
                    if let Some(variables) = &item.optional_vars {
                        self.bind_target(variables, Kind::Assignment);
                    }
                }
                self.statements(body);
            }
            StmtKind::Try { body, handlers, orelse, finalbody } | StmtKind::TryStar { body, handlers, orelse, finalbody } => {
                self.handled.push(handlers.iter().any(is_name_error_handler));
                let saved = self.branch.clone();
                let key = node as *const Stmt as usize;
                self.branch = [saved.clone(), vec![(key, 0)]].concat();
                self.statements(body);
                self.handled.pop();
                for (index, handler) in handlers.iter().enumerate() {
                    self.branch = [saved.clone(), vec![(key, index + 1)]].concat();
                    self.handler(handler);
                }
                self.branch = [saved.clone(), vec![(key, 0)]].concat();
                self.statements(orelse);
                self.branch = saved;
                self.statements(finalbody);
            }
            StmtKind::Match { subject, cases } => {
                self.expression(subject);
                let saved = self.branch.clone();
                for (index, case) in cases.iter().enumerate() {
                    self.branch = [saved.clone(), vec![(node as *const Stmt as usize, index)]].concat();
                    self.pattern(&case.pattern);
                    if let Some(guard) = &case.guard {
                        self.expression(guard);
                    }
                    self.statements(&case.body);
                }
                self.branch = saved;
            }
            StmtKind::Delete { targets } => self.delete(targets),
            StmtKind::Global { names } => {
                let scope = self.scope();
                self.scopes[scope].globals.extend(names.iter().map(|name| name.to_string()));
                for name in names {
                    if !self.scopes[0].bindings.contains_key(&**name) {
                        let id = self.new_binding(name, Kind::Declaration, Place::Plain(node.loc));
                        self.bindings[id].used = true;
                        self.scopes[0].bindings.insert(name.to_string(), id);
                    }
                }
            }
            StmtKind::Nonlocal { names } => {
                for name in names {
                    let owner = self.stack[..self.stack.len() - 1]
                        .iter()
                        .rev()
                        .copied()
                        .find(|&scope| self.scopes[scope].kind == ScopeKind::Function && self.scopes[scope].bindings.contains_key(&**name));
                    if let Some(owner) = owner {
                        let id = self.scopes[owner].bindings[&**name];
                        self.mark_used(id);
                        let scope = self.scope();
                        self.scopes[scope].nonlocals.insert(name.to_string(), owner);
                    }
                }
            }
            StmtKind::Import { names } => {
                for alias in names {
                    let name = alias.asname.as_deref().unwrap_or_else(|| first_part(&alias.name)).to_string();
                    let id = self.new_binding(&name, Kind::Import, Place::Alias(alias.loc, alias.asname.as_deref().map(str::to_string)));
                    let binding = &mut self.bindings[id];
                    binding.full_name = alias.name.to_string();
                    binding.reexport = alias.asname.as_deref() == Some(&*alias.name);
                    binding.has_asname = alias.asname.is_some();
                    binding.statement_line = node.loc.line;
                    self.add_binding(id);
                }
            }
            StmtKind::ImportFrom { module, names, level } => {
                let module_name = format!("{}{}", ".".repeat(*level as usize), module.as_deref().unwrap_or(""));
                for alias in names {
                    if &*alias.name == "*" {
                        let scope = self.scope();
                        self.scopes[scope].stars += 1;
                        let place = Place::Plain(node.loc);
                        self.report("F403", &place, format!("`from {module_name} import *` used; unable to detect undefined names."), 0);
                        if self.scopes[scope].kind != ScopeKind::Module {
                            self.report("F406", &place, format!("`from {module_name} import *` only allowed at module level."), 0);
                        }
                        continue;
                    }
                    let kind = if module_name == "__future__" { Kind::Future } else { Kind::Import };
                    let name = alias.asname.as_deref().unwrap_or(&alias.name).to_string();
                    let id = self.new_binding(&name, kind, Place::Alias(alias.loc, alias.asname.as_deref().map(str::to_string)));
                    let binding = &mut self.bindings[id];
                    binding.full_name = format!("{module_name}.{}", alias.name);
                    binding.reexport = alias.asname.as_deref() == Some(&*alias.name);
                    binding.from_import = true;
                    binding.has_asname = alias.asname.is_some();
                    binding.statement_line = node.loc.line;
                    binding.used = kind == Kind::Future;
                    self.add_binding(id);
                }
            }
            _ => self.generic_statement(node),
        }
    }

    /// The fallback of visit_statement_none: child expressions and
    /// statements in field order (Return, Raise, Assert, While, ...).
    fn generic_statement(&mut self, node: &'a Stmt) {
        match &node.kind {
            StmtKind::Return { value } => {
                if let Some(value) = value {
                    self.expression(value);
                }
            }
            StmtKind::Raise { exc, cause } => {
                for expression in [exc, cause].into_iter().flatten() {
                    self.expression(expression);
                }
            }
            StmtKind::Assert { test, msg } => {
                self.expression(test);
                if let Some(message) = msg {
                    self.expression(message);
                }
            }
            StmtKind::While { test, body, orelse } => {
                self.expression(test);
                self.statements(body);
                self.statements(orelse);
            }
            _ => {}
        }
    }

    fn assign(&mut self, targets: &'a [Expr], value: &'a Expr) {
        self.expression(value);
        for target in targets {
            if targets.len() == 1 {
                if let (ExprKind::Tuple { elts: left, .. } | ExprKind::List { elts: left, .. }, ExprKind::Tuple { elts: right, .. } | ExprKind::List { elts: right, .. }) =
                    (&target.kind, &value.kind)
                {
                    if left.len() == right.len() && !left.iter().chain(right).any(|element| matches!(element.kind, ExprKind::Starred { .. })) {
                        for element in left {
                            self.bind_target(element, Kind::Assignment);
                        }
                        continue;
                    }
                }
            }
            self.bind_target(target, Kind::Assignment);
        }
        let target_refs: Vec<&Expr> = targets.iter().collect();
        self.collect_exports(&target_refs, value);
    }

    /// visit_Expr: also __all__.append(...) / __all__.extend(...).
    fn expression_statement(&mut self, value: &'a Expr) {
        self.expression(value);
        let ExprKind::Call { func, args, .. } = &value.kind else { return };
        let ExprKind::Attribute { value: object, attr, .. } = &func.kind else { return };
        if name_id(object) != Some("__all__") || args.is_empty() || !self.branch.is_empty() {
            return;
        }
        match &**attr {
            "append" => {
                if let Some(text) = str_text(&args[0]) {
                    if self.scope() == 0 {
                        self.exports.push((text, args[0].loc, args[0].loc.line));
                    }
                }
            }
            "extend" => self.collect_exports(&[&**object], &args[0]),
            _ => {}
        }
    }

    fn handler(&mut self, handler: &'a ExceptHandler) {
        if let Some(type_) = &handler.type_ {
            self.expression(type_);
        }
        let Some(name) = &handler.name else {
            self.statements(&handler.body);
            return;
        };
        let scope = self.scope();
        let previous = self.scopes[scope].bindings.get(&**name).copied();
        let id = self.bind(name, Kind::Handler, Place::Handler(handler.loc, name.to_string()));
        self.statements(&handler.body);
        if !self.bindings[id].used {
            let place = self.bindings[id].place.clone();
            self.report("F841", &place, format!("Local variable `{name}` is assigned to but never used."), 0);
        }
        let scope = self.scope();
        if self.scopes[scope].bindings.get(&**name) == Some(&id) {
            self.scopes[scope].bindings.remove(&**name);
            if let Some(previous) = previous {
                self.scopes[scope].bindings.insert(name.to_string(), previous);
            }
        }
    }

    fn pattern(&mut self, pattern: &'a Pattern) {
        struct Patterns<'p> {
            found: Vec<&'p Pattern>,
        }
        impl<'p> Visitor<'p> for Patterns<'p> {
            fn visit_pattern(&mut self, node: &'p Pattern) {
                self.found.push(node);
                refactrail_parser::ast::walk_pattern(self, node);
            }
            fn visit_expr(&mut self, _node: &'p Expr) {}
        }
        let mut collector = Patterns { found: Vec::new() };
        collector.visit_pattern(pattern);
        for node in collector.found {
            match &node.kind {
                PatternKind::MatchValue { value } => self.expression(value),
                PatternKind::MatchClass { cls, .. } => self.expression(cls),
                PatternKind::MatchMapping { keys, .. } => {
                    for key in keys {
                        self.expression(key);
                    }
                }
                _ => {}
            }
            let name = match &node.kind {
                PatternKind::MatchAs { name, .. } | PatternKind::MatchStar { name } => name.as_deref(),
                PatternKind::MatchMapping { rest, .. } => rest.as_deref(),
                _ => None,
            };
            if let Some(name) = name {
                self.bind(name, Kind::Assignment, Place::Plain(node.loc));
            }
        }
    }

    fn delete(&mut self, targets: &'a [Expr]) {
        for target in targets {
            let Some(name) = name_id(target) else {
                self.expression(target);
                continue;
            };
            let mut scope = self.scope();
            let is_global = self.scopes[scope].globals.contains(name);
            if is_global {
                scope = 0;
            }
            if let Some(&id) = self.scopes[scope].bindings.get(name) {
                self.bindings[id].used = true;
                if self.branch.is_empty() && !is_global {
                    self.scopes[scope].bindings.remove(name);
                }
            } else if self.find_binding(name).is_none() && !BUILTINS.contains(name) {
                self.report("F821", &Place::Plain(target.loc), format!("Undefined name `{name}`."), 0);
            }
        }
    }

    /// enter_type_scope_info: true when a type parameter scope was pushed.
    fn enter_type_scope(&mut self, type_params: &'a [TypeParam]) -> bool {
        if type_params.is_empty() {
            return false;
        }
        self.scopes.push(Scope::new(ScopeKind::Type));
        self.stack.push(self.scopes.len() - 1);
        let parts: Vec<(&str, Option<&'a Expr>, Option<&'a Expr>, Loc)> = type_params
            .iter()
            .map(|parameter| match &parameter.kind {
                TypeParamKind::TypeVar { name, bound, default_value } => (&**name, bound.as_deref(), default_value.as_deref(), parameter.loc),
                TypeParamKind::ParamSpec { name, default_value } | TypeParamKind::TypeVarTuple { name, default_value } => {
                    (&**name, None, default_value.as_deref(), parameter.loc)
                }
            })
            .collect();
        for &(name, _, _, loc) in &parts {
            self.bind(name, Kind::Argument, Place::Plain(loc));
        }
        for &(_, bound, default, _) in &parts {
            for expression in [bound, default].into_iter().flatten() {
                self.expression(expression);
            }
        }
        true
    }

    /// run_function_none.
    fn run_function(&mut self, arguments: &'a Arguments, body: Body<'a>) {
        let mut scope = Scope::new(ScopeKind::Function);
        scope.locals = collect_local_names(body);
        self.scopes.push(scope);
        let scope = self.scopes.len() - 1;
        self.stack.push(scope);
        for argument in arguments_of(arguments) {
            self.bind(&argument.arg, Kind::Argument, Place::Plain(argument.loc));
        }
        match body {
            Body::Statements(statements) => self.statements(statements),
            Body::Expression(expression) => self.expression(expression),
        }
        self.finished.push(scope);
        self.stack.pop();
    }

    // ----- targets and expressions -----

    fn bind_target(&mut self, target: &'a Expr, kind: Kind) {
        match &target.kind {
            ExprKind::Name { id, .. } => {
                self.bind(id, kind, Place::Plain(target.loc));
            }
            ExprKind::Tuple { elts, .. } | ExprKind::List { elts, .. } => {
                let element_kind = if kind == Kind::Assignment { Kind::Unpacked } else { kind };
                for element in elts {
                    self.bind_target(element, element_kind);
                }
            }
            ExprKind::Starred { value, .. } => self.bind_target(value, kind),
            _ => self.expression(target),
        }
    }

    /// visit_expression_none.
    fn expression(&mut self, node: &'a Expr) {
        match &node.kind {
            ExprKind::Name { id, ctx } => match ctx {
                ExprContext::Load => self.handle_load(id, node.loc),
                ExprContext::Store => {
                    self.bind(id, Kind::Assignment, Place::Plain(node.loc));
                }
                ExprContext::Del => {}
            },
            ExprKind::Lambda { args, body } => {
                for default in args.defaults.iter().chain(args.kw_defaults.iter().flatten()) {
                    self.expression(default);
                }
                self.schedule(Work::Function(args, Body::Expression(body)));
            }
            ExprKind::ListComp { .. } | ExprKind::SetComp { .. } | ExprKind::DictComp { .. } | ExprKind::GeneratorExp { .. } => self.comprehension(node),
            ExprKind::Call { func, args, keywords } => {
                if !self.typing_call(func, args, keywords) {
                    for child in children(node) {
                        self.expression(child);
                    }
                }
            }
            ExprKind::NamedExpr { target, value } => {
                self.expression(value);
                if let Some(id) = name_id(target) {
                    self.bind_walrus(id, target.loc);
                }
            }
            _ => {
                for child in children(node) {
                    self.expression(child);
                }
            }
        }
    }

    fn comprehension(&mut self, node: &'a Expr) {
        let (generators, parts): (&'a [refactrail_parser::ast::Comprehension], Vec<&'a Expr>) = match &node.kind {
            ExprKind::ListComp { elt, generators } | ExprKind::SetComp { elt, generators } | ExprKind::GeneratorExp { elt, generators } => (generators, vec![&**elt]),
            ExprKind::DictComp { key, value, generators } => (generators, vec![&**key, &**value]),
            _ => return,
        };
        let Some(first) = generators.first() else { return };
        self.expression(&first.iter);
        self.scopes.push(Scope::new(ScopeKind::Comprehension));
        self.stack.push(self.scopes.len() - 1);
        for (index, generator) in generators.iter().enumerate() {
            if index > 0 {
                self.expression(&generator.iter);
            }
            self.bind_target(&generator.target, Kind::Assignment);
            for condition in &generator.ifs {
                self.expression(condition);
            }
        }
        for part in parts {
            self.expression(part);
        }
        self.stack.pop();
    }

    /// bind_walrus_none: bind in the nearest non-comprehension scope.
    fn bind_walrus(&mut self, name: &str, loc: Loc) {
        let mut index = self.stack.len() - 1;
        while index > 0 && self.scopes[self.stack[index]].kind == ScopeKind::Comprehension {
            index -= 1;
        }
        let saved = self.stack.clone();
        self.stack.truncate(index + 1);
        self.bind(name, Kind::Assignment, Place::Plain(loc));
        self.stack = saved;
    }

    fn annotation(&mut self, annotation: &'a Expr) {
        if self.future_annotations {
            self.schedule(Work::TypeExpression(annotation));
        } else {
            self.type_expression(annotation);
        }
    }

    /// The (line, column base) of a string constant's parsed contents.
    fn string_shift(&self, node: &Expr) -> (u32, u32) {
        let loc = self.shifted(node.loc);
        (loc.line, loc.col + 1)
    }

    /// visit_type_expression_none for the module's own nodes.
    fn type_expression(&mut self, node: &'a Expr) {
        if let Some(text) = str_text(node) {
            if let Some(module) = parse_string_annotation(&text) {
                let (line, base) = self.string_shift(node);
                self.schedule(Work::StringAnnotation(module, line, base));
            }
            return;
        }
        match &node.kind {
            ExprKind::Subscript { value, slice, .. } if self.is_typing_name(value, "Literal") => {
                let _ = slice;
                self.expression(value);
            }
            ExprKind::Subscript { value, slice, .. } if self.is_typing_name(value, "Annotated") && matches!(slice.kind, ExprKind::Tuple { .. }) => {
                self.expression(value);
                if let ExprKind::Tuple { elts, .. } = &slice.kind {
                    if let Some(first) = elts.first() {
                        for metadata in &elts[1..] {
                            self.expression(metadata);
                        }
                        self.type_expression(first);
                    }
                }
            }
            ExprKind::Name { id, .. } => self.handle_load(id, node.loc),
            _ => {
                for child in children(node) {
                    self.type_expression(child);
                }
            }
        }
    }

    /// visit_type_expression_none for a parsed string annotation (only
    /// reads; nothing in it is deferred except nested strings).
    fn parsed_type_expression(&mut self, node: &Expr) {
        if let Some(text) = str_text(node) {
            if let Some(module) = parse_string_annotation(&text) {
                let (line, base) = self.string_shift(node);
                self.schedule(Work::StringAnnotation(module, line, base));
            }
            return;
        }
        match &node.kind {
            ExprKind::Subscript { value, .. } if self.is_typing_name(value, "Literal") => self.parsed_reads(value),
            ExprKind::Subscript { value, slice, .. } if self.is_typing_name(value, "Annotated") && matches!(slice.kind, ExprKind::Tuple { .. }) => {
                self.parsed_reads(value);
                if let ExprKind::Tuple { elts, .. } = &slice.kind {
                    if let Some(first) = elts.first() {
                        for metadata in &elts[1..] {
                            self.parsed_reads(metadata);
                        }
                        self.parsed_type_expression(first);
                    }
                }
            }
            ExprKind::Name { id, .. } => self.handle_load(id, node.loc),
            _ => {
                for child in children(node) {
                    self.parsed_type_expression(child);
                }
            }
        }
    }

    /// The name reads of a parsed expression.
    fn parsed_reads(&mut self, node: &Expr) {
        if let ExprKind::Name { id, ctx: ExprContext::Load } = &node.kind {
            self.handle_load(id, node.loc);
            return;
        }
        for child in children(node) {
            self.parsed_reads(child);
        }
    }

    /// visit_typing_call_bool: cast() and TypeVar() take types as strings.
    fn typing_call(&mut self, func: &'a Expr, args: &'a [Expr], keywords: &'a [refactrail_parser::ast::Keyword]) -> bool {
        if self.is_typing_name(func, "cast") && !args.is_empty() {
            self.expression(func);
            self.type_expression(&args[0]);
            for argument in &args[1..] {
                self.expression(argument);
            }
            return true;
        }
        if self.is_typing_name(func, "TypeVar") {
            self.expression(func);
            for argument in args.iter().skip(1) {
                self.type_expression(argument);
            }
            for keyword in keywords {
                if keyword.arg.as_deref() == Some("bound") {
                    self.type_expression(&keyword.value);
                } else {
                    self.expression(&keyword.value);
                }
            }
            return true;
        }
        false
    }

    /// is_typing_name_bool.
    fn is_typing_name(&self, node: &Expr, name: &str) -> bool {
        match &node.kind {
            ExprKind::Attribute { value, attr, .. } => {
                if &**attr != name {
                    return false;
                }
                let Some(base) = name_id(value) else { return false };
                self.find_binding(base).is_some_and(|id| matches!(self.bindings[id].full_name.as_str(), "typing" | "typing_extensions"))
            }
            ExprKind::Name { id, .. } => self.find_binding(id).is_some_and(|binding| {
                let full = &self.bindings[binding].full_name;
                full.strip_prefix("typing.").or_else(|| full.strip_prefix("typing_extensions.")) == Some(name)
            }),
            _ => false,
        }
    }

    // ----- exports and unused bindings -----

    /// collect_exports_none.
    fn collect_exports(&mut self, targets: &[&Expr], value: &Expr) {
        if self.scope() != 0 || !targets.iter().any(|target| name_id(target) == Some("__all__")) {
            return;
        }
        let statement_line = value.loc.line;
        let mut pending = vec![value];
        while let Some(part) = pending.pop() {
            match &part.kind {
                ExprKind::BinOp { left, op: Operator::Add, right } => {
                    pending.push(left);
                    pending.push(right);
                }
                ExprKind::List { elts, .. } | ExprKind::Tuple { elts, .. } => {
                    for element in elts {
                        if let Some(text) = str_text(element) {
                            self.exports.push((text, element.loc, statement_line));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// check_exports_none.
    fn check_exports(&mut self) {
        for (name, loc, statement_line) in std::mem::take(&mut self.exports) {
            if let Some(&id) = self.scopes[0].bindings.get(&name) {
                self.mark_used(id);
            } else if BUILTINS.contains(name.as_str()) {
                continue;
            } else if self.scopes[0].stars > 0 {
                self.report("F405", &Place::Plain(loc), format!("`{name}` may be undefined, or defined from star imports."), statement_line);
            } else if !self.is_submodule(&name) {
                self.report("F822", &Place::Plain(loc), format!("Undefined name `{name}` in `__all__`."), statement_line);
            }
        }
    }

    fn is_submodule(&self, name: &str) -> bool {
        if !self.is_init {
            return false;
        }
        let Some(folder) = &self.folder else { return false };
        folder.join(format!("{name}.py")).is_file() || folder.join(name).is_dir()
    }

    /// check_unused_imports_none.
    fn check_unused_imports(&mut self, scope: usize) {
        let history = self.scopes[scope].history.clone();
        let used_roots: FastSet<String> = history
            .iter()
            .map(|&id| &self.bindings[id])
            .filter(|binding| binding.kind == Kind::Import && binding.used && binding.has_asname && !binding.from_import)
            .map(|binding| first_part(&binding.full_name).to_string())
            .collect();
        let mut candidates: Vec<usize> = self.scopes[scope].bindings.values().copied().collect();
        candidates.sort_unstable();
        for &id in &history {
            let binding = &self.bindings[id];
            if binding.kind == Kind::Import && is_submodule_import(binding) && self.scopes[scope].bindings.get(&binding.name) != Some(&id) {
                candidates.push(id);
            }
        }
        for id in candidates {
            let binding = &self.bindings[id];
            if binding.kind != Kind::Import || binding.used {
                continue;
            }
            if binding.full_name.contains('.')
                && !binding.full_name.starts_with('.')
                && binding.name == first_part(&binding.full_name)
                && used_roots.contains(&binding.name)
            {
                continue;
            }
            if !binding.reexport {
                let message = format!("`{}` imported but unused.", binding.full_name.trim_start_matches('.'));
                let (place, parent) = (binding.place.clone(), binding.statement_line);
                self.report("F401", &place, message, parent);
            }
        }
    }

    /// check_function_scope_none.
    fn check_function_scope(&mut self, scope: usize) {
        self.check_unused_imports(scope);
        if self.scopes[scope].uses_locals {
            return;
        }
        let mut entries: Vec<(String, usize)> = self.scopes[scope].bindings.iter().map(|(name, &id)| (name.clone(), id)).collect();
        entries.sort_unstable_by_key(|entry| entry.1);
        for (name, id) in entries {
            let binding = &self.bindings[id];
            if binding.used || self.scopes[scope].globals.contains(&name) || is_dummy(&name) || TRACEBACK_NAMES.contains(&name.as_str()) {
                continue;
            }
            let place = binding.place.clone();
            match binding.kind {
                Kind::Assignment => self.report("F841", &place, format!("Local variable `{name}` is assigned to but never used."), 0),
                Kind::Annotation => self.report("F842", &place, format!("Local variable `{name}` is annotated but never used."), 0),
                _ => {}
            }
        }
    }
}

/// Run the scope checker on a module.
pub fn check_module(tree: &Module, source: &SourceFile, path: &str) -> Vec<CompatFinding> {
    let mut checker = ScopeChecker::new(tree, source, path);
    checker.run(tree);
    checker.out
}
