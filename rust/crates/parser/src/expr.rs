//! Expressions, call arguments, parameters and assignment targets.

use refactrail_lexer::Kind;

use crate::ast::{
    Arg, Arguments, BoolOperator, CmpOperator, Comprehension, Expr, ExprContext, ExprKind, Id, Keyword, Operator,
    UnaryOperator,
};
use crate::literals;
use crate::node::{Constant, Loc};
use crate::parser::{expr_name, set_context, ParseError, PResult, Parser};

/// (precedence level, operator) of a binary operator's symbol code;
/// higher binds tighter.
#[inline(always)]
fn binary_operator(code: u8) -> Option<(u8, Operator)> {
    Some(match code {
        14 => (1, Operator::BitOr),
        26 => (2, Operator::BitXor),
        15 => (3, Operator::BitAnd),
        27 => (4, Operator::LShift),
        28 => (4, Operator::RShift),
        10 => (5, Operator::Add),
        11 => (5, Operator::Sub),
        12 => (6, Operator::Mult),
        13 => (6, Operator::Div),
        41 => (6, Operator::FloorDiv),
        20 => (6, Operator::Mod),
        43 => (6, Operator::MatMult),
        _ => return None,
    })
}

#[inline(always)]
pub fn expr(kind: ExprKind, loc: Loc) -> Expr {
    Expr { kind, loc }
}

impl<'a> Parser<'a> {
    /// Whether the current token can begin an expression.
    pub fn starts_expression(&self) -> bool {
        match self.kind() {
            // Not a hard keyword, or one of the keywords that start one
            // (not, lambda, await, True, False, None).
            Kind::Name => !crate::parser::is_keyword_code(self.code()) || matches!(self.code(), 76 | 74 | 57 | 52 | 50 | 51),
            Kind::Number | Kind::String | Kind::FStringStart => true,
            // ( [ { - + ~ * ...
            Kind::Op => matches!(self.code(), 1 | 3 | 5 | 11 | 10 | 25 | 12 | 46),
            _ => false,
        }
    }

    pub fn star_expressions(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let first = self.star_expression()?;
        if !self.at_op(",") {
            return Ok(first);
        }
        let mut elts = vec![first];
        while self.eat_op(",").is_some() {
            if !self.starts_expression() {
                break;
            }
            elts.push(self.star_expression()?);
        }
        Ok(tuple(elts, self.span(start)))
    }

    #[inline]
    pub fn star_expression(&mut self) -> PResult<Expr> {
        if let Some(star) = self.eat_op("*") {
            let value = self.bitwise_or()?;
            return Ok(starred(value, self.span(star)));
        }
        self.expression()
    }

    #[inline]
    pub fn star_named_expression(&mut self) -> PResult<Expr> {
        if let Some(star) = self.eat_op("*") {
            let value = self.bitwise_or()?;
            return Ok(starred(value, self.span(star)));
        }
        self.named_expression()
    }

    pub fn named_expression(&mut self) -> PResult<Expr> {
        if self.invalid_mode {
            self.check_named_expression(false)?;
        }
        self.named_expression_arg()
    }

    /// Parse `f()` without the result, restoring the mode; None when it
    /// fails. Used for the second pass's lookahead rules.
    fn probe<T>(&mut self, parse: impl FnOnce(&mut Self) -> PResult<T>) -> Option<T> {
        let mode = self.invalid_mode;
        self.invalid_mode = false;
        let result = parse(self).ok();
        self.invalid_mode = mode;
        result
    }

