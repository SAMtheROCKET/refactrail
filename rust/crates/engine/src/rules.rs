//! Every rule, ported from src/refactrail/rules/*.py over RefacTrail's own
//! CPython-shaped tree. Messages and positions must match the Python
//! engine byte for byte.

use crate::facts::{
    constant_candidates, decorator_names, docstring_of, has_docstring, is_dunder, is_main_block, is_type_checking_block,
    type_alias_names, FunctionFact, UPPER_CASE,
};
use crate::scopes::{Binding, Kind, ScopeKind};
use crate::settings::{is_protocol_hook, severity_of, Settings, GENERIC_NAMES, VERBS};
use crate::source::{python_indent, python_strip, SourceFile};
use crate::walk::{
    alias_binding, all_args, any_expr, constant, definition_name, name_id, positional_args, walk_expr, walk_stmt, Arg,
    CmpOperator, Constant, Expr, ExprContext, ExprKind, Module, Stmt, StmtKind, UnaryOperator, Visitor,
};
use regex::Regex;
use refactrail_parser::fast_hash::FastSet;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

static SNAKE_CASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^_{0,2}[a-z][a-z0-9_]*_{0,2}$").expect("valid pattern"));
static PASCAL_CASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^_?[A-Z][A-Za-z0-9]*$").expect("valid pattern"));
static ENTRY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\*{0,2}([A-Za-z_][A-Za-z0-9_]*)\s*(\(|:|$)").expect("valid pattern")
});
static DASHES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-{3,}$").expect("valid pattern"));
const EXEMPT_VERB_DECORATORS: [&str; 5] =
    ["property", "cached_property", "functools.cached_property", "overload", "typing.overload"];
const ACCESSOR_SUFFIXES: [&str; 3] = [".setter", ".getter", ".deleter"];

pub struct Finding {
    pub line: usize,
    pub column: usize,
    pub code: &'static str,
    pub message: String,
}

/// RuleContext from rules/context.py.
pub struct Context<'s, 'a> {
    pub source: &'s SourceFile<'s>,
    pub body: &'a [Stmt],
    pub settings: &'s Settings,
    pub bindings: Vec<Binding<'a>>,
    pub constants: HashMap<String, &'a Stmt>,
    pub type_aliases: HashSet<String>,
    pub functions: Vec<FunctionFact<'a>>,
    pub classes: Vec<&'a Stmt>,
    pub findings: Vec<Finding>,
    /// collect_bound_names_set, computed on the first lookup.
    bound: BoundNames<'a>,
}

impl<'a> Context<'_, 'a> {

    pub fn report(&mut self, code: &'static str, position: (usize, usize), message: String) {
        if !self.settings.is_enabled(code) {
            return;
        }
        if let Some(suppressed) = self.source.noqa.get(&position.0) {
            match suppressed {
                None => return,
                Some(codes) if codes.iter().any(|prefix| code.starts_with(prefix.as_str())) => return,
                _ => {}
            }
        }
        let _ = severity_of(code);
        self.findings.push(Finding { line: position.0, column: position.1, code, message });
    }

    fn position(&self, loc: refactrail_parser::Loc) -> (usize, usize) {
        self.source.position(loc)
    }

    fn definition(&self, node: &Stmt) -> (usize, usize) {
        self.source.definition_name(node.loc, definition_name(node))
    }
}

// ---------------------------------------------------------------- layout

pub fn check_line_length(context: &mut Context) {
    let limit = context.settings.line_length;
    let long_lines: Vec<(usize, usize)> = context
        .source
        .lines
        .iter()
        .enumerate()
        // A line of at most `limit` bytes has at most `limit` characters.
        .filter(|(_, line)| line.len() > limit)
        .map(|(index, line)| (index + 1, line.chars().count()))
        .filter(|(_, length)| *length > limit)
        .collect();
    for (number, length) in long_lines {
        context.report("RT101", (number, limit + 1), format!("Line has {length} characters; limit is {limit}."));
    }
}

fn constant_target_expr(statement: &Stmt) -> Option<&Expr> {
    match &statement.kind {
        StmtKind::Assign { targets, .. } => targets.first(),
        StmtKind::AnnAssign { target, .. } => Some(target),
        _ => None,
    }
}

