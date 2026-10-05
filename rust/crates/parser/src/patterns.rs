//! `match` statements (soft keyword) and their patterns.

use refactrail_lexer::Kind;

use crate::ast::{Expr, ExprContext, ExprKind, MatchCase, Operator, Pattern, PatternKind, Stmt, StmtKind, UnaryOperator};
use crate::expr::{constant_node, expr, name_node, tuple};
use crate::node::{Constant, Loc};
use crate::parser::{stmt, PResult, Parser};

fn pattern(kind: PatternKind, loc: Loc) -> Pattern {
    Pattern { kind, loc }
}

impl<'a> Parser<'a> {
    /// Parse a match statement, or return None (position restored) when
    /// `match` is an ordinary name here.
    pub fn try_match_statement(&mut self) -> PResult<Option<Stmt>> {
        let start = self.pos;
        let next = self.kind_at(start + 1);
        if matches!(next, Kind::Newline | Kind::EndMarker)
            || (next == Kind::Op && matches!(self.text_at(start + 1), "=" | "." | ":" | "," | ")" | "]" | "}" | ";"))
        {
            return Ok(None);
        }
        self.bump();
        let subject = match self.match_subject() {
            Ok(subject) if self.at_op(":") && self.kind_at(self.pos + 1) == Kind::Newline => subject,
            _ => {
                self.pos = start;
                return Ok(None);
            }
        };
        self.bump();
        self.bump();
        if self.kind() != Kind::Indent {
            let line = self.loc(start).line;
            return Err(self.demote(self.error(&format!("expected an indented block after 'match' statement on line {line}"))));
        }
        self.bump();
        let mut cases = Vec::new();
        while self.at_kw("case") {
            cases.push(self.case_block()?);
        }
        if cases.is_empty() {
            return Err(self.error("invalid syntax"));
        }
        if self.kind() == Kind::Dedent {
            self.bump();
        } else if self.kind() != Kind::EndMarker {
            return Err(self.error("invalid syntax"));
        }
        let end = self.last_content_end();
        Ok(Some(stmt(StmtKind::Match { subject: Box::new(subject), cases }, self.loc(start).to(end))))
    }

    fn match_subject(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let first = self.star_named_expression()?;
        if !self.at_op(",") {
            return Ok(first);
        }
        let mut elts = vec![first];
        while self.eat_op(",").is_some() {
            if self.at_op(":") {
                break;
            }
            elts.push(self.star_named_expression()?);
        }
        Ok(tuple(elts, self.span(start)))
    }

    fn case_block(&mut self) -> PResult<MatchCase> {
        let keyword = self.bump();
        let pattern = Box::new(self.patterns()?);
        let guard = if self.eat_kw("if").is_some() { Some(Box::new(self.named_expression()?)) } else { None };
        self.expect_colon(false)?;
        let body = self.block("'case' statement", keyword)?;
        Ok(MatchCase { pattern, guard, body })
    }

