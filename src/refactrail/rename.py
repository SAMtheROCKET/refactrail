"""Read-only local rename proposals with exact source spans and refusals."""

import ast
from copy import deepcopy
import difflib
from hashlib import sha256
import keyword
import re
from pathlib import Path
import symtable
import unicodedata

from refactrail.engine import parse_quietly_node
from refactrail.lexical import collect_scope_limits_list
from refactrail.source import decode_source_text

FUNCTION_NODES_TUPLE = (ast.FunctionDef, ast.AsyncFunctionDef)
DEFINITION_NODES_TUPLE = (*FUNCTION_NODES_TUPLE, ast.ClassDef)
NESTED_SCOPES_TUPLE = (
    ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef, ast.Lambda,
    ast.ListComp, ast.SetComp, ast.DictComp, ast.GeneratorExp,
)


def select_rename_function(tree_node: ast.Module, function_str: str,
                           text_str: str, old_str: str,
                           new_str: str) -> ast.AST:
    """Validate the bounded function-local rename subset.

    Args:
        tree_node (ast.Module): Compiled source.
        function_str (str): Unique undecorated top-level function.
        text_str (str): Decoded source.
        old_str (str): Existing local binding.
        new_str (str): Proposed explicit name.
    Returns:
        ast.AST: Eligible function or a located refusal.
    Warnings:
        Parameters, nested scopes, globals and reflection are excluded.
    """
    matches_list = [node for node in tree_node.body
                    if isinstance(node, FUNCTION_NODES_TUPLE)
                    and node.name == function_str]
    if len(matches_list) != 1:
        raise ValueError(
            "RENAME001 line 1: select a unique top-level function")
    function_node = matches_list[0]
    nodes_list = [child_node for statement in function_node.body
                  for child_node in ast.walk(statement)]
    if function_node.decorator_list or any(
        isinstance(node, NESTED_SCOPES_TUPLE) for node in nodes_list
    ):
        raise ValueError(f"RENAME002 line {function_node.lineno}: decorators "
                         "and nested scopes need broader reference analysis")
    limits_list = collect_scope_limits_list(tree_node)
    if limits_list:
        raise ValueError("RENAME003: " + "; ".join(limits_list))
    validate_rename_names_none(tree_node, function_node, text_str, old_str,
                               new_str)
    return function_node


def validate_rename_names_none(tree_node: ast.Module, function_node: ast.AST,
                               text_str: str, old_str: str,
                               new_str: str) -> None:
    """Reject parameters, captures, normalized collisions and invalid names.

    Args:
        tree_node (ast.Module): Source tree.
        function_node (ast.AST): Selected function.
        text_str (str): Compilable original source.
        old_str (str): Current binding identifier.
        new_str (str): Explicit replacement identifier.
    Returns:
        None: Raises a located refusal for unsupported requests.
    Warnings:
        String references and external introspection are not rewritten.
    """
    if not new_str.isidentifier() or keyword.iskeyword(new_str) or (
        unicodedata.normalize("NFKC", new_str) != new_str
    ):
        raise ValueError(
            "RENAME004 line 1: replacement must be a normalized identifier")
    table_info = next(child_info for child_info in symtable.symtable(
        text_str, "<rename>", "exec").get_children()
        if child_info.get_name() == function_node.name
        and child_info.get_lineno() == function_node.lineno)
    if old_str not in table_info.get_identifiers():
        raise_rename_none("RENAME005", function_node,
                          f"Unknown local {old_str}")
    symbol_info = table_info.lookup(old_str)
    if not symbol_info.is_local() or symbol_info.is_parameter():
        raise_rename_none("RENAME005", function_node,
                          "Only local non-parameter bindings are supported")
    validate_rename_collisions_none(tree_node, function_node, table_info,
                                    old_str, new_str)


def replace_name_spans_str(text_str: str, function_node: ast.AST,
                            old_str: str, new_str: str) -> str:
    """Replace only AST Name tokens inside the selected function body.

    Args:
        text_str (str): Source with original line endings.
        function_node (ast.AST): Eligible selected function.
        old_str (str): Exact compiler identifier.
        new_str (str): Validated explicit replacement.
    Returns:
        str: Proposed source preserving unrelated bytes and tokens.
    Warnings:
        Attributes, keyword labels, comments and string contents are retained.
    """
    normalized_bytes = text_str.encode("utf-8")
    lines_list = re.findall(rb"[^\r\n]*(?:\r\n|\r|\n|$)",
                            normalized_bytes)[:-1]
    offsets_list = [0]
    for line_bytes in lines_list:
        offsets_list.append(offsets_list[-1] + len(line_bytes))
    edits_list = [(offsets_list[node.lineno - 1] + node.col_offset,
                   offsets_list[node.end_lineno - 1] + node.end_col_offset)
                  for statement in function_node.body
                  for node in ast.walk(statement)
                  if isinstance(node, ast.Name) and node.id == old_str]
    if not edits_list:
        raise_rename_none("RENAME005", function_node,
                          "No supported name spans")
    for start_int, end_int in sorted(edits_list, reverse=True):
        normalized_bytes = (normalized_bytes[:start_int]
                            + new_str.encode("utf-8")
                            + normalized_bytes[end_int:])
    return normalized_bytes.decode("utf-8")


