//! pycodestyle-compatible codes E4 and E7 (mirror of
//! compat_pycodestyle.py). Findings must match the Python engine.

use crate::lexical::LexicalReport;
use crate::source::SourceFile;
use crate::walk::{walk_expr, walk_stmt, CmpOperator, Constant, Expr, ExprContext, ExprKind, Module, Stmt, StmtKind, UnaryOperator, Visitor};
use refactrail_lexer::Kind;
use refactrail_parser::LexedToken as Token;
use refactrail_parser::ast::{Arg, ExceptHandler};
use refactrail_parser::fast_hash::{FastMap, FastSet};
use std::sync::LazyLock;

/// A compatible-code finding: (code, (line, column), message, parent line).
pub type CompatFinding = (&'static str, (usize, usize), String, usize);

const COMPOUND_KEYWORDS: [&str; 12] = ["if", "elif", "else", "for", "while", "with", "try", "except", "finally", "class", "def", "async"];
const AMBIGUOUS_NAMES: [&str; 3] = ["l", "O", "I"];
const AMBIGUOUS_MESSAGE: &str = "Ambiguous variable name; l, O and I look like 1 and 0.";
const SETUP_CALLS: [&str; 17] = [
    "sys.path.append",
    "sys.path.insert",
    "sys.path.extend",
    "sys.path.remove",
    "sys.path.pop",
    "sys.path.clear",
    "sys.path.reverse",
    "sys.path.sort",
    "os.putenv",
    "os.unsetenv",
    "os.environ.update",
    "os.environ.pop",
    "os.environ.clear",
    "os.environ.setdefault",
    "os.environ.popitem",
    "matplotlib.use",
    "pytest.importorskip",
];
static BUILTIN_TYPES: LazyLock<FastSet<&'static str>> =
    LazyLock::new(|| include_str!("../data/builtin_types.txt").lines().filter(|line| !line.is_empty()).collect());

fn is_trivia(token: &Token) -> bool {
    matches!(token.kind, Kind::Comment | Kind::Nl)
}

/// find_next_significant_info.
fn next_significant(tokens: &[Token], index: usize) -> Option<usize> {
    (index + 1..tokens.len()).find(|&position| !is_trivia(&tokens[position]))
}

fn is_line_end(tokens: &[Token], index: Option<usize>) -> bool {
    index.is_none_or(|position| matches!(tokens[position].kind, Kind::Newline | Kind::EndMarker))
}

/// collect_soft_compound_set: (line, 0-based character column) of match
/// statements and case patterns.
fn soft_compound_set(tree: &Module, source: &SourceFile) -> FastSet<(usize, usize)> {
    struct Soft<'s, 'a> {
        source: &'s SourceFile<'a>,
        found: FastSet<(usize, usize)>,
    }
    impl<'t> Visitor<'t> for Soft<'_, '_> {
        fn visit_stmt(&mut self, node: &'t Stmt) {
            if let StmtKind::Match { cases, .. } = &node.kind {
                let (line, column) = self.source.position(node.loc);
                self.found.insert((line, column - 1));
                for case in cases {
                    let (line, column) = self.source.position(case.pattern.loc);
                    self.found.insert((line, column - 1));
                }
            }
            walk_stmt(self, node);
        }
    }
    let mut soft = Soft { source, found: FastSet::default() };
    for statement in &tree.body {
        soft.visit_stmt(statement);
    }
    soft.found
}

/// (line, 0-based character column) of a token's start.
fn token_start(source: &SourceFile, token: &Token) -> (usize, usize) {
    let (line, column) = source.locate(token.start);
    (line, column - 1)
}

fn token_text<'t>(token: &Token, text: &'t str) -> &'t str {
    &text[token.start..token.end]
}

