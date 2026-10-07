"""Generate RefacTrail's typed Python syntax tree from CPython's ASDL.

Usage: python scripts/gen_ast.py rust/crates/parser/src/ast.rs

The ASDL below is CPython 3.14's Parser/Python.asdl (module part only), a
superset of 3.12 and 3.13: optional fields that are None are omitted by
ast.dump in every version, so one writer serves all three.
The output defines one Rust type per node class, an exact
`ast.dump(..., include_attributes=...)` writer and a Visitor whose default
walks visit children in CPython's field order. Do not edit the generated
file by hand; change this script and regenerate.
"""

import re
import sys
from pathlib import Path

ASDL = """
stmt = FunctionDef(identifier name, arguments args, stmt* body, expr* decorator_list, expr? returns, string? type_comment, type_param* type_params)
     | AsyncFunctionDef(identifier name, arguments args, stmt* body, expr* decorator_list, expr? returns, string? type_comment, type_param* type_params)
     | ClassDef(identifier name, expr* bases, keyword* keywords, stmt* body, expr* decorator_list, type_param* type_params)
     | Return(expr? value)
     | Delete(expr* targets)
     | Assign(expr* targets, expr value, string? type_comment)
     | TypeAlias(expr name, type_param* type_params, expr value)
     | AugAssign(expr target, operator op, expr value)
     | AnnAssign(expr target, expr annotation, expr? value, int simple)
     | For(expr target, expr iter, stmt* body, stmt* orelse, string? type_comment)
     | AsyncFor(expr target, expr iter, stmt* body, stmt* orelse, string? type_comment)
     | While(expr test, stmt* body, stmt* orelse)
     | If(expr test, stmt* body, stmt* orelse)
     | With(withitem* items, stmt* body, string? type_comment)
     | AsyncWith(withitem* items, stmt* body, string? type_comment)
     | Match(expr subject, match_case* cases)
     | Raise(expr? exc, expr? cause)
     | Try(stmt* body, excepthandler* handlers, stmt* orelse, stmt* finalbody)
     | TryStar(stmt* body, excepthandler* handlers, stmt* orelse, stmt* finalbody)
     | Assert(expr test, expr? msg)
     | Import(alias* names)
     | ImportFrom(identifier? module, alias* names, int level)
     | Global(identifier* names)
     | Nonlocal(identifier* names)
     | Expr(expr value)
     | Pass
     | Break
     | Continue
     attributes
expr = BoolOp(boolop op, expr* values)
     | NamedExpr(expr target, expr value)
     | BinOp(expr left, operator op, expr right)
     | UnaryOp(unaryop op, expr operand)
     | Lambda(arguments args, expr body)
     | IfExp(expr test, expr body, expr orelse)
     | Dict(expr?* keys, expr* values)
     | Set(expr* elts)
     | ListComp(expr elt, comprehension* generators)
     | SetComp(expr elt, comprehension* generators)
     | DictComp(expr key, expr value, comprehension* generators)
     | GeneratorExp(expr elt, comprehension* generators)
     | Await(expr value)
     | Yield(expr? value)
     | YieldFrom(expr value)
     | Compare(expr left, cmpop* ops, expr* comparators)
     | Call(expr func, expr* args, keyword* keywords)
     | FormattedValue(expr value, int conversion, expr? format_spec)
     | Interpolation(expr value, constant str, int conversion, expr? format_spec)
     | JoinedStr(expr* values)
     | TemplateStr(expr* values)
     | Constant(constant value, string? kind)
     | Attribute(expr value, identifier attr, expr_context ctx)
     | Subscript(expr value, expr slice, expr_context ctx)
     | Starred(expr value, expr_context ctx)
     | Name(identifier id, expr_context ctx)
     | List(expr* elts, expr_context ctx)
     | Tuple(expr* elts, expr_context ctx)
     | Slice(expr? lower, expr? upper, expr? step)
     attributes
pattern = MatchValue(expr value)
        | MatchSingleton(constant value)
        | MatchSequence(pattern* patterns)
        | MatchMapping(expr* keys, pattern* patterns, identifier? rest)
        | MatchClass(expr cls, pattern* patterns, identifier* kwd_attrs, pattern* kwd_patterns)
        | MatchStar(identifier? name)
        | MatchAs(pattern? pattern, identifier? name)
        | MatchOr(pattern* patterns)
        attributes
type_param = TypeVar(identifier name, expr? bound, expr? default_value)
           | ParamSpec(identifier name, expr? default_value)
           | TypeVarTuple(identifier name, expr? default_value)
           attributes
excepthandler = (expr? type, identifier? name, stmt* body) attributes ExceptHandler
arguments = (arg* posonlyargs, arg* args, arg? vararg, arg* kwonlyargs, expr?* kw_defaults, arg? kwarg, expr* defaults)
arg = (identifier arg, expr? annotation, string? type_comment) attributes
keyword = (identifier? arg, expr value) attributes
alias = (identifier name, identifier? asname) attributes
withitem = (expr context_expr, expr? optional_vars)
match_case = (pattern pattern, expr? guard, stmt* body)
comprehension = (expr target, expr iter, expr* ifs, int is_async)
"""

