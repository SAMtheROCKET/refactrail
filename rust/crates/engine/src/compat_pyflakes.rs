//! Pyflakes-compatible codes that need no scope analysis and can occur in
//! code that compiles: F501-F509, F521-F525, F541, F601, F602, F631-F634,
//! F722 and F901 (mirror of compat_formats.py and compat_pyflakes.py).
//! Files the compiler rejects (F404, F407, F62x, F70x) are linted by the
//! Python engine. Findings must match the Python engine.

use crate::compat_pycodestyle::CompatFinding;
use crate::source::{python_strip, SourceFile};
use crate::walk::{walk_expr, walk_stmt, CmpOperator, Constant, Expr, ExprKind, Module, Operator, Stmt, StmtKind, Visitor};
use refactrail_lexer::{Kind, Token};
use refactrail_parser::fast_hash::FastSet;

const PERCENT_FLAGS: &str = "#0- +";
const PERCENT_CONVERSIONS: &str = "diouxXeEfFgGcrsab%";

/// PercentSummary.
#[derive(Default)]
struct PercentSummary {
    positional: usize,
    keys: FastSet<String>,
    starred: bool,
}

enum PercentError {
    Invalid(&'static str),
    Unsupported(char),
}

/// skip_mapping_key_int.
fn skip_mapping_key(chars: &[char], index: usize) -> Result<usize, PercentError> {
    let mut depth = 0usize;
    for (position, &character) in chars.iter().enumerate().skip(index) {
        if character == '(' {
            depth += 1;
        } else if character == ')' {
            depth -= 1;
            if depth == 0 {
                return Ok(position + 1);
            }
        }
    }
    Err(PercentError::Invalid("incomplete mapping key"))
}

/// skip_quantity_tuple.
fn skip_quantity(chars: &[char], mut index: usize) -> (usize, bool) {
    if chars.get(index) == Some(&'*') {
        return (index + 1, true);
    }
    while chars.get(index).is_some_and(|character| character.is_ascii_digit()) {
        index += 1;
    }
    (index, false)
}

/// parse_spec_int.
fn parse_spec(chars: &[char], mut index: usize, summary: &mut PercentSummary) -> Result<usize, PercentError> {
    let mut key = None;
    if chars.get(index) == Some(&'(') {
        let end = skip_mapping_key(chars, index)?;
        key = Some(chars[index + 1..end - 1].iter().collect::<String>());
        index = end;
    }
    while chars.get(index).is_some_and(|character| PERCENT_FLAGS.contains(*character)) {
        index += 1;
    }
    let (after_width, star_width) = skip_quantity(chars, index);
    index = after_width;
    let mut star_precision = false;
    if chars.get(index) == Some(&'.') {
        let (after_precision, star) = skip_quantity(chars, index + 1);
        index = after_precision;
        star_precision = star;
    }
    while chars.get(index).is_some_and(|character| "hlL".contains(*character)) {
        index += 1;
    }
    let Some(&conversion) = chars.get(index) else {
        return Err(PercentError::Invalid("incomplete format"));
    };
    if !PERCENT_CONVERSIONS.contains(conversion) {
        return Err(PercentError::Unsupported(conversion));
    }
    match key {
        None => summary.positional += 1,
        Some(key) => {
            summary.keys.insert(key);
        }
    }
    for star in [star_width, star_precision] {
        if star {
            summary.positional += 1;
            summary.starred = true;
        }
    }
    Ok(index + 1)
}

/// summarise_percent_info.
fn summarise_percent(chars: &[char]) -> Result<PercentSummary, PercentError> {
    let mut summary = PercentSummary::default();
    let find = |from: usize| (from..chars.len()).find(|&position| chars[position] == '%');
    let mut index = find(0);
    while let Some(position) = index {
        if chars.get(position + 1) == Some(&'%') {
            index = find(position + 2);
            continue;
        }
        let after = parse_spec(chars, position + 1, &mut summary)?;
        index = find(after);
    }
    Ok(summary)
}

/// Python's repr() of a one-character string, for F509 messages.
fn char_repr(character: char) -> String {
    match character {
        '\'' => "\"'\"".into(),
        '\\' => "'\\\\'".into(),
        '\n' => "'\\n'".into(),
        '\r' => "'\\r'".into(),
        '\t' => "'\\t'".into(),
        _ if (character as u32) < 0x20 || character as u32 == 0x7f => format!("'\\x{:02x}'", character as u32),
        _ => format!("'{character}'"),
    }
}

fn is_sequence(expression: &Expr) -> bool {
    matches!(
        expression.kind,
        ExprKind::List { .. } | ExprKind::Tuple { .. } | ExprKind::Set { .. } | ExprKind::ListComp { .. } | ExprKind::SetComp { .. } | ExprKind::GeneratorExp { .. }
    )
}

fn is_mapping(expression: &Expr) -> bool {
    matches!(expression.kind, ExprKind::Dict { .. } | ExprKind::DictComp { .. })
}

fn is_single_value(expression: &Expr) -> bool {
    matches!(
        expression.kind,
        ExprKind::List { .. }
            | ExprKind::Set { .. }
            | ExprKind::Dict { .. }
            | ExprKind::ListComp { .. }
            | ExprKind::SetComp { .. }
            | ExprKind::DictComp { .. }
            | ExprKind::GeneratorExp { .. }
            | ExprKind::Constant { .. }
            | ExprKind::JoinedStr { .. }
    )
}

fn str_text(expression: &Expr) -> Option<String> {
    match &expression.kind {
        ExprKind::Constant { value: Constant::Str(points), .. } => {
            Some(points.iter().map(|&point| char::from_u32(point).unwrap_or('\u{fffd}')).collect())
        }
        _ => None,
    }
}

/// The checks, collecting findings.
pub struct SyntaxChecks<'s, 'a> {
    pub source: &'s SourceFile<'a>,
    pub tokens: &'s [Token],
    pub out: Vec<CompatFinding>,
    spec_ids: FastSet<usize>,
}

impl<'s, 'a> SyntaxChecks<'s, 'a> {
    pub fn new(source: &'s SourceFile<'a>, tokens: &'s [Token]) -> Self {
        SyntaxChecks { source, tokens, out: Vec::new(), spec_ids: FastSet::default() }
    }

