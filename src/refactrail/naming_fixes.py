"""Safe renames for RT102 (constants) and RT201 (single-character names).

A constant bound once at module level becomes UPPER_CASE everywhere in
the file. A single-character loop or with target gets a name derived
from what it holds: ``for row in rows``, ``for index in range(...)``,
``with open(...) as file_handle``. Every rename is refused when
the new name is already used, when a function shadows the old name,
when the module reads its namespace dynamically (eval, exec, globals(),
locals(), vars(), __all__, star imports) or when the rewritten file
does not map back to the original syntax tree name for name.
"""

import ast
import builtins
from copy import deepcopy
from pathlib import Path
import re
import symtable

from refactrail.facts import UPPER_CASE_PATTERN, find_constant_candidates_dict

DYNAMIC_NAMES_FROZENSET = frozenset(("eval", "exec", "globals", "locals",
                                     "vars", "__all__", "__import__"))
SCOPE_NODES_TUPLE = (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef,
                     ast.Lambda, ast.ListComp, ast.SetComp, ast.DictComp,
                     ast.GeneratorExp)
# Loop targets over these calls get these names.
CALL_TARGETS_DICT = {"range": "index", "listdir": "file_name",
                     "DictReader": "row", "reader": "row", "keys": "key",
                     "values": "value", "readlines": "line",
                     "iterdir": "path", "glob": "path", "rglob": "path"}
IRREGULAR_SINGULARS_DICT = {"data": "record", "people": "person",
                            "children": "child", "indices": "index",
                            "series": "series", "statuses": "status",
                            "readers": "reader", "reader": "row",
                            "lines": "line", "matrices": "matrix"}


def find_singular_str(plural_str: str) -> str:
    """The singular of a plural variable name, or "".

    Args:
        plural_str (str): A name such as "rows", "entries" or "boxes".
    Returns:
        str: "row", "entry", "box"; "" when the name is not plural.
    Warnings:
        English plurals only; the result is checked for collisions by
        the caller.
    """
    last_str = plural_str.split("_")[-1]
    prefix_str = plural_str[:len(plural_str) - len(last_str)]
    if last_str in IRREGULAR_SINGULARS_DICT:
        return prefix_str + IRREGULAR_SINGULARS_DICT[last_str]
    if last_str.endswith(("us", "is", "ss")):
        return ""  # status, analysis, class: not plurals
    for suffix_str, replacement_str in (("ies", "y"), ("sses", "ss"),
                                        ("xes", "x"), ("ches", "ch"),
                                        ("shes", "sh"), ("s", "")):
        if last_str.endswith(suffix_str) and len(last_str) > len(
                suffix_str) + 1 and not last_str.endswith("ss"):
            return prefix_str + last_str[:-len(suffix_str)] + replacement_str
    return ""


def suggest_loop_name_str(iterable_node: ast.expr) -> str:
    """A name for the target of a loop over an expression, or "".

    Args:
        iterable_node (ast.expr): What the loop iterates over.
    Returns:
        str: "row" for rows, "index" for range(...), and so on.
    Warnings:
        Only plain names, attributes and well-known calls are named.
    """
    if isinstance(iterable_node, ast.Call):
        function_node = iterable_node.func
        called_str = (function_node.attr if isinstance(
            function_node, ast.Attribute) else getattr(function_node, "id",
                                                       ""))
        return CALL_TARGETS_DICT.get(called_str, "")
    if isinstance(iterable_node, ast.Attribute):
        return find_singular_str(iterable_node.attr)
    if isinstance(iterable_node, ast.Name):
        return find_singular_str(iterable_node.id)
    return ""


def list_short_bindings_list(scope_node: ast.AST) -> list[tuple[str, str]]:
    """Single-character bindings of a scope with a suggested name.

    Args:
        scope_node (ast.AST): A module or function.
    Returns:
        list[tuple[str, str]]: (old name, new name) pairs, first binding
            wins, for loop targets and open() handles.
    Warnings:
        Nested scopes are not searched; other bindings are left alone.
    """
    pairs_list: list[tuple[str, str]] = []
    pending_list = list(scope_node.body)
    while pending_list:
        node = pending_list.pop(0)
        if isinstance(node, SCOPE_NODES_TUPLE):
            continue
        pending_list.extend(ast.iter_child_nodes(node))
        old_str, new_str = "", ""
        if isinstance(node, (ast.For, ast.AsyncFor)) and isinstance(
                node.target, ast.Name):
            old_str = node.target.id
            new_str = suggest_loop_name_str(node.iter)
        elif isinstance(node, ast.withitem) and isinstance(
                node.optional_vars, ast.Name) and isinstance(
                    node.context_expr, ast.Call) and getattr(
                        node.context_expr.func, "id", "") == "open":
            old_str, new_str = node.optional_vars.id, "file_handle"
        if len(old_str) == 1 and old_str != "_" and new_str and all(
                old_str != known_str for known_str, _ in pairs_list):
            pairs_list.append((old_str, new_str))
    return pairs_list


