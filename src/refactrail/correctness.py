"""General correctness analysis independent of RefacTrail style profiles."""

import ast
import warnings

from refactrail.correctness_checks import (
    FUNCTION_NODES_TUPLE, check_comparison_none, check_defaults_none,
    check_dictionary_none, check_finally_none, check_statement_lists_none,
    report_node_none,
)
from refactrail.compat_pycodestyle import (
    check_import_position_none, check_pycodestyle_node_none,
    check_statement_tokens_none,
)
from refactrail.compat_pyflakes import (
    check_pyflakes_module_none, check_pyflakes_node_none,
)
from refactrail.compat_scope_checker import check_scope_codes_none
from refactrail.engine import parse_quietly_node
from refactrail.models import Finding, Settings, is_code_enabled_bool
from refactrail.lexical import check_lexical_none
from refactrail.rules.context import RuleContext
from refactrail.source import build_source_file, decode_source_text

# Compiler errors that the F rules report as findings (Pyflakes reports
# them on code that parses), so linting continues with the syntax tree.
LINT_COMPILE_ERRORS_TUPLE = (
    ("'break' outside loop", "F701"),
    ("'continue' not properly in loop", "F702"),
    ("'return' outside function", "F706"),
    ("'yield' outside function", "F704"),
    ("'await' outside function", "F704"),
    ("default 'except:' must be last", "F707"),
    ("from __future__ imports must occur at the beginning of the file",
     "F404"),
    ("future feature", "F407"),
    ("multiple starred expressions in assignment", "F622"),
    ("too many expressions in star-unpacking assignment", "F621"),
    ("import * only allowed at module level", "F406"),
)
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
    "E401": "Multiple imports on one line",
    "E402": "Module-level import not at top of file",
    "E701": "Multiple statements on one line (colon)",
    "E702": "Multiple statements on one line (semicolon)",
    "E703": "Statement ends with an unnecessary semicolon",
    "E711": "Comparison to None",
    "E712": "Comparison to True or False",
    "E713": "Membership test should use 'not in'",
    "E714": "Identity test should use 'is not'",
    "E721": "Type comparison with == or !=",
    "E722": "Bare except",
    "E731": "Lambda assigned to a name",
    "E741": "Ambiguous variable name",
    "E742": "Ambiguous class name",
    "E743": "Ambiguous function name",
    "F404": "Late __future__ import",
    "F407": "Undefined __future__ feature",
    "F501": "Invalid %-format string",
    "F502": "%-format expected a mapping",
    "F503": "%-format expected a sequence",
    "F504": "%-format has unused named arguments",
    "F505": "%-format is missing arguments",
    "F506": "%-format mixes positional and named placeholders",
    "F507": "%-format placeholder count mismatch",
    "F508": "%-format * specifier requires a sequence",
    "F509": "%-format has an unsupported format character",
    "F521": "Invalid str.format() string",
    "F522": "str.format() has unused named arguments",
    "F523": "str.format() has unused positional arguments",
    "F524": "str.format() is missing arguments",
    "F525": "str.format() mixes automatic and manual numbering",
    "F541": "f-string without placeholders",
    "F601": "Dictionary key literal repeated",
    "F602": "Dictionary key variable repeated",
    "F621": "Too many expressions in star-unpacking assignment",
    "F622": "Two starred expressions in assignment",
    "F631": "Assert test is a non-empty tuple",
    "F632": "'is' comparison with a literal",
    "F633": "print >> is invalid",
    "F634": "If test is a non-empty tuple",
    "F701": "break outside loop",
    "F702": "continue outside loop",
    "F704": "yield or await outside function",
    "F706": "return outside function",
    "F707": "Bare except is not the last handler",
    "F722": "Syntax error in forward annotation",
    "F901": "raise NotImplemented",
    "F401": "Unused import",
    "F402": "Import shadowed by loop variable",
    "F403": "Star import used",
    "F405": "Name may be undefined or from star imports",
    "F406": "Star import outside module level",
    "F811": "Redefinition of unused name",
    "F821": "Undefined name",
    "F822": "Undefined name in __all__",
    "F823": "Local variable referenced before assignment",
    "F841": "Unused local variable",
    "F842": "Unused annotated local variable",
}