ENUMS = {
    "expr_context": ("ExprContext", ["Load", "Store", "Del"]),
    "boolop": ("BoolOperator", ["And", "Or"]),
    "operator": ("Operator", ["Add", "Sub", "Mult", "MatMult", "Div", "Mod", "Pow", "LShift", "RShift",
                              "BitOr", "BitXor", "BitAnd", "FloorDiv"]),
    "unaryop": ("UnaryOperator", ["Invert", "Not", "UAdd", "USub"]),
    "cmpop": ("CmpOperator", ["Eq", "NotEq", "Lt", "LtE", "Gt", "GtE", "Is", "IsNot", "In", "NotIn"]),
}
SUMS = {"stmt": "Stmt", "expr": "Expr", "pattern": "Pattern", "type_param": "TypeParam"}
PRODUCTS = {"excepthandler": "ExceptHandler", "arguments": "Arguments", "arg": "Arg", "keyword": "Keyword",
            "alias": "Alias", "withitem": "WithItem", "match_case": "MatchCase", "comprehension": "Comprehension"}
RUST_KEYWORDS = {"type": "type_", "str": "str_"}


def parse_asdl():
    sums, products = {}, {}
    text = re.sub(r"\n\s*\|", " |", ASDL.strip())
    text = re.sub(r"\n\s+attributes", " attributes", text)
    for line in text.splitlines():
        name, body = [part.strip() for part in line.split("=", 1)]
        if body.startswith("("):
            fields_text, rest = body[1:].split(")", 1)
            products[name] = (parse_fields(fields_text), "attributes" in rest, rest.replace("attributes", "").strip())
        else:
            attributes = body.endswith("attributes")
            body = body.removesuffix("attributes").strip()
            constructors = []
            for part in body.split("|"):
                part = part.strip()
                match = re.match(r"(\w+)(?:\((.*)\))?$", part)
                constructors.append((match.group(1), parse_fields(match.group(2) or "")))
            sums[name] = (constructors, attributes)
    return sums, products


def parse_fields(text):
    fields = []
    for item in [item.strip() for item in text.split(",") if item.strip()]:
        kind, name = item.split()
        fields.append((kind, name))
    return fields


def rust_type(kind):
    base = kind.rstrip("*?")
    list_of_optional = kind.endswith("?*")
    many, optional = kind.endswith("*"), kind.endswith("?") and not list_of_optional
    if base == "identifier":
        inner = "Id"
    elif base == "string":
        inner = "Box<str>"
    elif base == "int":
        inner = "i64"
    elif base == "constant":
        inner = "Constant"
    elif base in ENUMS:
        inner = ENUMS[base][0]
    elif base in SUMS:
        inner = SUMS[base]
    else:
        inner = PRODUCTS[base]
    boxed = base in SUMS or base == "arguments"
    if list_of_optional:
        return f"Vec<Option<{inner}>>"
    if many:
        return f"Vec<{inner}>"
    if optional:
        return f"Option<Box<{inner}>>" if boxed else f"Option<{inner}>"
    return f"Box<{inner}>" if boxed else inner


def field_name(name):
    return RUST_KEYWORDS.get(name, name)