def check_dynamic_bool(tree_node: ast.Module) -> bool:
    """Whether the module reads its namespace in ways renames break.

    Args:
        tree_node (ast.Module): The module.
    Returns:
        bool: True for eval, exec, globals(), locals(), vars(),
            __all__, __import__ or a star import.
    Warnings:
        String-based access through getattr is not detected.
    """
    return any(
        (isinstance(node, ast.Name) and node.id in DYNAMIC_NAMES_FROZENSET)
        or (isinstance(node, ast.ImportFrom)
            and any(alias.name == "*" for alias in node.names))
        or isinstance(node, (ast.Global, ast.Nonlocal))
        for node in ast.walk(tree_node))


def list_all_identifiers_set(table_info: symtable.SymbolTable) -> set[str]:
    """Every identifier of a symbol table and its children.

    Args:
        table_info (symtable.SymbolTable): A table.
    Returns:
        set[str]: Identifiers used anywhere in it.
    Warnings:
        None.
    """
    found_set = set(table_info.get_identifiers())
    for child_info in table_info.get_children():
        found_set |= list_all_identifiers_set(child_info)
    return found_set


def is_shadowed_bool(table_info: symtable.SymbolTable, name_str: str) -> bool:
    """Whether a nested scope binds the module-level name itself.

    Args:
        table_info (symtable.SymbolTable): The module table.
        name_str (str): The module-level name.
    Returns:
        bool: True when some function, class or comprehension assigns
            it, takes it as a parameter or imports it.
    Warnings:
        None.
    """
    pending_list = list(table_info.get_children())
    while pending_list:
        child_info = pending_list.pop()
        pending_list.extend(child_info.get_children())
        if name_str in child_info.get_identifiers():
            symbol_info = child_info.lookup(name_str)
            if symbol_info.is_assigned() or symbol_info.is_parameter() or (
                    symbol_info.is_imported()):
                return True
    return False


def replace_names_str(text_str: str, nodes_list: list[ast.Name],
                      new_str: str) -> str:
    """Replace the exact source spans of Name nodes.

    Args:
        text_str (str): Source text with "\\n" line endings.
        nodes_list (list[ast.Name]): The names to replace.
        new_str (str): Their new identifier.
    Returns:
        str: The new text; comments, strings and attributes untouched.
    Warnings:
        Offsets are UTF-8 byte offsets, as the AST reports them.
    """
    data_bytes = text_str.encode("utf-8")
    starts_list = [0]
    for line_bytes in re.findall(rb"[^\n]*\n|[^\n]+$", data_bytes):
        starts_list.append(starts_list[-1] + len(line_bytes))
    spans_list = sorted({(starts_list[node.lineno - 1] + node.col_offset,
                          starts_list[node.end_lineno - 1]
                          + node.end_col_offset) for node in nodes_list},
                        reverse=True)
    for start_int, end_int in spans_list:
        data_bytes = (data_bytes[:start_int] + new_str.encode("utf-8")
                      + data_bytes[end_int:])
    return data_bytes.decode("utf-8")


def is_mapped_back_bool(old_tree: ast.Module, new_str: str, old_str: str,
                        new_name_str: str, scope_str: str | None,
                        is_everywhere_bool: bool) -> bool:
    """Whether the new text is the old tree with one name renamed.

    Args:
        old_tree (ast.Module): The original tree.
        new_str (str): The renamed text.
        old_str (str): The old name.
        new_name_str (str): The new name.
        scope_str (str | None): The function renamed in, or None.
        is_everywhere_bool (bool): Whether the rename covered every
            scope (constants) rather than one scope's own statements.
    Returns:
        bool: True when renaming new_name_str back to old_str, in the
            renamed scope only, gives the old tree.
    Warnings:
        None.
    """
    try:
        new_tree = ast.parse(new_str)
    except SyntaxError:
        return False
    scope_node = find_scope_node(new_tree, scope_str)
    if scope_node is None:
        return False
    nodes_list = (list(ast.walk(new_tree)) if is_everywhere_bool
                  else list_own_nodes_list(scope_node))
    for node in nodes_list:
        if isinstance(node, ast.Name) and node.id == new_name_str:
            node.id = old_str
    return ast.dump(new_tree) == ast.dump(old_tree)