    /// CPython's invalid_named_expression rules (second pass). At the
    /// start of a statement (`statement`), they only apply to targets an
    /// assignment cannot take.
    pub fn check_named_expression(&mut self, statement: bool) -> PResult<()> {
        let start = self.pos;
        if !(self.at_identifier() && self.op_at(start + 1, ":=")) {
            let found = self.probe(|this| {
                let target = this.expression()?;
                if !this.at_op(":=") {
                    return Ok(None);
                }
                this.bump();
                this.expression()?;
                Ok(Some(target))
            });
            self.pos = start;
            if let Some(Some(target)) = found {
                let message = format!("cannot use assignment expressions with {}", expr_name(&target));
                return Err(self.specific(target.loc, &message));
            }
        }
        if !statement && self.at_identifier() && self.op_at(start + 1, "=") {
            let name_loc = self.loc(start);
            let matched = self.probe(|this| {
                this.bump();
                this.bump();
                this.bitwise_or()?;
                Ok(!this.at_op("=") && !this.at_op(":="))
            });
            self.pos = start;
            if matched == Some(true) {
                return Err(self.specific(name_loc, "invalid syntax. Maybe you meant '==' or ':=' instead of '='?"));
            }
        }
        let excluded = self.at_op("[") || self.at_op("(") || self.at_kw("True") || self.at_kw("None") || self.at_kw("False");
        if !excluded {
            let found = self.probe(|this| {
                let target = this.bitwise_or()?;
                if !this.at_op("=") {
                    return Ok(None);
                }
                this.bump();
                this.bitwise_or()?;
                Ok((!this.at_op("=") && !this.at_op(":=")).then_some(target))
            });
            self.pos = start;
            if let Some(Some(target)) = found {
                let assignable = matches!(
                    target.kind,
                    ExprKind::Name { .. } | ExprKind::Attribute { .. } | ExprKind::Subscript { .. } | ExprKind::Starred { .. }
                );
                if !(statement && assignable) {
                    let message = format!("cannot assign to {} here. Maybe you meant '==' instead of '='?", expr_name(&target));
                    return Err(self.specific(target.loc, &message));
                }
            }
        }
        Ok(())
    }

    /// `named_expression` without the second-pass checks: call arguments
    /// and generator elements use CPython's assignment_expression rule.
    pub fn named_expression_arg(&mut self) -> PResult<Expr> {
        if self.at_identifier() && self.op_at(self.pos + 1, ":=") {
            let start = self.bump();
            let target = Box::new(name_node(crate::parser::identifier(self.text_at(start)), ExprContext::Store, self.loc(start)));
            self.bump();
            let value = Box::new(self.expression()?);
            return Ok(expr(ExprKind::NamedExpr { target, value }, self.span(start)));
        }
        self.expression()
    }

    /// CPython's invalid_expression rule: two expressions side by side
    /// inside brackets suggest a missing comma.
    fn check_missing_comma(&mut self) -> PResult<()> {
        let start = self.pos;
        if !self.comma_checked.insert(start) {
            return Ok(());
        }
        if self.kind() == Kind::Name
            && (matches!(self.kind_at(start + 1), Kind::String | Kind::FStringStart) || is_soft_keyword_prefix(self.text()))
        {
            return Ok(());
        }
        let first = match self.disjunction() {
            Ok(first) => first,
            Err(error) if error.message.starts_with("invalid syntax. Perhaps") => return Err(error),
            Err(_) => {
                self.pos = start;
                return Ok(());
            }
        };
        if !self.starts_expression() || self.at_op("*") {
            self.pos = start;
            return Ok(());
        }
        self.invalid_mode = false;
        let second_ok = self.expression().is_ok();
        self.invalid_mode = true;
        let level = self.level_at(self.pos.saturating_sub(1));
        self.pos = start;
        let legacy = matches!(&first.kind, ExprKind::Name { id, .. } if &**id == "print" || &**id == "exec");
        if !second_ok || level == 0 || legacy {
            return Ok(());
        }
        let loc = first.loc;
        Err(ParseError { line: loc.line, col: loc.col, offset: 0, message: "invalid syntax. Perhaps you forgot a comma?".into() })
    }

    pub fn expression(&mut self) -> PResult<Expr> {
        if self.invalid_mode {
            self.check_missing_comma()?;
        }
        if self.at_kw("lambda") {
            return self.lambda();
        }
        let start = self.pos;
        let body = self.disjunction()?;
        if !self.at_kw("if") {
            return Ok(body);
        }
        self.bump();
        let test = self.disjunction()?;
        if self.invalid_mode && !self.at_kw("else") && !self.at_op(":") {
            return Err(self.specific(body.loc, "expected 'else' after 'if' expression"));
        }
        self.expect_kw("else")?;
        let orelse = self.expression()?;
        Ok(expr(
            ExprKind::IfExp { test: Box::new(test), body: Box::new(body), orelse: Box::new(orelse) },
            self.span(start),
        ))
    }

