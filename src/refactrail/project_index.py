"""Static project inventory and conservative import-impact relationships."""

import ast
from hashlib import sha256
from pathlib import Path

from refactrail.config import load_settings
from refactrail.discovery import discover_python_files_list
from refactrail.engine import parse_quietly_node
from refactrail.source import decode_source_text

PROJECT_LIMITS_LIST = [
    "Import edges are syntax evidence, not resolved runtime imports.",
    "Dynamic imports, search-path changes and monkey patches are unknown.",
    "Symbols and import impact do not prove call graphs or API compatibility.",
]


def collect_symbols_list(tree_node: ast.Module) -> list:
    """Record named definitions with lexical parent names and source spans.

    Args:
        tree_node (ast.Module): Compiled source tree.
    Returns:
        list: Deterministic class/function definition records.
    Warnings:
        Conditional definitions are recorded without reachability claims.
    """
    records_list = []
    pending_list = [(tree_node, "")]
    while pending_list:
        node, parent_str = pending_list.pop()
        if isinstance(node, (
            ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef
        )):
            parent_str = ".".join(
                part_str for part_str in (parent_str, node.name) if part_str)
            records_list.append({"name": parent_str,
                "kind": type(node).__name__, "line": node.lineno,
                "end_line": node.end_lineno})
        children_list = list(ast.iter_child_nodes(node))
        pending_list.extend((child_node, parent_str)
                            for child_node in reversed(children_list))
    return sorted(records_list, key=lambda record_dict: (
        record_dict["line"], record_dict["name"]))


def collect_imports_list(tree_node: ast.Module) -> list:
    """Describe static import requests without importing their modules.

    Args:
        tree_node (ast.Module): Compiled source tree.
    Returns:
        list: Located import requests and explicit alias spellings.
    Warnings:
        Imports inside branches and functions are possible relationships.
    """
    records_list = []
    for node in ast.walk(tree_node):
        if isinstance(node, ast.Import):
            records_list.extend({"module": alias_node.name, "level": 0,
                "names": [], "line": node.lineno}
                for alias_node in node.names)
        elif isinstance(node, ast.ImportFrom):
            records_list.append({"module": node.module or "",
                "level": node.level,
                "names": [entry.name for entry in node.names],
                "line": node.lineno})
    return sorted(records_list, key=lambda record_dict: (
        record_dict["line"], record_dict["module"]))


def read_project_file_dict(path_info: Path, root_path: Path) -> dict:
    """Read and compile one indexed source without executing it.

    Args:
        path_info (Path): Regular Python or stub file.
        root_path (Path): Explicit import root.
    Returns:
        dict: Source hash, definitions, imports and declared module path.
    Warnings:
        Module names assume the root is on sys.path; no runtime resolution.
    """
    raw_bytes = path_info.read_bytes()
    text_str = decode_source_text(raw_bytes)
    if text_str is None:
        raise ValueError(f"Unsupported source encoding: {path_info}")
    try:
        tree_node = parse_quietly_node(text_str, str(path_info))
    except (SyntaxError, ValueError) as error:
        raise ValueError(f"Cannot index {path_info}: {error}") from error
    relative_path = path_info.relative_to(root_path)
    parts_tuple = relative_path.with_suffix("").parts
    package_bool = parts_tuple[-1] == "__init__"
    module_str = ".".join(parts_tuple[:-1] if package_bool else parts_tuple)
    return {"path": relative_path.as_posix(), "module": module_str,
        "package": package_bool,
        "source_sha256": sha256(raw_bytes).hexdigest(),
        "symbols": collect_symbols_list(tree_node),
        "imports": collect_imports_list(tree_node)}


def resolve_import_candidates_list(file_dict: dict, import_dict: dict) -> list:
    """Compute possible local module names for a static import request.

    Args:
        file_dict (dict): Importing module's identity.
        import_dict (dict): Located import request.
    Returns:
        list: Ordered possible module names, including package prefixes.
    Warnings:
        From-import members may be attributes or submodules.
    """
    module_str = import_dict["module"]
    level_int = import_dict["level"]
    if level_int:
        package_list = file_dict["module"].split(".")
        if not file_dict["package"]:
            package_list = package_list[:-1]
        if level_int > len(package_list):
            return []
        parent_list = package_list[:len(package_list) - level_int + 1]
        module_str = ".".join(
            [*parent_list, *([module_str] if module_str else [])])
    names_list = [module_str, *(module_str + "." + name_str
                               for name_str in import_dict["names"]
                               if name_str != "*")]
    return sorted({".".join(name_str.split(".")[:index_int])
                   for name_str in names_list if name_str
                   for index_int in range(1, len(name_str.split(".")) + 1)})


