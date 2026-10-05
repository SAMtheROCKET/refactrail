"""Pyflakes-compatible codes that need no scope analysis (F4, F6, F7, F9).

Codes and meanings follow Pyflakes so ``# noqa`` comments carry over;
the analysis is RefacTrail's own, over Python's syntax tree and tokens.
"""

import __future__
import ast
import tokenize

from refactrail.compat_formats import (
    check_format_call_none, check_percent_format_none,
)
from refactrail.rules.context import RuleContext
from refactrail.source import locate_node_tuple

FUTURE_FEATURES_FROZENSET = frozenset(__future__.all_feature_names)
FUNCTION_SCOPES_TUPLE = (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)
LOOP_NODES_TUPLE = (ast.For, ast.AsyncFor, ast.While)
FSTRING_START_INT = getattr(tokenize, "FSTRING_START", -1)
NON_TYPE_SUBSCRIPTS_FROZENSET = frozenset(("Literal", "Annotated"))
PLACEMENT_MESSAGES_DICT = {
    ast.Break: ("F701", "'break' outside loop."),
    ast.Continue: ("F702", "'continue' not properly in loop."),
    ast.Return: ("F706", "'return' outside function."),
    ast.Yield: ("F704", "'yield' outside function."),
    ast.YieldFrom: ("F704", "'yield from' outside function."),
    ast.Await: ("F704", "'await' outside function."),
}


def report_node_none(context_info: RuleContext, node: ast.AST,
                     code_str: str, message_str: str) -> None:
    """Report a finding at a node's start.

    Args:
        context_info (RuleContext): Findings.
        node (ast.AST): Located node.
        code_str (str): Rule code.
        message_str (str): Explanation.
    Returns:
        None: Adds the finding.
    Warnings:
        None.
    """
    context_info.report_none(
        code_str, locate_node_tuple(context_info.source_info, node),
        message_str)


def check_future_imports_none(context_info: RuleContext) -> None:
    """F404 and F407: late or unknown __future__ imports.

    Args:
        context_info (RuleContext): Findings and tree.
    Returns:
        None: Adds findings at the import or the unknown feature.
    Warnings:
        None.
    """
    is_early_bool = True
    for index_int, node in enumerate(context_info.tree.body):
        is_future = isinstance(node, ast.ImportFrom) and (
            node.module == "__future__" and node.level == 0)
        if is_future and not is_early_bool:
            report_node_none(context_info, node, "F404",
                             "from __future__ imports must come first.")
        if is_future:
            for alias in node.names:
                if alias.name not in FUTURE_FEATURES_FROZENSET:
                    report_node_none(context_info, alias, "F407",
                                     f"Future feature {alias.name!r} is not "
                                     "defined.")
            continue
        is_docstring = index_int == 0 and isinstance(node, ast.Expr) and (
            isinstance(node.value, ast.Constant)
            and isinstance(node.value.value, str))
        if not is_docstring:
            is_early_bool = False


def build_key_str(node: ast.AST) -> str:
    """A comparable form of a dictionary key, ignoring positions.

    Args:
        node (ast.AST): Key expression.
    Returns:
        str: The tree dump, with each constant's type for 1 and 1.0.
    Warnings:
        None.
    """
    if isinstance(node, ast.Constant):
        return f"{type(node.value).__name__}:{node.value!r}"
    return ast.dump(node)


def check_repeated_keys_none(context_info: RuleContext,
                             node: ast.Dict) -> None:
    """F601 and F602: a key repeated in a dict display.

    Args:
        context_info (RuleContext): Findings.
        node (ast.Dict): Dict display.
    Returns:
        None: Adds a finding at every repeat after the first.
    Warnings:
        None.
    """
    seen_set: set[str] = set()
    for key in node.keys:
        if key is None:
            continue
        key_str = build_key_str(key)
        if key_str not in seen_set:
            seen_set.add(key_str)
            continue
        if isinstance(key, ast.Name):
            report_node_none(context_info, key, "F602",
                             f"Dictionary key {key.id!r} repeated.")
        elif isinstance(key, (ast.Constant, ast.Tuple, ast.JoinedStr)):
            report_node_none(context_info, key, "F601",
                             "Dictionary key literal repeated.")


def check_starred_targets_none(context_info: RuleContext,
                               node: ast.Tuple | ast.List) -> None:
    """F621 and F622: invalid starred unpacking targets.

    Args:
        context_info (RuleContext): Findings.
        node (ast.Tuple | ast.List): Store-context sequence.
    Returns:
        None: Adds findings at the target.
    Warnings:
        None.
    """
    if not isinstance(node.ctx, ast.Store):
        return
    starred_list = [index_int for index_int, element in enumerate(node.elts)
                    if isinstance(element, ast.Starred)]
    if len(starred_list) > 1:
        report_node_none(context_info, node, "F622",
                         "Two starred expressions in assignment.")
    elif starred_list and (starred_list[0] >= 1 << 8 or len(node.elts)
                           - starred_list[0] - 1 >= 1 << 24):
        report_node_none(context_info, node, "F621",
                         "Too many expressions in star-unpacking "
                         "assignment.")