def parse_lint_error_node(text_str: str, path_str: str,
                          error: Exception,
                          settings_info: Settings) -> ast.Module | None:
    """Keep linting when the compiler only rejects a lint-checked misuse.

    Args:
        text_str (str): Decoded source.
        path_str (str): Source label.
        error (Exception): The compile error.
        settings_info (Settings): Selected codes.
    Returns:
        ast.Module | None: The syntax tree when the error is one that a
        selected F rule reports itself (such as F701 for 'break' outside
        a loop) and the source parses; else None.
    Warnings:
        The caller still reports RT001 when that F finding is absent or
        suppressed, so a file that does not compile is never clean.
    """
    code_str = find_lint_error_code_str(error)
    if not code_str or not is_code_enabled_bool(settings_info, code_str):
        return None
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return ast.parse(text_str, filename=path_str)
    except (SyntaxError, ValueError):
        return None


def find_lint_error_code_str(error: Exception) -> str:
    """The F code that reports a compile error, if any.

    Args:
        error (Exception): The compile error.
    Returns:
        str: For example "F701", or "" for other errors.
    Warnings:
        None.
    """
    message_str = str(getattr(error, "msg", ""))
    if not isinstance(error, SyntaxError):
        return ""
    return next((code_str for prefix_str, code_str in LINT_COMPILE_ERRORS_TUPLE
                 if message_str.startswith(prefix_str)), "")


def build_compile_error_info(path_str: str, error: Exception) -> Finding:
    """The unsuppressible RT001 finding for a compile error.

    Args:
        path_str (str): Source label.
        error (Exception): The error.
    Returns:
        Finding: RT001 at the error's position.
    Warnings:
        None.
    """
    return Finding(path_str, getattr(error, "lineno", None) or 1,
                   getattr(error, "offset", None) or 1, "RT001", "error",
                   f"Syntax error: {error}")


def run_checks_none(context_info: RuleContext) -> None:
    """Run every correctness and compatible check on a parsed file.

    Args:
        context_info (RuleContext): Parsed file, settings and findings.
    Returns:
        None: Adds findings to the context.
    Warnings:
        None.
    """
    spec_ids_set: set[int] = set()
    for node in ast.walk(context_info.tree):
        check_node_none(context_info, node)
        check_pyflakes_node_none(context_info, node, spec_ids_set)
    check_lexical_none(context_info)
    check_pyflakes_module_none(context_info)
    check_scope_codes_none(context_info)
    if any(context_info.is_enabled_bool(code_str)
           for code_str in ("E701", "E702", "E703")):
        check_statement_tokens_none(context_info)
    if context_info.is_enabled_bool("E402"):
        check_import_position_none(context_info)


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
    check_pycodestyle_node_none(context_info, node)


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
    compile_error = None
    try:
        tree_node = parse_quietly_node(text_str, path_str)
    except (SyntaxError, ValueError) as error:
        compile_error = error
        tree_node = parse_lint_error_node(text_str, path_str, error,
                                          settings_info)
        if tree_node is None:
            return [build_compile_error_info(path_str, error)]
    context_info = RuleContext(build_source_file(path_str, text_str),
                               tree_node, settings_info)
    run_checks_none(context_info)
    return merge_compile_error_list(sorted(set(context_info.findings_list)),
                                    path_str, compile_error)


def merge_compile_error_list(findings_list: list[Finding], path_str: str,
                             compile_error: Exception | None
                             ) -> list[Finding]:
    """Keep RT001 unless its compile error is reported as an F code.

    Args:
        findings_list (list[Finding]): Sorted findings.
        path_str (str): Source label for findings.
        compile_error (Exception | None): The compiler's error, if any.
    Returns:
        list[Finding]: The findings, with RT001 first when the error is
            not already reported under its mapped code.
    Warnings:
        None.
    """
    if compile_error is not None and not any(
            finding.code == find_lint_error_code_str(compile_error)
            for finding in findings_list):
        findings_list.insert(0, build_compile_error_info(path_str,
                                                         compile_error))
    return findings_list
