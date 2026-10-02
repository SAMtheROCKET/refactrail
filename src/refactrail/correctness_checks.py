"""Independent correctness rules based on explicit Python syntax evidence."""

import ast

from refactrail.rules.context import RuleContext
from refactrail.source import locate_node_tuple

MUTABLE_NODES_TUPLE = (
    ast.List, ast.Dict, ast.Set, ast.ListComp, ast.DictComp, ast.SetComp,
)
FUNCTION_NODES_TUPLE = (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)
TERMINATOR_NODES_TUPLE = (ast.Return, ast.Raise, ast.Break, ast.Continue)


def report_node_none(
    context_info: RuleContext, node: ast.AST, code_str: str, message_str: str,
) -> None:
    """Report a syntax observation with Unicode source coordinates.

    Args:
        context_info (RuleContext): Active findings and suppression state.
        node (ast.AST): Located source node.
        code_str (str): Correctness code.
        message_str (str): Evidence and review guidance.
    Returns:
        None: Adds an enabled, unsuppressed finding.
    Warnings:
        Findings do not establish domain intent.
    """
    context_info.report_none(
        code_str, locate_node_tuple(context_info.source_info, node),
        message_str,
    )


def has_mutable_default_bool(node: ast.AST) -> bool:
    """Recognize displays that allocate mutable default state.

    Args:
        node (ast.AST): Default expression.
    Returns:
        bool: True for mutable displays, including inside literal tuples.
    Warnings:
        Calls, names and annotations are deliberately not inferred.
    """
    return isinstance(node, MUTABLE_NODES_TUPLE) or (
        isinstance(node, ast.Tuple)
        and any(has_mutable_default_bool(element) for element in node.elts)
    )


def check_defaults_none(context_info: RuleContext, node: ast.AST) -> None:
    """Report mutable defaults in function and lambda declarations.

    Args:
        context_info (RuleContext): Active findings.
        node (ast.AST): Function, asynchronous function or lambda.
    Returns:
        None: Adds RC101 findings.
    Warnings:
        Shared default state can be intentional and requires review.
    """
    defaults_list = [*node.args.defaults, *node.args.kw_defaults]
    for default_node in defaults_list:
        if default_node is not None and has_mutable_default_bool(
            default_node
        ):
            report_node_none(context_info, default_node, "RC101",
                             "Mutable default state is shared across calls.")


def is_literal_key_bool(node: ast.AST | None) -> bool:
    """Limit dictionary-key evaluation to builtin literal syntax.

    Args:
        node (ast.AST | None): Key, or None for dictionary unpacking.
    Returns:
        bool: True for constants, signed numbers and literal tuples.
    Warnings:
        No names, operators, calls or user objects are evaluated.
    """
    if isinstance(node, ast.Constant):
        return True
    if isinstance(node, ast.Tuple):
        return all(is_literal_key_bool(element) for element in node.elts)
    return (
        isinstance(node, ast.UnaryOp)
        and isinstance(node.op, (ast.UAdd, ast.USub))
        and isinstance(node.operand, ast.Constant)
        and type(node.operand.value) in (int, float, complex)
    )


def check_dictionary_none(
    context_info: RuleContext, node: ast.Dict,
) -> None:
    """Report repeated literal keys within an uninterrupted known region.

    Args:
        context_info (RuleContext): Active findings.
        node (ast.Dict): Dictionary display.
    Returns:
        None: Adds RC102 at each later duplicate key.
    Warnings:
        Dynamic keys and unpacking reset tracking to avoid guessing effects.
    """
    seen_dict: dict = {}
    for key_node in node.keys:
        if not is_literal_key_bool(key_node):
            seen_dict.clear()
            continue
        try:
            literal_value = ast.literal_eval(key_node)
            earlier_int = seen_dict.get(literal_value)
            if earlier_int is not None:
                report_node_none(
                    context_info, key_node, "RC102",
                    "Duplicate literal dictionary key; earlier key at "
                    f"line {earlier_int}.",
                )
            seen_dict[literal_value] = key_node.lineno
        except (TypeError, ValueError):
            seen_dict.clear()


def is_identity_literal_bool(node: ast.AST) -> bool:
    """Recognize literal operands whose identity is usually unintended.

    Args:
        node (ast.AST): Comparison operand.
    Returns:
        bool: True for non-singleton scalar and tuple literals.
    Warnings:
        Values of names and calls remain unknown.
    """
    if isinstance(node, ast.Tuple):
        return True
    if isinstance(node, ast.Constant):
        return type(node.value) in (int, float, complex, str, bytes)
    return isinstance(node, ast.UnaryOp) and is_literal_key_bool(node)


def check_comparison_none(
    context_info: RuleContext, node: ast.Compare,
) -> None:
    """Report identity comparisons against non-singleton literals.

    Args:
        context_info (RuleContext): Active findings.
        node (ast.Compare): Comparison chain.
    Returns:
        None: Adds at most one RC103 for the chain.
    Warnings:
        No automatic equality rewrite is proposed.
    """
    operands_list = [node.left, *node.comparators]
    for index_int, operator_node in enumerate(node.ops):
        if isinstance(operator_node, (ast.Is, ast.IsNot)) and any(
            is_identity_literal_bool(operand_node)
            for operand_node in operands_list[index_int:index_int + 2]
        ):
            report_node_none(context_info, node, "RC103",
                             "Identity comparison uses a non-singleton "
                             "literal; review whether equality is intended.")
            return


def check_statement_lists_none(
    context_info: RuleContext, node: ast.AST,
) -> None:
    """Find the first direct statement after an unconditional terminator.

    Args:
        context_info (RuleContext): Active findings.
        node (ast.AST): Node that may contain statement lists.
    Returns:
        None: Adds RC105 findings.
    Warnings:
        Branch reachability and exception-flow effects are not inferred.
    """
    for field_str, children_list in ast.iter_fields(node):
        if not isinstance(children_list, list):
            continue
        terminated_bool = False
        for child_node in children_list:
            if not isinstance(child_node, ast.stmt):
                continue
            if terminated_bool:
                report_node_none(context_info, child_node, "RC105",
                                 "Statement follows an unconditional "
                                 "control-flow terminator in this block.")
                break
            terminated_bool = isinstance(child_node, TERMINATOR_NODES_TUPLE)


def check_finally_none(
    context_info: RuleContext, node: ast.Try | ast.TryStar,
) -> None:
    """Report finally exits that can override a pending exception or return.

    Args:
        context_info (RuleContext): Active findings.
        node (ast.Try | ast.TryStar): Exception-handling statement.
    Returns:
        None: Adds RC107 observations.
    Warnings:
        Nested declarations are separate scopes; local loop exits differ.
    """
    pending_list = [(statement_node, 0) for statement_node in node.finalbody]
    while pending_list:
        child_node, depth_int = pending_list.pop()
        if isinstance(child_node, (*FUNCTION_NODES_TUPLE, ast.ClassDef)):
            continue
        if isinstance(child_node, ast.Return) or (
            isinstance(child_node, (ast.Break, ast.Continue)) and not depth_int
        ):
            report_node_none(context_info, child_node, "RC107",
                             "Control flow leaves finally and can override "
                             "a pending exception or return.")
        if isinstance(child_node, (ast.For, ast.AsyncFor, ast.While)):
            pending_list.extend((descendant_node, depth_int + 1)
                                for descendant_node in child_node.body)
            pending_list.extend((descendant_node, depth_int)
                                for descendant_node in child_node.orelse)
            continue
        pending_list.extend((descendant_node, depth_int)
                            for descendant_node in ast.iter_child_nodes(
                                child_node))
