//! Module facts shared by several rules (mirror of facts.py).

use crate::walk::{
    as_function, constant, definition_name, for_each_child_block, is_str_constant, name_id, str_constant, CmpOperator,
    Constant, Expr, ExprKind, Operator, Stmt, StmtKind, UnaryOperator,
};
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

pub static UPPER_CASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^_?[A-Z][A-Z0-9_]*$").expect("valid pattern"));

pub static TYPE_ALIAS_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^_?[A-Z][A-Za-z0-9]*$").expect("valid pattern"));

fn is_none_constant(expression: &Expr) -> bool {
    matches!(constant(expression), Some(Constant::None))
}

/// is_type_expression_bool: dotted names, their subscripts and | unions.
pub fn is_type_expression(expression: &Expr) -> bool {
    let mut target = expression;
    match &expression.kind {
        ExprKind::Subscript { value, .. } => target = value,
        ExprKind::BinOp { left, op: Operator::BitOr, right } => {
            return [left, right].iter().all(|side| is_type_expression(side) || is_none_constant(side));
        }
        _ => {}
    }
    while let ExprKind::Attribute { value, .. } = &target.kind {
        target = value;
    }
    matches!(target.kind, ExprKind::Name { .. })
}

/// find_type_alias_name_str: the alias a top-level statement defines.
pub fn type_alias_name(statement: &Stmt) -> Option<&str> {
    match &statement.kind {
        StmtKind::TypeAlias { name, .. } => name_id(name),
        StmtKind::AnnAssign { target, annotation, value, .. } => {
            let target = name_id(target)?;
            let is_alias = value.is_some()
                && match &annotation.kind {
                    ExprKind::Name { id, .. } => &**id == "TypeAlias",
                    ExprKind::Attribute { value, attr, .. } => &**attr == "TypeAlias" && name_id(value) == Some("typing"),
                    _ => false,
                };
            is_alias.then_some(target)
        }
        StmtKind::Assign { targets, value, .. } => {
            let [target] = targets.as_slice() else { return None };
            let name = name_id(target)?;
            (TYPE_ALIAS_NAME.is_match(name) && !UPPER_CASE.is_match(name) && is_type_expression(value)).then_some(name)
        }
        _ => None,
    }
}

/// find_type_alias_names_set.
pub fn type_alias_names(body: &[Stmt]) -> std::collections::HashSet<String> {
    body.iter().filter_map(type_alias_name).map(str::to_string).collect()
}

/// A function (or async function) and whether it sits directly in a class body.
pub struct FunctionFact<'a> {
    pub node: &'a Stmt,
    pub is_method: bool,
}

impl<'a> FunctionFact<'a> {
    pub fn name(&self) -> &'a str {
        definition_name(self.node)
    }

    pub fn function(&self) -> crate::walk::Function<'a> {
        as_function(self.node).expect("a function definition")
    }
}

/// is_dunder_bool: starts and ends with "__" and is longer than 4.
pub fn is_dunder(name: &str) -> bool {
    name.chars().count() > 4 && name.starts_with("__") && name.ends_with("__")
}

/// read_docstring_str: the first statement's plain string, if any.
pub fn docstring_of(body: &[Stmt]) -> Option<String> {
    match &body.first()?.kind {
        StmtKind::Expr { value } => str_constant(value),
        _ => None,
    }
}

/// Whether a body starts with a plain-string docstring.
pub fn has_docstring(body: &[Stmt]) -> bool {
    matches!(body.first().map(|first| &first.kind), Some(StmtKind::Expr { value })
        if matches!(constant(value), Some(Constant::Str(_))))
}

/// read_decorator_names_list: dotted names, "" for other expressions.
pub fn decorator_names(decorators: &[Expr]) -> Vec<String> {
    decorators
        .iter()
        .map(|decorator| {
            let mut target = decorator;
            if let ExprKind::Call { func, .. } = &target.kind {
                target = func;
            }
            let mut parts = Vec::new();
            while let ExprKind::Attribute { value, attr, .. } = &target.kind {
                parts.insert(0, &**attr);
                target = value;
            }
            match name_id(target) {
                Some(name) => {
                    parts.insert(0, name);
                    parts.join(".")
                }
                None => String::new(),
            }
        })
        .collect()
}