pub fn check_constant_names(context: &mut Context) {
    let mut reports = Vec::new();
    for (name, statement) in &context.constants {
        if UPPER_CASE.is_match(name) {
            continue;
        }
        let Some(target) = constant_target_expr(statement) else { continue };
        reports.push((context.position(target.loc), format!("Constant '{name}' should be UPPER_CASE ('{}').", name.to_uppercase())));
    }
    for (position, message) in reports {
        context.report("RT102", position, message);
    }
}

/// list_assigned_names_list.
fn assigned_names(statement: &Stmt) -> Vec<&str> {
    match &statement.kind {
        StmtKind::Assign { targets, .. } => targets.iter().filter_map(name_id).collect(),
        StmtKind::AnnAssign { target, .. } => name_id(target).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn is_constant_expression_statement(statement: &Stmt) -> bool {
    matches!(&statement.kind, StmtKind::Expr { value } if matches!(value.kind, ExprKind::Constant { .. }))
}

fn is_preamble_statement(statement: &Stmt, constants: &HashMap<String, &Stmt>, type_aliases: &HashSet<String>) -> bool {
    if matches!(statement.kind, StmtKind::Import { .. } | StmtKind::ImportFrom { .. } | StmtKind::TypeAlias { .. })
        || is_constant_expression_statement(statement)
        || is_type_checking_block(statement)
    {
        return true;
    }
    let names = assigned_names(statement);
    !names.is_empty()
        && names.iter().all(|name| {
            constants.contains_key(*name) || is_dunder(name) || type_aliases.contains(*name) || UPPER_CASE.is_match(name)
        })
}

pub fn check_constant_placement(context: &mut Context) {
    let mut reports = Vec::new();
    let mut code_started = false;
    for statement in context.body {
        if !code_started {
            code_started = !is_preamble_statement(statement, &context.constants, &context.type_aliases);
            continue;
        }
        for name in assigned_names(statement) {
            if context.constants.contains_key(name) || UPPER_CASE.is_match(name) {
                let Some(target) = constant_target_expr(statement) else { continue };
                reports.push((context.position(target.loc), format!("Constant '{name}' should be defined after the imports, before other code.")));
            }
        }
    }
    for (position, message) in reports {
        context.report("RT103", position, message);
    }
}

fn has_call(expression: Option<&Expr>) -> bool {
    expression.is_some_and(|expression| any_expr(expression, &|node| matches!(node.kind, ExprKind::Call { .. })))
}

fn is_structure_statement(statement: &Stmt) -> bool {
    match &statement.kind {
        StmtKind::Import { .. }
        | StmtKind::ImportFrom { .. }
        | StmtKind::Pass
        | StmtKind::FunctionDef { .. }
        | StmtKind::AsyncFunctionDef { .. }
        | StmtKind::ClassDef { .. }
        | StmtKind::TypeAlias { .. } => true,
        _ if is_constant_expression_statement(statement) => true,
        _ if is_main_block(statement) || is_type_checking_block(statement) => true,
        StmtKind::Assign { targets, value, .. } => {
            let names = assigned_names(statement);
            let constant = !names.is_empty()
                && names.len() == targets.len()
                && names.iter().all(|name| UPPER_CASE.is_match(name) || is_dunder(name));
            constant || !has_call(Some(value))
        }
        StmtKind::AnnAssign { value, .. } => {
            let names = assigned_names(statement);
            let constant = names.len() == 1 && names.iter().all(|name| UPPER_CASE.is_match(name) || is_dunder(name));
            constant || !has_call(value.as_deref())
        }
        StmtKind::Try { body, handlers, orelse, finalbody } => body
            .iter()
            .chain(orelse)
            .chain(finalbody)
            .chain(handlers.iter().flat_map(|handler| &handler.body))
            .all(is_structure_statement),
        _ => false,
    }
}

pub fn check_module_code(context: &mut Context) {
    let offending = context.body.iter().find(|statement| !is_structure_statement(statement));
    if let Some(statement) = offending {
        let position = context.position(statement.loc);
        context.report(
            "RT504",
            position,
            "Module runs code at import time; move it into functions called from the main block.".to_string(),
        );
    }
}

// ---------------------------------------------------------------- naming

pub fn check_short_names(context: &mut Context) {
    let mut reports = Vec::new();
    for binding in &context.bindings {
        if binding.name.chars().count() != 1 || binding.name == "_" {
            continue;
        }
        if binding.kind == Kind::Import && binding.imported_name.chars().count() == 1 {
            continue;
        }
        reports.push(((binding.line, binding.column), format!("Name '{}' is a single character; use a meaningful name.", binding.name)));
    }
    for (position, message) in reports {
        context.report("RT201", position, message);
    }
}

fn is_verb_exempt(fact: &FunctionFact) -> bool {
    let name = fact.name();
    if is_dunder(name) || name == "main" {
        return true;
    }
    if fact.is_method && is_protocol_hook(name) {
        return true;
    }
    decorator_names(fact.function().decorator_list).iter().any(|decorator| {
        EXEMPT_VERB_DECORATORS.contains(&decorator.as_str()) || ACCESSOR_SUFFIXES.iter().any(|suffix| decorator.ends_with(suffix))
    })
}

pub fn check_verb_names(context: &mut Context) {
    let mut reports = Vec::new();
    for fact in &context.functions {
        if is_verb_exempt(fact) {
            continue;
        }
        let name = fact.name();
        let first_word = name.trim_start_matches('_').split('_').next().unwrap_or("");
        let is_lower = first_word.chars().any(char::is_lowercase) && !first_word.chars().any(char::is_uppercase);
        if is_lower && VERBS.contains(first_word.to_lowercase().as_str()) {
            continue;
        }
        reports.push((context.definition(fact.node), format!("Function '{name}' should start with a verb describing its operation.")));
    }
    for (position, message) in reports {
        context.report("RT202", position, message);
    }
}

/// Names bound anywhere in a module, collected on the first lookup (most
/// files never ask).
pub struct BoundNames<'a> {
    body: &'a [Stmt],
    names: std::cell::OnceCell<FastSet<&'a str>>,
}

impl BoundNames<'_> {
    fn contains(&self, name: &str) -> bool {
        self.names.get_or_init(|| collect_bound_names(self.body)).contains(name)
    }
}

