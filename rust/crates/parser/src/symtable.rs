//! Python symbol tables with the scopes and flags CPython 3.12's
//! `symtable` module reports, and the errors its symbol table pass raises.
//!
//! Each table records its names in definition order with CPython's raw
//! flags: definition bits (`DEF_*`, `USE`) and, after analysis, the
//! resolved scope (`LOCAL`, `GLOBAL_EXPLICIT`, `GLOBAL_IMPLICIT`, `FREE`,
//! `CELL`) shifted by `SCOPE_OFFSET`. List, set and dict comprehensions
//! are inlined into their enclosing table as in CPython 3.12 (PEP 709).
//! Behaviour is verified against CPython by `scripts/symtable_parity.py`.

use crate::ast::{
    Alias, Arguments, Comprehension as Generator, ExceptHandler, Expr, ExprContext, ExprKind, Keyword, MatchCase, Module,
    Pattern, PatternKind, Stmt, StmtKind, TypeParam, TypeParamKind, WithItem,
};
use crate::compile::CompileError;
use crate::fast_hash::{FastMap, FastSet};
use crate::node::Loc;
use crate::small_str::SmallStr;

pub const DEF_GLOBAL: u32 = 1;
pub const DEF_LOCAL: u32 = 2;
pub const DEF_PARAM: u32 = 4;
pub const DEF_NONLOCAL: u32 = 8;
pub const USE: u32 = 16;
pub const DEF_FREE: u32 = 32;
pub const DEF_FREE_CLASS: u32 = 64;
pub const DEF_IMPORT: u32 = 128;
pub const DEF_ANNOT: u32 = 256;
pub const DEF_COMP_ITER: u32 = 512;
pub const DEF_TYPE_PARAM: u32 = 1024;
pub const DEF_COMP_CELL: u32 = 2048;
pub const DEF_BOUND: u32 = DEF_LOCAL | DEF_PARAM | DEF_IMPORT;
pub const SCOPE_OFFSET: u32 = 12;
pub const SCOPE_MASK: u32 = 15;
pub const LOCAL: u32 = 1;
pub const GLOBAL_EXPLICIT: u32 = 2;
pub const GLOBAL_IMPLICIT: u32 = 3;
pub const FREE: u32 = 4;
pub const CELL: u32 = 5;

type Check = Result<(), CompileError>;

/// Tables up to this many names are searched linearly.
const INDEXED_AFTER: usize = 8;

fn error_at(loc: Loc, message: String) -> CompileError {
    CompileError { line: loc.line, offset: loc.col + 1, message }
}

/// The kind of a table (CPython's `_Py_block_ty`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockType {
    Function,
    Class,
    Module,
    Annotation,
    TypeVarBound,
    TypeAlias,
    TypeParam,
}

impl BlockType {
    /// `SymbolTable.get_type()`.
    pub fn name(self) -> &'static str {
        match self {
            BlockType::Function => "function",
            BlockType::Class => "class",
            BlockType::Module => "module",
            BlockType::Annotation => "annotation",
            BlockType::TypeVarBound => "TypeVar bound",
            BlockType::TypeAlias => "type alias",
            BlockType::TypeParam => "type parameter",
        }
    }

    fn is_function_like(self) -> bool {
        matches!(self, BlockType::Function | BlockType::TypeVarBound | BlockType::TypeAlias | BlockType::TypeParam)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComprehensionKind {
    List,
    Set,
    Dict,
    Generator,
}

/// One scope's symbol table.
pub struct Table {
    pub block: BlockType,
    pub name: SmallStr,
    pub lineno: u32,
    names: Vec<SmallStr>,
    flags: Vec<u32>,
    index: FastMap<SmallStr, usize>,
    /// Child tables, in creation order (inlined comprehensions replaced
    /// by their own children).
    pub children: Vec<usize>,
    directives: Vec<(SmallStr, Loc)>,
    nested: bool,
    free: bool,
    child_free: bool,
    generator: bool,
    coroutine: bool,
    comprehension: Option<ComprehensionKind>,
    comp_inlined: bool,
    comp_iter_target: bool,
    comp_iter_expr: u32,
    can_see_class_scope: bool,
}

impl Table {
    fn new(block: BlockType, name: &str, lineno: u32) -> Table {
        Table {
            block,
            name: name.into(),
            lineno,
            names: Vec::with_capacity(8),
            flags: Vec::with_capacity(8),
            index: FastMap::default(),
            children: Vec::new(),
            directives: Vec::new(),
            nested: false,
            free: false,
            child_free: false,
            generator: false,
            coroutine: false,
            comprehension: None,
            comp_inlined: false,
            comp_iter_target: false,
            comp_iter_expr: 0,
            can_see_class_scope: false,
        }
    }

    /// Index of a (mangled) name: a scan for small tables, a hash index
    /// once a table holds more than INDEXED_AFTER names.
    fn position(&self, name: &str) -> Option<usize> {
        if self.names.len() <= INDEXED_AFTER {
            return self.names.iter().position(|entry| entry.as_str() == name);
        }
        self.index.get(name).copied()
    }

    /// Raw flags of a (mangled) name; 0 when absent.
    pub fn get(&self, name: &str) -> u32 {
        self.position(name).map_or(0, |at| self.flags[at])
    }

    /// Whether the table has an entry for the name.
    pub fn contains(&self, name: &str) -> bool {
        self.position(name).is_some()
    }

    /// (name, raw flags) in definition order.
    pub fn symbols(&self) -> impl Iterator<Item = (&str, u32)> {
        self.names.iter().map(|name| name.as_str()).zip(self.flags.iter().copied())
    }

    fn set(&mut self, name: &str, value: u32) {
        match self.position(name) {
            Some(at) => self.flags[at] = value,
            None => {
                self.names.push(name.into());
                self.flags.push(value);
                let count = self.names.len();
                if count == INDEXED_AFTER + 1 {
                    self.index = self.names.iter().enumerate().map(|(at, entry)| (entry.clone(), at)).collect();
                } else if count > INDEXED_AFTER + 1 {
                    self.index.insert(name.into(), count - 1);
                }
            }
        }
    }

    fn remove(&mut self, name: &str) {
        let Some(at) = self.position(name) else { return };
        self.names.remove(at);
        self.flags.remove(at);
        self.index = if self.names.len() > INDEXED_AFTER {
            self.names.iter().enumerate().map(|(at, entry)| (entry.clone(), at)).collect()
        } else {
            FastMap::default()
        };
    }

    fn scope_of(&self, name: &str) -> u32 {
        (self.get(name) >> SCOPE_OFFSET) & SCOPE_MASK
    }
}

/// All tables of a module; table 0 is the module ("top").
pub struct SymbolTable {
    pub tables: Vec<Table>,
}

/// CPython's name mangling: `__name` in class `C` becomes `_C__name`.
pub fn mangle<'n>(private: Option<&str>, name: &'n str) -> std::borrow::Cow<'n, str> {
    let Some(private) = private else { return name.into() };
    if !name.starts_with("__") || name.ends_with("__") || name.contains('.') {
        return name.into();
    }
    let stripped = private.trim_start_matches('_');
    if stripped.is_empty() {
        return name.into();
    }
    let mut mangled = String::with_capacity(1 + stripped.len() + name.len());
    mangled.push('_');
    mangled.push_str(stripped);
    mangled.push_str(name);
    mangled.into()
}