/// is_compound_start_bool.
fn is_compound_start(tokens: &[Token], index: usize, text: &str, source: &SourceFile, soft: &FastSet<(usize, usize)>) -> bool {
    let token = &tokens[index];
    if token.kind != Kind::Name {
        return false;
    }
    let word = token_text(token, text);
    if COMPOUND_KEYWORDS.contains(&word) {
        return true;
    }
    match word {
        "match" => soft.contains(&token_start(source, token)),
        "case" => next_significant(tokens, index).is_some_and(|next| soft.contains(&token_start(source, &tokens[next]))),
        _ => false,
    }
}

/// find_header_keyword_str.
fn header_keyword<'t>(tokens: &[Token], index: usize, text: &'t str, source: &SourceFile, soft: &FastSet<(usize, usize)>) -> &'t str {
    if !is_compound_start(tokens, index, text, source, soft) {
        return "";
    }
    let keyword = token_text(&tokens[index], text);
    if keyword == "async" {
        return next_significant(tokens, index).map_or("", |next| token_text(&tokens[next], text));
    }
    keyword
}

/// E701, E702 and E703 from the token stream (check_statement_tokens_none).
pub fn check_statement_tokens(tokens: &[Token], text: &str, tree: &Module, source: &SourceFile, out: &mut Vec<CompatFinding>) {
    let soft = soft_compound_set(tree, source);
    let (mut depth, mut lambdas, mut string_depth) = (0usize, 0usize, 0usize);
    let mut keyword = "";
    let mut is_start = true;
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            Kind::FStringStart | Kind::TStringStart => {
                string_depth += 1;
            }
            Kind::FStringEnd | Kind::TStringEnd => {
                string_depth -= 1;
                continue;
            }
            _ => {}
        }
        if string_depth > 0 || is_trivia(token) {
            continue;
        }
        if matches!(token.kind, Kind::Newline | Kind::Indent | Kind::Dedent) {
            keyword = "";
            is_start = true;
            continue;
        }
        if is_start {
            keyword = header_keyword(tokens, index, text, source, &soft);
            is_start = false;
            lambdas = 0;
        }
        let word = token_text(token, text);
        if token.kind != Kind::Op {
            if word == "lambda" && depth == 0 {
                lambdas += 1;
            }
            continue;
        }
        match word {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth = depth.saturating_sub(1),
            ";" if depth == 0 => {
                let (line, column) = token_start(source, token);
                if is_line_end(tokens, next_significant(tokens, index)) {
                    out.push(("E703", (line, column + 1), "Statement ends with an unnecessary semicolon.".into(), 0));
                } else {
                    out.push(("E702", (line, column + 1), "Multiple statements on one line (semicolon).".into(), 0));
                }
                is_start = true;
            }
            ":" if depth == 0 && lambdas > 0 => lambdas -= 1,
            ":" if depth == 0 && !keyword.is_empty() => {
                check_compound_colon(tokens, index, text, source, keyword, out);
                keyword = "";
                is_start = true;
            }
            _ => {}
        }
    }
}

/// check_compound_colon_none.
fn check_compound_colon(tokens: &[Token], index: usize, text: &str, source: &SourceFile, keyword: &str, out: &mut Vec<CompatFinding>) {
    let next = next_significant(tokens, index);
    if is_line_end(tokens, next) || keyword == "def" {
        return;
    }
    if keyword == "class" {
        if let Some(next) = next {
            if token_text(&tokens[next], text) == "..." && is_line_end(tokens, next_significant(tokens, next)) {
                return;
            }
        }
    }
    let (line, column) = token_start(source, &tokens[index]);
    out.push(("E701", (line, column + 1), "Multiple statements on one line (colon).".into(), 0));
}

/// is_singleton_bool for None / True / False.
fn is_none(expression: &Expr) -> bool {
    matches!(&expression.kind, ExprKind::Constant { value: Constant::None, .. })
}

fn is_bool_constant(expression: &Expr) -> bool {
    matches!(&expression.kind, ExprKind::Constant { value: Constant::True | Constant::False, .. })
}

/// is_plain_constant_bool: a constant other than None, True and False.
fn is_plain_constant(expression: &Expr) -> bool {
    matches!(&expression.kind, ExprKind::Constant { .. }) && !is_none(expression) && !is_bool_constant(expression)
}