def list_own_nodes_list(scope_node: ast.AST) -> list[ast.AST]:
    """A scope's own nodes, nested functions, classes and lambdas aside.

    Args:
        scope_node (ast.AST): A module or function.
    Returns:
        list[ast.AST]: Nodes of its body outside nested scopes.
    Warnings:
        None.
    """
    found_list: list[ast.AST] = []
    pending_list = list(scope_node.body)
    while pending_list:
        node = pending_list.pop()
        if isinstance(node, SCOPE_NODES_TUPLE):
            continue
        found_list.append(node)
        pending_list.extend(ast.iter_child_nodes(node))
    return found_list


def rename_in_scope_str(text_str: str, scope_node: ast.AST,
                        old_str: str, new_str: str) -> str | None:
    """Rename a name in one scope's own statements, nested scopes aside.

    Args:
        text_str (str): Source text.
        scope_node (ast.AST): The module or function.
        old_str (str): The name.
        new_str (str): Its replacement.
    Returns:
        str | None: The new text, or None when the scope also binds
            the name as an except-handler name.
    Warnings:
        Callers check with is_read_inside_bool that no nested scope
        reads the scope's binding.
    """
    names_list: list[ast.Name] = []
    for node in list_own_nodes_list(scope_node):
        if isinstance(node, ast.ExceptHandler) and node.name == old_str:
            return None
        if isinstance(node, ast.Name) and node.id == old_str:
            names_list.append(node)
    return replace_names_str(text_str, names_list, new_str)


def is_read_inside_bool(table_info: symtable.SymbolTable,
                        scope_node: ast.AST, old_str: str) -> bool:
    """Whether a scope nested in the renamed one reads its binding.

    Args:
        table_info (symtable.SymbolTable): The module table.
        scope_node (ast.AST): The module or the function renamed in.
        old_str (str): The name.
    Returns:
        bool: True when a nested function, class or comprehension uses
            the name without binding it itself (a global read or a
            closure), so renaming only the outer scope would break it.
    Warnings:
        None.
    """
    outer_info = table_info if isinstance(scope_node, ast.Module) else next(
        (child_info for child_info in list_tables_list(table_info)
         if child_info.get_type() == "function"
         and child_info.get_lineno() == scope_node.lineno
         and child_info.get_name() == scope_node.name), None)
    if outer_info is None:
        return True
    return any(old_str in child_info.get_identifiers()
               and not child_info.lookup(old_str).is_local()
               for child_info in list_tables_list(outer_info)[1:])


def rename_everywhere_str(text_str: str, tree_node: ast.Module,
                          old_str: str, new_str: str) -> str:
    """Rename a module-level name in every scope that reads it.

    Args:
        text_str (str): Source text.
        tree_node (ast.Module): Its tree.
        old_str (str): The module-level name (not shadowed anywhere).
        new_str (str): Its replacement.
    Returns:
        str: The new text.
    Warnings:
        Callers must have checked shadowing and dynamic access first.
    """
    return replace_names_str(text_str, [
        node for node in ast.walk(tree_node)
        if isinstance(node, ast.Name) and node.id == old_str], new_str)


def check_free_name_bool(table_info: symtable.SymbolTable,
                         scope_node: ast.AST, new_str: str,
                         is_global_bool: bool) -> bool:
    """Whether a new name cannot collide where the renamed name lives.

    Args:
        table_info (symtable.SymbolTable): The module table.
        scope_node (ast.AST): The module or the function renamed in.
        new_str (str): The proposed name.
        is_global_bool (bool): Whether the renamed name is module-level
            (then every scope that could read it must not use the new
            name, except as its own local).
    Returns:
        bool: True when renaming to it cannot collide.
    Warnings:
        Builtins are never reused.
    """
    if not new_str.isidentifier() or hasattr(builtins, new_str) or (
            new_str in table_info.get_identifiers()):
        return False
    if is_global_bool:
        return not any(
            new_str in child_info.get_identifiers()
            and not child_info.lookup(new_str).is_local()
            for child_info in list_tables_list(table_info)[1:])
    function_info = next((
        child_info for child_info in list_tables_list(table_info)
        if child_info.get_type() == "function"
        and child_info.get_lineno() == scope_node.lineno
        and child_info.get_name() == scope_node.name), None)
    return function_info is not None and new_str not in (
        list_all_identifiers_set(function_info))


