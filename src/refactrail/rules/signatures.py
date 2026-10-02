"""Check strict dtype suffixes against explicit builtin annotations."""

import ast

from refactrail.rules.context import RuleContext
from refactrail.rules.naming import (
    collect_bound_names_set, is_verb_exempt_bool,
)
from refactrail.facts import read_decorator_names_list
from refactrail.source import (
    locate_definition_name_tuple, locate_node_tuple,
)

ANNOTATED_TYPES_SET = frozenset({
    "int", "float", "str", "bool", "list", "dict", "set", "tuple",
    "bytes", "complex",
})
GENERIC_TYPES_SET = frozenset({"list", "dict", "set", "tuple"})


def read_annotation_dtype_str(
    annotation_node: ast.expr | None, bound_set: set[str],
) -> str:
    """Read a builtin dtype declaration without evaluating an annotation.

    Args:
        annotation_node (ast.expr | None): Explicit annotation syntax.
        bound_set (set[str]): Names that might shadow builtin types.
    Returns:
        str: Declared dtype, or empty for unresolved annotations.
    Warnings:
        An annotation is not a verified runtime value type.
    """
    if isinstance(annotation_node, ast.Constant):
        return "none" if annotation_node.value is None else ""
    allowed_set = ANNOTATED_TYPES_SET
    if isinstance(annotation_node, ast.Subscript):
        allowed_set = GENERIC_TYPES_SET
        annotation_node = annotation_node.value
    if isinstance(annotation_node, ast.Name) and (
        annotation_node.id in allowed_set - bound_set
    ):
        return annotation_node.id
    return ""


def check_signature_suffixes_none(context: RuleContext) -> None:
    """Report return and parameter suffixes in the strict profile.

    Args:
        context (RuleContext): File facts and enabled rules.
    Returns:
        None: Adds RT206 and RT207 findings.
    Warnings:
        Findings are proposals; public interfaces are never renamed.
    """
    if not context.settings_info.strict_bool:
        return
    bound_set = collect_bound_names_set(context.tree)
    for fact in context.functions_list:
        if is_verb_exempt_bool(fact.node, fact.is_method):
            continue
        dtype_str = read_annotation_dtype_str(fact.node.returns, bound_set)
        if dtype_str and not fact.node.name.endswith(f"_{dtype_str}"):
            context.report_none("RT206", locate_definition_name_tuple(
                context.source_info, fact.node), (
                f"Function '{fact.node.name}' declares return type "
                f"{dtype_str}; use suffix '_{dtype_str}'."))
        check_parameter_suffixes_none(context, fact.node, fact.is_method,
                                      bound_set)


def check_parameter_suffixes_none(
    context: RuleContext, function_node: ast.FunctionDef,
    method_bool: bool, bound_set: set[str],
) -> None:
    """Compare parameter spellings with their explicit dtype declarations.

    Args:
        context (RuleContext): Findings destination.
        function_node (ast.FunctionDef): Function or asynchronous function.
        method_bool (bool): Whether it is a class method.
        bound_set (set[str]): Potentially shadowed builtin names.
    Returns:
        None: Adds RT207 findings for ordinary parameters.
    Warnings:
        Varargs and keyword collectors are excluded: their stored type
        differs from the annotated element type.
    """
    arguments_node = function_node.args
    positional_list = [*arguments_node.posonlyargs, *arguments_node.args]
    if method_bool and "staticmethod" not in read_decorator_names_list(
        function_node,
    ):
        positional_list = positional_list[1:]
    for parameter_node in [*positional_list, *arguments_node.kwonlyargs]:
        dtype_str = read_annotation_dtype_str(parameter_node.annotation,
                                               bound_set)
        if dtype_str and not parameter_node.arg.endswith(f"_{dtype_str}"):
            context.report_none("RT207", locate_node_tuple(
                context.source_info, parameter_node), (
                f"Parameter '{parameter_node.arg}' declares type "
                f"{dtype_str}; use suffix '_{dtype_str}'."))
