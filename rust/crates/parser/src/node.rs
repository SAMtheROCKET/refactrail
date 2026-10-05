//! Syntax tree nodes shaped like CPython's `ast` module, and a writer that
//! reproduces `ast.dump(tree, include_attributes=...)` byte for byte.

use crate::unicode;

thread_local! {
    /// The Python version whose `repr` the tree dump mirrors (which code
    /// points are printable); set by `dump_module_version`.
    pub(crate) static REPR_VERSION: std::cell::Cell<refactrail_lexer::Version> =
        const { std::cell::Cell::new(refactrail_lexer::Version::Py312) };
}

/// Source span: 1-based lines, UTF-8 byte columns (as CPython's `ast`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Loc {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl Loc {
    pub fn to(self, end: Loc) -> Loc {
        Loc { line: self.line, col: self.col, end_line: end.end_line, end_col: end.end_col }
    }
}

/// A Python constant value as `ast.Constant` holds it.
#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    None,
    True,
    False,
    Ellipsis,
    /// A str; code points, so lone surrogates from escapes survive.
    Str(Vec<u32>),
    Bytes(Vec<u8>),
    /// Decimal digits of an int of any size.
    Int(crate::small_str::SmallStr),
    Float(f64),
    Complex(f64),
}

impl Constant {
    pub fn str_from(text: &str) -> Constant {
        Constant::Str(text.chars().map(|c| c as u32).collect())
    }
}

/// The class of a node (its `ast` class name), as a small enum so that
/// kind checks are integer compares and kind matches jump tables.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Module,
    FunctionDef,
    AsyncFunctionDef,
    ClassDef,
    Return,
    Delete,
    Assign,
    TypeAlias,
    AugAssign,
    AnnAssign,
    For,
    AsyncFor,
    While,
    If,
    With,
    AsyncWith,
    Match,
    Raise,
    Try,
    TryStar,
    Assert,
    Import,
    ImportFrom,
    Global,
    Nonlocal,
    Expr,
    Pass,
    Break,
    Continue,
    BoolOp,
    NamedExpr,
    BinOp,
    UnaryOp,
    Lambda,
    IfExp,
    Dict,
    Set,
    ListComp,
    SetComp,
    DictComp,
    GeneratorExp,
    Await,
    Yield,
    YieldFrom,
    Compare,
    Call,
    FormattedValue,
    JoinedStr,
    Constant,
    Attribute,
    Subscript,
    Starred,
    Name,
    List,
    Tuple,
    Slice,
    comprehension,
    ExceptHandler,
    arguments,
    arg,
    keyword,
    alias,
    withitem,
    match_case,
    MatchValue,
    MatchSingleton,
    MatchSequence,
    MatchMapping,
    MatchClass,
    MatchStar,
    MatchAs,
    MatchOr,
    TypeVar,
    ParamSpec,
    TypeVarTuple,
}

