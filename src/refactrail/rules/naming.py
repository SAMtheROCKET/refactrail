"""Naming rules: meaningful names, verbs, dtype suffixes, conventions."""

import ast
import re

from refactrail.facts import (
    UPPER_CASE_PATTERN, is_dunder_bool, read_decorator_names_list,
)
from refactrail.rules.context import (
    GENERIC_NAMES_SET, VERBS_SET, RuleContext, is_protocol_hook_bool,
)
from refactrail.scopes import Binding
from refactrail.source import locate_definition_name_tuple

SNAKE_CASE_PATTERN = re.compile(r"^_{0,2}[a-z][a-z0-9_]*_{0,2}$")
PASCAL_CASE_PATTERN = re.compile(r"^_?[A-Z][A-Za-z0-9]*$")
EXEMPT_VERB_DECORATORS_TUPLE = ("property", "cached_property",
                                "functools.cached_property", "overload",
                                "typing.overload")
ACCESSOR_SUFFIXES_TUPLE = (".setter", ".getter", ".deleter")
BUILTIN_RESULTS_DICT = {
    "int": "int", "len": "int", "float": "float", "str": "str",
    "bool": "bool", "list": "list", "sorted": "list", "dict": "dict",
    "set": "set", "tuple": "tuple", "bytes": "bytes",
}
DISPLAY_TYPES_TUPLE = (
    ((ast.List, ast.ListComp), "list"), ((ast.Dict, ast.DictComp), "dict"),
    ((ast.Set, ast.SetComp), "set"), ((ast.Tuple,), "tuple"),
    ((ast.JoinedStr,), "str"),
)


def check_short_names_none(context: RuleContext) -> None:
    """Report single-character names (RT201).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds findings to the context.
    Warnings:
        "_" and imports of single-character names are allowed.
    """
    for binding in context.bindings_list:
        if len(binding.name) != 1 or binding.name == "_":
            continue
        if binding.kind == "import" and len(binding.imported_name) == 1:
            continue
        context.report_none("RT201", (binding.line, binding.column), (
            f"Name '{binding.name}' is a single character; use a "
            "meaningful name."))


def is_verb_exempt_bool(fact_node: ast.AST, is_method_bool: bool) -> bool:
    """Tell whether a function is exempt from the verb rule.

    Args:
        fact_node (ast.AST): Function node.
        is_method_bool (bool): Whether it is defined in a class body.
    Returns:
        bool: True for dunders, main, accessors, overloads and
            framework hooks.
    Warnings:
        None.
    """
    name_str = fact_node.name
    if is_dunder_bool(name_str) or name_str == "main":
        return True
    if is_method_bool and is_protocol_hook_bool(name_str):
        return True
    return any(decorator_str in EXEMPT_VERB_DECORATORS_TUPLE
               or decorator_str.endswith(ACCESSOR_SUFFIXES_TUPLE)
               for decorator_str in read_decorator_names_list(fact_node))


def check_verb_names_none(context: RuleContext) -> None:
    """Report functions whose first word is not a verb (RT202).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds findings to the context.
    Warnings:
        The verb list is a heuristic shared by both engines.
    """
    for fact in context.functions_list:
        if is_verb_exempt_bool(fact.node, fact.is_method):
            continue
        first_word_str = fact.node.name.lstrip("_").split("_")[0]
        if first_word_str.lower() in VERBS_SET and first_word_str.islower():
            continue
        context.report_none("RT202", locate_definition_name_tuple(
            context.source_info, fact.node), (
            f"Function '{fact.node.name}' should start with a verb "
            "describing its operation."))


def collect_bound_names_set(tree: ast.Module) -> set[str]:
    """List every name bound anywhere in a file.

    Args:
        tree (ast.Module): Parsed module.
    Returns:
        set[str]: Names of assignments, definitions, imports, parameters.
    Warnings:
        Used to tell whether a builtin such as len is rebound.
    """
    names_set: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.Name) and not isinstance(node.ctx,
                                                         ast.Load):
            names_set.add(node.id)
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef,
                               ast.ClassDef)):
            names_set.add(node.name)
        elif isinstance(node, ast.arg):
            names_set.add(node.arg)
        elif isinstance(node, ast.alias):
            names_set.add(node.asname or node.name.split(".")[0])
    return names_set