    fn report(&mut self, code: &'static str, expression_loc: refactrail_parser::Loc, message: String) {
        let position = self.source.position(expression_loc);
        self.out.push((code, position, message, 0));
    }

    /// check_percent_format_none.
    fn percent(&mut self, node: &Expr, left: &Expr, right: &Expr) {
        let chars: Vec<char> = match &left.kind {
            ExprKind::Constant { value: Constant::Str(points), .. } => points.iter().map(|&point| char::from_u32(point).unwrap_or('\u{fffd}')).collect(),
            ExprKind::Constant { value: Constant::Bytes(bytes), .. } => bytes.iter().map(|&byte| byte as char).collect(),
            _ => return,
        };
        let summary = match summarise_percent(&chars) {
            Ok(summary) => summary,
            Err(PercentError::Unsupported(character)) => {
                self.report("F509", node.loc, format!("'...' % ... has an unsupported format character {}.", char_repr(character)));
                return;
            }
            Err(PercentError::Invalid(error)) => {
                self.report("F501", node.loc, format!("'...' % ... has an invalid format string: {error}."));
                return;
            }
        };
        // check_percent_sequence_none
        if summary.positional > 0 && !summary.keys.is_empty() {
            self.report("F506", node.loc, "'...' % ... mixes positional and named placeholders.".into());
        }
        if !summary.keys.is_empty() && is_sequence(right) {
            self.report("F502", node.loc, "'...' % ... expected a mapping but got a sequence.".into());
        }
        let count: Option<usize> = match &right.kind {
            ExprKind::Tuple { elts, .. } => {
                if elts.iter().any(|element| matches!(element.kind, ExprKind::Starred { .. })) {
                    None
                } else {
                    Some(elts.len())
                }
            }
            _ if is_single_value(right) => Some(1),
            _ => (summary.positional == 0).then_some(1),
        };
        if let Some(count) = count {
            if summary.keys.is_empty() && summary.positional != count {
                self.report("F507", node.loc, format!("'...' % ... has {} placeholder(s) but {count} substitution(s).", summary.positional));
            }
        }
        self.percent_mapping(node, right, &summary);
    }