/// The AST checks, with the facts they need.
pub struct NodeChecks<'s, 'a> {
    pub source: &'s SourceFile<'a>,
    /// read_resolutions_dict: (line, column) -> resolution.
    pub resolutions: FastMap<(usize, usize), &'static str>,
    /// bound_names_set, used when there are no resolutions; computed on
    /// the first lookup.
    bound: std::cell::OnceCell<FastSet<&'a str>>,
    tree: &'a Module,
    pub out: Vec<CompatFinding>,
}

impl<'s, 'a> NodeChecks<'s, 'a> {
    pub fn new(source: &'s SourceFile<'a>, lexical: &LexicalReport, tree: &'a Module) -> Self {
        let resolutions = lexical.reads.iter().map(|read| ((read.line, read.column), read.resolution)).collect();
        NodeChecks { source, resolutions, bound: std::cell::OnceCell::new(), tree, out: Vec::new() }
    }

    fn report(&mut self, code: &'static str, position: (usize, usize), message: &str) {
        self.out.push((code, position, message.to_string(), 0));
    }

    /// is_builtin_read_bool.
    fn is_builtin_read(&self, expression: &Expr, name: &str) -> bool {
        if self.resolutions.is_empty() {
            return !self.bound.get_or_init(|| collect_bound_names(self.tree)).contains(name);
        }
        self.resolutions.get(&self.source.position(expression.loc)) == Some(&"builtin_or_implicit")
    }

    /// is_type_expression_bool.
    fn is_type_expression(&self, expression: &Expr) -> bool {
        match &expression.kind {
            ExprKind::Call { func, .. } => matches!(&func.kind, ExprKind::Name { id, .. } if &**id == "type" && self.is_builtin_read(func, id)),
            ExprKind::Name { id, .. } => BUILTIN_TYPES.contains(&**id) && self.is_builtin_read(expression, id),
            _ => false,
        }
    }

    fn check_compare(&mut self, node: &Expr, left: &Expr, ops: &[CmpOperator], comparators: &[Expr]) {
        let operands: Vec<&Expr> = std::iter::once(left).chain(comparators).collect();
        // E711 / E712 (check_literal_comparison_none)
        for (index, operator) in ops.iter().enumerate() {
            if !matches!(operator, CmpOperator::Eq | CmpOperator::NotEq) {
                continue;
            }
            let (left, right) = (operands[index], operands[index + 1]);
            let mut checked: Vec<&Expr> = Vec::new();
            if index == 0 && !is_plain_constant(right) {
                checked.push(left);
            }
            if !is_plain_constant(left) {
                checked.push(right);
            }
            for operand in checked {
                if is_none(operand) {
                    self.report("E711", self.source.position(operand.loc), "Comparison to None should use 'is' or 'is not'.");
                } else if is_bool_constant(operand) {
                    self.report("E712", self.source.position(node.loc), "Avoid equality comparisons to True or False; test the value itself.");
                }
            }
        }
        // E721 (check_type_comparison_none)
        for (index, operator) in ops.iter().enumerate() {
            let (left, right) = (operands[index], operands[index + 1]);
            if matches!(operator, CmpOperator::Eq | CmpOperator::NotEq)
                && (self.is_type_expression(left) || self.is_type_expression(right))
                && !(is_dtype_expression(left) || is_dtype_expression(right))
            {
                self.report("E721", self.source.position(node.loc), "Use 'is' or isinstance() to compare types.");
                return;
            }
        }
    }

    fn identifier(&self, loc: refactrail_parser::Loc, name: &str) -> (usize, usize) {
        identifier_position(self.source, loc, name)
    }
}