/// The builtin names the dtype rules ask about; only these need tracking
/// in the bound-name set.
fn is_dtype_builtin(name: &str) -> bool {
    matches!(name, "int" | "len" | "float" | "str" | "bool" | "list" | "sorted" | "dict" | "set" | "tuple" | "bytes" | "complex")
}

/// collect_bound_names_set: names bound anywhere (Store/Del, defs, params,
/// imports), restricted to the builtins the dtype rules look up.
fn collect_bound_names(body: &[Stmt]) -> FastSet<&str> {
    struct Bound<'a> {
        names: FastSet<&'a str>,
    }
    impl<'a> Visitor<'a> for Bound<'a> {
        fn visit_stmt(&mut self, node: &'a Stmt) {
            match &node.kind {
                StmtKind::FunctionDef { name, .. } | StmtKind::AsyncFunctionDef { name, .. } | StmtKind::ClassDef { name, .. } => {
                    if is_dtype_builtin(name) {
                        self.names.insert(name);
                    }
                }
                StmtKind::Import { names } | StmtKind::ImportFrom { names, .. } => {
                    self.names.extend(names.iter().map(alias_binding).filter(|name| is_dtype_builtin(name)));
                }
                _ => {}
            }
            walk_stmt(self, node);
        }

        fn visit_expr(&mut self, node: &'a Expr) {
            if let ExprKind::Name { id, ctx } = &node.kind {
                if *ctx != ExprContext::Load && is_dtype_builtin(id) {
                    self.names.insert(id);
                }
            }
            walk_expr(self, node);
        }

        fn visit_arg(&mut self, node: &'a Arg) {
            if is_dtype_builtin(&node.arg) {
                self.names.insert(&node.arg);
            }
            refactrail_parser::ast::walk_arg(self, node);
        }
    }
    let mut bound = Bound { names: FastSet::default() };
    for statement in body {
        bound.visit_stmt(statement);
    }
    bound.names
}