impl NodeKind {
    pub fn name(self) -> &'static str {
        match self {
            NodeKind::Module => "Module",
            NodeKind::FunctionDef => "FunctionDef",
            NodeKind::AsyncFunctionDef => "AsyncFunctionDef",
            NodeKind::ClassDef => "ClassDef",
            NodeKind::Return => "Return",
            NodeKind::Delete => "Delete",
            NodeKind::Assign => "Assign",
            NodeKind::TypeAlias => "TypeAlias",
            NodeKind::AugAssign => "AugAssign",
            NodeKind::AnnAssign => "AnnAssign",
            NodeKind::For => "For",
            NodeKind::AsyncFor => "AsyncFor",
            NodeKind::While => "While",
            NodeKind::If => "If",
            NodeKind::With => "With",
            NodeKind::AsyncWith => "AsyncWith",
            NodeKind::Match => "Match",
            NodeKind::Raise => "Raise",
            NodeKind::Try => "Try",
            NodeKind::TryStar => "TryStar",
            NodeKind::Assert => "Assert",
            NodeKind::Import => "Import",
            NodeKind::ImportFrom => "ImportFrom",
            NodeKind::Global => "Global",
            NodeKind::Nonlocal => "Nonlocal",
            NodeKind::Expr => "Expr",
            NodeKind::Pass => "Pass",
            NodeKind::Break => "Break",
            NodeKind::Continue => "Continue",
            NodeKind::BoolOp => "BoolOp",
            NodeKind::NamedExpr => "NamedExpr",
            NodeKind::BinOp => "BinOp",
            NodeKind::UnaryOp => "UnaryOp",
            NodeKind::Lambda => "Lambda",
            NodeKind::IfExp => "IfExp",
            NodeKind::Dict => "Dict",
            NodeKind::Set => "Set",
            NodeKind::ListComp => "ListComp",
            NodeKind::SetComp => "SetComp",
            NodeKind::DictComp => "DictComp",
            NodeKind::GeneratorExp => "GeneratorExp",
            NodeKind::Await => "Await",
            NodeKind::Yield => "Yield",
            NodeKind::YieldFrom => "YieldFrom",
            NodeKind::Compare => "Compare",
            NodeKind::Call => "Call",
            NodeKind::FormattedValue => "FormattedValue",
            NodeKind::JoinedStr => "JoinedStr",
            NodeKind::Constant => "Constant",
            NodeKind::Attribute => "Attribute",
            NodeKind::Subscript => "Subscript",
            NodeKind::Starred => "Starred",
            NodeKind::Name => "Name",
            NodeKind::List => "List",
            NodeKind::Tuple => "Tuple",
            NodeKind::Slice => "Slice",
            NodeKind::comprehension => "comprehension",
            NodeKind::ExceptHandler => "ExceptHandler",
            NodeKind::arguments => "arguments",
            NodeKind::arg => "arg",
            NodeKind::keyword => "keyword",
            NodeKind::alias => "alias",
            NodeKind::withitem => "withitem",
            NodeKind::match_case => "match_case",
            NodeKind::MatchValue => "MatchValue",
            NodeKind::MatchSingleton => "MatchSingleton",
            NodeKind::MatchSequence => "MatchSequence",
            NodeKind::MatchMapping => "MatchMapping",
            NodeKind::MatchClass => "MatchClass",
            NodeKind::MatchStar => "MatchStar",
            NodeKind::MatchAs => "MatchAs",
            NodeKind::MatchOr => "MatchOr",
            NodeKind::TypeVar => "TypeVar",
            NodeKind::ParamSpec => "ParamSpec",
            NodeKind::TypeVarTuple => "TypeVarTuple",
        }
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

/// A field value of a node.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Node(Box<Node>),
    List(Vec<Value>),
    /// An explicit None inside a list or a required field.
    None,
    Ident(String),
    Int(i64),
    Const(Constant),
    /// A field-less node such as `Load()` or `Add()`, without allocation.
    Tag(&'static str),
}

/// A syntax tree node: class name, fields in `_fields` order (optional
/// fields that are None are left out, as `ast.dump` omits them) and the
/// location attributes when the class has them.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub fields: Vec<(&'static str, Value)>,
    pub loc: Option<Loc>,
}

impl Node {
    pub fn new(kind: NodeKind, loc: Loc) -> Node {
        Node { kind, fields: Vec::with_capacity(field_capacity(kind)), loc: Some(loc) }
    }

    pub fn bare(kind: NodeKind) -> Node {
        Node { kind, fields: Vec::with_capacity(field_capacity(kind)), loc: None }
    }

    pub fn with(mut self, name: &'static str, value: Value) -> Node {
        self.fields.push((name, value));
        self
    }

    /// Add an optional field only when present.
    pub fn maybe(mut self, name: &'static str, value: Option<Value>) -> Node {
        if let Some(value) = value {
            self.fields.push((name, value));
        }
        self
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|(field, _)| *field == name).map(|(_, value)| value)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
        self.fields.iter_mut().find(|(field, _)| *field == name).map(|(_, value)| value)
    }

    pub fn loc(&self) -> Loc {
        self.loc.unwrap_or_default()
    }

    /// The class name of a tag field (`ctx`, `op`).
    pub fn tag(&self, name: &str) -> Option<&'static str> {
        match self.get(name) {
            Some(Value::Tag(tag)) => Some(tag),
            _ => None,
        }
    }

    /// The class names of a list of tags (`Compare.ops`).
    pub fn tags(&self, name: &str) -> Vec<&'static str> {
        match self.get(name) {
            Some(Value::List(items)) => {
                items.iter().filter_map(|item| if let Value::Tag(tag) = item { Some(*tag) } else { None }).collect()
            }
            _ => Vec::new(),
        }
    }
}