def is_constant_value_bool(node: ast.AST) -> bool:
    """Whether a node is a literal constant (tuples of them included).

    Args:
        node (ast.AST): Expression.
    Returns:
        bool: True for numbers, strings, bytes and constant tuples.
    Warnings:
        None.
    """
    if isinstance(node, ast.Tuple):
        return all(is_constant_value_bool(element) for element in node.elts)
    return isinstance(node, ast.Constant)


def is_non_singleton_bool(node: ast.AST) -> bool:
    """A literal whose identity is not guaranteed.

    Args:
        node (ast.AST): Expression.
    Returns:
        bool: True for constants other than None, True, False and
        Ellipsis, for constant tuples, and for list, dict and set
        displays and comprehensions (always a new object).
    Warnings:
        f-strings and negative numbers (unary operations) are excluded.
    """
    if isinstance(node, (ast.List, ast.Dict, ast.Set, ast.ListComp,
                         ast.DictComp, ast.SetComp)):
        return True
    return is_constant_value_bool(node) and not (
        isinstance(node, ast.Constant) and (
            node.value is None or node.value is True or node.value is False
            or node.value is Ellipsis))


def check_is_literal_none(context_info: RuleContext,
                          node: ast.Compare) -> None:
    """F632: 'is' or 'is not' with a literal.

    Args:
        context_info (RuleContext): Findings.
        node (ast.Compare): Comparison.
    Returns:
        None: Adds findings at the comparison.
    Warnings:
        None.
    """
    operands_list = [node.left, *node.comparators]
    for index_int, operator in enumerate(node.ops):
        left, right = operands_list[index_int:index_int + 2]
        if isinstance(operator, (ast.Is, ast.IsNot)) and (
                is_non_singleton_bool(left) or is_non_singleton_bool(right)):
            report_node_none(context_info, node, "F632",
                             "Use == or != to compare literals, not 'is'.")


def check_fstring_none(context_info: RuleContext,
                       node: ast.JoinedStr) -> None:
    """F541: an f-string without placeholders.

    Args:
        context_info (RuleContext): Findings and tokens.
        node (ast.JoinedStr): f-string expression (implicitly joined
            parts are one node).
    Returns:
        None: Adds a finding at each f-string part of the expression.
    Warnings:
        Plain string parts joined to it are not reported.
    """
    if any(isinstance(part_node, ast.FormattedValue)
           for part_node in node.values):
        return
    start_tuple = (node.lineno, node.col_offset)
    end_tuple = (node.end_lineno, node.end_col_offset)
    lines_list = context_info.source_info.lines
    for token_info in context_info.tokens_list:
        if token_info.type != FSTRING_START_INT:
            continue
        line_int, column_int = token_info.start
        byte_int = len(lines_list[line_int - 1][:column_int].encode())
        if start_tuple <= (line_int, byte_int) < end_tuple:
            context_info.report_none("F541", (line_int, column_int + 1),
                                     "f-string without any placeholders.")


def list_placed_children_list(node: ast.AST, in_function_bool: bool,
                              in_loop_bool: bool) -> list[tuple]:
    """A node's children with the context they run in.

    Args:
        node (ast.AST): Parent node.
        in_function_bool (bool): Whether the parent is in a function.
        in_loop_bool (bool): Whether the parent is inside a loop body.
    Returns:
        list[tuple]: (child, in_function, in_loop) entries. Function and
        lambda bodies are functions without a loop; class bodies are
        neither; a loop's body is in a loop, its else clause is not.
    Warnings:
        Decorators, defaults and bases are visited with the body's
        context; they cannot hold these statements.
    """
    if isinstance(node, FUNCTION_SCOPES_TUPLE):
        return [(child, True, False) for child in ast.iter_child_nodes(node)]
    if isinstance(node, ast.ClassDef):
        return [(child, False, False) for child in ast.iter_child_nodes(node)]
    if isinstance(node, LOOP_NODES_TUPLE):
        body_ids_set = {id(statement) for statement in node.body}
        return [(child, in_function_bool,
                 in_loop_bool or id(child) in body_ids_set)
                for child in ast.iter_child_nodes(node)]
    return [(child, in_function_bool, in_loop_bool)
            for child in ast.iter_child_nodes(node)]


def check_placement_none(context_info: RuleContext) -> None:
    """F701, F702, F704 and F706: statements outside a loop or function.

    Args:
        context_info (RuleContext): Findings and tree.
    Returns:
        None: Adds a finding at each misplaced statement or expression.
    Warnings:
        Iterative, so deeply nested files do not exhaust the stack.
        Top-level await (notebooks, asyncio REPL) is reported too.
    """
    pending_list = [(context_info.tree, False, False)]
    while pending_list:
        node, in_function, in_loop = pending_list.pop()
        entry_tuple = PLACEMENT_MESSAGES_DICT.get(type(node))
        if entry_tuple is not None:
            code_str, message_str = entry_tuple
            is_misplaced = (not in_loop if code_str in ("F701", "F702")
                            else not in_function)
            if is_misplaced:
                report_node_none(context_info, node, code_str, message_str)
        pending_list.extend(list_placed_children_list(node, in_function,
                                                      in_loop))