/// Build and analyse the symbol tables of a module.
pub fn build(module: &Module, future_annotations: bool) -> Result<SymbolTable, CompileError> {
    let mut tables = visit_module(module, future_annotations)?;
    analyze_tables(&mut tables)?;
    Ok(SymbolTable { tables })
}

/// Only the errors of the symbol table pass. The analysis can only fail
/// for names with a global or nonlocal directive (statements, or `:=` in
/// a comprehension), so it is skipped when no table records one.
pub fn check(module: &Module, future_annotations: bool) -> Result<(), CompileError> {
    let mut tables = visit_module(module, future_annotations)?;
    if tables.iter().any(|table| !table.directives.is_empty()) {
        analyze_tables(&mut tables)?;
    }
    Ok(())
}

/// The visit pass: raw definition flags for every table.
fn visit_module(module: &Module, future_annotations: bool) -> Result<Vec<Table>, CompileError> {
    let mut builder = Builder { tables: Vec::new(), stack: Vec::new(), private: None, future_annotations };
    builder.enter_block(BlockType::Module, "top", 0, false);
    for statement in &module.body {
        builder.statement(statement)?;
    }
    builder.exit_block();
    Ok(builder.tables)
}

/// The analysis pass: resolved scopes for every table.
fn analyze_tables(tables: &mut Vec<Table>) -> Check {
    let mut free = FastSet::default();
    let global = FastSet::default();
    let type_params = FastSet::default();
    analyze_block(tables, 0, None, &mut free, &global, &type_params, None)
}

// ----- the visit pass ---------------------------------------------------

struct Builder {
    tables: Vec<Table>,
    stack: Vec<usize>,
    /// The class name used for mangling (CPython's `st_private`).
    private: Option<SmallStr>,
    future_annotations: bool,
}

impl Builder {
    fn current(&self) -> usize {
        *self.stack.last().expect("an open block")
    }

    fn cur(&mut self) -> &mut Table {
        let at = self.current();
        &mut self.tables[at]
    }