def plan_rename_dict(path_str: str, function_str: str, old_str: str,
                     new_str: str) -> dict:
    """Prepare a source-linked local rename diff without writing anything.

    Args:
        path_str (str): Regular Python source file.
        function_str (str): Top-level function containing the binding.
        old_str (str): Existing function-local name.
        new_str (str): User-provided meaningful replacement.
    Returns:
        dict: Reviewed proposal with source hash and remaining risks.
    Warnings:
        No apply command is authorized by this partial analysis.
    """
    source_path = Path(path_str)
    if source_path.is_symlink() or source_path.is_junction():
        raise ValueError("Linked rename inputs are unsupported")
    raw_bytes = source_path.read_bytes()
    text_str = decode_source_text(raw_bytes)
    if text_str is None:
        raise ValueError("Rename input must be UTF-8 Python source")
    tree_node = parse_quietly_node(text_str, path_str)
    function_node = select_rename_function(tree_node, function_str, text_str,
                                           old_str, new_str)
    output_str = replace_name_spans_str(
        text_str, function_node, old_str, new_str)
    validate_rename_none(tree_node, output_str, function_str, old_str, new_str)
    return {"schema_version": "refactrail-rename-1", "path": path_str,
        "source_sha256": sha256(raw_bytes).hexdigest(),
        "status": "candidate_for_review", "can_apply": False,
        "behavior_verified": False, "function": function_str,
        "old_name": old_str, "new_name": new_str,
        "diff": "".join(difflib.unified_diff(text_str.splitlines(True),
            output_str.splitlines(True), fromfile=path_str, tofile=path_str)),
        "proposed_source": output_str,
        "limitations": [
            "Strings, comments and external references are unchanged.",
            "Tracing, frame inspection and local names remain observable.",
            "Recheck the source hash and run regressions before manual use."]}


def validate_rename_none(original_node: ast.Module, output_str: str,
                          function_str: str, old_str: str,
                          new_str: str) -> None:
    """Verify the proposal changes only selected body Name identifiers.

    Args:
        original_node (ast.Module): Original compiled source tree.
        output_str (str): Proposed source.
        function_str (str): Selected function.
        old_str (str): Original binding.
        new_str (str): Replacement name.
    Returns:
        None: Raises when the structural rename contract is violated.
    Warnings:
        This is a structural check, not a behavior proof.
    """
    output_node = parse_quietly_node(output_str, "<rename proposal>")
    expected_node = deepcopy(original_node)
    function_node = next(node for node in expected_node.body
                         if isinstance(node, FUNCTION_NODES_TUPLE)
                         and node.name == function_str)
    for statement in function_node.body:
        for node in ast.walk(statement):
            if isinstance(node, ast.Name) and node.id == old_str:
                node.id = new_str
    if ast.dump(expected_node) != ast.dump(output_node):
        raise_rename_none("RENAME008", function_node,
                          "Proposal changed unexpected syntax")


def validate_rename_collisions_none(tree_node: ast.Module,
                                    function_node: ast.AST,
                                    table_info: symtable.SymbolTable,
                                    old_str: str, new_str: str) -> None:
    """Reject replacement collisions and non-Name binding syntax.

    Args:
        tree_node (ast.Module): Complete original tree.
        function_node (ast.AST): Selected function.
        table_info (symtable.SymbolTable): Selected compiler scope.
        old_str (str): Existing local binding.
        new_str (str): Proposed identifier.
    Returns:
        None: Raises a source-linked refusal for unsupported bindings.
    Warnings:
        This deliberately excludes imports, exception and pattern bindings.
    """
    if new_str == old_str or any(
        isinstance(node, ast.Name) and node.id == new_str
        or isinstance(node, ast.arg) and node.arg == new_str
        or isinstance(node, DEFINITION_NODES_TUPLE)
        and node.name == new_str for node in ast.walk(tree_node)
    ) or new_str in table_info.get_identifiers():
        raise_rename_none("RENAME006", function_node,
                          "Replacement collides with a source name")
    if any(is_opaque_binding_bool(node, old_str)
           for node in ast.walk(function_node)):
        raise_rename_none("RENAME007", function_node,
                          "Pattern, exception and import bindings "
                          "are excluded")


def is_opaque_binding_bool(node: ast.AST, name_str: str) -> bool:
    """Detect bindings stored outside ordinary AST Name nodes.

    Args:
        node (ast.AST): Node in the selected function.
        name_str (str): Requested original binding.
    Returns:
        bool: True for unsupported alias, pattern or handler binding.
    Warnings:
        These cases require dedicated edits and reference analysis.
    """
    if isinstance(node, (ast.ExceptHandler, ast.MatchAs, ast.MatchStar)):
        return node.name == name_str
    if isinstance(node, ast.MatchMapping):
        return node.rest == name_str
    if isinstance(node, ast.alias):
        return (node.asname or node.name.split(".")[0]) == name_str
    return False


def raise_rename_none(code_str: str, node: ast.AST, message_str: str) -> None:
    """Raise a refusal with the original source line.

    Args:
        code_str (str): Stable refusal category.
        node (ast.AST): Original source location.
        message_str (str): Concrete reason for refusal.
    Returns:
        None: Always raises ValueError.
    Warnings:
        Refusals never create a partially rewritten proposal.
    """
    raise ValueError(f"{code_str} line {node.lineno}: {message_str}")