def check_annotation_strings_none(context_info: RuleContext,
                                  annotation: ast.AST | None) -> None:
    """F722: string annotations that are not valid expressions.

    Args:
        context_info (RuleContext): Findings.
        annotation (ast.AST | None): An annotation expression.
    Returns:
        None: Adds a finding at each unparsable string.
    Warnings:
        Literal[...] values and Annotated[...] metadata are not types.
    """
    if annotation is None:
        return
    pending_list = [annotation]
    while pending_list:
        node = pending_list.pop()
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            try:
                ast.parse(node.value.strip(), mode="eval")
            except SyntaxError:
                report_node_none(context_info, node, "F722",
                                 "Syntax error in forward annotation "
                                 f"{node.value!r}.")
            continue
        if isinstance(node, ast.Subscript):
            name_str = (node.value.attr if isinstance(node.value,
                                                      ast.Attribute)
                        else getattr(node.value, "id", ""))
            if name_str == "Literal":
                continue
            if name_str == "Annotated" and isinstance(node.slice, ast.Tuple):
                pending_list.append(node.slice.elts[0])
                continue
        pending_list.extend(ast.iter_child_nodes(node))


def collect_annotations_list(node: ast.AST) -> list[ast.AST | None]:
    """The annotations a node carries.

    Args:
        node (ast.AST): Any node.
    Returns:
        list[ast.AST | None]: Parameter, return and variable annotations.
    Warnings:
        None.
    """
    if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
        arguments = node.args
        return [argument.annotation for argument in (
            *arguments.posonlyargs, *arguments.args, *arguments.kwonlyargs,
            arguments.vararg, arguments.kwarg) if argument is not None] + [
                node.returns]
    if isinstance(node, ast.AnnAssign):
        return [node.annotation]
    return []


def check_statement_shapes_none(context_info: RuleContext,
                                node: ast.AST) -> None:
    """F631, F633, F634 and F901 from single nodes.

    Args:
        context_info (RuleContext): Findings.
        node (ast.AST): Current node.
    Returns:
        None: Adds findings.
    Warnings:
        None.
    """
    if isinstance(node, ast.Assert) and isinstance(node.test, ast.Tuple) and (
            node.test.elts):
        report_node_none(context_info, node, "F631",
                         "Assert test is a non-empty tuple, which is always "
                         "true.")
    elif isinstance(node, ast.If) and isinstance(node.test, ast.Tuple) and (
            node.test.elts):
        report_node_none(context_info, node.test, "F634",
                         "If test is a tuple, which is always true.")
    elif isinstance(node, ast.BinOp) and isinstance(node.op, ast.RShift) and (
            isinstance(node.left, ast.Name) and node.left.id == "print"):
        report_node_none(context_info, node.left, "F633",
                         "Use of >> is invalid with print function.")
    elif isinstance(node, ast.Raise) and node.exc is not None:
        exception = node.exc
        if isinstance(exception, ast.Call):
            exception = exception.func
        if isinstance(exception, ast.Name) and (
                exception.id == "NotImplemented"):
            report_node_none(context_info, node.exc, "F901",
                             "raise NotImplemented should be raise "
                             "NotImplementedError.")


def check_pyflakes_node_none(context_info: RuleContext, node: ast.AST,
                             spec_ids_set: set[int]) -> None:
    """Dispatch the node-level Pyflakes-compatible checks.

    Args:
        context_info (RuleContext): Findings.
        node (ast.AST): Current node of the shared traversal.
        spec_ids_set (set[int]): ids of f-string format specs, which are
            not f-strings of their own.
    Returns:
        None: Adds findings.
    Warnings:
        None.
    """
    if isinstance(node, ast.BinOp):
        check_percent_format_none(context_info, node)
    elif isinstance(node, ast.Call):
        check_format_call_none(context_info, node)
    elif isinstance(node, ast.JoinedStr) and id(node) not in spec_ids_set:
        check_fstring_none(context_info, node)
    elif isinstance(node, ast.Dict):
        check_repeated_keys_none(context_info, node)
    elif isinstance(node, (ast.Tuple, ast.List)):
        check_starred_targets_none(context_info, node)
    elif isinstance(node, ast.Compare):
        check_is_literal_none(context_info, node)
    elif isinstance(node, ast.FormattedValue) and node.format_spec:
        spec_ids_set.add(id(node.format_spec))
    elif isinstance(node, ast.Try | ast.TryStar):
        for handler in node.handlers[:-1]:
            if handler.type is None:
                report_node_none(context_info, handler, "F707",
                                 "An except: block is not the last "
                                 "exception handler.")
    check_statement_shapes_none(context_info, node)
    for annotation in collect_annotations_list(node):
        check_annotation_strings_none(context_info, annotation)


def check_pyflakes_module_none(context_info: RuleContext) -> None:
    """Run the module-level Pyflakes-compatible checks.

    Args:
        context_info (RuleContext): Findings and tree.
    Returns:
        None: Adds F404, F407 and F70x findings.
    Warnings:
        None.
    """
    check_future_imports_none(context_info)
    check_placement_none(context_info)
