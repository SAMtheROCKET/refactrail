//! General correctness checks RC101-RC107 (mirror of correctness.py and
//! correctness_checks.py): syntax evidence only, nothing is evaluated
//! beyond literal keys.

use crate::walk::{walk_expr, walk_stmt, Constant, Expr, ExprKind, Stmt, StmtKind, UnaryOperator, Visitor};
use refactrail_parser::ast::{Arguments, CmpOperator, ExceptHandler, MatchCase};
use refactrail_parser::Loc;

/// A finding before positions are converted: (code, location, message).
pub type Observation = (&'static str, Loc, String);

fn is_mutable_default(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::List { .. } | ExprKind::Dict { .. } | ExprKind::Set { .. } | ExprKind::ListComp { .. } | ExprKind::DictComp { .. } | ExprKind::SetComp { .. } => true,
        ExprKind::Tuple { elts, .. } => elts.iter().any(is_mutable_default),
        _ => false,
    }
}

/// is_literal_key_bool: constants, signed numbers and literal tuples.
fn is_literal_key(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Constant { .. } => true,
        ExprKind::Tuple { elts, .. } => elts.iter().all(is_literal_key),
        ExprKind::UnaryOp { op: UnaryOperator::UAdd | UnaryOperator::USub, operand } => {
            matches!(&operand.kind, ExprKind::Constant { value: Constant::Int(_) | Constant::Float(_) | Constant::Complex(_), .. })
        }
        _ => false,
    }
}

/// A canonical form of a literal key: two keys are equal in Python
/// (and so collide in a dict) exactly when their forms are equal.
#[derive(PartialEq)]
enum KeyValue {
    /// A real number as an exact value: integral numbers as decimal digits
    /// with a sign, other floats by their bits, infinities by sign.
    Number(String),
    /// A complex number with a nonzero imaginary part: (real, imaginary).
    Complex(String, String),
    Str(Vec<u32>),
    Bytes(Vec<u8>),
    None,
    Ellipsis,
    Tuple(Vec<KeyValue>),
}

/// Exact decimal form of an integral float, or its bits otherwise.
fn float_key(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if value.is_nan() {
        return format!("nan{}", value.to_bits());
    }
    if value.fract() != 0.0 {
        return format!("f{}", value.to_bits());
    }
    // An integral float: its exact integer value in decimal.
    let negative = value < 0.0;
    let bits = value.abs().to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i64 - 1075;
    let mantissa = (bits & ((1 << 52) - 1)) | (1 << 52);
    let mut digits = decimal_of(mantissa);
    if exponent >= 0 {
        for _ in 0..exponent {
            digits = double_decimal(&digits);
        }
    } else {
        // Integral with a negative exponent: shift the mantissa down.
        digits = decimal_of(mantissa >> (-exponent) as u32);
    }
    if negative { format!("-{digits}") } else { digits }
}

fn decimal_of(value: u64) -> String {
    value.to_string()
}