/// Most fields a node of this class can have (its `_fields` count), so
/// each node allocates its field storage once and exactly.
fn field_capacity(kind: NodeKind) -> usize {
    match kind {
        NodeKind::Pass | NodeKind::Break | NodeKind::Continue => 0,
        NodeKind::Expr | NodeKind::Return | NodeKind::Delete | NodeKind::Global | NodeKind::Nonlocal | NodeKind::Import | NodeKind::Await | NodeKind::Yield | NodeKind::YieldFrom
        | NodeKind::JoinedStr | NodeKind::Set | NodeKind::MatchValue | NodeKind::MatchSingleton | NodeKind::MatchSequence | NodeKind::MatchStar | NodeKind::MatchOr
        | NodeKind::ParamSpec | NodeKind::TypeVarTuple => 1,
        NodeKind::Name | NodeKind::Constant | NodeKind::Starred | NodeKind::List | NodeKind::Tuple | NodeKind::BoolOp | NodeKind::UnaryOp | NodeKind::Lambda | NodeKind::Dict | NodeKind::ListComp
        | NodeKind::SetComp | NodeKind::GeneratorExp | NodeKind::keyword | NodeKind::alias | NodeKind::withitem | NodeKind::Raise | NodeKind::Assert | NodeKind::NamedExpr
        | NodeKind::MatchAs | NodeKind::TypeVar | NodeKind::Module => 2,
        NodeKind::Attribute | NodeKind::Subscript | NodeKind::BinOp | NodeKind::Call | NodeKind::Compare | NodeKind::IfExp | NodeKind::Slice | NodeKind::Assign | NodeKind::AugAssign
        | NodeKind::If | NodeKind::While | NodeKind::With | NodeKind::AsyncWith | NodeKind::ImportFrom | NodeKind::FormattedValue | NodeKind::DictComp | NodeKind::arg
        | NodeKind::ExceptHandler | NodeKind::match_case | NodeKind::MatchMapping | NodeKind::TypeAlias | NodeKind::Match => 3,
        NodeKind::AnnAssign | NodeKind::For | NodeKind::AsyncFor | NodeKind::Try | NodeKind::TryStar | NodeKind::comprehension | NodeKind::MatchClass => 4,
        NodeKind::ClassDef | NodeKind::FunctionDef | NodeKind::AsyncFunctionDef | NodeKind::arguments => 7,
    }
}

impl From<Node> for Value {
    fn from(node: Node) -> Value {
        Value::Node(Box::new(node))
    }
}

/// `ast.dump(node, include_attributes=attributes)`.
pub fn dump(node: &Node, attributes: bool) -> String {
    let mut out = String::with_capacity(4096);
    write_node(&mut out, node, attributes);
    out
}

fn write_node(out: &mut String, node: &Node, attributes: bool) {
    out.push_str(node.kind.name());
    out.push('(');
    let mut first = true;
    for (name, value) in &node.fields {
        if !first {
            out.push_str(", ");
        }
        first = false;
        out.push_str(name);
        out.push('=');
        write_value(out, value, attributes);
    }
    if attributes {
        if let Some(loc) = node.loc {
            for (name, number) in [
                ("lineno", loc.line),
                ("col_offset", loc.col),
                ("end_lineno", loc.end_line),
                ("end_col_offset", loc.end_col),
            ] {
                if !first {
                    out.push_str(", ");
                }
                first = false;
                out.push_str(name);
                out.push('=');
                out.push_str(&number.to_string());
            }
        }
    }
    out.push(')');
}

fn write_value(out: &mut String, value: &Value, attributes: bool) {
    match value {
        Value::Node(node) => write_node(out, node, attributes),
        Value::List(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_value(out, item, attributes);
            }
            out.push(']');
        }
        Value::None => out.push_str("None"),
        Value::Ident(name) => write_str_repr(out, &name.chars().map(|c| c as u32).collect::<Vec<_>>()),
        Value::Int(number) => out.push_str(&number.to_string()),
        Value::Const(constant) => write_constant(out, constant),
        Value::Tag(tag) => {
            out.push_str(tag);
            out.push_str("()");
        }
    }
}

/// Python's `repr()` of an identifier or string field.
pub fn write_ident(out: &mut String, name: &str) {
    if name.bytes().all(|byte| byte.is_ascii_graphic() && byte != b'\\' && byte != b'\'') {
        out.push('\'');
        out.push_str(name);
        out.push('\'');
        return;
    }
    write_str_repr(out, &name.chars().map(|c| c as u32).collect::<Vec<_>>());
}