/// infer_value_dtype_str.
fn infer_dtype(value: &Expr, bound: &BoundNames) -> &'static str {
    match &value.kind {
        ExprKind::Constant { value, .. } => match value {
            Constant::True | Constant::False => "bool",
            Constant::Int(_) => "int",
            Constant::Float(_) => "float",
            Constant::Str(_) => "str",
            Constant::Bytes(_) => "bytes",
            _ => "",
        },
        ExprKind::UnaryOp { op, operand } => match op {
            UnaryOperator::Not => "bool",
            UnaryOperator::USub | UnaryOperator::UAdd => match infer_dtype(operand, bound) {
                "int" => "int",
                "float" => "float",
                _ => "",
            },
            _ => "",
        },
        ExprKind::List { .. } | ExprKind::ListComp { .. } => "list",
        ExprKind::Dict { .. } | ExprKind::DictComp { .. } => "dict",
        ExprKind::Set { .. } | ExprKind::SetComp { .. } => "set",
        ExprKind::Tuple { .. } => "tuple",
        ExprKind::JoinedStr { .. } => "str",
        ExprKind::Compare { left, ops, comparators } => {
            let scalar = [&**left].into_iter().chain(comparators).all(|part| matches!(part.kind, ExprKind::Constant { .. }));
            let identity =
                ops.iter().all(|op| matches!(op, CmpOperator::Is | CmpOperator::IsNot | CmpOperator::In | CmpOperator::NotIn));
            if scalar || identity { "bool" } else { "" }
        }
        ExprKind::Call { func, .. } => match name_id(func) {
            Some(name) => {
                let dtype = match name {
                    "int" | "len" => "int",
                    "float" => "float",
                    "str" => "str",
                    "bool" => "bool",
                    "list" | "sorted" => "list",
                    "dict" => "dict",
                    "set" => "set",
                    "tuple" => "tuple",
                    "bytes" => "bytes",
                    _ => "",
                };
                if dtype.is_empty() || bound.contains(name) { "" } else { dtype }
            }
            _ => "",
        },
        _ => "",
    }
}

pub fn check_dtype_suffixes(context: &mut Context) {
    if !context.settings.is_enabled("RT203") {
        return;
    }
    let bound = &context.bound;
    let mut reports = Vec::new();
    for binding in &context.bindings {
        let Some(value) = binding.value else { continue };
        if binding.kind != Kind::Variable || is_dunder(binding.name) {
            continue;
        }
        if !matches!(binding.scope_kind, ScopeKind::Module | ScopeKind::Function) {
            continue;
        }
        if binding.scope_kind == ScopeKind::Module
            && (context.constants.contains_key(binding.name) || UPPER_CASE.is_match(binding.name))
        {
            continue;
        }
        let dtype = infer_dtype(value, bound);
        if !dtype.is_empty() && !(binding.name.ends_with(dtype) && binding.name[..binding.name.len() - dtype.len()].ends_with('_')) {
            reports.push((
                (binding.line, binding.column),
                format!("Variable '{}' holds a {dtype}; name it '{}_{dtype}'.", binding.name, binding.name.trim_end_matches('_')),
            ));
        }
    }
    for (position, message) in reports {
        context.report("RT203", position, message);
    }
}

fn is_convention_exempt(binding: &Binding, constants: &HashMap<String, &Stmt>, type_aliases: &HashSet<String>) -> bool {
    if binding.scope_kind == ScopeKind::Module && type_aliases.contains(binding.name) {
        return true;
    }
    if binding.kind == Kind::Import || is_dunder(binding.name) || binding.name.trim_matches('_').is_empty() {
        return true;
    }
    if matches!(binding.scope_kind, ScopeKind::Module | ScopeKind::Class)
        && (UPPER_CASE.is_match(binding.name) || constants.contains_key(binding.name))
    {
        return true;
    }
    binding.scope_kind == ScopeKind::Class && is_protocol_hook(binding.name)
}