def dump_value(kind, access):
    """Rust statements writing one field; `access` evaluates to a reference."""
    base = kind.rstrip("*?")
    if kind.endswith("?*"):
        return (f"out.push('['); for (index, item) in ({access}).iter().enumerate() {{ if index > 0 {{ out.push_str(\", \"); }} "
                f"match item {{ Some(value) => {dump_one(base, 'value')}, None => out.push_str(\"None\") }} }} out.push(']');")
    if kind.endswith("*"):
        return (f"out.push('['); for (index, item) in ({access}).iter().enumerate() {{ if index > 0 {{ out.push_str(\", \"); }} "
                f"{dump_one(base, 'item')}; }} out.push(']');")
    return dump_one(base, access) + ";"


def dump_one(base, access):
    if base in ("identifier", "string"):
        return f"write_ident(out, {access})"
    if base == "int":
        return f"out.push_str(&({access}).to_string())"
    if base == "constant":
        return f"write_constant(out, {access})"
    if base in ENUMS:
        return f"{{ out.push_str({access}.name()); out.push_str(\"()\") }}"
    return f"write_{base}(out, {access}, attributes)"


def dump_fields(fields, prefix):
    lines = []
    for kind, name in fields:
        access = f"{prefix}{field_name(name)}"
        if kind.endswith("?") and not kind.endswith("?*"):
            lines.append(f"        if let Some(value) = {access} {{ sep(out, &mut first); out.push_str(\"{name}=\"); "
                         + dump_value(kind.rstrip("?"), "value") + " }")
        else:
            lines.append(f"        sep(out, &mut first); out.push_str(\"{name}=\"); " + dump_value(kind, access))
    return "\n".join(lines)


def walk_fields(fields, prefix):
    lines = []
    for kind, name in fields:
        base = kind.rstrip("*?")
        method = {"stmt": "visit_stmt", "expr": "visit_expr", "pattern": "visit_pattern", "type_param": "visit_type_param",
                  "excepthandler": "visit_excepthandler", "arguments": "visit_arguments", "arg": "visit_arg",
                  "keyword": "visit_keyword", "alias": "visit_alias", "withitem": "visit_withitem",
                  "match_case": "visit_match_case", "comprehension": "visit_comprehension"}.get(base)
        if method is None:
            continue
        access = f"{prefix}{field_name(name)}"
        if kind.endswith("?*"):
            lines.append(f"        for item in {access}.iter().flatten() {{ visitor.{method}(item); }}")
        elif kind.endswith("*"):
            lines.append(f"        for item in &{access} {{ visitor.{method}(item); }}")
        elif kind.endswith("?"):
            lines.append(f"        if let Some(item) = &{access} {{ visitor.{method}(item); }}")
        else:
            lines.append(f"        visitor.{method}(&{access});")
    return "\n".join(lines)