/// Python's `repr()` of a constant.
pub fn write_constant(out: &mut String, constant: &Constant) {
    match constant {
        Constant::None => out.push_str("None"),
        Constant::True => out.push_str("True"),
        Constant::False => out.push_str("False"),
        Constant::Ellipsis => out.push_str("Ellipsis"),
        Constant::Str(points) => write_str_repr(out, points),
        Constant::Bytes(bytes) => write_bytes_repr(out, bytes),
        Constant::Int(digits) => out.push_str(digits),
        Constant::Float(value) => out.push_str(&float_repr(*value, true)),
        Constant::Complex(value) => {
            out.push_str(&float_repr(*value, false));
            out.push('j');
        }
    }
}

fn write_str_repr(out: &mut String, points: &[u32]) {
    let has_single = points.contains(&('\'' as u32));
    let has_double = points.contains(&('"' as u32));
    let quote = if has_single && !has_double { '"' } else { '\'' };
    out.push(quote);
    for &point in points {
        match point {
            0x5c => out.push_str("\\\\"),
            0x09 => out.push_str("\\t"),
            0x0a => out.push_str("\\n"),
            0x0d => out.push_str("\\r"),
            _ if point == quote as u32 => {
                out.push('\\');
                out.push(quote);
            }
            _ if point < 0x20 || point == 0x7f => out.push_str(&format!("\\x{point:02x}")),
            _ if point < 0x7f => out.push(char::from_u32(point).unwrap_or('?')),
            _ if !unicode::is_printable_version(point, REPR_VERSION.with(|version| version.get())) => {
                if point <= 0xff {
                    out.push_str(&format!("\\x{point:02x}"));
                } else if point <= 0xffff {
                    out.push_str(&format!("\\u{point:04x}"));
                } else {
                    out.push_str(&format!("\\U{point:08x}"));
                }
            }
            _ => out.push(char::from_u32(point).unwrap_or('?')),
        }
    }
    out.push(quote);
}

fn write_bytes_repr(out: &mut String, bytes: &[u8]) {
    let quote = if bytes.contains(&b'\'') && !bytes.contains(&b'"') { b'"' } else { b'\'' };
    out.push('b');
    out.push(quote as char);
    for &byte in bytes {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'\t' => out.push_str("\\t"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            _ if byte == quote => {
                out.push('\\');
                out.push(quote as char);
            }
            0x20..=0x7e => out.push(byte as char),
            _ => out.push_str(&format!("\\x{byte:02x}")),
        }
    }
    out.push(quote as char);
}

/// Exact decimal digits of a positive finite float, and the exponent of
/// the first digit (value = d.ddd * 10^exponent).
fn exact_decimal(value: f64) -> (String, i32) {
    let bits = value.to_bits();
    let raw_exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let (mantissa, mut power) = if raw_exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1u64 << 52), raw_exponent - 1075)
    };
    // Little-endian base 10^9 limbs.
    let mut limbs: Vec<u64> = vec![mantissa % 1_000_000_000, mantissa / 1_000_000_000 % 1_000_000_000, mantissa / 1_000_000_000_000_000_000];
    let multiply = |limbs: &mut Vec<u64>, factor: u64| {
        let mut carry = 0u64;
        for limb in limbs.iter_mut() {
            let product = *limb * factor + carry;
            *limb = product % 1_000_000_000;
            carry = product / 1_000_000_000;
        }
        while carry > 0 {
            limbs.push(carry % 1_000_000_000);
            carry /= 1_000_000_000;
        }
    };
    let mut decimal_shift = 0i32;
    if power >= 0 {
        while power > 0 {
            let step = power.min(29);
            multiply(&mut limbs, 1u64 << step);
            power -= step;
        }
    } else {
        decimal_shift = -power;
        let mut remaining = -power;
        while remaining > 0 {
            let step = remaining.min(12);
            multiply(&mut limbs, 5u64.pow(step as u32));
            remaining -= step;
        }
    }
    while limbs.len() > 1 && limbs.last() == Some(&0) {
        limbs.pop();
    }
    let mut text = limbs.last().copied().unwrap_or(0).to_string();
    for limb in limbs.iter().rev().skip(1) {
        text.push_str(&format!("{limb:09}"));
    }
    let exponent = text.len() as i32 - 1 - decimal_shift;
    let trimmed = text.trim_end_matches('0').to_string();
    (if trimmed.is_empty() { "0".into() } else { trimmed }, exponent)
}