/// find_identifier_tuple: the first whole-word occurrence of a name at or
/// after a node's start, within the node's lines.
pub fn identifier_position(source: &SourceFile, loc: refactrail_parser::Loc, name: &str) -> (usize, usize) {
        let (line, column) = source.position(loc);
        let end_line = (loc.end_line as usize).max(line);
        for offset in 0..=(end_line - line) {
            let Some(text) = source.lines.get(line - 1 + offset) else { break };
            let characters: Vec<char> = text.chars().collect();
            let name_characters: Vec<char> = name.chars().collect();
            let start = if offset == 0 { column - 1 } else { 0 };
            let mut found = start;
            while found + name_characters.len() <= characters.len() {
                if characters[found..found + name_characters.len()] == name_characters[..] {
                    let before = if found > 0 { Some(characters[found - 1]) } else { None };
                    let after = characters.get(found + name_characters.len()).copied();
                    let is_word = |character: Option<char>| character.is_some_and(crate::lexical::is_word_char);
                    if !is_word(before) && !is_word(after) {
                        return (line + offset, found + 1);
                    }
                }
                found += 1;
            }
        }
        (line, column)
    }

/// Whether any == or != comparison could be an E721 type comparison
/// (a side is a type() call or a builtin type name). Without one, the
/// name resolutions E721 needs are not computed.
pub fn has_type_comparison_candidate(tree: &Module) -> bool {
    struct Candidates {
        found: bool,
    }
    fn is_candidate(expression: &Expr) -> bool {
        match &expression.kind {
            ExprKind::Call { func, .. } => matches!(&func.kind, ExprKind::Name { id, .. } if &**id == "type"),
            ExprKind::Name { id, .. } => BUILTIN_TYPES.contains(&**id),
            _ => false,
        }
    }
    impl<'t> Visitor<'t> for Candidates {
        fn visit_expr(&mut self, node: &'t Expr) {
            if self.found {
                return;
            }
            if let ExprKind::Compare { left, ops, comparators } = &node.kind {
                if ops.iter().any(|operator| matches!(operator, CmpOperator::Eq | CmpOperator::NotEq))
                    && std::iter::once(&**left).chain(comparators).any(is_candidate)
                {
                    self.found = true;
                    return;
                }
            }
            walk_expr(self, node);
        }
    }
    let mut candidates = Candidates { found: false };
    for statement in &tree.body {
        candidates.visit_stmt(statement);
        if candidates.found {
            break;
        }
    }
    candidates.found
}

/// is_dtype_expression_bool.
fn is_dtype_expression(expression: &Expr) -> bool {
    let target = match &expression.kind {
        ExprKind::Call { func, .. } => func,
        _ => expression,
    };
    matches!(&target.kind, ExprKind::Attribute { attr, .. } if &**attr == "dtype")
}

impl<'t> Visitor<'t> for NodeChecks<'_, '_> {
    fn visit_stmt(&mut self, node: &'t Stmt) {
        match &node.kind {
            StmtKind::Import { names } if names.len() > 1 => {
                self.report("E401", self.source.position(node.loc), "Multiple imports on one line.");
            }
            StmtKind::Assign { targets, value, .. } => {
                if matches!(value.kind, ExprKind::Lambda { .. }) && targets.len() == 1 && matches!(targets[0].kind, ExprKind::Name { .. }) {
                    self.report("E731", self.source.position(node.loc), "Do not assign a lambda expression; use a def.");
                }
            }
            StmtKind::AnnAssign { target, value: Some(value), .. } => {
                if matches!(value.kind, ExprKind::Lambda { .. }) && matches!(target.kind, ExprKind::Name { .. }) {
                    self.report("E731", self.source.position(node.loc), "Do not assign a lambda expression; use a def.");
                }
            }
            StmtKind::Global { names } | StmtKind::Nonlocal { names } => {
                for name in names {
                    if AMBIGUOUS_NAMES.contains(&&**name) {
                        let position = self.identifier(node.loc, name);
                        self.report("E741", position, AMBIGUOUS_MESSAGE);
                    }
                }
            }
            StmtKind::FunctionDef { name, .. } | StmtKind::AsyncFunctionDef { name, .. } | StmtKind::ClassDef { name, .. } => {
                if AMBIGUOUS_NAMES.contains(&&**name) {
                    let code = if matches!(node.kind, StmtKind::ClassDef { .. }) { "E742" } else { "E743" };
                    self.report(code, self.source.definition_name(node.loc, name), AMBIGUOUS_MESSAGE);
                }
            }
            _ => {}
        }
        walk_stmt(self, node);
    }