/// is_main_block_bool: `if __name__ == "__main__":` either way round.
pub fn is_main_block(statement: &Stmt) -> bool {
    let StmtKind::If { test, .. } = &statement.kind else { return false };
    let ExprKind::Compare { left, ops, comparators } = &test.kind else { return false };
    if ops.as_slice() != [CmpOperator::Eq] {
        return false;
    }
    let operands = [&**left].into_iter().chain(comparators);
    let names: Vec<&str> = operands.clone().filter_map(name_id).collect();
    let constants: Vec<&Expr> = operands.filter(|side| matches!(side.kind, ExprKind::Constant { .. })).collect();
    names == ["__name__"] && constants.len() == 1 && is_str_constant(constants[0], "__main__")
}

/// is_immutable_literal_bool.
pub fn is_immutable_literal(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Constant { value, .. } => !matches!(value, Constant::Ellipsis),
        ExprKind::UnaryOp { op: UnaryOperator::USub | UnaryOperator::UAdd, operand } => {
            matches!(constant(operand), Some(Constant::Int(_) | Constant::Float(_) | Constant::Complex(_)))
        }
        ExprKind::Tuple { elts, .. } => elts.iter().all(is_immutable_literal),
        _ => false,
    }
}

/// is_type_checking_block_bool.
pub fn is_type_checking_block(statement: &Stmt) -> bool {
    let StmtKind::If { test, .. } = &statement.kind else { return false };
    match &test.kind {
        ExprKind::Name { id, .. } => &**id == "TYPE_CHECKING",
        ExprKind::Attribute { value, attr, .. } => &**attr == "TYPE_CHECKING" && name_id(value) == Some("typing"),
        _ => false,
    }
}

/// find_constant_target_node: the name of a single-target literal assignment.
pub fn constant_target(statement: &Stmt) -> Option<&Expr> {
    let (target, value) = match &statement.kind {
        StmtKind::Assign { targets, value, .. } => {
            let [target] = targets.as_slice() else { return None };
            (target, &**value)
        }
        StmtKind::AnnAssign { target, value: Some(value), .. } => (&**target, &**value),
        _ => return None,
    };
    (matches!(target.kind, ExprKind::Name { .. }) && is_immutable_literal(value)).then_some(target)
}

/// find_constant_candidates_dict: name -> assignment statement.
pub fn constant_candidates(body: &[Stmt]) -> HashMap<String, &Stmt> {
    let counts = crate::bindings_count::count_module_bindings(body);
    let mut candidates = HashMap::new();
    for statement in body {
        let Some(target) = constant_target(statement) else {
            continue;
        };
        let name = name_id(target).unwrap_or("");
        if counts.get(name) != Some(&1) || name == "_" || is_dunder(name) {
            continue;
        }
        candidates.insert(name.to_string(), statement);
    }
    candidates
}

/// collect_function_facts_list plus every class, at any depth.
pub fn collect_definitions<'a>(
    body: &'a [Stmt],
    in_class_body: bool,
    functions: &mut Vec<FunctionFact<'a>>,
    classes: &mut Vec<&'a Stmt>,
) {
    for statement in body {
        match &statement.kind {
            StmtKind::FunctionDef { body: inner, .. } | StmtKind::AsyncFunctionDef { body: inner, .. } => {
                functions.push(FunctionFact { node: statement, is_method: in_class_body });
                collect_definitions(inner, false, functions, classes);
            }
            StmtKind::ClassDef { body: inner, .. } => {
                classes.push(statement);
                collect_definitions(inner, true, functions, classes);
            }
            _ => for_each_child_block(statement, |block| collect_definitions(block, false, functions, classes)),
        }
    }
}