pub fn check_conventions(context: &mut Context) {
    let mut reports = Vec::new();
    for binding in &context.bindings {
        if is_convention_exempt(binding, &context.constants, &context.type_aliases) {
            continue;
        }
        let position = (binding.line, binding.column);
        if binding.kind == Kind::Class {
            if !PASCAL_CASE.is_match(binding.name) {
                reports.push((position, format!("Class '{}' should be PascalCase.", binding.name)));
            }
        } else if !SNAKE_CASE.is_match(binding.name) {
            reports.push((position, format!("Name '{}' should be snake_case.", binding.name)));
        }
    }
    for (position, message) in reports {
        context.report("RT204", position, message);
    }
}

pub fn check_generic_names(context: &mut Context) {
    if !context.settings.is_enabled("RT205") {
        return;
    }
    let mut reports = Vec::new();
    for binding in &context.bindings {
        if binding.kind != Kind::Import && GENERIC_NAMES.contains(binding.name) {
            reports.push(((binding.line, binding.column), format!("Name '{}' is generic; describe what it holds.", binding.name)));
        }
    }
    for (position, message) in reports {
        context.report("RT205", position, message);
    }
}

// ---------------------------------------------------------- documentation

fn is_stub_function(body: &[Stmt]) -> bool {
    match body {
        [only] => match &only.kind {
            StmtKind::Pass => true,
            StmtKind::Expr { value } => matches!(constant(value), Some(Constant::Ellipsis)),
            _ => false,
        },
        _ => false,
    }
}

pub fn check_missing_docstrings(context: &mut Context) {
    if !context.body.is_empty() && !has_docstring(context.body) {
        context.report("RT301", (1, 1), "Missing docstring in module.".to_string());
    }
    let mut reports = Vec::new();
    for class in &context.classes {
        let StmtKind::ClassDef { name, body, .. } = &class.kind else { continue };
        if !has_docstring(body) {
            reports.push((context.definition(class), format!("Missing docstring in class '{name}'.")));
        }
    }
    for fact in &context.functions {
        let function = fact.function();
        if has_docstring(function.body) || is_stub_function(function.body) {
            continue;
        }
        if decorator_names(function.decorator_list).iter().any(|name| name == "overload" || name == "typing.overload") {
            continue;
        }
        reports.push((context.definition(fact.node), format!("Missing docstring in function '{}'.", fact.name())));
    }
    for (position, message) in reports {
        context.report("RT301", position, message);
    }
}

fn normalize_docstring(docstring: &str) -> String {
    docstring.replace("\r\n", "\n").replace('\r', "\n")
}

/// find_sections_dict: canonical section -> first line index.
fn find_sections(docstring: &str) -> HashMap<&'static str, usize> {
    let lines: Vec<&str> = docstring.split('\n').collect();
    let mut sections = HashMap::new();
    for (index, line) in lines.iter().enumerate() {
        let stripped = python_strip(line);
        let mut section = match stripped {
            "Args:" | "Arguments:" | "Parameters:" => Some("Args"),
            "Returns:" => Some("Returns"),
            "Yields:" => Some("Yields"),
            "Warnings:" => Some("Warnings"),
            "Raises:" => Some("Raises"),
            _ => None,
        };
        if section.is_none() {
            let numpy = match stripped {
                "Parameters" => Some("Args"),
                "Returns" => Some("Returns"),
                "Yields" => Some("Yields"),
                "Warnings" => Some("Warnings"),
                _ => None,
            };
            if numpy.is_some() && lines.get(index + 1).is_some_and(|next| DASHES.is_match(python_strip(next))) {
                section = numpy;
            }
        }
        if let Some(section) = section {
            sections.entry(section).or_insert(index);
        }
    }
    sections
}

/// Whether a method's first positional parameter is implicit (self/cls).
fn has_implicit_first(fact: &FunctionFact, positional: &[&Arg]) -> bool {
    fact.is_method
        && !positional.is_empty()
        && !decorator_names(fact.function().decorator_list).iter().any(|name| name == "staticmethod")
}