    fn lambda(&mut self) -> PResult<Expr> {
        let start = self.bump();
        let args = Box::new(self.parameters(true)?);
        self.expect_op(":")?;
        let body = Box::new(self.expression()?);
        Ok(expr(ExprKind::Lambda { args, body }, self.span(start)))
    }

    #[inline(always)]
    pub fn disjunction(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let first = self.conjunction()?;
        if !self.at_kw("or") {
            return Ok(first);
        }
        let mut values = vec![first];
        while self.eat_kw("or").is_some() {
            values.push(self.conjunction()?);
        }
        Ok(expr(ExprKind::BoolOp { op: BoolOperator::Or, values }, self.span(start)))
    }

    #[inline(always)]
    fn conjunction(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let first = self.inversion()?;
        if !self.at_kw("and") {
            return Ok(first);
        }
        let mut values = vec![first];
        while self.eat_kw("and").is_some() {
            values.push(self.inversion()?);
        }
        Ok(expr(ExprKind::BoolOp { op: BoolOperator::And, values }, self.span(start)))
    }

    #[inline(always)]
    fn inversion(&mut self) -> PResult<Expr> {
        if self.at_kw("not") {
            return self.not_expression();
        }
        self.comparison()
    }

    #[inline(never)]
    fn not_expression(&mut self) -> PResult<Expr> {
        let start = self.bump();
        let operand = Box::new(self.inversion()?);
        Ok(expr(ExprKind::UnaryOp { op: UnaryOperator::Not, operand }, self.span(start)))
    }

    fn comparison_operator(&mut self) -> Option<CmpOperator> {
        let op = match self.kind() {
            Kind::Op => match self.code() {
                21 => CmpOperator::Eq,
                22 => CmpOperator::NotEq,
                16 => CmpOperator::Lt,
                23 => CmpOperator::LtE,
                17 => CmpOperator::Gt,
                24 => CmpOperator::GtE,
                _ => return None,
            },
            Kind::Name => match self.code() {
                72 => CmpOperator::In,
                76 if self.kw_at(self.pos + 1, "in") => {
                    self.bump();
                    CmpOperator::NotIn
                }
                73 if self.kw_at(self.pos + 1, "not") => {
                    self.bump();
                    CmpOperator::IsNot
                }
                73 => CmpOperator::Is,
                _ => return None,
            },
            _ => return None,
        };
        self.bump();
        Some(op)
    }

    #[inline(always)]
    fn comparison(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let left = self.bitwise_or()?;
        let Some(first_op) = self.comparison_operator() else { return Ok(left) };
        let mut ops = vec![first_op];
        let mut comparators = vec![self.bitwise_or()?];
        while let Some(op) = self.comparison_operator() {
            ops.push(op);
            comparators.push(self.bitwise_or()?);
        }
        Ok(expr(ExprKind::Compare { left: Box::new(left), ops, comparators }, self.span(start)))
    }

    #[inline]
    pub fn bitwise_or(&mut self) -> PResult<Expr> {
        self.binary(1)
    }

    /// Precedence climbing over the six left-associative binary levels
    /// (one frame instead of six nested ones per operand).
    fn binary(&mut self, min_level: u8) -> PResult<Expr> {
        let start = self.pos;
        let mut left = self.factor()?;
        while self.kind() == Kind::Op {
            let Some((op_level, op)) = binary_operator(self.code()) else { break };
            if op_level < min_level {
                break;
            }
            self.bump();
            let right = self.binary(op_level + 1)?;
            left = expr(ExprKind::BinOp { left: Box::new(left), op, right: Box::new(right) }, self.span(start));
        }
        Ok(left)
    }

    #[inline(always)]
    fn factor(&mut self) -> PResult<Expr> {
        if self.kind() == Kind::Op && matches!(self.code(), 10 | 11 | 25) {
            return self.unary_expression();
        }
        self.power()
    }

    #[inline(never)]
    fn unary_expression(&mut self) -> PResult<Expr> {
        let op = match self.code() {
            10 => UnaryOperator::UAdd,
            11 => UnaryOperator::USub,
            _ => UnaryOperator::Invert,
        };
        let start = self.bump();
        let operand = Box::new(self.factor()?);
        Ok(expr(ExprKind::UnaryOp { op, operand }, self.span(start)))
    }

