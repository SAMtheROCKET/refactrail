"""General correctness analysis independent of RefacTrail style profiles."""

import ast

from refactrail.correctness_checks import (
    FUNCTION_NODES_TUPLE, check_comparison_none, check_defaults_none,
    check_dictionary_none, check_finally_none, check_statement_lists_none,
    report_node_none,
)
from refactrail.engine import parse_quietly_node
from refactrail.models import Finding, Settings
from refactrail.lexical import check_lexical_none
from refactrail.rules.context import RuleContext
from refactrail.source import build_source_file, decode_source_text

CORRECTNESS_TITLES_DICT = {
    "RC201": "Unresolved lexical name",
    "RC202": "Import without lexical use",
    "RC101": "Mutable default argument",
    "RC102": "Duplicate literal dictionary key",
    "RC103": "Identity comparison with a non-singleton literal",
    "RC104": "Bare except handler",
    "RC105": "Unreachable statement after a direct terminator",
    "RC106": "Nonempty tuple assertion",
    "RC107": "Control-flow exit from finally",
}


def check_node_none(context_info: RuleContext, node: ast.AST) -> None:
    """Dispatch independent checks during a shared syntax traversal.

    Args:
        context_info (RuleContext): Findings and suppression state.
        node (ast.AST): Current syntax node.
    Returns:
        None: Adds applicable RC observations.
    Warnings:
        Checks are syntax evidence, not whole-program proofs.
    """
    if isinstance(node, FUNCTION_NODES_TUPLE):
        check_defaults_none(context_info, node)
    if isinstance(node, ast.Dict):
        check_dictionary_none(context_info, node)
    if isinstance(node, ast.Compare):
        check_comparison_none(context_info, node)
    if isinstance(node, ast.ExceptHandler) and node.type is None:
        report_node_none(context_info, node, "RC104",
                         "Bare except also catches process-control "
                         "exceptions such as KeyboardInterrupt.")
    if isinstance(node, ast.Assert) and isinstance(node.test, ast.Tuple):
        if node.test.elts and not any(isinstance(element, ast.Starred)
                                     for element in node.test.elts):
            report_node_none(context_info, node.test, "RC106",
                             "A nonempty tuple assertion tests the tuple's "
                             "truthiness, not its first element.")
    if isinstance(node, (ast.Try, ast.TryStar)):
        check_finally_none(context_info, node)
    check_statement_lists_none(context_info, node)


def check_correctness_list(
    path_str: str, raw_bytes: bytes,
    select_tuple: tuple[str, ...] = ("RC",),
    ignore_tuple: tuple[str, ...] = (),
) -> list[Finding]:
    """Check general correctness without importing or executing the source.

    Args:
        path_str (str): Source label for findings.
        raw_bytes (bytes): Original UTF-8 source, optionally with a BOM.
        select_tuple (tuple[str, ...]): Enabled rule prefixes.
        ignore_tuple (tuple[str, ...]): Disabled ordinary rule prefixes.
    Returns:
        list[Finding]: Sorted observations or an unsuppressible input error.
    Warnings:
        This independent engine does not infer domain intent or runtime types.
    """
    settings_info = Settings(select=select_tuple, ignore=ignore_tuple)
    text_str = decode_source_text(raw_bytes)
    if text_str is None:
        return [Finding(path_str, 1, 1, "RT002", "error",
                        "File is not UTF-8 or declares another encoding.")]
    try:
        tree_node = parse_quietly_node(text_str, path_str)
    except (SyntaxError, ValueError) as error:
        return [Finding(path_str, getattr(error, "lineno", None) or 1,
                        getattr(error, "offset", None) or 1, "RT001",
                        "error", f"Syntax error: {error}")]
    context_info = RuleContext(build_source_file(path_str, text_str),
                               tree_node, settings_info)
    for node in ast.walk(tree_node):
        check_node_none(context_info, node)
    check_lexical_none(context_info)
    return sorted(set(context_info.findings_list))