/// list_parameters_list: parameters without the implicit one.
fn parameter_names(fact: &FunctionFact) -> Vec<String> {
    let arguments = fact.function().args;
    let positional = positional_args(arguments);
    let mut names: Vec<String> = all_args(arguments).map(|arg| arg.arg.to_string()).collect();
    if has_implicit_first(fact, &positional) {
        let implicit = &*positional[0].arg;
        if let Some(index) = names.iter().position(|name| name == implicit) {
            names.remove(index);
        }
    }
    names
}

fn is_generator(body: &[Stmt]) -> bool {
    struct Finder {
        found: bool,
    }
    impl<'a> Visitor<'a> for Finder {
        fn visit_stmt(&mut self, node: &'a Stmt) {
            match node.kind {
                _ if self.found => {}
                StmtKind::FunctionDef { .. } | StmtKind::AsyncFunctionDef { .. } | StmtKind::ClassDef { .. } => {}
                _ => walk_stmt(self, node),
            }
        }

        fn visit_expr(&mut self, node: &'a Expr) {
            match node.kind {
                _ if self.found => {}
                ExprKind::Yield { .. } | ExprKind::YieldFrom { .. } => self.found = true,
                ExprKind::Lambda { .. } => {}
                _ => walk_expr(self, node),
            }
        }
    }
    let mut finder = Finder { found: false };
    for statement in body {
        finder.visit_stmt(statement);
    }
    finder.found
}

fn missing_parts(fact: &FunctionFact, docstring: &str, strict: bool) -> Vec<&'static str> {
    let sections = find_sections(docstring);
    let mut missing = Vec::new();
    if python_strip(docstring).is_empty() {
        missing.push("summary line");
    }
    if !parameter_names(fact).is_empty() && !sections.contains_key("Args") {
        missing.push("Args section");
    }
    let returns = sections.contains_key("Returns") || (sections.contains_key("Yields") && is_generator(fact.function().body));
    if fact.name() != "__init__" && !returns {
        missing.push("Returns section");
    }
    if strict && !sections.contains_key("Warnings") {
        missing.push("Warnings section");
    }
    missing
}

pub fn check_docstring_sections(context: &mut Context) {
    let strict = context.settings.strict();
    let mut reports = Vec::new();
    for fact in &context.functions {
        let Some(docstring) = docstring_of(fact.function().body) else { continue };
        let docstring = normalize_docstring(&docstring);
        let position = context.definition(fact.node);
        for part in missing_parts(fact, &docstring, strict) {
            reports.push((position, format!("Docstring of '{}' has no {part}.", fact.name())));
        }
    }
    for (position, message) in reports {
        context.report("RT302", position, message);
    }
}

/// read_documented_names_list with collect_section_lines_tuple.
fn documented_names(docstring: &str) -> Option<Vec<String>> {
    let sections = find_sections(docstring);
    let heading = *sections.get("Args")?;
    let lines: Vec<&str> = docstring.split('\n').collect();
    let other_headings: HashSet<usize> = sections.values().copied().filter(|index| *index != heading).collect();
    let heading_indent = python_indent(lines[heading]);
    let start = heading + 1;
    let numpy = start < lines.len() && DASHES.is_match(python_strip(lines[start]));
    let mut body = Vec::new();
    for (index, line) in lines.iter().enumerate().skip(start + usize::from(numpy)) {
        if python_strip(line).is_empty() {
            continue;
        }
        let indent = python_indent(line);
        if other_headings.contains(&index) || indent < heading_indent + usize::from(!numpy) {
            break;
        }
        body.push((indent, python_strip(line)));
    }
    let entry_indent = if numpy { Some(heading_indent) } else { body.first().map(|(indent, _)| *indent) };
    Some(
        body.iter()
            .filter(|(indent, _)| Some(*indent) == entry_indent)
            .filter_map(|(_, text)| ENTRY.captures(text).map(|captures| captures[1].to_string()))
            .filter(|name| name != "None")
            .collect(),
    )
}