    #[inline(always)]
    fn power(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let base = self.await_primary()?;
        if self.eat_op("**").is_none() {
            return Ok(base);
        }
        let exponent = self.factor()?;
        Ok(expr(ExprKind::BinOp { left: Box::new(base), op: Operator::Pow, right: Box::new(exponent) }, self.span(start)))
    }

    #[inline(always)]
    fn await_primary(&mut self) -> PResult<Expr> {
        if self.at_kw("await") {
            return self.await_expression();
        }
        self.primary()
    }

    #[inline(never)]
    fn await_expression(&mut self) -> PResult<Expr> {
        let start = self.bump();
        let value = Box::new(self.primary()?);
        Ok(expr(ExprKind::Await { value }, self.span(start)))
    }

    #[inline(always)]
    pub fn primary(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let mut node = self.atom()?;
        loop {
            if self.at_op(".") {
                self.bump();
                let (attr, _) = self.expect_name()?;
                node = expr(ExprKind::Attribute { value: Box::new(node), attr, ctx: ExprContext::Load }, self.span(start));
            } else if self.at_op("(") {
                let (args, keywords) = self.call_arguments()?;
                node = expr(ExprKind::Call { func: Box::new(node), args, keywords }, self.span(start));
            } else if self.at_op("[") {
                self.bump();
                let slice = Box::new(self.slices()?);
                self.expect_op("]")?;
                node = expr(ExprKind::Subscript { value: Box::new(node), slice, ctx: ExprContext::Load }, self.span(start));
            } else {
                return Ok(node);
            }
        }
    }

    fn slices(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let first = self.slice()?;
        if !self.at_op(",") {
            return Ok(first);
        }
        let mut elts = vec![first];
        while self.eat_op(",").is_some() {
            if self.at_op("]") {
                break;
            }
            elts.push(self.slice()?);
        }
        Ok(tuple(elts, self.span(start)))
    }

    fn slice_bound(&mut self) -> PResult<Option<Box<Expr>>> {
        if self.starts_expression() && !self.at_op("*") {
            Ok(Some(Box::new(self.expression()?)))
        } else {
            Ok(None)
        }
    }

    fn slice(&mut self) -> PResult<Expr> {
        if let Some(star) = self.eat_op("*") {
            let value = self.bitwise_or()?;
            return Ok(starred(value, self.span(star)));
        }
        let start = self.pos;
        let lower = if self.at_op(":") { None } else { Some(self.named_expression()?) };
        if !self.at_op(":") {
            return lower.ok_or_else(|| self.error("invalid syntax"));
        }
        self.bump();
        let upper = self.slice_bound()?;
        let step = if self.eat_op(":").is_some() { self.slice_bound()? } else { None };
        Ok(expr(ExprKind::Slice { lower: lower.map(Box::new), upper, step }, self.span(start)))
    }

    /// Parse `( arguments )`, returning positional args and keywords.
    pub fn call_arguments(&mut self) -> PResult<(Vec<Expr>, Vec<Keyword>)> {
        let open = self.expect_op("(")?;
        let (mut args, mut keywords) = (Vec::new(), Vec::new());
        let (mut seen_keyword, mut seen_double_star) = (false, false);
        let mut misplaced: Option<&'static str> = None;
        while !self.at_op(")") {
            let start = self.pos;
            if self.eat_op("*").is_some() {
                if seen_double_star {
                    return Err(self.demote(self.specific(self.loc(start - 1), "iterable argument unpacking follows keyword argument unpacking")));
                }
                let value = self.expression()?;
                args.push(starred(value, self.span(start)));
            } else if self.eat_op("**").is_some() {
                let value = Box::new(self.expression()?);
                seen_double_star = true;
                keywords.push(Keyword { arg: None, value, loc: self.span(start) });
            } else if self.at_identifier() && self.op_at(self.pos + 1, "=") {
                let name: Id = crate::parser::identifier(self.text());
                self.bump();
                self.bump();
                let value = Box::new(self.expression()?);
                seen_keyword = true;
                keywords.push(Keyword { arg: Some(name), value, loc: self.span(start) });
            } else {
                let value = self.named_expression_arg()?;
                if self.invalid_mode && self.at_op("=") {
                    return Err(self.specific(value.loc, "expression cannot contain assignment, perhaps you meant \"==\"?"));
                }
                if (seen_keyword || seen_double_star) && !self.at_comprehension() {
                    if !self.invalid_mode {
                        return Err(self.error("invalid syntax"));
                    }
                    misplaced = Some(if seen_double_star {
                        "positional argument follows keyword argument unpacking"
                    } else {
                        "positional argument follows keyword argument"
                    });
                }
                if self.at_comprehension() {
                    let generators = self.comprehensions()?;
                    if !args.is_empty() || !keywords.is_empty() || self.at_op(",") {
                        let loc = value.loc;
                        return Err(self.demote(ParseError {
                            line: loc.line,
                            col: loc.col,
                            offset: 0,
                            message: "Generator expression must be parenthesized".into(),
                        }));
                    }
                    let close = self.expect_op(")")?;
                    let genexp = expr(ExprKind::GeneratorExp { elt: Box::new(value), generators }, self.loc(open).to(self.loc(close)));
                    args.push(genexp);
                    return Ok((args, keywords));
                }
                args.push(value);
            }
            if self.eat_op(",").is_none() {
                break;
            }
        }
        let close = self.expect_op(")")?;
        if let Some(message) = misplaced {
            return Err(self.specific(self.loc(close), message));
        }
        Ok((args, keywords))
    }

