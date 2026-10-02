"""Compiler-backed lexical evidence and general name diagnostics."""

import ast
import builtins
from io import StringIO
import tokenize
from pathlib import Path
import re
import symtable
import sys

from refactrail.engine import parse_quietly_node
from refactrail.lexical_scopes import ScopeCollector
from refactrail.rules.context import RuleContext
from refactrail.source import build_source_file, locate_node_tuple

IMPLICIT_NAMES_SET = {
    "__name__", "__file__", "__package__", "__doc__", "__builtins__",
    "__spec__", "__loader__", "__cached__", "__annotations__", "__path__",
}
DYNAMIC_NAMES_SET = {"exec", "eval", "globals", "locals", "vars"}
FLOW_LIMIT_STR = (
    "Lexical bindings do not prove initialization, branch reachability, "
    "exception cleanup, deletion effects or call-time availability."
)
DEEP_RECURSION_LIMIT_INT = 100_000


def visit_deeply_none(collector_info: ScopeCollector,
                      tree_node: ast.Module) -> None:
    """Visit a tree whose nesting can exceed the default recursion limit.

    Args:
        collector_info (ScopeCollector): Recursive syntax visitor.
        tree_node (ast.Module): Tree of source that already compiled.
    Returns:
        None: The collector holds the gathered evidence.
    Warnings:
        Valid code can nest expressions hundreds of levels deep, and the
        visitor uses two Python frames per level. Python-to-Python calls
        do not consume the C stack on CPython 3.11+, so a higher limit is
        safe; the previous limit is always restored.
    """
    previous_int = sys.getrecursionlimit()
    sys.setrecursionlimit(max(previous_int, DEEP_RECURSION_LIMIT_INT))
    try:
        collector_info.visit(tree_node)
    finally:
        sys.setrecursionlimit(previous_int)


def collect_scope_limits_list(tree_node: ast.Module) -> list[str]:
    """Identify constructs outside the bounded lexical diagnostic contract.

    Args:
        tree_node (ast.Module): Parsed source.
    Returns:
        list[str]: Stable reasons to suppress uncertain RC2 diagnostics.
    Warnings:
        Absence of these patterns does not establish runtime equivalence.
    """
    limits_set: set[str] = set()
    for node in ast.walk(tree_node):
        if isinstance(node, ast.ImportFrom) and any(
            alias_node.name == "*" for alias_node in node.names
        ):
            limits_set.add(f"Wildcard import at line {node.lineno}.")
        if isinstance(node, ast.Name) and node.id in DYNAMIC_NAMES_SET and (
            isinstance(node.ctx, ast.Load)
        ):
            limits_set.add(f"Dynamic namespace access at line {node.lineno}.")
        if isinstance(node, ast.TypeAlias) or getattr(node, "type_params", []):
            limits_set.add(f"Type parameter scope at line {node.lineno}.")
    return sorted(limits_set)


def is_bound_bool(symbol_info: symtable.Symbol) -> bool:
    """Tell whether the compiler records an actual binding candidate.

    Args:
        symbol_info (symtable.Symbol): Compiler name flags.
    Returns:
        bool: Assignment, import, parameter or definition evidence.
    Warnings:
        Bindings may be conditional, deleted or annotation-only.
    """
    return (symbol_info.is_assigned() or symbol_info.is_imported()
            or symbol_info.is_parameter() or symbol_info.is_namespace())


def resolve_binding_tuple(collector_info: ScopeCollector,
                          table_info: symtable.SymbolTable,
                          name_str: str) -> tuple[str, int | None]:
    """Resolve a compiler name to a lexical owner or explicit unknown.

    Args:
        collector_info (ScopeCollector): Compiler tables and parents.
        table_info (symtable.SymbolTable): Scope containing the load.
        name_str (str): Compiler identifier, including private mangling.
    Returns:
        tuple[str, int | None]: Resolution category and owner identity.
    Warnings:
        Runtime globals injection and call timing are not resolved.
    """
    try:
        symbol_info = table_info.lookup(name_str)
    except KeyError:
        symbol_info = None
    if symbol_info is not None and symbol_info.is_local() and (
        is_bound_bool(symbol_info)
    ):
        return "local", table_info.get_id()
    if symbol_info is not None and symbol_info.is_free():
        owner_int = find_closure_owner_int(
            collector_info, table_info, name_str)
        if owner_int is not None:
            return "closure", owner_int
        if name_str == "__class__":
            return "implicit_class", None
    root_info = collector_info.root_info
    if name_str in collector_info.walrus_globals_set:
        return "module", root_info.get_id()
    if name_str in root_info.get_identifiers() and is_bound_bool(
        root_info.lookup(name_str)
    ):
        return "module", root_info.get_id()
    if name_str in vars(builtins) or name_str in IMPLICIT_NAMES_SET:
        return "builtin_or_implicit", None
    return "unresolved", None