/// Multiply a decimal digit string by two.
fn double_decimal(digits: &str) -> String {
    let mut out = Vec::with_capacity(digits.len() + 1);
    let mut carry = 0;
    for byte in digits.bytes().rev() {
        let doubled = (byte - b'0') * 2 + carry;
        out.push(b'0' + doubled % 10);
        carry = doubled / 10;
    }
    if carry > 0 {
        out.push(b'0' + carry);
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn int_key(digits: &str, negative: bool) -> String {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        return "0".into();
    }
    if negative { format!("-{trimmed}") } else { trimmed.to_string() }
}

/// ast.literal_eval of a literal key, as a canonical value.
fn key_value(expression: &Expr) -> Option<KeyValue> {
    match &expression.kind {
        ExprKind::Constant { value, .. } => Some(constant_key(value, false)),
        ExprKind::Tuple { elts, .. } => elts.iter().map(key_value).collect::<Option<Vec<_>>>().map(KeyValue::Tuple),
        ExprKind::UnaryOp { op, operand } => {
            let negative = *op == UnaryOperator::USub;
            match &operand.kind {
                ExprKind::Constant { value, .. } => Some(constant_key(value, negative)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn constant_key(value: &Constant, negative: bool) -> KeyValue {
    match value {
        Constant::True => KeyValue::Number("1".into()),
        Constant::False => KeyValue::Number("0".into()),
        Constant::None => KeyValue::None,
        Constant::Ellipsis => KeyValue::Ellipsis,
        Constant::Str(points) => KeyValue::Str(points.clone()),
        Constant::Bytes(bytes) => KeyValue::Bytes(bytes.clone()),
        Constant::Int(digits) => KeyValue::Number(int_key(digits, negative)),
        Constant::Float(number) => KeyValue::Number(float_key(if negative { -number } else { *number })),
        // A complex literal is purely imaginary: (0 or -0, +-imag).
        Constant::Complex(imaginary) => {
            let imaginary = if negative { -imaginary } else { *imaginary };
            if imaginary == 0.0 {
                KeyValue::Number("0".into())
            } else {
                KeyValue::Complex("0".into(), float_key(imaginary))
            }
        }
    }
}

fn check_dictionary(keys: &[Option<Expr>], observations: &mut Vec<Observation>) {
    let mut seen: Vec<(KeyValue, u32)> = Vec::new();
    for key in keys {
        let Some(key) = key.as_ref().filter(|key| is_literal_key(key)) else {
            seen.clear();
            continue;
        };
        let Some(value) = key_value(key) else {
            seen.clear();
            continue;
        };
        match seen.iter_mut().find(|(earlier, _)| *earlier == value) {
            Some((_, line)) => {
                observations.push(("RC102", key.loc, format!("Duplicate literal dictionary key; earlier key at line {line}.")));
                *line = key.loc.line;
            }
            None => seen.push((value, key.loc.line)),
        }
    }
}

fn is_identity_literal(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Tuple { .. } => true,
        ExprKind::Constant { value, .. } => {
            matches!(value, Constant::Int(_) | Constant::Float(_) | Constant::Complex(_) | Constant::Str(_) | Constant::Bytes(_))
        }
        ExprKind::UnaryOp { .. } => is_literal_key(expression),
        _ => false,
    }
}

fn check_comparison(expression: &Expr, observations: &mut Vec<Observation>) {
    let ExprKind::Compare { left, ops, comparators } = &expression.kind else { return };
    let operands: Vec<&Expr> = std::iter::once(&**left).chain(comparators).collect();
    for (index, op) in ops.iter().enumerate() {
        if matches!(op, CmpOperator::Is | CmpOperator::IsNot)
            && operands[index..(index + 2).min(operands.len())].iter().any(|operand| is_identity_literal(operand))
        {
            observations.push((
                "RC103",
                expression.loc,
                "Identity comparison uses a non-singleton literal; review whether equality is intended.".into(),
            ));
            return;
        }
    }
}

fn check_defaults(arguments: &Arguments, observations: &mut Vec<Observation>) {
    for default in arguments.defaults.iter().chain(arguments.kw_defaults.iter().flatten()) {
        if is_mutable_default(default) {
            observations.push(("RC101", default.loc, "Mutable default state is shared across calls.".into()));
        }
    }
}

fn is_terminator(statement: &Stmt) -> bool {
    matches!(statement.kind, StmtKind::Return { .. } | StmtKind::Raise { .. } | StmtKind::Break | StmtKind::Continue)
}

/// RC105: the first statement after an unconditional terminator.
fn check_statement_list(body: &[Stmt], observations: &mut Vec<Observation>) {
    let mut terminated = false;
    for statement in body {
        if terminated {
            observations.push(("RC105", statement.loc, "Statement follows an unconditional control-flow terminator in this block.".into()));
            break;
        }
        terminated = is_terminator(statement);
    }
}

/// RC107: return, or break/continue outside a nested loop, in finally.
fn check_finally(finalbody: &[Stmt], observations: &mut Vec<Observation>) {
    struct Finally<'o> {
        depth: u32,
        observations: &'o mut Vec<Observation>,
    }
    impl<'a> Visitor<'a> for Finally<'_> {
        fn visit_stmt(&mut self, node: &'a Stmt) {
            match &node.kind {
                StmtKind::FunctionDef { .. } | StmtKind::AsyncFunctionDef { .. } | StmtKind::ClassDef { .. } => return,
                StmtKind::Return { .. } => self.report(node.loc),
                StmtKind::Break | StmtKind::Continue if self.depth == 0 => self.report(node.loc),
                _ => {}
            }
            match &node.kind {
                StmtKind::For { body, orelse, .. } | StmtKind::AsyncFor { body, orelse, .. } | StmtKind::While { body, orelse, .. } => {
                    self.depth += 1;
                    for statement in body {
                        self.visit_stmt(statement);
                    }
                    self.depth -= 1;
                    for statement in orelse {
                        self.visit_stmt(statement);
                    }
                }
                _ => walk_stmt(self, node),
            }
        }

        fn visit_expr(&mut self, node: &'a Expr) {
            if !matches!(node.kind, ExprKind::Lambda { .. }) {
                walk_expr(self, node);
            }
        }
    }
    impl Finally<'_> {
        fn report(&mut self, loc: Loc) {
            self.observations.push(("RC107", loc, "Control flow leaves finally and can override a pending exception or return.".into()));
        }
    }
    let mut finder = Finally { depth: 0, observations };
    for statement in finalbody {
        finder.visit_stmt(statement);
    }
}

/// Run RC101-RC107 over a module.
pub fn check_module(body: &[Stmt]) -> Vec<Observation> {
    struct Checks {
        observations: Vec<Observation>,
    }
    impl<'a> Visitor<'a> for Checks {
        fn visit_stmt(&mut self, node: &'a Stmt) {
            match &node.kind {
                StmtKind::FunctionDef { args, .. } | StmtKind::AsyncFunctionDef { args, .. } => check_defaults(args, &mut self.observations),
                StmtKind::Assert { test, .. } => {
                    if let ExprKind::Tuple { elts, .. } = &test.kind {
                        if !elts.is_empty() && !elts.iter().any(|element| matches!(element.kind, ExprKind::Starred { .. })) {
                            self.observations.push((
                                "RC106",
                                test.loc,
                                "A nonempty tuple assertion tests the tuple's truthiness, not its first element.".into(),
                            ));
                        }
                    }
                }
                StmtKind::Try { finalbody, .. } | StmtKind::TryStar { finalbody, .. } => check_finally(finalbody, &mut self.observations),
                _ => {}
            }
            for_each_statement_list(node, &mut |list| check_statement_list(list, &mut self.observations));
            walk_stmt(self, node);
        }

        fn visit_expr(&mut self, node: &'a Expr) {
            match &node.kind {
                ExprKind::Lambda { args, .. } => check_defaults(args, &mut self.observations),
                ExprKind::Dict { keys, .. } => check_dictionary(keys, &mut self.observations),
                ExprKind::Compare { .. } => check_comparison(node, &mut self.observations),
                _ => {}
            }
            walk_expr(self, node);
        }

        fn visit_excepthandler(&mut self, node: &'a ExceptHandler) {
            if node.type_.is_none() {
                self.observations.push((
                    "RC104",
                    node.loc,
                    "Bare except also catches process-control exceptions such as KeyboardInterrupt.".into(),
                ));
            }
            check_statement_list(&node.body, &mut self.observations);
            refactrail_parser::ast::walk_excepthandler(self, node);
        }

        fn visit_match_case(&mut self, node: &'a MatchCase) {
            check_statement_list(&node.body, &mut self.observations);
            refactrail_parser::ast::walk_match_case(self, node);
        }
    }
    let mut checks = Checks { observations: Vec::new() };
    check_statement_list(body, &mut checks.observations);
    for statement in body {
        checks.visit_stmt(statement);
    }
    checks.observations
}

/// The statement lists that are fields of a statement (body, orelse,
/// finalbody); handler and case bodies are checked by their own nodes.
fn for_each_statement_list<'a>(statement: &'a Stmt, visit: &mut impl FnMut(&'a [Stmt])) {
    match &statement.kind {
        StmtKind::FunctionDef { body, .. } | StmtKind::AsyncFunctionDef { body, .. } | StmtKind::ClassDef { body, .. } => visit(body),
        StmtKind::For { body, orelse, .. }
        | StmtKind::AsyncFor { body, orelse, .. }
        | StmtKind::While { body, orelse, .. }
        | StmtKind::If { body, orelse, .. } => {
            visit(body);
            visit(orelse);
        }
        StmtKind::With { body, .. } | StmtKind::AsyncWith { body, .. } => visit(body),
        StmtKind::Try { body, orelse, finalbody, .. } | StmtKind::TryStar { body, orelse, finalbody, .. } => {
            visit(body);
            visit(orelse);
            visit(finalbody);
        }
        _ => {}
    }
}