    fn patterns(&mut self) -> PResult<Pattern> {
        let start = self.pos;
        let first = self.maybe_star_pattern()?;
        if !self.at_op(",") {
            if matches!(first.kind, PatternKind::MatchStar { .. }) {
                return Ok(pattern(PatternKind::MatchSequence { patterns: vec![first] }, self.span(start)));
            }
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat_op(",").is_some() {
            if self.at_op(":") || self.at_kw("if") {
                break;
            }
            items.push(self.maybe_star_pattern()?);
        }
        Ok(pattern(PatternKind::MatchSequence { patterns: items }, self.span(start)))
    }

    fn maybe_star_pattern(&mut self) -> PResult<Pattern> {
        if let Some(star) = self.eat_op("*") {
            let (name, _) = self.expect_name()?;
            let name = (&*name != "_").then_some(name);
            return Ok(pattern(PatternKind::MatchStar { name }, self.span(star)));
        }
        self.pattern()
    }

    fn pattern(&mut self) -> PResult<Pattern> {
        let start = self.pos;
        let first = self.or_pattern()?;
        if self.eat_kw("as").is_none() {
            return Ok(first);
        }
        let (name, _) = self.expect_name()?;
        if &*name == "_" {
            return Err(self.demote(self.error("cannot use '_' as a target")));
        }
        Ok(pattern(PatternKind::MatchAs { pattern: Some(Box::new(first)), name: Some(name) }, self.span(start)))
    }

    fn or_pattern(&mut self) -> PResult<Pattern> {
        let start = self.pos;
        let first = self.closed_pattern()?;
        if !self.at_op("|") {
            return Ok(first);
        }
        let mut items = vec![first];
        while self.eat_op("|").is_some() {
            items.push(self.closed_pattern()?);
        }
        Ok(pattern(PatternKind::MatchOr { patterns: items }, self.span(start)))
    }

    fn closed_pattern(&mut self) -> PResult<Pattern> {
        let start = self.pos;
        match self.kind() {
            Kind::Number => return self.value_pattern_from_expression(),
            Kind::Op if self.at_op("-") => return self.value_pattern_from_expression(),
            Kind::String | Kind::FStringStart | Kind::TStringStart => {
                let value = self.strings()?;
                let loc = value.loc;
                return Ok(pattern(PatternKind::MatchValue { value: Box::new(value) }, loc));
            }
            Kind::Op if self.at_op("(") || self.at_op("[") => return self.sequence_pattern(),
            Kind::Op if self.at_op("{") => return self.mapping_pattern(),
            Kind::Name => {}
            _ => return Err(self.error("invalid syntax")),
        }
        let singleton = match self.code() {
            51 => Some(Constant::None),
            52 => Some(Constant::True),
            50 => Some(Constant::False),
            _ => None,
        };
        if let Some(constant) = singleton {
            self.bump();
            return Ok(pattern(PatternKind::MatchSingleton { value: constant }, self.loc(start)));
        }
        let (name, _) = self.expect_name()?;
        let dotted = self.at_op(".");
        if !dotted && !self.at_op("(") {
            let name = (&*name != "_").then_some(name);
            return Ok(pattern(PatternKind::MatchAs { pattern: None, name }, self.loc(start)));
        }
        let mut target = name_node(name, ExprContext::Load, self.loc(start));
        while self.eat_op(".").is_some() {
            let (attr, _) = self.expect_name()?;
            target = expr(ExprKind::Attribute { value: Box::new(target), attr, ctx: ExprContext::Load }, self.span(start));
        }
        if self.at_op("(") {
            return self.class_pattern(start, target);
        }
        Ok(pattern(PatternKind::MatchValue { value: Box::new(target) }, self.span(start)))
    }

    fn value_pattern_from_expression(&mut self) -> PResult<Pattern> {
        let start = self.pos;
        let value = self.signed_sum()?;
        Ok(pattern(PatternKind::MatchValue { value: Box::new(value) }, self.span(start)))
    }

    /// A signed number, optionally `+` or `-` another (complex literals).
    fn signed_sum(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let mut value = self.signed_number()?;
        if self.at_op("+") || self.at_op("-") {
            let op = if self.at_op("+") { Operator::Add } else { Operator::Sub };
            self.bump();
            let right = self.signed_number()?;
            value = expr(ExprKind::BinOp { left: Box::new(value), op, right: Box::new(right) }, self.span(start));
        }
        Ok(value)
    }

    fn signed_number(&mut self) -> PResult<Expr> {
        let start = self.pos;
        let negative = self.eat_op("-").is_some();
        if self.kind() != Kind::Number {
            return Err(self.error("invalid syntax"));
        }
        let constant = crate::literals::number(self.text()).map_err(|message| self.error(&message))?;
        let index = self.bump();
        let number = constant_node(constant, self.loc(index));
        if !negative {
            return Ok(number);
        }
        Ok(expr(ExprKind::UnaryOp { op: UnaryOperator::USub, operand: Box::new(number) }, self.span(start)))
    }

    fn sequence_pattern(&mut self) -> PResult<Pattern> {
        let open = self.bump();
        let close = if self.op_at(open, "(") { ")" } else { "]" };
        let mut items = Vec::new();
        let mut comma = false;
        while !self.at_op(close) {
            items.push(self.maybe_star_pattern()?);
            if self.eat_op(",").is_none() {
                break;
            }
            comma = true;
        }
        self.expect_op(close)?;
        if close == ")" && items.len() == 1 && !comma && !matches!(items[0].kind, PatternKind::MatchStar { .. }) {
            return Ok(items.pop().expect("one item"));
        }
        Ok(pattern(PatternKind::MatchSequence { patterns: items }, self.span(open)))
    }

    fn mapping_pattern(&mut self) -> PResult<Pattern> {
        let open = self.bump();
        let (mut keys, mut patterns, mut rest) = (Vec::new(), Vec::new(), None);
        while !self.at_op("}") {
            if self.eat_op("**").is_some() {
                rest = Some(self.expect_name()?.0);
            } else {
                let key = match self.kind() {
                    Kind::String | Kind::FStringStart | Kind::TStringStart => self.strings()?,
                    Kind::Number => self.signed_sum()?,
                    Kind::Op if self.at_op("-") => self.signed_sum()?,
                    Kind::Name if matches!(self.code(), 50..=52) => {
                        let constant = match self.code() {
                            51 => Constant::None,
                            52 => Constant::True,
                            _ => Constant::False,
                        };
                        let index = self.bump();
                        constant_node(constant, self.loc(index))
                    }
                    _ => {
                        let start = self.pos;
                        let (name, _) = self.expect_name()?;
                        let mut target = name_node(name, ExprContext::Load, self.loc(start));
                        while self.eat_op(".").is_some() {
                            let (attr, _) = self.expect_name()?;
                            target = expr(ExprKind::Attribute { value: Box::new(target), attr, ctx: ExprContext::Load }, self.span(start));
                        }
                        target
                    }
                };
                self.expect_op(":")?;
                keys.push(key);
                patterns.push(self.pattern()?);
            }
            if self.eat_op(",").is_none() {
                break;
            }
        }
        self.expect_op("}")?;
        Ok(pattern(PatternKind::MatchMapping { keys, patterns, rest }, self.span(open)))
    }

    fn class_pattern(&mut self, start: usize, cls: Expr) -> PResult<Pattern> {
        self.bump();
        let (mut patterns, mut kwd_attrs, mut kwd_patterns) = (Vec::new(), Vec::new(), Vec::new());
        while !self.at_op(")") {
            if self.at_identifier() && self.op_at(self.pos + 1, "=") {
                let (name, _) = self.expect_name()?;
                self.bump();
                kwd_attrs.push(name);
                kwd_patterns.push(self.pattern()?);
            } else {
                patterns.push(self.pattern()?);
            }
            if self.eat_op(",").is_none() {
                break;
            }
        }
        self.expect_op(")")?;
        Ok(pattern(PatternKind::MatchClass { cls: Box::new(cls), patterns, kwd_attrs, kwd_patterns }, self.span(start)))
    }
}