    fn visit_expr(&mut self, node: &'t Expr) {
        match &node.kind {
            ExprKind::Compare { left, ops, comparators } => self.check_compare(node, left, ops, comparators),
            ExprKind::UnaryOp { op: UnaryOperator::Not, operand } => {
                if let ExprKind::Compare { ops, .. } = &operand.kind {
                    if ops.len() == 1 {
                        match ops[0] {
                            CmpOperator::In => self.report("E713", self.source.position(operand.loc), "Membership tests should use 'not in'."),
                            CmpOperator::Is => self.report("E714", self.source.position(operand.loc), "Identity tests should use 'is not'."),
                            _ => {}
                        }
                    }
                }
            }
            ExprKind::Name { id, ctx: ExprContext::Store } if AMBIGUOUS_NAMES.contains(&&**id) => {
                self.report("E741", self.source.position(node.loc), AMBIGUOUS_MESSAGE);
            }
            _ => {}
        }
        walk_expr(self, node);
    }

    fn visit_arg(&mut self, node: &'t Arg) {
        if AMBIGUOUS_NAMES.contains(&&*node.arg) {
            self.report("E741", self.source.position(node.loc), AMBIGUOUS_MESSAGE);
        }
        refactrail_parser::ast::walk_arg(self, node);
    }

    fn visit_excepthandler(&mut self, node: &'t ExceptHandler) {
        let (type_, name, body) = (&node.type_, &node.name, &node.body);
        if type_.is_none() && !body.iter().any(|statement| matches!(statement.kind, StmtKind::Raise { exc: None, .. })) {
            self.report("E722", self.source.position(node.loc), "Do not use a bare except.");
        }
        if let Some(name) = name {
            if AMBIGUOUS_NAMES.contains(&&**name) {
                let position = self.identifier(node.loc, name);
                self.report("E741", position, AMBIGUOUS_MESSAGE);
            }
        }
        refactrail_parser::ast::walk_excepthandler(self, node);
    }
}

/// collect_bound_names_set: every name bound anywhere in the file.
fn collect_bound_names(tree: &Module) -> FastSet<&str> {
    struct Bound<'a> {
        names: FastSet<&'a str>,
    }
    impl<'a> Visitor<'a> for Bound<'a> {
        fn visit_stmt(&mut self, node: &'a Stmt) {
            match &node.kind {
                StmtKind::FunctionDef { name, .. } | StmtKind::AsyncFunctionDef { name, .. } | StmtKind::ClassDef { name, .. } => {
                    self.names.insert(name);
                }
                StmtKind::Import { names } | StmtKind::ImportFrom { names, .. } => {
                    for alias in names {
                        let bound: &str = match &alias.asname {
                            Some(asname) => asname,
                            None => alias.name.split('.').next().unwrap_or(""),
                        };
                        self.names.insert(bound);
                    }
                }
                _ => {}
            }
            walk_stmt(self, node);
        }
        fn visit_expr(&mut self, node: &'a Expr) {
            if let ExprKind::Name { id, ctx } = &node.kind {
                if *ctx != ExprContext::Load {
                    self.names.insert(id);
                }
            }
            walk_expr(self, node);
        }
        fn visit_arg(&mut self, node: &'a Arg) {
            self.names.insert(&node.arg);
            refactrail_parser::ast::walk_arg(self, node);
        }
    }
    let mut bound = Bound { names: FastSet::default() };
    for statement in &tree.body {
        bound.visit_stmt(statement);
    }
    bound.names
}