    /// check_percent_mapping_none.
    fn percent_mapping(&mut self, node: &Expr, right: &Expr, summary: &PercentSummary) {
        if !is_mapping(right) {
            return;
        }
        if summary.positional > 1 {
            self.report("F503", node.loc, "'...' % ... expected a sequence but got a mapping.".into());
        }
        if summary.starred {
            self.report("F508", node.loc, "'...' % ... with * specifier requires a sequence.".into());
        }
        let ExprKind::Dict { keys, .. } = &right.kind else { return };
        if summary.positional > 0 {
            return;
        }
        let given: Vec<String> = keys.iter().flatten().filter_map(str_text).collect();
        let mut extra: Vec<&String> = given.iter().filter(|key| !summary.keys.contains(*key)).collect::<FastSet<_>>().into_iter().collect();
        extra.sort_by(|first, second| python_order(first, second));
        if !extra.is_empty() {
            let joined = extra.iter().map(|key| key.as_str()).collect::<Vec<_>>().join(", ");
            self.report("F504", node.loc, format!("'...' % ... has unused named argument(s): {joined}."));
        }
        if given.len() == keys.len() {
            let given_set: FastSet<&String> = given.iter().collect();
            let mut missing: Vec<&String> = summary.keys.iter().filter(|key| !given_set.contains(key)).collect();
            missing.sort_by(|first, second| python_order(first, second));
            if !missing.is_empty() {
                let joined = missing.iter().map(|key| key.as_str()).collect::<Vec<_>>().join(", ");
                self.report("F505", node.loc, format!("'...' % ... is missing argument(s) for placeholder(s): {joined}."));
            }
        }
    }

    /// check_format_call_none.
    fn format_call(&mut self, node: &Expr, func: &Expr, args: &[Expr], keywords: &[refactrail_parser::ast::Keyword]) {
        let ExprKind::Attribute { value, attr, .. } = &func.kind else { return };
        if &**attr != "format" {
            return;
        }
        let Some(text) = str_text(value) else { return };
        let summary = match summarise_format(&text) {
            Ok(summary) => summary,
            Err(error) => {
                self.report("F521", node.loc, format!("'...'.format(...) has an invalid format string: {error}."));
                return;
            }
        };
        if summary.automatic > 0 && !summary.indices.is_empty() {
            self.report("F525", node.loc, "'...'.format(...) mixes automatic and manual numbering.".into());
        }
        let used: FastSet<usize> = (0..summary.automatic).chain(summary.indices.iter().copied()).collect();
        let mut extra_named: Vec<&str> =
            keywords.iter().filter_map(|keyword| keyword.arg.as_deref()).filter(|name| !summary.keys.contains(*name)).collect();
        extra_named.sort_by(|first, second| python_order(first, second));
        let extra_positional: Vec<String> = args
            .iter()
            .enumerate()
            .filter(|(index, argument)| !matches!(argument.kind, ExprKind::Starred { .. }) && !used.contains(index))
            .map(|(index, _)| index.to_string())
            .collect();
        if !extra_named.is_empty() {
            self.report("F522", node.loc, format!("'...'.format(...) has unused named argument(s): {}.", extra_named.join(", ")));
        }
        if !extra_positional.is_empty() {
            self.report("F523", node.loc, format!("'...'.format(...) has unused positional argument(s): {}.", extra_positional.join(", ")));
        }
        if args.iter().any(|argument| matches!(argument.kind, ExprKind::Starred { .. })) || keywords.iter().any(|keyword| keyword.arg.is_none()) {
            return;
        }
        let given: FastSet<&str> = keywords.iter().filter_map(|keyword| keyword.arg.as_deref()).collect();
        let mut missing_numbers: Vec<String> = used.iter().filter(|&&index| index >= args.len()).map(|index| index.to_string()).collect();
        missing_numbers.sort_by(|first, second| python_order(first, second));
        let mut missing_keys: Vec<&String> = summary.keys.iter().filter(|key| !given.contains(key.as_str())).collect();
        missing_keys.sort_by(|first, second| python_order(first, second));
        let missing: Vec<String> = missing_numbers.into_iter().chain(missing_keys.into_iter().cloned()).collect();
        if !missing.is_empty() {
            self.report("F524", node.loc, format!("'...'.format(...) is missing argument(s) for placeholder(s): {}.", missing.join(", ")));
        }
    }