def list_tables_list(table_info: symtable.SymbolTable
                     ) -> list[symtable.SymbolTable]:
    """A symbol table and all tables nested in it.

    Args:
        table_info (symtable.SymbolTable): The outer table.
    Returns:
        list[symtable.SymbolTable]: The table first, then its children.
    Warnings:
        None.
    """
    tables_list = [table_info]
    for child_info in table_info.get_children():
        tables_list += list_tables_list(child_info)
    return tables_list


def is_used_elsewhere_bool(path_str: str, name_str: str) -> bool:
    """Whether another Python file next to this one mentions a name.

    Args:
        path_str (str): The file being fixed.
        name_str (str): The module-level name.
    Returns:
        bool: True when a sibling .py file contains the name as a word
            (it may import it), so renaming would break that file.
    Warnings:
        Only the file's own folder is searched.
    """
    pattern = re.compile(rf"\b{re.escape(name_str)}\b")
    source_path = Path(path_str)
    for sibling_path in source_path.parent.glob("*.py"):
        if sibling_path.resolve() == source_path.resolve():
            continue
        try:
            if pattern.search(sibling_path.read_text(encoding="utf-8",
                                                     errors="replace")):
                return True
        except OSError:
            return True
    return False


def apply_naming_fixes_tuple(text_str: str, path_str: str,
                             fix_constants_bool: bool,
                             fix_short_names_bool: bool
                             ) -> tuple[str, list[str], list[str]]:
    """Rename constants to UPPER_CASE and single-character names.

    Args:
        text_str (str): Module text with "\\n" line endings.
        path_str (str): Its path (siblings are checked for constants).
        fix_constants_bool (bool): Whether RT102 is enabled.
        fix_short_names_bool (bool): Whether RT201 is enabled.
    Returns:
        tuple: New text, applied renames ("old -> new") and notes about
            renames that were refused.
    Warnings:
        Each rename is verified separately; a refused one leaves the
        text unchanged.
    """
    tree_node = ast.parse(text_str)
    if check_dynamic_bool(tree_node):
        return text_str, [], ["renames skipped: the module uses eval, "
                              "exec, globals(), locals(), vars(), "
                              "__all__, global or a star import"]
    applied_list: list[str] = []
    notes_list: list[str] = []
    if fix_constants_bool:
        text_str = rename_constants_str(text_str, path_str, applied_list,
                                        notes_list)
    if fix_short_names_bool:
        text_str = rename_short_names_str(text_str, applied_list,
                                          notes_list)
    return text_str, applied_list, notes_list


def rename_constants_str(text_str: str, path_str: str,
                         applied_list: list[str],
                         notes_list: list[str]) -> str:
    """Rename every non-UPPER_CASE constant (RT102).

    Args:
        text_str (str): Module text.
        path_str (str): Its path.
        applied_list (list[str]): Applied renames (extended).
        notes_list (list[str]): Refusals (extended).
    Returns:
        str: The new text.
    Warnings:
        Constants of package modules (an __init__.py beside the file)
        and constants another file in the folder mentions are kept:
        other code may import them by name.
    """
    candidates_list = [
        name_str for name_str in find_constant_candidates_dict(
            ast.parse(text_str)) if not UPPER_CASE_PATTERN.match(name_str)]
    if candidates_list and (Path(path_str).parent / "__init__.py").exists():
        notes_list.append("RT102: constants kept: the file is part of a "
                          "package, so other code may import them")
        return text_str
    for old_str in candidates_list:
        new_str = old_str.upper()
        if is_used_elsewhere_bool(path_str, old_str):
            notes_list.append(f"RT102: {old_str} kept: another file in "
                              "the folder mentions it")
            continue
        text_str = try_rename_str(text_str, None, old_str, new_str,
                                  applied_list, notes_list)
    return text_str