pub fn check_documented_arguments(context: &mut Context) {
    let mut reports = Vec::new();
    for fact in &context.functions {
        let Some(docstring) = docstring_of(fact.function().body) else { continue };
        let docstring = normalize_docstring(&docstring);
        let Some(documented) = documented_names(&docstring) else { continue };
        let parameters = parameter_names(fact);
        let position = context.definition(fact.node);
        let name = fact.name();
        for parameter in &parameters {
            if !documented.contains(parameter) {
                reports.push((position, format!("Docstring of '{name}' does not describe parameter '{parameter}'.")));
            }
        }
        for entry in &documented {
            if !parameters.contains(entry) {
                reports.push((position, format!("Docstring of '{name}' describes unknown parameter '{entry}'.")));
            }
        }
    }
    for (position, message) in reports {
        context.report("RT303", position, message);
    }
}

// ------------------------------------------------------- annotations, size

pub fn check_parameter_annotations(context: &mut Context) {
    let mut reports = Vec::new();
    for fact in &context.functions {
        let arguments = fact.function().args;
        let positional = positional_args(arguments);
        let implicit = has_implicit_first(fact, &positional).then(|| positional[0]);
        for parameter in all_args(arguments) {
            if implicit.is_some_and(|implicit| std::ptr::eq(implicit, parameter)) || parameter.annotation.is_some() {
                continue;
            }
            reports.push((
                context.position(parameter.loc),
                format!("Parameter '{}' of '{}' has no type annotation.", parameter.arg, fact.name()),
            ));
        }
    }
    for (position, message) in reports {
        context.report("RT401", position, message);
    }
}

pub fn check_return_annotations(context: &mut Context) {
    let strict = context.settings.strict();
    let mut reports = Vec::new();
    for fact in &context.functions {
        if fact.function().returns.is_some() || (fact.name() == "__init__" && !strict) {
            continue;
        }
        reports.push((context.definition(fact.node), format!("Function '{}' has no return annotation.", fact.name())));
    }
    for (position, message) in reports {
        context.report("RT402", position, message);
    }
}

pub fn check_function_sizes(context: &mut Context) {
    let preferred = context.settings.function_preferred_lines;
    let maximum = context.settings.function_max_lines;
    let mut reports = Vec::new();
    for fact in &context.functions {
        let loc = fact.node.loc;
        let first_line =
            fact.function().decorator_list.iter().map(|decorator| decorator.loc.line).chain([loc.line]).min().unwrap_or(loc.line) as usize;
        let span = loc.end_line as usize - first_line + 1;
        let name = fact.name();
        if span > maximum {
            reports.push(("RT502", context.definition(fact.node), format!("Function '{name}' spans {span} lines; limit {maximum}.")));
        } else if span > preferred {
            reports.push(("RT501", context.definition(fact.node), format!("Function '{name}' spans {span} lines; preferred {preferred}.")));
        }
    }
    for (code, position, message) in reports {
        context.report(code, position, message);
    }
}

pub fn check_main_block_size(context: &mut Context) {
    let limit = context.settings.main_max_lines;
    let mut reports = Vec::new();
    for statement in context.body {
        if !is_main_block(statement) {
            continue;
        }
        let loc = statement.loc;
        let span = (loc.end_line - loc.line + 1) as usize;
        if span > limit {
            reports.push((context.position(loc), format!("The main block spans {span} lines; limit {limit}.")));
        }
    }
    for (position, message) in reports {
        context.report("RT503", position, message);
    }
}

fn annotated_dtype<'n>(node: Option<&'n Expr>, bound: &BoundNames) -> &'n str {
    let Some(node) = node else { return "" };
    match &node.kind {
        ExprKind::Constant { value: Constant::None, .. } => "none",
        ExprKind::Name { id, .. }
            if ["int", "float", "str", "bool", "list", "dict", "set", "tuple", "bytes", "complex"].contains(&&**id)
                && !bound.contains(id) =>
        {
            id
        }
        ExprKind::Subscript { value, .. } => {
            let dtype = annotated_dtype(Some(value), bound);
            if ["list", "dict", "set", "tuple"].contains(&dtype) { dtype } else { "" }
        }
        _ => "",
    }
}