    fn mangle<'n>(&self, name: &'n str) -> std::borrow::Cow<'n, str> {
        mangle(self.private.as_deref(), name)
    }

    /// Open a block; annotation blocks are not linked to their parent.
    fn enter_block(&mut self, block: BlockType, name: &str, lineno: u32, _annotation: bool) {
        let mut table = Table::new(block, name, lineno);
        let previous = self.stack.last().copied();
        if let Some(previous) = previous {
            let parent = &self.tables[previous];
            table.nested = parent.nested || parent.block.is_function_like();
            table.comp_iter_expr = parent.comp_iter_expr;
        }
        let index = self.tables.len();
        self.tables.push(table);
        self.stack.push(index);
        if block == BlockType::Annotation {
            return;
        }
        if let Some(previous) = previous {
            self.tables[previous].children.push(index);
        }
    }

    fn exit_block(&mut self) {
        self.stack.pop();
    }

    fn lookup(&self, name: &str) -> u32 {
        self.tables[self.current()].get(&self.mangle(name))
    }

    fn add_def_in(&mut self, table: usize, name: &str, flag: u32, loc: Loc) -> Check {
        let mangled = self.mangle(name);
        let target = &mut self.tables[table];
        let mut value = match target.position(&mangled) {
            Some(at) => {
                let existing = target.flags[at];
                if flag & DEF_PARAM != 0 && existing & DEF_PARAM != 0 {
                    return Err(error_at(loc, format!("duplicate argument '{name}' in function definition")));
                }
                if flag & DEF_TYPE_PARAM != 0 && existing & DEF_TYPE_PARAM != 0 {
                    return Err(error_at(loc, format!("duplicate type parameter '{name}'")));
                }
                existing | flag
            }
            None => flag,
        };
        if target.comp_iter_target {
            if value & (DEF_GLOBAL | DEF_NONLOCAL) != 0 {
                return Err(error_at(loc, format!("comprehension inner loop cannot rebind assignment expression target '{name}'")));
            }
            value |= DEF_COMP_ITER;
        }
        target.set(&mangled, value);
        if flag & DEF_PARAM == 0 && flag & DEF_GLOBAL != 0 {
            let module = &mut self.tables[0];
            let value = flag | module.get(&mangled);
            module.set(&mangled, value);
        }
        Ok(())
    }

    fn add_def(&mut self, name: &str, flag: u32, loc: Loc) -> Check {
        let current = self.current();
        self.add_def_in(current, name, flag, loc)
    }

    fn record_directive(&mut self, name: &str, loc: Loc) {
        let mangled = self.mangle(name);
        self.cur().directives.push((mangled.as_ref().into(), loc));
    }

    fn statements(&mut self, body: &[Stmt]) -> Check {
        body.iter().try_for_each(|statement| self.statement(statement))
    }

    fn exprs(&mut self, items: &[Expr]) -> Check {
        items.iter().try_for_each(|item| self.expr(item))
    }

    fn maybe_expr(&mut self, item: &Option<Box<Expr>>) -> Check {
        item.as_deref().map_or(Ok(()), |item| self.expr(item))
    }

    fn defaults(&mut self, arguments: &Arguments) -> Check {
        self.exprs(&arguments.defaults)?;
        for default in arguments.kw_defaults.iter().flatten() {
            self.expr(default)?;
        }
        Ok(())
    }

    fn enter_type_param_block(&mut self, name: &str, has_defaults: bool, has_kwdefaults: bool, class: bool, loc: Loc) -> Check {
        let in_class = self.tables[self.current()].block == BlockType::Class;
        self.enter_block(BlockType::TypeParam, name, loc.line, false);
        if in_class {
            self.cur().can_see_class_scope = true;
            self.add_def("__classdict__", USE, loc)?;
        }
        if class {
            self.add_def(".type_params", DEF_LOCAL, loc)?;
            self.add_def(".type_params", USE, loc)?;
            // CPython keeps this mangling prefix after the class.
            self.private = Some(name.into());
            self.add_def(".generic_base", DEF_LOCAL, loc)?;
            self.add_def(".generic_base", USE, loc)?;
        }
        if has_defaults {
            self.add_def(".defaults", DEF_PARAM, loc)?;
        }
        if has_kwdefaults {
            self.add_def(".kwdefaults", DEF_PARAM, loc)?;
        }
        Ok(())
    }

    fn type_params(&mut self, params: &[TypeParam]) -> Check {
        for param in params {
            match &param.kind {
                TypeParamKind::TypeVar { name, bound } => {
                    self.add_def(name, DEF_TYPE_PARAM | DEF_LOCAL, param.loc)?;
                    if let Some(bound) = bound {
                        let in_class = self.tables[self.current()].can_see_class_scope;
                        self.enter_block(BlockType::TypeVarBound, name, param.loc.line, false);
                        self.cur().can_see_class_scope = in_class;
                        if in_class {
                            self.add_def("__classdict__", USE, bound.loc)?;
                        }
                        self.expr(bound)?;
                        self.exit_block();
                    }
                }
                TypeParamKind::TypeVarTuple { name } | TypeParamKind::ParamSpec { name } => {
                    self.add_def(name, DEF_TYPE_PARAM | DEF_LOCAL, param.loc)?;
                }
            }
        }
        Ok(())
    }

    fn annotation(&mut self, annotation: &Expr) -> Check {
        if self.future_annotations {
            self.enter_block(BlockType::Annotation, "_annotation", annotation.loc.line, true);
        }
        self.expr(annotation)?;
        if self.future_annotations {
            self.exit_block();
        }
        Ok(())
    }

    fn annotations(&mut self, statement_loc: Loc, arguments: &Arguments, returns: &Option<Box<Expr>>) -> Check {
        if self.future_annotations {
            self.enter_block(BlockType::Annotation, "_annotation", statement_loc.line, true);
        }
        for arg in arguments.posonlyargs.iter().chain(&arguments.args) {
            self.maybe_expr(&arg.annotation)?;
        }
        if let Some(vararg) = &arguments.vararg {
            self.maybe_expr(&vararg.annotation)?;
        }
        if let Some(kwarg) = &arguments.kwarg {
            self.maybe_expr(&kwarg.annotation)?;
        }
        for arg in &arguments.kwonlyargs {
            self.maybe_expr(&arg.annotation)?;
        }
        if self.future_annotations {
            self.exit_block();
        }
        if let Some(returns) = returns {
            self.annotation(returns)?;
        }
        Ok(())
    }

    fn parameters(&mut self, arguments: &Arguments) -> Check {
        for arg in arguments.posonlyargs.iter().chain(&arguments.args).chain(&arguments.kwonlyargs) {
            self.add_def(&arg.arg, DEF_PARAM, arg.loc)?;
        }
        if let Some(vararg) = &arguments.vararg {
            self.add_def(&vararg.arg, DEF_PARAM, vararg.loc)?;
        }
        if let Some(kwarg) = &arguments.kwarg {
            self.add_def(&kwarg.arg, DEF_PARAM, kwarg.loc)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn function(
        &mut self,
        statement: &Stmt,
        name: &str,
        args: &Arguments,
        body: &[Stmt],
        decorators: &[Expr],
        returns: &Option<Box<Expr>>,
        type_params: &[TypeParam],
        is_async: bool,
    ) -> Check {
        let loc = statement.loc;
        self.add_def(name, DEF_LOCAL, loc)?;
        self.defaults(args)?;
        self.exprs(decorators)?;
        if !type_params.is_empty() {
            let has_kwdefaults = args.kw_defaults.iter().any(Option::is_some);
            // CPython's defaults sequence is never NULL here, so a generic
            // function always gets the implicit .defaults parameter.
            self.enter_type_param_block(name, true, has_kwdefaults, false, loc)?;
            self.type_params(type_params)?;
        }
        self.annotations(loc, args, returns)?;
        self.enter_block(BlockType::Function, name, loc.line, false);
        if is_async {
            self.cur().coroutine = true;
        }
        self.parameters(args)?;
        self.statements(body)?;
        self.exit_block();
        if !type_params.is_empty() {
            self.exit_block();
        }
        Ok(())
    }

    fn class(&mut self, statement: &Stmt) -> Check {
        let StmtKind::ClassDef { name, bases, keywords, body, decorator_list, type_params } = &statement.kind else {
            return Ok(());
        };
        let loc = statement.loc;
        self.add_def(name, DEF_LOCAL, loc)?;
        self.exprs(decorator_list)?;
        if !type_params.is_empty() {
            self.enter_type_param_block(name, false, false, true, loc)?;
            self.type_params(type_params)?;
        }
        self.exprs(bases)?;
        for keyword in keywords {
            self.expr(&keyword.value)?;
        }
        self.enter_block(BlockType::Class, name, loc.line, false);
        let saved = self.private.replace(name.clone());
        if !type_params.is_empty() {
            self.add_def("__type_params__", DEF_LOCAL, loc)?;
            self.add_def(".type_params", USE, loc)?;
        }
        self.statements(body)?;
        self.private = saved;
        self.exit_block();
        if !type_params.is_empty() {
            self.exit_block();
        }
        Ok(())
    }

    fn global_or_nonlocal(&mut self, loc: Loc, names: &[SmallStr], global: bool) -> Check {
        let word = if global { "global" } else { "nonlocal" };
        for name in names {
            let current = self.lookup(name);
            if current & (DEF_PARAM | DEF_LOCAL | USE | DEF_ANNOT) != 0 {
                let message = if current & DEF_PARAM != 0 {
                    format!("name '{name}' is parameter and {word}")
                } else if current & USE != 0 {
                    format!("name '{name}' is used prior to {word} declaration")
                } else if current & DEF_ANNOT != 0 {
                    format!("annotated name '{name}' can't be {word}")
                } else {
                    format!("name '{name}' is assigned to before {word} declaration")
                };
                return Err(error_at(loc, message));
            }
            self.add_def(name, if global { DEF_GLOBAL } else { DEF_NONLOCAL }, loc)?;
            self.record_directive(name, loc);
        }
        Ok(())
    }

    fn statement(&mut self, statement: &Stmt) -> Check {
        let loc = statement.loc;
        match &statement.kind {
            StmtKind::FunctionDef { name, args, body, decorator_list, returns, type_params, .. } => {
                self.function(statement, name, args, body, decorator_list, returns, type_params, false)
            }
            StmtKind::AsyncFunctionDef { name, args, body, decorator_list, returns, type_params, .. } => {
                self.function(statement, name, args, body, decorator_list, returns, type_params, true)
            }
            StmtKind::ClassDef { .. } => self.class(statement),
            StmtKind::TypeAlias { name, type_params, value } => {
                self.expr(name)?;
                let alias_name = match &name.kind {
                    ExprKind::Name { id, .. } => id.as_str(),
                    _ => "",
                };
                let in_class = self.tables[self.current()].block == BlockType::Class;
                if !type_params.is_empty() {
                    self.enter_type_param_block(alias_name, false, false, false, loc)?;
                    self.type_params(type_params)?;
                }
                self.enter_block(BlockType::TypeAlias, alias_name, loc.line, false);
                self.cur().can_see_class_scope = in_class;
                if in_class {
                    self.add_def("__classdict__", USE, value.loc)?;
                }
                self.expr(value)?;
                self.exit_block();
                if !type_params.is_empty() {
                    self.exit_block();
                }
                Ok(())
            }
            StmtKind::Return { value } => self.maybe_expr(value),
            StmtKind::Delete { targets } => self.exprs(targets),
            StmtKind::Assign { targets, value, .. } => {
                self.exprs(targets)?;
                self.expr(value)
            }
            StmtKind::AnnAssign { target, annotation, value, simple } => {
                if let ExprKind::Name { id, .. } = &target.kind {
                    let current = self.lookup(id);
                    let at_module = self.current() == 0;
                    if current & (DEF_GLOBAL | DEF_NONLOCAL) != 0 && !at_module && *simple != 0 {
                        let which = if current & DEF_GLOBAL != 0 { "global" } else { "nonlocal" };
                        return Err(error_at(loc, format!("annotated name '{id}' can't be {which}")));
                    }
                    if *simple != 0 {
                        self.add_def(id, DEF_ANNOT | DEF_LOCAL, target.loc)?;
                    } else if value.is_some() {
                        self.add_def(id, DEF_LOCAL, target.loc)?;
                    }
                } else {
                    self.expr(target)?;
                }
                self.annotation(annotation)?;
                self.maybe_expr(value)
            }
            StmtKind::AugAssign { target, value, .. } => {
                self.expr(target)?;
                self.expr(value)
            }
            StmtKind::For { target, iter, body, orelse, .. } | StmtKind::AsyncFor { target, iter, body, orelse, .. } => {
                self.expr(target)?;
                self.expr(iter)?;
                self.statements(body)?;
                self.statements(orelse)
            }
            StmtKind::While { test, body, orelse } | StmtKind::If { test, body, orelse } => {
                self.expr(test)?;
                self.statements(body)?;
                self.statements(orelse)
            }
            StmtKind::With { items, body, .. } | StmtKind::AsyncWith { items, body, .. } => {
                items.iter().try_for_each(|item| self.with_item(item))?;
                self.statements(body)
            }
            StmtKind::Match { subject, cases } => {
                self.expr(subject)?;
                cases.iter().try_for_each(|case| self.match_case(case))
            }
            StmtKind::Raise { exc, cause } => {
                if let Some(exc) = exc {
                    self.expr(exc)?;
                    self.maybe_expr(cause)?;
                }
                Ok(())
            }
            StmtKind::Try { body, handlers, orelse, finalbody } | StmtKind::TryStar { body, handlers, orelse, finalbody } => {
                self.statements(body)?;
                self.statements(orelse)?;
                handlers.iter().try_for_each(|handler| self.handler(handler))?;
                self.statements(finalbody)
            }
            StmtKind::Assert { test, msg } => {
                self.expr(test)?;
                self.maybe_expr(msg)
            }
            StmtKind::Import { names } | StmtKind::ImportFrom { names, .. } => names.iter().try_for_each(|alias| self.alias(alias)),
            StmtKind::Global { names } => self.global_or_nonlocal(loc, names, true),
            StmtKind::Nonlocal { names } => self.global_or_nonlocal(loc, names, false),
            StmtKind::Expr { value } => self.expr(value),
            StmtKind::Pass | StmtKind::Break | StmtKind::Continue => Ok(()),
        }
    }

    fn with_item(&mut self, item: &WithItem) -> Check {
        self.expr(&item.context_expr)?;
        self.maybe_expr(&item.optional_vars)
    }

    fn handler(&mut self, handler: &ExceptHandler) -> Check {
        self.maybe_expr(&handler.type_)?;
        if let Some(name) = &handler.name {
            self.add_def(name, DEF_LOCAL, handler.loc)?;
        }
        self.statements(&handler.body)
    }

    fn match_case(&mut self, case: &MatchCase) -> Check {
        self.pattern(&case.pattern)?;
        self.maybe_expr(&case.guard)?;
        self.statements(&case.body)
    }

    fn alias(&mut self, alias: &Alias) -> Check {
        let name = alias.asname.as_deref().unwrap_or(&alias.name);
        if name == "*" {
            if self.tables[self.current()].block != BlockType::Module {
                return Err(error_at(alias.loc, "import * only allowed at module level".into()));
            }
            return Ok(());
        }
        let stored = name.split('.').next().unwrap_or(name);
        self.add_def(stored, DEF_IMPORT, alias.loc)
    }

    fn pattern(&mut self, pattern: &Pattern) -> Check {
        match &pattern.kind {
            PatternKind::MatchValue { value } => self.expr(value),
            PatternKind::MatchSingleton { .. } => Ok(()),
            PatternKind::MatchSequence { patterns } | PatternKind::MatchOr { patterns } => {
                patterns.iter().try_for_each(|item| self.pattern(item))
            }
            PatternKind::MatchStar { name } => match name {
                Some(name) => self.add_def(name, DEF_LOCAL, pattern.loc),
                None => Ok(()),
            },
            PatternKind::MatchMapping { keys, patterns, rest } => {
                self.exprs(keys)?;
                patterns.iter().try_for_each(|item| self.pattern(item))?;
                match rest {
                    Some(rest) => self.add_def(rest, DEF_LOCAL, pattern.loc),
                    None => Ok(()),
                }
            }
            PatternKind::MatchClass { cls, patterns, kwd_patterns, .. } => {
                self.expr(cls)?;
                patterns.iter().chain(kwd_patterns).try_for_each(|item| self.pattern(item))
            }
            PatternKind::MatchAs { pattern: inner, name } => {
                if let Some(inner) = inner {
                    self.pattern(inner)?;
                }
                match name {
                    Some(name) => self.add_def(name, DEF_LOCAL, pattern.loc),
                    None => Ok(()),
                }
            }
        }
    }

    fn keyword(&mut self, keyword: &Keyword) -> Check {
        self.expr(&keyword.value)
    }

    /// Errors for constructs not allowed in annotation-like blocks.
    fn check_annotation_block(&self, what: &str, loc: Loc) -> Check {
        let message = match self.tables[self.current()].block {
            BlockType::Annotation => format!("{what} cannot be used within an annotation"),
            BlockType::TypeVarBound => format!("{what} cannot be used within a TypeVar bound"),
            BlockType::TypeAlias => format!("{what} cannot be used within a type alias"),
            BlockType::TypeParam => format!("{what} cannot be used within the definition of a generic"),
            _ => return Ok(()),
        };
        Err(error_at(loc, message))
    }

    fn expr(&mut self, expr: &Expr) -> Check {
        let loc = expr.loc;
        match &expr.kind {
            ExprKind::NamedExpr { target, value } => {
                self.check_annotation_block("named expression", loc)?;
                self.named_expression(loc, target, value)
            }
            ExprKind::BoolOp { values, .. } => self.exprs(values),
            ExprKind::BinOp { left, right, .. } => {
                self.expr(left)?;
                self.expr(right)
            }
            ExprKind::UnaryOp { operand, .. } => self.expr(operand),
            ExprKind::Lambda { args, body } => {
                if self.tables[self.current()].can_see_class_scope {
                    return Err(error_at(loc, "Cannot use lambda in annotation scope within class scope".into()));
                }
                self.defaults(args)?;
                self.enter_block(BlockType::Function, "lambda", loc.line, false);
                self.parameters(args)?;
                self.expr(body)?;
                self.exit_block();
                Ok(())
            }
            ExprKind::IfExp { test, body, orelse } => {
                self.expr(test)?;
                self.expr(body)?;
                self.expr(orelse)
            }
            ExprKind::Dict { keys, values } => {
                for key in keys.iter().flatten() {
                    self.expr(key)?;
                }
                self.exprs(values)
            }
            ExprKind::Set { elts } => self.exprs(elts),
            ExprKind::GeneratorExp { elt, generators } => {
                self.comprehension(loc, "genexpr", ComprehensionKind::Generator, generators, elt, None)
            }
            ExprKind::ListComp { elt, generators } => {
                self.comprehension(loc, "listcomp", ComprehensionKind::List, generators, elt, None)
            }
            ExprKind::SetComp { elt, generators } => {
                self.comprehension(loc, "setcomp", ComprehensionKind::Set, generators, elt, None)
            }
            ExprKind::DictComp { key, value, generators } => {
                self.comprehension(loc, "dictcomp", ComprehensionKind::Dict, generators, key, Some(value))
            }
            ExprKind::Yield { value } => {
                self.check_annotation_block("yield expression", loc)?;
                self.maybe_expr(value)?;
                self.yielded(loc)
            }
            ExprKind::YieldFrom { value } => {
                self.check_annotation_block("yield expression", loc)?;
                self.expr(value)?;
                self.yielded(loc)
            }
            ExprKind::Await { value } => {
                self.check_annotation_block("await expression", loc)?;
                self.expr(value)?;
                self.cur().coroutine = true;
                Ok(())
            }
            ExprKind::Compare { left, comparators, .. } => {
                self.expr(left)?;
                self.exprs(comparators)
            }
            ExprKind::Call { func, args, keywords } => {
                self.expr(func)?;
                self.exprs(args)?;
                keywords.iter().try_for_each(|keyword| self.keyword(keyword))
            }
            ExprKind::FormattedValue { value, format_spec, .. } => {
                self.expr(value)?;
                self.maybe_expr(format_spec)
            }
            ExprKind::JoinedStr { values } => self.exprs(values),
            ExprKind::Constant { .. } => Ok(()),
            ExprKind::Attribute { value, .. } | ExprKind::Starred { value, .. } => self.expr(value),
            ExprKind::Subscript { value, slice, .. } => {
                self.expr(value)?;
                self.expr(slice)
            }
            ExprKind::Slice { lower, upper, step } => {
                self.maybe_expr(lower)?;
                self.maybe_expr(upper)?;
                self.maybe_expr(step)
            }
            ExprKind::Name { id, ctx } => {
                let load = *ctx == ExprContext::Load;
                self.add_def(id, if load { USE } else { DEF_LOCAL }, loc)?;
                // `super` counts as a use of __class__.
                if load && self.tables[self.current()].block.is_function_like() && &**id == "super" {
                    self.add_def("__class__", USE, loc)?;
                }
                Ok(())
            }
            ExprKind::List { elts, .. } | ExprKind::Tuple { elts, .. } => self.exprs(elts),
        }
    }

    fn yielded(&mut self, loc: Loc) -> Check {
        self.cur().generator = true;
        let message = match self.tables[self.current()].comprehension {
            None => return Ok(()),
            Some(ComprehensionKind::List) => "'yield' inside list comprehension",
            Some(ComprehensionKind::Set) => "'yield' inside set comprehension",
            Some(ComprehensionKind::Dict) => "'yield' inside dict comprehension",
            Some(ComprehensionKind::Generator) => "'yield' inside generator expression",
        };
        Err(error_at(loc, message.into()))
    }

    fn named_expression(&mut self, loc: Loc, target: &Expr, value: &Expr) -> Check {
        if self.tables[self.current()].comp_iter_expr > 0 {
            return Err(error_at(loc, "assignment expression cannot be used in a comprehension iterable expression".into()));
        }
        if self.tables[self.current()].comprehension.is_some() {
            self.extend_named_expression_scope(target)?;
        }
        self.expr(value)?;
        self.expr(target)
    }

    fn extend_named_expression_scope(&mut self, target: &Expr) -> Check {
        let ExprKind::Name { id, .. } = &target.kind else { return Ok(()) };
        let loc = target.loc;
        for position in (0..self.stack.len()).rev() {
            let table = self.stack[position];
            let mangled = self.mangle(id);
            let flags = self.tables[table].get(&mangled);
            if self.tables[table].comprehension.is_some() {
                if flags & DEF_COMP_ITER != 0 && flags & DEF_LOCAL != 0 {
                    return Err(error_at(loc, format!("assignment expression cannot rebind comprehension iteration variable '{id}'")));
                }
                continue;
            }
            match self.tables[table].block {
                BlockType::Function => {
                    self.add_def(id, if flags & DEF_GLOBAL != 0 { DEF_GLOBAL } else { DEF_NONLOCAL }, loc)?;
                    self.record_directive(id, loc);
                    return self.add_def_in(table, id, DEF_LOCAL, loc);
                }
                BlockType::Module => {
                    self.add_def(id, DEF_GLOBAL, loc)?;
                    self.record_directive(id, loc);
                    return self.add_def_in(table, id, DEF_GLOBAL, loc);
                }
                BlockType::Class => {
                    return Err(error_at(loc, "assignment expression within a comprehension cannot be used in a class body".into()));
                }
                BlockType::TypeParam => {
                    return Err(error_at(
                        loc,
                        "assignment expression within a comprehension cannot be used within the definition of a generic".into(),
                    ));
                }
                BlockType::TypeAlias => {
                    return Err(error_at(loc, "assignment expression within a comprehension cannot be used in a type alias".into()));
                }
                BlockType::TypeVarBound => {
                    return Err(error_at(loc, "assignment expression within a comprehension cannot be used in a TypeVar bound".into()));
                }
                BlockType::Annotation => {}
            }
        }
        Ok(())
    }

    fn comprehension(
        &mut self,
        loc: Loc,
        name: &str,
        kind: ComprehensionKind,
        generators: &[Generator],
        elt: &Expr,
        value: Option<&Expr>,
    ) -> Check {
        if self.tables[self.current()].can_see_class_scope {
            return Err(error_at(loc, "Cannot use comprehension in annotation scope within class scope".into()));
        }
        let Some(outermost) = generators.first() else { return Ok(()) };
        self.cur().comp_iter_expr += 1;
        let result = self.expr(&outermost.iter);
        self.cur().comp_iter_expr -= 1;
        result?;
        self.enter_block(BlockType::Function, name, loc.line, false);
        self.cur().comprehension = Some(kind);
        if outermost.is_async != 0 {
            self.cur().coroutine = true;
        }
        let table_loc = Loc { line: loc.line, col: loc.col, end_line: loc.end_line, end_col: loc.end_col };
        self.add_def(".0", DEF_PARAM, table_loc)?;
        self.cur().comp_iter_target = true;
        let result = self.expr(&outermost.target);
        self.cur().comp_iter_target = false;
        result?;
        self.exprs(&outermost.ifs)?;
        for generator in &generators[1..] {
            self.cur().comp_iter_target = true;
            let result = self.expr(&generator.target);
            self.cur().comp_iter_target = false;
            result?;
            self.cur().comp_iter_expr += 1;
            let result = self.expr(&generator.iter);
            self.cur().comp_iter_expr -= 1;
            result?;
            self.exprs(&generator.ifs)?;
            if generator.is_async != 0 {
                self.cur().coroutine = true;
            }
        }
        if let Some(value) = value {
            self.expr(value)?;
        }
        self.expr(elt)?;
        let is_generator = kind == ComprehensionKind::Generator;
        self.cur().generator = is_generator;
        let is_async = self.tables[self.current()].coroutine && !is_generator;
        self.exit_block();
        if is_async {
            self.cur().coroutine = true;
        }
        Ok(())
    }
}

// ----- the analysis pass ------------------------------------------------

type NameSet = FastSet<SmallStr>;

/// The sets analyze_name reads and updates for one block.
struct NameSets<'s, 'p> {
    bound: Bound<'p>,
    /// Names bound in the block, when it is function-like (else unused).
    local: Option<&'s mut NameSet>,
    free: &'s mut NameSet,
    global: &'s mut NameSet,
    type_params: &'s mut NameSet,
}

/// CPython's analyze_name: decide the scope of one name; returns the
/// scope and whether the block becomes "free".
fn analyze_name(
    tables: &[Table],
    table: usize,
    name: &SmallStr,
    flags: u32,
    sets: &mut NameSets,
    class_entry: Option<usize>,
) -> Result<(u32, bool), CompileError> {
    if flags & DEF_GLOBAL != 0 {
        if flags & DEF_NONLOCAL != 0 {
            return Err(directive_error(&tables[table], name, format!("name '{name}' is nonlocal and global")));
        }
        sets.global.insert(name.clone());
        sets.bound.remove(name);
        return Ok((GLOBAL_EXPLICIT, false));
    }
    if flags & DEF_NONLOCAL != 0 {
        if sets.bound.base.is_none() {
            return Err(directive_error(&tables[table], name, "nonlocal declaration not allowed at module level".into()));
        }
        if !sets.bound.contains(name) {
            return Err(directive_error(&tables[table], name, format!("no binding for nonlocal '{name}' found")));
        }
        if sets.type_params.contains(name) {
            return Err(directive_error(&tables[table], name, format!("nonlocal binding not allowed for type parameter '{name}'")));
        }
        sets.free.insert(name.clone());
        return Ok((FREE, true));
    }
    if flags & DEF_BOUND != 0 {
        if let Some(local) = sets.local.as_mut() {
            local.insert(name.clone());
        }
        if !sets.global.is_empty() {
            sets.global.remove(name);
        }
        if flags & DEF_TYPE_PARAM != 0 {
            sets.type_params.insert(name.clone());
        } else if !sets.type_params.is_empty() {
            sets.type_params.remove(name);
        }
        return Ok((LOCAL, false));
    }
    if let Some(class_entry) = class_entry {
        let class_flags = tables[class_entry].get(name);
        if class_flags & DEF_GLOBAL != 0 {
            return Ok((GLOBAL_EXPLICIT, false));
        } else if class_flags & DEF_BOUND != 0 && class_flags & DEF_NONLOCAL == 0 {
            return Ok((GLOBAL_IMPLICIT, false));
        }
    }
    if sets.bound.contains(name) {
        sets.free.insert(name.clone());
        return Ok((FREE, true));
    }
    if sets.global.contains(name) {
        return Ok((GLOBAL_IMPLICIT, false));
    }
    Ok((GLOBAL_IMPLICIT, tables[table].nested))
}

fn directive_error(table: &Table, name: &str, message: String) -> CompileError {
    let loc = table.directives.iter().find(|(directive, _)| directive.as_str() == name).map_or(Loc::default(), |(_, loc)| *loc);
    error_at(loc, message)
}

fn is_free_in_any_child(tables: &[Table], table: usize, name: &str) -> bool {
    tables[table].children.iter().any(|&child| tables[child].scope_of(name) == FREE)
}

/// Copy an inlined comprehension's names into the enclosing table;
/// `scopes` holds the enclosing table's scope per symbol index.
fn inline_comprehension(
    tables: &mut [Table],
    table: usize,
    comp: usize,
    scopes: &mut Vec<u32>,
    comp_free: &mut NameSet,
    inlined_cells: &mut NameSet,
) {
    let mut remove_dunder_class = false;
    for at in 0..tables[comp].names.len() {
        let comp_flags = tables[comp].flags[at];
        if comp_flags & DEF_PARAM != 0 {
            continue;
        }
        let name = tables[comp].names[at].clone();
        let mut scope = (comp_flags >> SCOPE_OFFSET) & SCOPE_MASK;
        let mut only_flags = comp_flags & ((1 << SCOPE_OFFSET) - 1);
        if scope == CELL || only_flags & DEF_COMP_CELL != 0 {
            inlined_cells.insert(name.clone());
        }
        match tables[table].position(name.as_str()) {
            None => {
                if scope == FREE && tables[table].block == BlockType::Class && name.as_str() == "__class__" {
                    scope = GLOBAL_IMPLICIT;
                    only_flags &= !DEF_FREE;
                    comp_free.remove(&name);
                    remove_dunder_class = true;
                }
                tables[table].set(&name, only_flags);
                scopes.push(scope);
            }
            Some(existing_at) => {
                let existing = tables[table].flags[existing_at];
                if existing & DEF_BOUND != 0 && !is_free_in_any_child(tables, comp, &name) && tables[table].block != BlockType::Class {
                    comp_free.remove(&name);
                }
            }
        }
    }
    tables[comp].free = !comp_free.is_empty();
    if remove_dunder_class {
        tables[comp].remove("__class__");
    }
}

/// The names bound in enclosing scopes (None at module level), as the
/// parent's set plus the names this block removed (CPython's copy).
struct Bound<'p> {
    base: Option<&'p NameSet>,
    removed: NameSet,
}