def rename_short_names_str(text_str: str, applied_list: list[str],
                           notes_list: list[str]) -> str:
    """Rename single-character loop targets and file handles (RT201).

    Args:
        text_str (str): Module text.
        applied_list (list[str]): Applied renames (extended).
        notes_list (list[str]): Refusals (extended).
    Returns:
        str: The new text.
    Warnings:
        Module-level names and each top-level function or method are
        handled separately; nested functions are left alone.
    """
    tree_node = ast.parse(text_str)
    scopes_list: list[tuple[str | None, ast.AST]] = []
    for node in tree_node.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            scopes_list.append((node.name, node))
        elif isinstance(node, ast.ClassDef):
            scopes_list += [(f"{node.name}.{child.name}", child)
                            for child in node.body if isinstance(
                                child, (ast.FunctionDef,
                                        ast.AsyncFunctionDef))]
    # Functions first: a module-level rename must not collide with names
    # that functions still had before their own renames.
    scopes_list.append((None, tree_node))
    for scope_str, scope_node in scopes_list:
        for old_str, new_str in list_short_bindings_list(scope_node):
            text_str = try_rename_str(text_str, scope_str, old_str, new_str,
                                      applied_list, notes_list)
    return text_str


def find_scope_node(tree_node: ast.Module, scope_str: str | None
                    ) -> ast.AST | None:
    """Find a module, function or method node by its qualified name.

    Args:
        tree_node (ast.Module): The module.
        scope_str (str | None): None for the module, "name" or
            "Class.name".
    Returns:
        ast.AST | None: The node, or None when it is gone.
    Warnings:
        None.
    """
    if scope_str is None:
        return tree_node
    for node in tree_node.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and (
                node.name == scope_str):
            return node
        if isinstance(node, ast.ClassDef) and scope_str.startswith(
                node.name + "."):
            return next((child for child in node.body if getattr(
                child, "name", "") == scope_str.split(".", 1)[1]), None)
    return None


def find_refusal_str(text_str: str, scope_node: ast.AST, old_str: str,
                     new_str: str, is_constant_bool: bool) -> str:
    """Why a rename would be unsafe, or "".

    Args:
        text_str (str): Module text.
        scope_node (ast.AST): The module or function renamed in.
        old_str (str): The name.
        new_str (str): Its replacement.
        is_constant_bool (bool): Whether every scope is renamed.
    Returns:
        str: The reason, such as "row is already used"; "" when safe.
    Warnings:
        The rewritten text is still checked by is_mapped_back_bool.
    """
    table_info = symtable.symtable(text_str, "<rename>", "exec")
    if not check_free_name_bool(table_info, scope_node, new_str,
                                isinstance(scope_node, ast.Module)):
        return f"{new_str} is already used"
    if is_constant_bool and is_shadowed_bool(table_info, old_str):
        return "a function binds the same name"
    if not is_constant_bool and is_read_inside_bool(table_info, scope_node,
                                                    old_str):
        return "a nested function reads it"
    return ""


def try_rename_str(text_str: str, scope_str: str | None, old_str: str,
                   new_str: str, applied_list: list[str],
                   notes_list: list[str]) -> str:
    """Rename one name if every check passes.

    Args:
        text_str (str): Module text.
        scope_str (str | None): None for module level or constants,
            else the function's qualified name.
        old_str (str): The name.
        new_str (str): Its replacement.
        applied_list (list[str]): Applied renames (extended).
        notes_list (list[str]): Refusals (extended).
    Returns:
        str: The renamed text, or the input when refused.
    Warnings:
        A function-scope rename only touches that function's body.
    """
    tree_node = ast.parse(text_str)
    scope_node = find_scope_node(tree_node, scope_str)
    if scope_node is None:
        return text_str
    is_constant_bool = new_str.isupper()
    refusal_str = find_refusal_str(text_str, scope_node, old_str, new_str,
                                   is_constant_bool)
    new_text_str = None if refusal_str else (
        rename_everywhere_str(text_str, tree_node, old_str, new_str)
        if is_constant_bool else rename_in_scope_str(
            text_str, scope_node, old_str, new_str))
    if new_text_str is None or not is_mapped_back_bool(
            deepcopy(tree_node), new_text_str, old_str, new_str, scope_str,
            is_constant_bool):
        reason_str = refusal_str or "the check after renaming failed"
        notes_list.append(f"{old_str} -> {new_str} in "
                          f"{scope_str or 'module level'} skipped: "
                          f"{reason_str}")
        return text_str
    applied_list.append(f"{old_str} -> {new_str}"
                        + (f" in {scope_str}" if scope_str else ""))
    return new_text_str