/// resolve_dotted_str.
fn resolve_dotted(expression: &Expr, aliases: &FastMap<&str, String>) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let mut current = expression;
    while let ExprKind::Attribute { value, attr, .. } = &current.kind {
        parts.push(attr);
        current = value;
    }
    let ExprKind::Name { id, .. } = &current.kind else { return String::new() };
    let Some(base) = aliases.get(&**id) else { return String::new() };
    let mut dotted = base.clone();
    for part in parts.iter().rev() {
        dotted.push('.');
        dotted.push_str(part);
    }
    dotted
}

/// record_aliases_none.
fn record_aliases<'a>(statement: &'a Stmt, aliases: &mut FastMap<&'a str, String>) {
    match &statement.kind {
        StmtKind::Import { names } => {
            for alias in names {
                match &alias.asname {
                    Some(asname) => {
                        aliases.insert(asname, alias.name.to_string());
                    }
                    None => {
                        let local = alias.name.split('.').next().unwrap_or("");
                        aliases.insert(local, local.to_string());
                    }
                }
            }
        }
        StmtKind::ImportFrom { module: Some(module), names, level: 0 } => {
            for alias in names {
                if &*alias.name != "*" {
                    let local: &str = alias.asname.as_deref().unwrap_or(&alias.name);
                    aliases.insert(local, format!("{module}.{}", alias.name));
                }
            }
        }
        _ => {}
    }
}

fn is_environ_target(target: &Expr, aliases: &FastMap<&str, String>) -> bool {
    matches!(&target.kind, ExprKind::Subscript { value, .. } if resolve_dotted(value, aliases) == "os.environ")
}

/// is_import_preamble_bool.
fn is_import_preamble(statement: &Stmt, aliases: &FastMap<&str, String>) -> bool {
    match &statement.kind {
        StmtKind::If { .. } | StmtKind::Try { .. } | StmtKind::TryStar { .. } | StmtKind::With { .. } | StmtKind::AsyncWith { .. } | StmtKind::Match { .. } => {
            return true;
        }
        _ => {}
    }
    let value = match &statement.kind {
        StmtKind::Expr { value } | StmtKind::Assign { value, .. } => Some(&**value),
        StmtKind::AnnAssign { value, .. } => value.as_deref(),
        _ => None,
    };
    if let Some(Expr { kind: ExprKind::Call { func, .. }, .. }) = value {
        if SETUP_CALLS.contains(&resolve_dotted(func, aliases).as_str()) {
            return true;
        }
    }
    let targets: Vec<&Expr> = match &statement.kind {
        StmtKind::Assign { targets, .. } | StmtKind::Delete { targets } => targets.iter().collect(),
        StmtKind::AugAssign { target, .. } | StmtKind::AnnAssign { target, .. } => vec![&**target],
        _ => Vec::new(),
    };
    if targets.iter().any(|target| is_environ_target(target, aliases)) {
        return true;
    }
    matches!(statement.kind, StmtKind::Assign { .. } | StmtKind::AnnAssign { .. })
        && targets.iter().all(|target| matches!(&target.kind, ExprKind::Name { id, .. } if id.starts_with("__") && id.ends_with("__")))
}

/// E402: module-level imports after other code (check_import_position_none).
pub fn check_import_position(tree: &Module, source: &SourceFile, out: &mut Vec<CompatFinding>) {
    let mut aliases: FastMap<&str, String> = FastMap::default();
    let mut has_boundary = false;
    for (index, statement) in tree.body.iter().enumerate() {
        if matches!(statement.kind, StmtKind::Import { .. } | StmtKind::ImportFrom { .. }) {
            if has_boundary {
                out.push(("E402", source.position(statement.loc), "Module-level import not at the top of the file.".into(), 0));
            }
            record_aliases(statement, &mut aliases);
            continue;
        }
        let is_docstring = index == 0
            && matches!(&statement.kind, StmtKind::Expr { value } if matches!(&value.kind, ExprKind::Constant { value: Constant::Str(_), .. }));
        if !(is_docstring || is_import_preamble(statement, &aliases)) {
            has_boundary = true;
        }
    }
}