    /// Parameters of a `def` (inside the parentheses) or a `lambda`.
    pub fn parameters(&mut self, lambda: bool) -> PResult<Arguments> {
        let end = if lambda { ":" } else { ")" };
        let (mut posonlyargs, mut args, mut defaults) = (Vec::new(), Vec::new(), Vec::new());
        let (mut kwonlyargs, mut kw_defaults) = (Vec::new(), Vec::new());
        let (mut vararg, mut kwarg) = (None, None);
        let mut seen_star = false;
        while !self.at_op(end) {
            if self.eat_op("/").is_some() {
                posonlyargs.append(&mut args);
            } else if let Some(star) = self.eat_op("*") {
                seen_star = true;
                if self.at_identifier() {
                    vararg = Some(self.parameter(lambda, true)?);
                } else if self.at_op(end) || (self.at_op(",") && (self.op_at(self.pos + 1, end) || self.op_at(self.pos + 1, "**"))) {
                    return Err(self.demote(self.specific(self.loc(star), "named arguments must follow bare *")));
                }
            } else if kwarg.is_some() {
                return Err(self.demote(self.error("arguments cannot follow var-keyword argument")));
            } else if self.eat_op("**").is_some() {
                kwarg = Some(self.parameter(lambda, false)?);
            } else {
                let param = self.parameter(lambda, false)?;
                let default = if self.eat_op("=").is_some() { Some(self.expression()?) } else { None };
                if !seen_star && default.is_none() && !defaults.is_empty() {
                    let loc = param.loc;
                    return Err(self.demote(ParseError {
                        line: loc.line,
                        col: loc.col,
                        offset: 0,
                        message: "parameter without a default follows parameter with a default".into(),
                    }));
                }
                if seen_star {
                    kwonlyargs.push(param);
                    kw_defaults.push(default);
                } else {
                    args.push(param);
                    if let Some(default) = default {
                        defaults.push(default);
                    }
                }
            }
            if self.eat_op(",").is_none() {
                break;
            }
        }
        Ok(Arguments { posonlyargs, args, vararg, kwonlyargs, kw_defaults, kwarg, defaults })
    }

    fn parameter(&mut self, lambda: bool, star: bool) -> PResult<Arg> {
        let (arg, start) = self.expect_name()?;
        let annotation = if !lambda && self.eat_op(":").is_some() {
            Some(Box::new(if star { self.star_expression()? } else { self.expression()? }))
        } else {
            None
        };
        Ok(Arg { arg, annotation, type_comment: None, loc: self.span(start) })
    }

    // ----- targets ------------------------------------------------------

    /// `star_targets`: for-loop and comprehension targets.
    pub fn star_targets(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let first = self.star_target()?;
        if !self.at_op(",") {
            return Ok(first);
        }
        let mut elts = vec![first];
        while self.eat_op(",").is_some() {
            if !(self.at_identifier() || self.at_op("(") || self.at_op("[") || self.at_op("*")) {
                break;
            }
            elts.push(self.star_target()?);
        }
        let mut node = tuple(elts, self.span(start));
        set_context(&mut node, ExprContext::Store).map_err(|error| self.demote(error))?;
        Ok(node)
    }

