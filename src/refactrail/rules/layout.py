"""Layout rules: line length, constants and code outside functions."""

import ast

from refactrail.facts import (
    FUNCTION_NODES_TUPLE, UPPER_CASE_PATTERN, is_dunder_bool,
    is_main_block_bool,
)
from refactrail.rules.context import RuleContext
from refactrail.source import locate_node_tuple


def check_line_length_none(context: RuleContext) -> None:
    """Report physical lines longer than the limit (RT101).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds findings to the context.
    Warnings:
        Length is counted in Unicode characters, not bytes.
    """
    limit_int = context.settings_info.line_length
    for number_int, line_str in enumerate(context.source_info.lines, 1):
        if len(line_str) > limit_int:
            context.report_none("RT101", (number_int, limit_int + 1), (
                f"Line has {len(line_str)} characters; limit is "
                f"{limit_int}."))


def check_constant_names_none(context: RuleContext) -> None:
    """Report constants whose names are not UPPER_CASE (RT102).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds findings to the context.
    Warnings:
        Only module-level names bound once to a literal are constants.
    """
    for name_str, statement_node in context.constants_dict.items():
        if UPPER_CASE_PATTERN.match(name_str):
            continue
        target_node = (statement_node.targets[0]
                       if isinstance(statement_node, ast.Assign)
                       else statement_node.target)
        context.report_none("RT102", locate_node_tuple(
            context.source_info, target_node), (
            f"Constant '{name_str}' should be UPPER_CASE "
            f"('{name_str.upper()}')."))


def is_type_checking_block_bool(node: ast.stmt) -> bool:
    """Tell whether a statement is if TYPE_CHECKING: or typing's version.

    Args:
        node (ast.stmt): Top-level statement.
    Returns:
        bool: True for if TYPE_CHECKING / if typing.TYPE_CHECKING.
    Warnings:
        Other aliases of typing are not recognised.
    """
    if not isinstance(node, ast.If):
        return False
    test_node = node.test
    return (isinstance(test_node, ast.Name)
            and test_node.id == "TYPE_CHECKING") or (
        isinstance(test_node, ast.Attribute)
        and test_node.attr == "TYPE_CHECKING"
        and isinstance(test_node.value, ast.Name)
        and test_node.value.id == "typing")


def list_assigned_names_list(node: ast.stmt) -> list[str]:
    """Return the plain names an = or annotated assignment binds.

    Args:
        node (ast.stmt): Statement.
    Returns:
        list[str]: Plain-name targets; empty for other statements.
    Warnings:
        Names inside tuple targets are not included.
    """
    if isinstance(node, ast.Assign):
        return [target_node.id for target_node in node.targets
                if isinstance(target_node, ast.Name)]
    if isinstance(node, ast.AnnAssign) and isinstance(node.target,
                                                      ast.Name):
        return [node.target.id]
    return []


def is_preamble_statement_bool(
    node: ast.stmt, constants_dict: dict, type_aliases_set: set[str],
) -> bool:
    """Tell whether a statement may appear before constants (RT103).

    Args:
        node (ast.stmt): Top-level statement.
        constants_dict (dict): Constant candidates by name.
        type_aliases_set (set[str]): Module-level type alias names.
    Returns:
        bool: True for docstrings, imports, constants, UPPER_CASE or
            dunder assignments, type aliases and if TYPE_CHECKING blocks.
    Warnings:
        None.
    """
    if isinstance(node, (ast.Import, ast.ImportFrom)) or (
        isinstance(node, ast.Expr) and isinstance(node.value, ast.Constant)
    ) or is_type_checking_block_bool(node) or isinstance(node,
                                                         ast.TypeAlias):
        return True
    names_list = list_assigned_names_list(node)
    return bool(names_list) and all(
        name_str in constants_dict or is_dunder_bool(name_str)
        or name_str in type_aliases_set
        or UPPER_CASE_PATTERN.match(name_str) for name_str in names_list)


def check_constant_placement_none(context: RuleContext) -> None:
    """Report constants defined after other code (RT103).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds findings to the context.
    Warnings:
        Constants belong right after the imports.
    """
    constants_dict = context.constants_dict
    code_started_bool = False
    for node in context.tree.body:
        if not code_started_bool:
            code_started_bool = not is_preamble_statement_bool(
                node, constants_dict, context.type_aliases_set)
            continue
        for name_str in list_assigned_names_list(node):
            if name_str in constants_dict or UPPER_CASE_PATTERN.match(
                name_str,
            ):
                target_node = (node.targets[0] if isinstance(node,
                                                             ast.Assign)
                               else node.target)
                context.report_none("RT103", locate_node_tuple(
                    context.source_info, target_node), (
                    f"Constant '{name_str}' should be defined after the "
                    "imports, before other code."))


def has_call_bool(node: ast.AST | None) -> bool:
    """Tell whether an expression contains a call anywhere.

    Args:
        node (ast.AST | None): Expression.
    Returns:
        bool: True when any ast.Call appears inside it.
    Warnings:
        Calls inside lambda bodies count too, as the spec defines.
    """
    return node is not None and any(isinstance(inner_node, ast.Call)
                                    for inner_node in ast.walk(node))


def is_structure_statement_bool(node: ast.stmt) -> bool:
    """Tell whether a top-level statement is allowed outside functions.

    Args:
        node (ast.stmt): Top-level statement (or one inside a try).
    Returns:
        bool: True for the kinds RT504 accepts.
    Warnings:
        A try statement is accepted when all its inner statements are.
    """
    if isinstance(node, (ast.Import, ast.ImportFrom, ast.Pass,
                         *FUNCTION_NODES_TUPLE, ast.ClassDef,
                         ast.TypeAlias)):
        return True
    if isinstance(node, ast.Expr) and isinstance(node.value, ast.Constant):
        return True
    if is_main_block_bool(node) or is_type_checking_block_bool(node):
        return True
    if isinstance(node, (ast.Assign, ast.AnnAssign)):
        names_list = list_assigned_names_list(node)
        targets_list = (node.targets if isinstance(node, ast.Assign)
                        else [node.target])
        constant_bool = bool(names_list) and len(names_list) == len(
            targets_list) and all(
            UPPER_CASE_PATTERN.match(name_str) or is_dunder_bool(name_str)
            for name_str in names_list)
        return constant_bool or not has_call_bool(node.value)
    if isinstance(node, ast.Try):
        inner_list = [*node.body, *node.orelse, *node.finalbody,
                      *(inner_node for handler in node.handlers
                        for inner_node in handler.body)]
        return all(is_structure_statement_bool(inner_node)
                   for inner_node in inner_list)
    return False


def check_module_code_none(context: RuleContext) -> None:
    """Report the first statement that runs code at import time (RT504).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds at most one finding.
    Warnings:
        One finding per file, at the first offending statement.
    """
    for node in context.tree.body:
        if not is_structure_statement_bool(node):
            context.report_none("RT504", locate_node_tuple(
                context.source_info, node), (
                "Module runs code at import time; move it into functions "
                "called from the main block."))
            return