def generate():
    sums, products = parse_asdl()
    out = ["// @generated by scripts/gen_ast.py from CPython 3.14's ASDL; do not edit.",
           "//! RefacTrail's typed Python syntax tree (CPython `ast` shape), its",
           "//! exact `ast.dump` writer and a visitor walking children in field order.",
           "",
           "#![allow(clippy::all)]",
           "",
           "use crate::node::{write_constant, write_ident, Constant, Loc};",
           "",
           "/// An identifier as CPython keeps it (inline when short).",
           "pub type Id = crate::small_str::SmallStr;",
           ""]
    for _, (rust, names) in ENUMS.items():
        out.append("#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]")
        out.append(f"pub enum {rust} {{ " + ", ".join(names) + " }")
        out.append(f"impl {rust} {{")
        out.append("    pub fn name(self) -> &'static str {")
        out.append("        match self { " + " ".join(f"{rust}::{n} => \"{n}\"," for n in names) + " }")
        out.append("    }")
        out.append("}")
        out.append("")
    for asdl_name, (constructors, attributes) in sums.items():
        rust = SUMS[asdl_name]
        out.append("#[derive(Clone, Debug, PartialEq)]")
        out.append(f"pub struct {rust} {{ pub kind: {rust}Kind, pub loc: Loc }}")
        out.append("")
        out.append("#[derive(Clone, Debug, PartialEq)]")
        out.append(f"pub enum {rust}Kind {{")
        for name, fields in constructors:
            if fields:
                inner = ", ".join(f"{field_name(fname)}: {rust_type(kind)}" for kind, fname in fields)
                out.append(f"    {name} {{ {inner} }},")
            else:
                out.append(f"    {name},")
        out.append("}")
        out.append(f"impl {rust}Kind {{")
        out.append("    /// The CPython class name.")
        out.append("    pub fn name(&self) -> &'static str {")
        out.append("        match self { " + " ".join(
            f"{rust}Kind::{name}{' { .. }' if fields else ''} => \"{name}\"," for name, fields in constructors) + " }")
        out.append("    }")
        out.append("}")
        out.append("")
    for asdl_name, (fields, attributes, _) in products.items():
        rust = PRODUCTS[asdl_name]
        members = [f"pub {field_name(fname)}: {rust_type(kind)}" for kind, fname in fields]
        if attributes:
            members.append("pub loc: Loc")
        out.append("#[derive(Clone, Debug, PartialEq)]")
        out.append(f"pub struct {rust} {{ " + ", ".join(members) + " }")
        out.append("")
    out.append("#[derive(Clone, Debug, PartialEq)]")
    out.append("pub struct Module { pub body: Vec<Stmt> }")
    out.append("")
    # ---- dump
    out.append("""fn sep(out: &mut String, first: &mut bool) {
    if !*first {
        out.push_str(", ");
    }
    *first = false;
}

fn write_loc(out: &mut String, first: &mut bool, loc: &Loc) {
    for (name, number) in [("lineno", loc.line), ("col_offset", loc.col), ("end_lineno", loc.end_line), ("end_col_offset", loc.end_col)] {
        sep(out, first);
        out.push_str(name);
        out.push('=');
        out.push_str(&number.to_string());
    }
}

/// `ast.dump(module, include_attributes=attributes)`.
pub fn dump_module(module: &Module, attributes: bool) -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("Module(body=[");
    for (index, item) in module.body.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        write_stmt(&mut out, item, attributes);
    }
    out.push_str("], type_ignores=[])");
    out
}
""")
    for asdl_name, (constructors, attributes) in sums.items():
        rust = SUMS[asdl_name]
        out.append(f"pub fn write_{asdl_name}(out: &mut String, node: &{rust}, attributes: bool) {{")
        out.append("    let mut first = true;")
        out.append("    match &node.kind {")
        for name, fields in constructors:
            binds = ", ".join(field_name(fname) for _, fname in fields)
            pattern = f"{rust}Kind::{name} {{ {binds} }}" if fields else f"{rust}Kind::{name}"
            out.append(f"        {pattern} => {{")
            out.append(f"        out.push_str(\"{name}(\");")
            out.append(dump_fields(fields, ""))
            out.append("        }")
        out.append("    }")
        out.append("    if attributes { write_loc(out, &mut first, &node.loc); }")
        out.append("    out.push(')');")
        out.append("}")
        out.append("")
    for asdl_name, (fields, attributes, class_name) in products.items():
        rust = PRODUCTS[asdl_name]
        out.append(f"pub fn write_{asdl_name}(out: &mut String, node: &{rust}, attributes: bool) {{")
        out.append("    let mut first = true;")
        out.append(f"    out.push_str(\"{class_name or asdl_name}(\");")
        out.append(dump_fields(fields, "&node."))
        if attributes:
            out.append("    if attributes { write_loc(out, &mut first, &node.loc); }")
        else:
            out.append("    let _ = attributes;")
        out.append("    out.push(')');")
        out.append("}")
        out.append("")
    # ---- visitor
    kinds = [("stmt", "Stmt"), ("expr", "Expr"), ("pattern", "Pattern"), ("type_param", "TypeParam"),
             ("excepthandler", "ExceptHandler"), ("arguments", "Arguments"), ("arg", "Arg"), ("keyword", "Keyword"),
             ("alias", "Alias"), ("withitem", "WithItem"), ("match_case", "MatchCase"), ("comprehension", "Comprehension")]
    out.append("/// A tree visitor; each default visits the node's children in field")
    out.append("/// order (CPython's `ast.iter_child_nodes`).")
    out.append("pub trait Visitor<'a>: Sized {")
    for asdl_name, rust in kinds:
        out.append(f"    fn visit_{asdl_name}(&mut self, node: &'a {rust}) {{ walk_{asdl_name}(self, node) }}")
    out.append("}")
    out.append("")
    for asdl_name, (constructors, _) in sums.items():
        rust = SUMS[asdl_name]
        out.append(f"pub fn walk_{asdl_name}<'a, V: Visitor<'a>>(visitor: &mut V, node: &'a {rust}) {{")
        out.append("    match &node.kind {")
        for name, fields in constructors:
            used = [fname for kind, fname in fields if kind.rstrip('*?') not in ("identifier", "string", "int", "constant") and kind.rstrip('*?') not in ENUMS]
            binds = ", ".join(field_name(f) for f in used)
            rest = ", .." if len(used) < len(fields) else ""
            if not fields:
                pattern = f"{rust}Kind::{name}"
            else:
                pattern = f"{rust}Kind::{name} {{ {binds}{rest} }}" if used else f"{rust}Kind::{name} {{ .. }}"
            body = walk_fields([(k, f) for k, f in fields if f in used], "").replace("&&", "&")
            body = re.sub(r"for item in &(\w+) ", r"for item in \1.iter() ", body)
            body = re.sub(r"if let Some\(item\) = &(\w+) ", r"if let Some(item) = \1 ", body)
            body = re.sub(r"visitor\.(\w+)\(&(\w+)\);", r"visitor.\1(\2);", body)
            out.append(f"        {pattern} => {{")
            out.append(body)
            out.append("        }")
        out.append("    }")
        out.append("}")
        out.append("")
    for asdl_name, (fields, _, _) in products.items():
        rust = PRODUCTS[asdl_name]
        body = walk_fields(fields, "node.")
        out.append(f"pub fn walk_{asdl_name}<'a, V: Visitor<'a>>(visitor: &mut V, node: &'a {rust}) {{")
        out.append(body if body else "    let _ = (visitor, node);")
        out.append("}")
        out.append("")
    out.extend(generate_mut_visitor(kinds, "\n".join(out)))
    return "\n".join(out) + "\n"