def infer_value_dtype_str(value_node: ast.expr, bound_set: set[str]) -> str:
    """Return the certain builtin type of an assigned value, or "".$

    Args:
        value_node (ast.expr): Assigned expression.
        bound_set (set[str]): Names rebound in the file.
    Returns:
        str: "int", "float", "str", "bool", "list", "dict", "set",
            "tuple", "bytes", or "" when the type is not certain.
    Warnings:
        Only the forms listed in RULES.md (RT203) count.
    """
    if isinstance(value_node, ast.Constant):
        constant_type = type(value_node.value)
        for python_type, name_str in ((bool, "bool"), (int, "int"),
                                      (float, "float"), (str, "str"),
                                      (bytes, "bytes")):
            if constant_type is python_type:
                return name_str
        return ""
    if isinstance(value_node, ast.Compare):
        return infer_comparison_dtype_str(value_node)
    if isinstance(value_node, ast.UnaryOp):
        if isinstance(value_node.op, ast.Not):
            return "bool"
        if isinstance(value_node.op, (ast.USub, ast.UAdd)):
            operand_str = infer_value_dtype_str(value_node.operand, bound_set)
            return operand_str if operand_str in ("int", "float") else ""
        return ""
    for node_types_tuple, name_str in DISPLAY_TYPES_TUPLE:
        if isinstance(value_node, node_types_tuple):
            return name_str
    if isinstance(value_node, ast.Call) and isinstance(value_node.func,
                                                       ast.Name):
        function_str = value_node.func.id
        if function_str in BUILTIN_RESULTS_DICT and (
            function_str not in bound_set
        ):
            return BUILTIN_RESULTS_DICT[function_str]
    return ""


def check_dtype_suffixes_none(context: RuleContext) -> None:
    """Report variables without the _<dtype> suffix of their type (RT203).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds findings to the context.
    Warnings:
        Strict profile only; only certain types are reported.
    """
    if not context.is_enabled_bool("RT203"):
        return
    bound_set = collect_bound_names_set(context.tree)
    for binding in context.bindings_list:
        if binding.kind != "variable" or binding.value_node is None or (
            is_dunder_bool(binding.name)
        ):
            continue
        if binding.scope_kind not in ("module", "function"):
            continue
        if binding.scope_kind == "module" and (
            binding.name in context.constants_dict
            or UPPER_CASE_PATTERN.match(binding.name)
        ):
            continue
        dtype_str = infer_value_dtype_str(binding.value_node, bound_set)
        if dtype_str and not binding.name.endswith(f"_{dtype_str}"):
            context.report_none("RT203", (binding.line, binding.column), (
                f"Variable '{binding.name}' holds a {dtype_str}; name it "
                f"'{binding.name.rstrip('_')}_{dtype_str}'."))


def is_convention_exempt_bool(binding: Binding, context: RuleContext) -> bool:
    """Tell whether a binding is exempt from case conventions (RT204).

    Args:
        binding (Binding): Binding to test.
        context (RuleContext): File being checked.
    Returns:
        bool: True for imports, dunders, constants, UPPER_CASE names in
            module or class scope, module type aliases and framework
            hooks.
    Warnings:
        None.
    """
    if binding.kind == "import" or is_dunder_bool(binding.name) or (
        not binding.name.strip("_")
    ):
        return True
    if binding.scope_kind in ("module", "class") and (
        UPPER_CASE_PATTERN.match(binding.name)
        or binding.name in context.constants_dict
    ):
        return True
    if binding.scope_kind == "module" and (
        binding.name in context.type_aliases_set
    ):
        return True
    return binding.scope_kind == "class" and is_protocol_hook_bool(
        binding.name)


def check_conventions_none(context: RuleContext) -> None:
    """Report names breaking snake_case or PascalCase (RT204).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds findings to the context.
    Warnings:
        None.
    """
    for binding in context.bindings_list:
        if is_convention_exempt_bool(binding, context):
            continue
        if binding.kind == "class":
            if not PASCAL_CASE_PATTERN.match(binding.name):
                context.report_none("RT204", (binding.line, binding.column),
                                    f"Class '{binding.name}' should be "
                                    "PascalCase.")
        elif not SNAKE_CASE_PATTERN.match(binding.name):
            context.report_none("RT204", (binding.line, binding.column),
                                f"Name '{binding.name}' should be "
                                "snake_case.")


def check_generic_names_none(context: RuleContext) -> None:
    """Report generic names such as data or temp (RT205).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds findings to the context.
    Warnings:
        Strict profile only; imports are not reported.
    """
    if not context.is_enabled_bool("RT205"):
        return
    for binding in context.bindings_list:
        if binding.kind != "import" and binding.name in GENERIC_NAMES_SET:
            context.report_none("RT205", (binding.line, binding.column), (
                f"Name '{binding.name}' is generic; describe what it "
                "holds."))


def infer_comparison_dtype_str(value_node: ast.Compare) -> str:
    """Recognize comparisons whose result has a builtin bool type.

    Args:
        value_node (ast.Compare): Comparison expression.
    Returns:
        str: bool for scalar literals or identity/membership, else empty.
    Warnings:
        Rich comparisons can return arrays and arbitrary user objects.
    """
    scalar_bool = all(isinstance(part_node, ast.Constant) for part_node
                      in [value_node.left, *value_node.comparators])
    identity_bool = all(isinstance(operator_node, (
        ast.Is, ast.IsNot, ast.In, ast.NotIn))
        for operator_node in value_node.ops)
    return "bool" if scalar_bool or identity_bool else ""