    /// check_fstring_none: F541 at each f-string part of a placeholder-free
    /// f-string expression.
    fn fstring(&mut self, node: &Expr, values: &[Expr]) {
        if values.iter().any(|part| matches!(part.kind, ExprKind::FormattedValue { .. })) {
            return;
        }
        let start = self.source.byte_offset(node.loc.line, node.loc.col);
        let end = self.source.byte_offset(node.loc.end_line, node.loc.end_col);
        for token in self.tokens {
            if token.kind == Kind::FStringStart && start <= token.start && token.start < end {
                let (line, column) = (token.start_pos.0 as usize, token.start_pos.1 as usize);
                self.out.push(("F541", (line, column + 1), "f-string without any placeholders.".into(), 0));
            }
        }
    }

    /// check_repeated_keys_none.
    fn repeated_keys(&mut self, keys: &[Option<Expr>]) {
        let mut seen: FastSet<String> = FastSet::default();
        for key in keys.iter().flatten() {
            let key_text = key_form(key);
            if seen.insert(key_text) {
                continue;
            }
            match &key.kind {
                ExprKind::Name { id, .. } => self.report("F602", key.loc, format!("Dictionary key '{id}' repeated.")),
                ExprKind::Constant { .. } | ExprKind::Tuple { .. } | ExprKind::JoinedStr { .. } => {
                    self.report("F601", key.loc, "Dictionary key literal repeated.".into())
                }
                _ => {}
            }
        }
    }

    /// check_is_literal_none.
    fn is_literal(&mut self, node: &Expr, left: &Expr, ops: &[CmpOperator], comparators: &[Expr]) {
        let operands: Vec<&Expr> = std::iter::once(left).chain(comparators).collect();
        for (index, operator) in ops.iter().enumerate() {
            if matches!(operator, CmpOperator::Is | CmpOperator::IsNot) && (is_non_singleton(operands[index]) || is_non_singleton(operands[index + 1])) {
                self.report("F632", node.loc, "Use == or != to compare literals, not 'is'.".into());
            }
        }
    }

    /// check_annotation_strings_none (F722).
    fn annotation(&mut self, annotation: &Expr) {
        let mut pending = vec![annotation];
        while let Some(node) = pending.pop() {
            if let Some(text) = str_text(node) {
                if !is_valid_expression(&text) {
                    let repr = python_str_repr(&text);
                    self.report("F722", node.loc, format!("Syntax error in forward annotation {repr}."));
                }
                continue;
            }
            if let ExprKind::Subscript { value, slice, .. } = &node.kind {
                let name: &str = match &value.kind {
                    ExprKind::Attribute { attr, .. } => attr,
                    ExprKind::Name { id, .. } => id,
                    _ => "",
                };
                if name == "Literal" {
                    continue;
                }
                if name == "Annotated" {
                    if let ExprKind::Tuple { elts, .. } = &slice.kind {
                        if let Some(first) = elts.first() {
                            pending.push(first);
                        }
                        continue;
                    }
                }
            }
            push_children(node, &mut pending);
        }
    }
}

/// Python's sorted() order for str lists (code point order).
fn python_order(first: &str, second: &str) -> std::cmp::Ordering {
    first.chars().cmp(second.chars())
}

/// is_non_singleton_bool.
fn is_non_singleton(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::List { .. } | ExprKind::Dict { .. } | ExprKind::Set { .. } | ExprKind::ListComp { .. } | ExprKind::DictComp { .. } | ExprKind::SetComp { .. } => true,
        ExprKind::Tuple { elts, .. } => elts.iter().all(is_constant_value),
        ExprKind::Constant { value, .. } => !matches!(value, Constant::None | Constant::True | Constant::False | Constant::Ellipsis),
        _ => false,
    }
}

/// is_constant_value_bool.
fn is_constant_value(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Tuple { elts, .. } => elts.iter().all(is_constant_value),
        ExprKind::Constant { .. } => true,
        _ => false,
    }
}

/// build_key_str: constants by type and value, other keys by structure.
fn key_form(expression: &Expr) -> String {
    match &expression.kind {
        ExprKind::Constant { value, .. } => format!("C{value:?}"),
        _ => strip_locations(&format!("{expression:?}")),
    }
}