def collect_lexical_dict(text_str: str, path_str: str,
                         tree_node: ast.Module) -> dict:
    """Build repeatable scope records and located reads for one source.

    Args:
        text_str (str): Decoded Python source.
        path_str (str): Source path used in diagnostics.
        tree_node (ast.Module): Already validated source tree.
    Returns:
        dict: Read, import and scope records with explicit limitations.
    Warnings:
        Compiler table identities are internal and never serialized.
    """
    report_dict = {"coverage": "partial", "initialization_verified": False,
                   "limitations": [FLOW_LIMIT_STR], "reads": [], "imports": [],
                   "scopes": [], "diagnostics_supported": False}
    limits_list = collect_scope_limits_list(tree_node)
    if limits_list:
        report_dict["limitations"].extend(limits_list)
        return report_dict
    collector_info = ScopeCollector(text_str, path_str)
    try:
        visit_deeply_none(collector_info, tree_node)
    except ValueError as error:
        report_dict["limitations"].append(str(error))
        return report_dict
    return build_lexical_report_dict(collector_info, text_str, path_str,
                                     tree_node, report_dict)


def build_lexical_report_dict(collector_info: ScopeCollector, text_str: str,
                              path_str: str, tree_node: ast.Module,
                              report_dict: dict) -> dict:
    """Serialize lexical evidence using deterministic scope identifiers.

    Args:
        collector_info (ScopeCollector): Completed compiler visitor.
        text_str (str): Original source.
        path_str (str): Source label.
        tree_node (ast.Module): Source tree.
        report_dict (dict): Initial coverage and limitation fields.
    Returns:
        dict: Completed source-linked report.
    Warnings:
        String mentions conservatively retain imports for forward types.
    """
    identities_dict = {identity_int: index_int for index_int, identity_int
                       in enumerate(collector_info.tables_dict)}
    source_info = build_source_file(path_str, text_str)
    used_set: set[tuple[int, str]] = set()
    for read_tuple in collector_info.reads_list:
        node, table_info, name_str, annotation_bool, isolated_bool = read_tuple
        category_str, owner_int = resolve_binding_tuple(
            collector_info, table_info, name_str)
        if isolated_bool:
            category_str, owner_int = "comprehension", None
        if owner_int is not None:
            used_set.add((owner_int, name_str))
        line_int, column_int = locate_node_tuple(source_info, node)
        report_dict["reads"].append({"name": node.id, "line": line_int,
            "column": column_int,
            "scope": identities_dict[table_info.get_id()],
            "resolution": category_str, "annotation": annotation_bool})
    report_dict["scopes"] = collect_scope_records_list(collector_info,
                                                      identities_dict)
    report_dict["imports"] = collect_import_records_list(
        collector_info, source_info, tree_node, used_set)
    report_dict["diagnostics_supported"] = True
    return report_dict


def collect_import_records_list(collector_info: ScopeCollector,
                                 source_info: object, tree_node: ast.Module,
                                 used_set: set[tuple[int, str]]) -> list:
    """Describe imported aliases and conservative usage evidence.

    Args:
        collector_info (ScopeCollector): Alias and compiler evidence.
        source_info (object): Source position converter input.
        tree_node (ast.Module): Original source tree.
        used_set (set[tuple[int, str]]): Lexical binding owners read by code.
    Returns:
        list: Located aliases with usage and exemption flags.
    Warnings:
        String mentions and re-exports suppress unused-import suggestions.
    """
    mentioned_set = {word_str for node in ast.walk(tree_node)
                     if isinstance(node, ast.Constant)
                     and isinstance(node.value, str)
                     for word_str in re.findall(r"\w+", node.value)}
    mentioned_set.update(collect_type_comment_names_set(source_info.text))
    records_list = []
    for node, table_info, name_str, export_bool in collector_info.imports_list:
        line_int, column_int = locate_node_tuple(source_info, node)
        try:
            symbol_info = table_info.lookup(name_str)
        except KeyError:
            continue
        used_bool = ((table_info.get_id(), name_str) in used_set
                     or symbol_info.is_referenced())
        records_list.append({"name": name_str, "line": line_int,
            "column": column_int, "used": used_bool,
            "exempt": (export_bool or name_str in mentioned_set
                       or symbol_info.is_assigned())})
    return records_list