def collect_project_edges_list(files_list: list[dict]) -> list:
    """Link import requests to all matching local files, including stubs.

    Args:
        files_list (list[dict]): Current source inventories.
    Returns:
        list: Source-linked candidate edges with explicit uncertainty.
    Warnings:
        Ambiguous module providers remain multiple candidates.
    """
    modules_dict: dict[str, list[str]] = {}
    for file_dict in files_list:
        modules_dict.setdefault(file_dict["module"], []).append(
            file_dict["path"])
    edges_list = []
    for file_dict in files_list:
        for import_dict in file_dict["imports"]:
            names_list = resolve_import_candidates_list(file_dict, import_dict)
            targets_list = sorted({path_str for name_str in names_list
                                   for path_str in
                                   modules_dict.get(name_str, [])})
            edges_list.append({"source": file_dict["path"],
                "line": import_dict["line"], "request": import_dict["module"],
                "candidates": targets_list,
                "resolution": ("local_candidates" if targets_list
                               else "unresolved")})
    return edges_list


def collect_impact_list(edges_list: list[dict],
                        changed_list: list[str]) -> list:
    """Compute transitive possible importers, including cycles.

    Args:
        edges_list (list[dict]): Current static candidate import edges.
        changed_list (list[str]): Root-relative changed source paths.
    Returns:
        list: Sorted changed files and possibly affected local importers.
    Warnings:
        Dynamic dependencies are absent; this is not a complete impact proof.
    """
    reverse_dict: dict[str, set[str]] = {}
    for edge_dict in edges_list:
        for target_str in edge_dict["candidates"]:
            reverse_dict.setdefault(target_str, set()).add(edge_dict["source"])
    affected_set = set(changed_list)
    pending_list = list(changed_list)
    while pending_list:
        for source_str in reverse_dict.get(pending_list.pop(), set()):
            if source_str not in affected_set:
                affected_set.add(source_str)
                pending_list.append(source_str)
    return sorted(affected_set)


def analyze_project_dict(root_str: str, changed_list: list[str] | None = None
                         ) -> dict:
    """Inventory a Python import root and report possible change impact.

    Args:
        root_str (str): Directory treated as an explicit import root.
        changed_list (list[str] | None): Root-relative paths for impact query.
    Returns:
        dict: Versioned source hashes, symbols, imports and impact evidence.
    Warnings:
        No target is executed or imported; dynamic edges remain unknown.
    """
    requested_path = Path(root_str)
    if requested_path.is_symlink() or requested_path.is_junction():
        raise ValueError("Linked index roots are unsupported")
    root_path = requested_path.resolve()
    if not root_path.is_dir():
        raise ValueError("index requires a Python import-root directory")
    files_list = [read_project_file_dict(Path(path_str), root_path)
                  for path_str in discover_python_files_list(
                      [str(root_path)], load_settings(root_path),
                      (".py", ".pyi"))]
    if not files_list:
        raise ValueError("No Python files to index")
    changed_list = normalize_changed_list(root_path, changed_list or [])
    providers_list = [*files_list, *collect_deleted_providers_list(
        files_list, changed_list)]
    edges_list = collect_project_edges_list(providers_list)
    return {"schema_version": "refactrail-index-1", "root": str(root_path),
        "files": files_list, "edges": edges_list,
        "impact": collect_impact_list(edges_list, changed_list),
        "coverage": "partial", "limitations": PROJECT_LIMITS_LIST.copy(),
        "behavior_verified": False}


def normalize_changed_list(root_path: Path, changed_list: list[str]) -> list:
    """Validate impact requests against the explicit project root.

    Args:
        root_path (Path): Absolute import root.
        changed_list (list[str]): Relative or absolute changed paths.
    Returns:
        list: Normalized relative paths, including deleted source files.
    Warnings:
        Paths escaping the root are rejected.
    """
    normalized_list = []
    for path_str in changed_list:
        candidate_path = (root_path / path_str).resolve()
        if not candidate_path.is_relative_to(root_path):
            raise ValueError(f"Changed path escapes index root: {path_str}")
        normalized_list.append(
            candidate_path.relative_to(root_path).as_posix())
    return sorted(set(normalized_list))


def collect_deleted_providers_list(files_list: list[dict],
                                    changed_list: list[str]) -> list:
    """Retain possible importers of explicitly named deleted modules.

    Args:
        files_list (list[dict]): Current discovered source inventory.
        changed_list (list[str]): Validated root-relative changed paths.
    Returns:
        list: Phantom module identities used only for impact edges.
    Warnings:
        Deleted providers are inferred from paths, not from runtime imports.
    """
    existing_set = {file_dict["path"] for file_dict in files_list}
    providers_list = []
    for path_str in changed_list:
        path_info = Path(path_str)
        if path_str in existing_set or path_info.suffix not in (".py", ".pyi"):
            continue
        parts_tuple = path_info.with_suffix("").parts
        package_bool = parts_tuple[-1] == "__init__"
        providers_list.append({"path": path_str,
            "module": ".".join(
                parts_tuple[:-1] if package_bool else parts_tuple),
            "package": package_bool, "imports": []})
    return providers_list