impl Bound<'_> {
    fn contains(&self, name: &str) -> bool {
        self.base.is_some_and(|base| base.contains(name)) && !self.removed.contains(name)
    }

    fn remove(&mut self, name: &SmallStr) {
        if self.base.is_some_and(|base| base.contains(name.as_str())) {
            self.removed.insert(name.clone());
        }
    }

    fn extend_into(&self, target: &mut NameSet) {
        if let Some(base) = self.base {
            target.extend(base.iter().filter(|name| !self.removed.contains(*name)).cloned());
        }
    }
}

/// CPython's analyze_block for one table and, recursively, its children.
fn analyze_block(
    tables: &mut Vec<Table>,
    table: usize,
    bound: Option<&NameSet>,
    free: &mut NameSet,
    global: &NameSet,
    type_params: &NameSet,
    class_entry: Option<usize>,
) -> Check {
    let block = tables[table].block;
    let function_like = block.is_function_like();
    let mut local = NameSet::default();
    let mut new_global = NameSet::default();
    let mut new_free = NameSet::default();
    let mut new_bound = NameSet::default();
    let mut inlined_cells = NameSet::default();
    let has_children = !tables[table].children.is_empty();
    if block == BlockType::Class {
        new_global.extend(global.iter().cloned());
        if has_children {
            if let Some(bound) = bound {
                new_bound.extend(bound.iter().cloned());
            }
        }
    }
    // analyze_name changes only this block's copies (CPython passes
    // copies to each child).
    let mut global_copy = global.clone();
    let mut type_params_copy = type_params.clone();
    let count = tables[table].names.len();
    let mut scopes: Vec<u32> = Vec::with_capacity(count);
    let mut becomes_free = false;
    let bound = {
        let mut sets = NameSets {
            bound: Bound { base: bound, removed: NameSet::default() },
            local: if function_like && has_children { Some(&mut local) } else { None },
            free,
            global: &mut global_copy,
            type_params: &mut type_params_copy,
        };
        for at in 0..count {
            let entry = &tables[table];
            let (scope, block_free) = analyze_name(tables, table, &entry.names[at], entry.flags[at], &mut sets, class_entry)?;
            scopes.push(scope);
            becomes_free |= block_free;
        }
        sets.bound
    };
    if becomes_free {
        tables[table].free = true;
    }
    if block != BlockType::Class {
        if has_children {
            if function_like {
                new_bound.extend(local.iter().cloned());
            }
            bound.extend_into(&mut new_bound);
        }
        new_global.extend(global_copy.iter().cloned());
    } else {
        new_bound.insert("__class__".into());
        new_bound.insert("__classdict__".into());
    }
    let mut any_inlined = false;
    for position in 0..tables[table].children.len() {
        let child = tables[table].children[position];
        let new_class_entry = if tables[child].can_see_class_scope {
            if block == BlockType::Class {
                Some(table)
            } else {
                class_entry
            }
        } else {
            None
        };
        let inline = tables[child].comprehension.is_some() && tables[child].comprehension != Some(ComprehensionKind::Generator);
        let mut child_free = new_free.clone();
        analyze_block(tables, child, Some(&new_bound), &mut child_free, &new_global, &type_params_copy, new_class_entry)?;
        if inline {
            inline_comprehension(tables, table, child, &mut scopes, &mut child_free, &mut inlined_cells);
            tables[child].comp_inlined = true;
            any_inlined = true;
        }
        new_free.extend(child_free);
        if tables[child].free || tables[child].child_free {
            tables[table].child_free = true;
        }
    }
    if any_inlined {
        // Splice the children of inlined comprehensions into ours.
        let children = std::mem::take(&mut tables[table].children);
        let mut spliced = Vec::with_capacity(children.len());
        for child in children {
            if tables[child].comp_inlined {
                spliced.extend(tables[child].children.iter().copied());
            } else {
                spliced.push(child);
            }
        }
        tables[table].children = spliced;
    }
    if function_like {
        if !new_free.is_empty() || !inlined_cells.is_empty() {
            for at in 0..scopes.len() {
                let name = &tables[table].names[at];
                if scopes[at] == LOCAL && (new_free.contains(name) || inlined_cells.contains(name)) {
                    scopes[at] = CELL;
                    new_free.remove(name);
                }
            }
        }
    } else if block == BlockType::Class {
        new_free.remove("__class__");
        new_free.remove("__classdict__");
    }
    let class_flag = block == BlockType::Class || tables[table].can_see_class_scope;
    update_symbols(&mut tables[table], &scopes, &bound, &new_free, &inlined_cells, class_flag);
    free.extend(new_free);
    Ok(())
}