/// A Debug rendering without its `loc: Loc { ... }` parts.
fn strip_locations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("loc: Loc {") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let end = after.find('}').map_or(after.len(), |position| position + 1);
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// ast.parse(text.strip(), mode="eval") succeeds.
fn is_valid_expression(text: &str) -> bool {
    let stripped = python_strip(text);
    if stripped.is_empty() {
        return false;
    }
    match refactrail_parser::parse(stripped) {
        Ok(module) => module.body.len() == 1 && matches!(module.body[0].kind, StmtKind::Expr { .. }),
        Err(_) => false,
    }
}

/// Python's repr() of a str (for F722 messages).
fn python_str_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') { '"' } else { '\'' };
    let mut out = String::new();
    out.push(quote);
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if character == quote => {
                out.push('\\');
                out.push(character);
            }
            _ if (character as u32) < 0x20 || character as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", character as u32)),
            _ => out.push(character),
        }
    }
    out.push(quote);
    out
}

/// Push an expression's direct sub-expressions (ast.iter_child_nodes).
fn push_children<'e>(node: &'e Expr, pending: &mut Vec<&'e Expr>) {
    struct Children<'e, 'p> {
        pending: &'p mut Vec<&'e Expr>,
        root: *const Expr,
    }
    impl<'e> Visitor<'e> for Children<'e, '_> {
        fn visit_expr(&mut self, node: &'e Expr) {
            if std::ptr::eq(node, self.root) {
                walk_expr(self, node);
            } else {
                self.pending.push(node);
            }
        }
    }
    Children { pending, root: node }.visit_expr(node);
}

/// The str.format() summary (summarise_format_info).
#[derive(Default)]
struct FormatSummary {
    automatic: usize,
    indices: FastSet<usize>,
    keys: FastSet<String>,
}

impl FormatSummary {
    fn add_field(&mut self, field: &str) {
        let key = field.split('.').next().unwrap_or("").split('[').next().unwrap_or("");
        if key.is_empty() {
            self.automatic += 1;
        } else if key.chars().all(|character| character.is_ascii_digit()) {
            if let Ok(index) = key.parse::<usize>() {
                self.indices.insert(index);
            } else {
                self.keys.insert(key.to_string());
            }
        } else {
            self.keys.insert(key.to_string());
        }
    }
}

/// One replacement field of a format string: (field name, format spec).
type Field = (String, Option<String>);

/// string.Formatter().parse: the fields of a format string, with
/// CPython's error messages (MarkupIterator_next and parse_field).
fn parse_format(text: &str) -> Result<Vec<Field>, &'static str> {
    let chars: Vec<char> = text.chars().collect();
    let mut fields = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let mut character = '\0';
        let mut markup = false;
        while index < chars.len() {
            character = chars[index];
            index += 1;
            if character == '{' || character == '}' {
                markup = true;
                break;
            }
        }
        let at_end = index >= chars.len();
        if character == '}' && (at_end || chars[index] != '}') {
            return Err("Single '}' encountered in format string");
        }
        if at_end && character == '{' {
            return Err("Single '{' encountered in format string");
        }
        if !markup {
            break;
        }
        if chars[index] == character {
            index += 1;
            continue;
        }
        fields.push(parse_field(&chars, &mut index)?);
    }
    Ok(fields)
}

/// parse_field: reads "name!c:spec}" from index, leaving index after it.
fn parse_field(chars: &[char], index: &mut usize) -> Result<Field, &'static str> {
    let name_start = *index;
    let mut character = '\0';
    while *index < chars.len() {
        character = chars[*index];
        *index += 1;
        match character {
            '{' => return Err("unexpected '{' in field name"),
            '[' => {
                while *index < chars.len() && chars[*index] != ']' {
                    *index += 1;
                }
                continue;
            }
            '}' | ':' | '!' => break,
            _ => continue,
        }
    }
    let name: String = chars[name_start..index.saturating_sub(1).max(name_start)].iter().collect();
    if character == '!' || character == ':' {
        if character == '!' {
            if *index >= chars.len() {
                return Err("end of string while looking for conversion specifier");
            }
            *index += 1;
            if *index < chars.len() {
                let next = chars[*index];
                *index += 1;
                if next == '}' {
                    return Ok((name, Some(String::new())));
                }
                if next != ':' {
                    return Err("expected ':' after conversion specifier");
                }
            }
        }
        let spec_start = *index;
        let mut count = 1;
        while *index < chars.len() {
            let current = chars[*index];
            *index += 1;
            if current == '{' {
                count += 1;
            } else if current == '}' {
                count -= 1;
                if count == 0 {
                    return Ok((name, Some(chars[spec_start..*index - 1].iter().collect())));
                }
            }
        }
        return Err("unmatched '{' in format spec");
    }
    if character != '}' {
        return Err("expected '}' before end of string");
    }
    Ok((name, Some(String::new())))
}