pub fn check_signature_suffixes(context: &mut Context) {
    if !context.settings.strict() {
        return;
    }
    let bound = &context.bound;
    let mut reports = Vec::new();
    for fact in &context.functions {
        if is_verb_exempt(fact) {
            continue;
        }
        let function = fact.function();
        let name = fact.name();
        let dtype = annotated_dtype(function.returns, bound);
        if !dtype.is_empty() && !name.ends_with(&format!("_{dtype}")) {
            reports.push(("RT206", context.definition(fact.node), format!("Function '{name}' declares return type {dtype}; use suffix '_{dtype}'.")));
        }
        let mut positional = positional_args(function.args);
        if has_implicit_first(fact, &positional) {
            positional.remove(0);
        }
        for parameter in positional.into_iter().chain(&function.args.kwonlyargs) {
            let dtype = annotated_dtype(parameter.annotation.as_deref(), bound);
            let parameter_name = &*parameter.arg;
            if !dtype.is_empty() && !parameter_name.ends_with(&format!("_{dtype}")) {
                reports.push(("RT207", context.position(parameter.loc), format!("Parameter '{parameter_name}' declares type {dtype}; use suffix '_{dtype}'.")));
            }
        }
    }
    for (code, position, message) in reports {
        context.report(code, position, message);
    }
}

/// Build the per-file context and run every rule (engine order).
pub fn run_rules<'s, 'a>(source: &'s SourceFile<'s>, module: &'a Module, settings: &'s Settings) -> Vec<Finding> {
    let body = &module.body[..];
    let mut functions = Vec::new();
    let mut classes = Vec::new();
    crate::facts::collect_definitions(body, false, &mut functions, &mut classes);
    let timing = TIMINGS.with(|timings| timings.borrow().is_some());
    let mut started = std::time::Instant::now();
    let mut lap = |label: &'static str| {
        if timing {
            let elapsed = started.elapsed().as_secs_f64();
            TIMINGS.with(|timings| {
                if let Some(map) = timings.borrow_mut().as_mut() {
                    *map.entry(label).or_insert(0.0) += elapsed;
                }
            });
            started = std::time::Instant::now();
        }
    };
    lap("definitions");
    let bindings = crate::scopes::BindingCollector::collect(source, body);
    lap("bindings");
    let constants = constant_candidates(body);
    lap("constants");
    let mut context = Context {
        source,
        bindings,
        constants,
        type_aliases: type_alias_names(body),
        body,
        settings,
        functions,
        classes,
        findings: Vec::new(),
        bound: BoundNames { body, names: std::cell::OnceCell::new() },
    };
    let rules: [(&str, fn(&mut Context)); 17] = [
        ("line_length", check_line_length), ("constant_names", check_constant_names),
        ("constant_placement", check_constant_placement), ("module_code", check_module_code),
        ("short_names", check_short_names), ("verb_names", check_verb_names),
        ("dtype_suffixes", check_dtype_suffixes), ("conventions", check_conventions),
        ("generic_names", check_generic_names), ("signature_suffixes", check_signature_suffixes),
        ("missing_docstrings", check_missing_docstrings), ("docstring_sections", check_docstring_sections),
        ("documented_arguments", check_documented_arguments), ("parameter_annotations", check_parameter_annotations),
        ("return_annotations", check_return_annotations), ("function_sizes", check_function_sizes),
        ("main_block_size", check_main_block_size),
    ];
    for (label, rule) in rules {
        rule(&mut context);
        lap(label);
    }
    context.findings
}

thread_local! {
    /// Development timing of each rule (enabled by start_rule_timing).
    static TIMINGS: std::cell::RefCell<Option<HashMap<&'static str, f64>>> = const { std::cell::RefCell::new(None) };
}

/// Development only: start accumulating per-rule timings on this thread.
#[doc(hidden)]
pub fn start_rule_timing() {
    TIMINGS.with(|timings| *timings.borrow_mut() = Some(HashMap::new()));
}

/// Development only: the accumulated per-rule seconds.
#[doc(hidden)]
pub fn rule_timings() -> Vec<(&'static str, f64)> {
    let mut list: Vec<(&'static str, f64)> =
        TIMINGS.with(|timings| timings.borrow().clone().unwrap_or_default().into_iter().collect());
    list.sort_by(|left, right| right.1.total_cmp(&left.1));
    list
}