/// Record each name's final scope and the free names passed upwards.
fn update_symbols(
    table: &mut Table,
    scopes: &[u32],
    bound: &Bound,
    free: &NameSet,
    inlined_cells: &NameSet,
    class_flag: bool,
) {
    for (at, scope) in scopes.iter().enumerate() {
        let mut flags = table.flags[at];
        if !inlined_cells.is_empty() && inlined_cells.contains(&table.names[at]) {
            flags |= DEF_COMP_CELL;
        }
        table.flags[at] = flags | (scope << SCOPE_OFFSET);
    }
    if free.is_empty() {
        return;
    }
    let mut free_names: Vec<&SmallStr> = free.iter().collect();
    free_names.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    for name in free_names {
        if let Some(at) = table.position(name.as_str()) {
            if class_flag {
                table.flags[at] |= DEF_FREE_CLASS;
            }
            continue;
        }
        if bound.base.is_some() && !bound.contains(name) {
            continue;
        }
        table.set(name, FREE << SCOPE_OFFSET);
    }
}

/// One line describing every table: "depth type name line" followed by
/// "depth . name flags" for each symbol (sorted), children in order.
pub fn dump(symbols: &SymbolTable) -> String {
    let mut entries = Vec::new();
    dump_table(symbols, 0, 0, &mut entries);
    entries.join("; ")
}

fn dump_table(symbols: &SymbolTable, table: usize, depth: usize, entries: &mut Vec<String>) {
    let entry = &symbols.tables[table];
    entries.push(format!("{depth} {} {} {}", entry.block.name(), entry.name, entry.lineno));
    let mut names: Vec<(&str, u32)> = entry.symbols().collect();
    names.sort_by(|left, right| left.0.cmp(right.0));
    for (name, flags) in names {
        entries.push(format!("{depth} . {name} {flags}"));
    }
    for &child in &entry.children {
        dump_table(symbols, child, depth + 1, entries);
    }
}