    pub fn star_target(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let mut node = if self.eat_op("*").is_some() {
            let value = self.star_target()?;
            starred(value, self.span(start))
        } else {
            self.primary()?
        };
        set_context(&mut node, ExprContext::Store).map_err(|error| self.demote(error))?;
        Ok(node)
    }

    pub fn comprehensions(&mut self) -> PResult<Vec<Comprehension>> {
        let mut generators = Vec::new();
        loop {
            let is_async = self.at_kw("async") && self.kw_at(self.pos + 1, "for");
            if is_async {
                self.bump();
            }
            if self.eat_kw("for").is_none() {
                break;
            }
            let target = Box::new(self.star_targets()?);
            self.expect_kw("in")?;
            let iter = Box::new(self.disjunction()?);
            let mut ifs = Vec::new();
            while self.eat_kw("if").is_some() {
                ifs.push(self.disjunction()?);
            }
            generators.push(Comprehension { target, iter, ifs, is_async: i64::from(is_async) });
        }
        Ok(generators)
    }

    fn at_comprehension(&self) -> bool {
        self.at_kw("for") || (self.at_kw("async") && self.kw_at(self.pos + 1, "for"))
    }

    pub fn yield_expression(&mut self) -> PResult<Expr> {
        let start = self.bump();
        if self.eat_kw("from").is_some() {
            let value = Box::new(self.expression()?);
            return Ok(expr(ExprKind::YieldFrom { value }, self.span(start)));
        }
        let value = if self.starts_expression() { Some(Box::new(self.star_expressions()?)) } else { None };
        Ok(expr(ExprKind::Yield { value }, self.span(start)))
    }

    // ----- atoms --------------------------------------------------------

    fn atom(&mut self) -> PResult<Expr> {
        let start = self.pos;
        match self.kind() {
            Kind::Name => {
                let constant = match self.code() {
                    52 => Some(Constant::True),
                    50 => Some(Constant::False),
                    51 => Some(Constant::None),
                    _ => None,
                };
                if let Some(constant) = constant {
                    self.bump();
                    return Ok(constant_node(constant, self.loc(start)));
                }
                if !self.at_identifier() {
                    return Err(self.error("invalid syntax"));
                }
                self.bump();
                Ok(name_node(crate::parser::identifier(self.text_at(start)), ExprContext::Load, self.loc(start)))
            }
            Kind::Number => {
                let constant = literals::number(self.text()).map_err(|message| self.error(&message))?;
                self.bump();
                Ok(constant_node(constant, self.loc(start)))
            }
            Kind::String | Kind::FStringStart => self.strings(),
            Kind::Op => match self.code() {
                1 => self.parenthesized(),
                3 => self.list_display(),
                5 => self.brace_display(),
                46 => {
                    self.bump();
                    Ok(constant_node(Constant::Ellipsis, self.loc(start)))
                }
                _ => Err(self.error("invalid syntax")),
            },
            _ => Err(self.error("invalid syntax")),
        }
    }

    fn parenthesized(&mut self) -> PResult<Expr> {
        let open = self.bump();
        if self.eat_op(")").is_some() {
            return Ok(tuple(Vec::new(), self.span(open)));
        }
        if self.at_kw("yield") {
            let value = self.yield_expression()?;
            self.expect_op(")")?;
            return Ok(value);
        }
        let first = self.star_named_expression()?;
        if self.at_comprehension() {
            let generators = self.comprehensions()?;
            self.expect_op(")")?;
            return Ok(expr(ExprKind::GeneratorExp { elt: Box::new(first), generators }, self.span(open)));
        }
        if self.at_op(")") {
            if matches!(first.kind, ExprKind::Starred { .. }) {
                return Err(self.demote(self.error("cannot use starred expression here")));
            }
            self.bump();
            return Ok(first);
        }
        let mut elts = vec![first];
        while self.eat_op(",").is_some() {
            if self.at_op(")") {
                break;
            }
            elts.push(self.star_named_expression()?);
        }
        self.expect_op(")")?;
        Ok(tuple(elts, self.span(open)))
    }