def analyze_lexical_dict(text_str: str, path_str: str = "<source>") -> dict:
    """Expose compiler scope evidence without executing checked code.

    Args:
        text_str (str): Python source.
        path_str (str): Source label.
    Returns:
        dict: Stable partial analysis with source locations and limitations.
    Warnings:
        Syntax failures propagate; no complete data-flow proof is claimed.
    """
    tree_node = parse_quietly_node(text_str, path_str)
    return collect_lexical_dict(text_str, path_str, tree_node)


def check_lexical_none(context_info: RuleContext) -> None:
    """Report unresolved loads and imports without lexical uses.

    Args:
        context_info (RuleContext): Compiled source and selected RC rules.
    Returns:
        None: Adds located observations using normal suppression policy.
    Warnings:
        Import side effects preclude automatic removal from these findings.
    """
    if not any(context_info.is_enabled_bool(code_str)
               for code_str in ("RC201", "RC202")):
        return
    source_info = context_info.source_info
    report_dict = collect_lexical_dict(source_info.text, source_info.path,
                                       context_info.tree)
    for read_dict in report_dict["reads"]:
        if read_dict["resolution"] == "unresolved" and not (
            read_dict["annotation"]
        ):
            context_info.report_none("RC201", (read_dict["line"],
                read_dict["column"]), f"No lexical binding found for "
                f"'{read_dict['name']}'; dynamic availability is unverified.")
    path_info = Path(source_info.path)
    if path_info.name == "__init__.py" or path_info.suffix == ".pyi":
        return
    for import_dict in report_dict["imports"]:
        if not import_dict["used"] and not import_dict["exempt"]:
            context_info.report_none("RC202", (import_dict["line"],
                import_dict["column"]), f"Import '{import_dict['name']}' has "
                "no lexical read; review exports and side effects.")


def find_closure_owner_int(collector_info: ScopeCollector,
                            table_info: symtable.SymbolTable,
                            name_str: str) -> int | None:
    """Find an enclosing function that owns a compiler free variable.

    Args:
        collector_info (ScopeCollector): Scope ancestry.
        table_info (symtable.SymbolTable): Reading scope.
        name_str (str): Compiler identifier.
    Returns:
        int | None: Owner identity if a function binding is found.
    Warnings:
        Class namespaces are excluded from ordinary closure lookup.
    """
    parent_info = collector_info.parents_dict.get(table_info.get_id())
    while parent_info is not None:
        if parent_info.get_type() == "function":
            if name_str in parent_info.get_identifiers() and (
                parent_info.lookup(name_str).is_local()
            ):
                return parent_info.get_id()
        parent_info = collector_info.parents_dict.get(parent_info.get_id())
    return None


def collect_scope_records_list(collector_info: ScopeCollector,
                                identities_dict: dict) -> list:
    """Serialize compiler scope summaries using stable local identities.

    Args:
        collector_info (ScopeCollector): Collected compiler tables.
        identities_dict (dict): Internal identity to stable integer mapping.
    Returns:
        list: Lexical scope names, locations, kinds and symbols.
    Warnings:
        Listed symbols do not guarantee initialized runtime values.
    """
    return [{"id": identities_dict[identity_int],
        "name": table_info.get_name(), "line": table_info.get_lineno(),
        "kind": table_info.get_type(),
        "symbols": sorted(table_info.get_identifiers())}
        for identity_int, table_info in collector_info.tables_dict.items()]


def collect_type_comment_names_set(text_str: str) -> set[str]:
    """Preserve imports referenced by real Python type comments.

    Args:
        text_str (str): Compilable source.
    Returns:
        set[str]: Identifier words in comment tokens beginning with type:.
    Warnings:
        This is conservative usage evidence, not type resolution.
    """
    normalized_str = text_str.replace("\r\n", "\n").replace("\r", "\n")
    return {name_str for token_info in tokenize.generate_tokens(
        StringIO(normalized_str).readline)
        if token_info.type == tokenize.COMMENT
        and re.match(r"#\s*type:", token_info.string)
        for name_str in re.findall(r"\w+", token_info.string)}