/// summarise_format_info.
fn summarise_format(text: &str) -> Result<FormatSummary, &'static str> {
    let mut summary = FormatSummary::default();
    for (name, spec) in parse_format(text)? {
        summary.add_field(&name);
        let Some(spec) = spec.filter(|spec| !spec.is_empty()) else { continue };
        for (inner_name, inner_spec) in parse_format(&spec)? {
            if inner_spec.as_deref().is_some_and(|inner| inner.contains('{')) {
                return Err("Max string recursion exceeded");
            }
            summary.add_field(&inner_name);
        }
    }
    Ok(summary)
}

impl<'t> Visitor<'t> for SyntaxChecks<'_, '_> {
    fn visit_stmt(&mut self, node: &'t Stmt) {
        match &node.kind {
            StmtKind::Assert { test, .. } => {
                if matches!(&test.kind, ExprKind::Tuple { elts, .. } if !elts.is_empty()) {
                    self.report("F631", node.loc, "Assert test is a non-empty tuple, which is always true.".into());
                }
            }
            StmtKind::If { test, .. } => {
                if matches!(&test.kind, ExprKind::Tuple { elts, .. } if !elts.is_empty()) {
                    self.report("F634", test.loc, "If test is a tuple, which is always true.".into());
                }
            }
            StmtKind::Raise { exc: Some(exception), .. } => {
                let target = match &exception.kind {
                    ExprKind::Call { func, .. } => &**func,
                    _ => &**exception,
                };
                if matches!(&target.kind, ExprKind::Name { id, .. } if &**id == "NotImplemented") {
                    self.report("F901", exception.loc, "raise NotImplemented should be raise NotImplementedError.".into());
                }
            }
            StmtKind::FunctionDef { args, returns, .. } | StmtKind::AsyncFunctionDef { args, returns, .. } => {
                for argument in crate::walk::all_args(args) {
                    if let Some(annotation) = &argument.annotation {
                        self.annotation(annotation);
                    }
                }
                if let Some(returns) = returns {
                    self.annotation(returns);
                }
            }
            StmtKind::AnnAssign { annotation, .. } => self.annotation(annotation),
            _ => {}
        }
        walk_stmt(self, node);
    }

    fn visit_expr(&mut self, node: &'t Expr) {
        match &node.kind {
            ExprKind::BinOp { left, op: Operator::Mod, right } => self.percent(node, left, right),
            ExprKind::BinOp { left, op: Operator::RShift, .. } => {
                if matches!(&left.kind, ExprKind::Name { id, .. } if &**id == "print") {
                    self.report("F633", left.loc, "Use of >> is invalid with print function.".into());
                }
            }
            ExprKind::Call { func, args, keywords } => self.format_call(node, func, args, keywords),
            ExprKind::JoinedStr { values } => {
                if !self.spec_ids.contains(&(node as *const Expr as usize)) {
                    self.fstring(node, values);
                }
            }
            ExprKind::Dict { keys, .. } => self.repeated_keys(keys),
            ExprKind::Compare { left, ops, comparators } => self.is_literal(node, left, ops, comparators),
            ExprKind::FormattedValue { format_spec: Some(spec), .. } => {
                self.spec_ids.insert(&**spec as *const Expr as usize);
            }
            _ => {}
        }
        walk_expr(self, node);
    }
}

/// Run the syntax F checks on a module.
pub fn check_module(tree: &Module, source: &SourceFile, tokens: &[Token]) -> Vec<CompatFinding> {
    let mut checks = SyntaxChecks::new(source, tokens);
    for statement in &tree.body {
        checks.visit_stmt(statement);
    }
    checks.out
}