def generate_mut_visitor(kinds, text):
    """The Visitor's mutable twin, derived from its generated walks."""
    out = ["/// The mutable twin of `Visitor`: each default visits the node's",
           "/// children in field order, so a rewrite can change any node.",
           "pub trait VisitorMut: Sized {"]
    for asdl_name, rust in kinds:
        out.append(f"    fn visit_{asdl_name}_mut(&mut self, node: &mut {rust}) "
                   f"{{ walk_{asdl_name}_mut(self, node) }}")
    out.append("}")
    out.append("")
    walks = text.split("pub trait Visitor<'a>: Sized {", 1)[1]
    for block in re.findall(r"^pub fn walk_.*?^}$", walks, re.S | re.M):
        block = re.sub(r"pub fn walk_(\w+)<'a, V: Visitor<'a>>\(visitor: &mut V, node: &'a (\w+)\)",
                       r"pub fn walk_\1_mut<V: VisitorMut>(visitor: &mut V, node: &mut \2)", block)
        block = block.replace("match &node.kind", "match &mut node.kind")
        block = re.sub(r"visitor\.visit_(\w+)\(", r"visitor.visit_\1_mut(", block)
        block = block.replace(".iter().flatten()", ".iter_mut().flatten()")
        block = block.replace(".iter()", ".iter_mut()")
        block = re.sub(r"for item in &(node\.\w+) ", r"for item in \1.iter_mut() ", block)
        block = re.sub(r"if let Some\(item\) = &(node\.\w+) ", r"if let Some(item) = &mut \1 ", block)
        block = re.sub(r"visitor\.(visit_\w+_mut)\(&(node\.\w+)\);", r"visitor.\1(&mut \2);", block)
        out.append(block)
        out.append("")
    return out


Path(sys.argv[1]).write_text(generate(), encoding="utf-8")
print("written", sys.argv[1])