    fn list_display(&mut self) -> PResult<Expr> {
        let open = self.bump();
        if self.eat_op("]").is_some() {
            return Ok(expr(ExprKind::List { elts: Vec::new(), ctx: ExprContext::Load }, self.span(open)));
        }
        let first = self.star_named_expression()?;
        if self.at_comprehension() {
            let generators = self.comprehensions()?;
            self.expect_op("]")?;
            return Ok(expr(ExprKind::ListComp { elt: Box::new(first), generators }, self.span(open)));
        }
        let mut elts = vec![first];
        while self.eat_op(",").is_some() {
            if self.at_op("]") {
                break;
            }
            elts.push(self.star_named_expression()?);
        }
        self.expect_op("]")?;
        Ok(expr(ExprKind::List { elts, ctx: ExprContext::Load }, self.span(open)))
    }

    fn brace_display(&mut self) -> PResult<Expr> {
        let open = self.bump();
        if self.eat_op("}").is_some() {
            return Ok(expr(ExprKind::Dict { keys: Vec::new(), values: Vec::new() }, self.span(open)));
        }
        let (mut keys, mut values) = (Vec::new(), Vec::new());
        if self.eat_op("**").is_some() {
            keys.push(None);
            values.push(self.bitwise_or()?);
        } else {
            let first = self.star_named_expression()?;
            if self.eat_op(":").is_none() {
                return self.set_display(open, first);
            }
            if matches!(first.kind, ExprKind::Starred { .. }) {
                self.pos -= 1;
                return Err(self.error("invalid syntax"));
            }
            let value = self.expression()?;
            if self.at_comprehension() {
                let generators = self.comprehensions()?;
                self.expect_op("}")?;
                return Ok(expr(
                    ExprKind::DictComp { key: Box::new(first), value: Box::new(value), generators },
                    self.span(open),
                ));
            }
            keys.push(Some(first));
            values.push(value);
        }
        while self.eat_op(",").is_some() {
            if self.at_op("}") {
                break;
            }
            if self.eat_op("**").is_some() {
                keys.push(None);
                values.push(self.bitwise_or()?);
            } else {
                let mode = self.invalid_mode;
                self.invalid_mode = false;
                let key = self.expression();
                self.invalid_mode = mode;
                let key = key?;
                if mode && !self.at_op(":") {
                    // Reported at the last character of the key.
                    let loc = key.loc;
                    let at = Loc { line: loc.line, col: loc.end_col.saturating_sub(1), end_line: loc.end_line, end_col: loc.end_col };
                    return Err(self.specific(at, "':' expected after dictionary key"));
                }
                keys.push(Some(key));
                self.expect_op(":")?;
                values.push(self.expression()?);
            }
        }
        self.expect_op("}")?;
        Ok(expr(ExprKind::Dict { keys, values }, self.span(open)))
    }

    fn set_display(&mut self, open: usize, first: Expr) -> PResult<Expr> {
        if self.at_comprehension() {
            let generators = self.comprehensions()?;
            self.expect_op("}")?;
            return Ok(expr(ExprKind::SetComp { elt: Box::new(first), generators }, self.span(open)));
        }
        let mut elts = vec![first];
        while self.eat_op(",").is_some() {
            if self.at_op("}") {
                break;
            }
            elts.push(self.star_named_expression()?);
        }
        self.expect_op("}")?;
        Ok(expr(ExprKind::Set { elts }, self.span(open)))
    }
}

/// CPython matches soft keywords with `strncmp(keyword, token, len(token))`,
/// so any prefix of `_`, `case`, `match` or `type` counts as one.
fn is_soft_keyword_prefix(text: &str) -> bool {
    !text.is_empty() && ["_", "case", "match", "type"].iter().any(|keyword| keyword.starts_with(text))
}

#[inline]
pub fn name_node(id: Id, ctx: ExprContext, loc: Loc) -> Expr {
    expr(ExprKind::Name { id, ctx }, loc)
}

#[inline]
pub fn constant_node(value: Constant, loc: Loc) -> Expr {
    expr(ExprKind::Constant { value, kind: None }, loc)
}

pub fn tuple(elts: Vec<Expr>, loc: Loc) -> Expr {
    expr(ExprKind::Tuple { elts, ctx: ExprContext::Load }, loc)
}

pub fn starred(value: Expr, loc: Loc) -> Expr {
    expr(ExprKind::Starred { value: Box::new(value), ctx: ExprContext::Load }, loc)
}