/// Python's dtoa breaks exact ties between shortest candidates toward an
/// even last digit; Rust's shortest formatting may round the other way.
fn half_even_digits(value: f64, shortest: String, exponent: i32) -> (String, i32) {
    let (exact, exact_exponent) = exact_decimal(value);
    let count = shortest.len();
    if exact.len() <= count || exact_exponent != exponent {
        return (shortest, exponent);
    }
    let mut kept: Vec<u8> = exact.as_bytes()[..count].to_vec();
    let rest = &exact.as_bytes()[count..];
    let round_up = match rest[0] {
        b'6'..=b'9' => true,
        b'5' => rest[1..].iter().any(|&b| b != b'0') || (kept[count - 1] - b'0') % 2 == 1,
        _ => false,
    };
    let mut new_exponent = exponent;
    if round_up {
        let mut index = count;
        loop {
            if index == 0 {
                kept.insert(0, b'1');
                kept.pop();
                new_exponent += 1;
                break;
            }
            index -= 1;
            if kept[index] == b'9' {
                kept[index] = b'0';
            } else {
                kept[index] += 1;
                break;
            }
        }
    }
    let mut candidate = String::from_utf8(kept).unwrap_or_default();
    while candidate.len() > 1 && candidate.ends_with('0') {
        candidate.pop();
    }
    let check = format!("{}.{}e{}", &candidate[..1], &candidate[1..], new_exponent);
    if check.trim_end_matches(".e").parse::<f64>().ok() == Some(value)
        || format!("{}e{}", candidate, new_exponent - candidate.len() as i32 + 1).parse::<f64>().ok() == Some(value)
    {
        (candidate, new_exponent)
    } else {
        (shortest, exponent)
    }
}

/// Python's float `repr()` ('r' format); `add_dot_zero` is false for the
/// parts of a complex number.
pub fn float_repr(value: f64, add_dot_zero: bool) -> String {
    if value.is_infinite() {
        return if value > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if value.is_nan() {
        return "nan".into();
    }
    // Rust's `{:e}` gives the shortest round-trip digits.
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
    let digits = if digits.chars().all(|c| c == '0') { "0".to_string() } else { digits };
    let (digits, exponent) = if digits == "0" { (digits, exponent) } else { half_even_digits(value.abs(), digits, exponent) };
    let decimal_point = exponent + 1;
    let mut text = String::new();
    if value.is_sign_negative() {
        text.push('-');
    }
    if digits == "0" {
        text.push('0');
        if add_dot_zero {
            text.push_str(".0");
        }
        return text;
    }
    if decimal_point > 16 || decimal_point < -3 {
        text.push_str(&digits[..1]);
        if digits.len() > 1 {
            text.push('.');
            text.push_str(&digits[1..]);
        }
        let sign = if exponent < 0 { '-' } else { '+' };
        text.push_str(&format!("e{sign}{:02}", exponent.abs()));
    } else if decimal_point <= 0 {
        text.push_str("0.");
        text.push_str(&"0".repeat((-decimal_point) as usize));
        text.push_str(&digits);
    } else {
        let point = decimal_point as usize;
        if digits.len() <= point {
            text.push_str(&digits);
            text.push_str(&"0".repeat(point - digits.len()));
            if add_dot_zero {
                text.push_str(".0");
            }
        } else {
            text.push_str(&digits[..point]);
            text.push('.');
            text.push_str(&digits[point..]);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_repr_matches_python() {
        for (value, expected) in [
            (2001599834386887.25, "2001599834386887.2"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (0.1, "0.1"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (123.456, "123.456"),
            (0.0, "0.0"),
        ] {
            assert_eq!(float_repr(value, true), expected);
        }
        assert_eq!(float_repr(2.0, false), "2");
    }

    #[test]
    fn str_repr_quotes_and_escapes() {
        let mut out = String::new();
        write_constant(&mut out, &Constant::str_from("it's \u{7f}\u{200b}é\t"));
        assert_eq!(out, r#""it's \x7f\u200bé\t""#);
    }
}
