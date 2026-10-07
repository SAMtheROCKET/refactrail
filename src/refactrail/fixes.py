"""Bounded rewrite proposals: annotations here, other edits via FuncLoom."""

from dataclasses import dataclass, field
from io import StringIO
import ast
import tokenize

from funcloom import refine_source_text
from funcloom.doc_facts import infer_returned_type_str

from refactrail.facts import FUNCTION_NODES_TUPLE
from refactrail.main_fixes import move_into_main_tuple
from refactrail.models import Settings, is_code_enabled_bool
from refactrail.naming_fixes import apply_naming_fixes_tuple

STUB_STATEMENTS_TUPLE = (ast.Pass, ast.Raise)


@dataclass
class FixOutcome:
    """Describe the fixes applied to one file.

    Args:
        text: Fixed text (the input when nothing applied).
        applied: One line per applied fix group.
        notes: Explanations, including refused fixes.
    Returns:
        FixOutcome: Plain data for reporting.
    Warnings:
        Compilation and structural checks do not prove behavior preservation.
    """

    text: str
    applied: list[str] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)


def is_none_returning_bool(node: ast.AST) -> bool:
    """Tell whether a function certainly returns None.

    Args:
        node (ast.AST): Function node without a return annotation.
    Returns:
        bool: True when it never returns a value and never yields, and
            is neither a stub nor an abstract method.
    Warnings:
        Stubs and abstract methods may be overridden to return values.
    """
    body_list = [statement_node for statement_node in node.body
                 if not (isinstance(statement_node, ast.Expr)
                         and isinstance(statement_node.value, ast.Constant))]
    if not body_list or all(isinstance(statement_node, STUB_STATEMENTS_TUPLE)
                            for statement_node in body_list):
        return False
    if node.decorator_list:
        return False
    pending_list: list[ast.AST] = list(node.body)
    while pending_list:
        current_node = pending_list.pop()
        if isinstance(current_node, (ast.Yield, ast.YieldFrom)) or (
            isinstance(current_node, ast.Return)
            and current_node.value is not None
        ):
            return False
        if not isinstance(current_node, (*FUNCTION_NODES_TUPLE,
                                         ast.ClassDef, ast.Lambda)):
            pending_list.extend(ast.iter_child_nodes(current_node))
    return True


def find_header_colons_dict(source_str: str) -> dict[tuple[int, int], tuple]:
    """Map each def keyword position to the colon ending its header.

    Args:
        source_str (str): Module text.
    Returns:
        dict: (line, column) of 'def' to (line, column) of its ':' as
            tokenize reports them (1-based lines, 0-based columns).
    Warnings:
        Columns are character offsets, unlike ast byte offsets.
    """
    colons_dict, open_list = {}, []
    depth_int = 0
    for token_info in tokenize.generate_tokens(StringIO(source_str).readline):
        if token_info.type == tokenize.NAME and token_info.string == "def":
            open_list.append((token_info.start, depth_int))
        elif token_info.type == tokenize.OP:
            if token_info.string in "([{":
                depth_int += 1
            elif token_info.string in ")]}":
                depth_int -= 1
            elif token_info.string == ":" and open_list and (
                open_list[-1][1] == depth_int
            ):
                colons_dict[open_list.pop()[0]] = token_info.start
    return colons_dict


def choose_return_annotation_str(node: ast.AST) -> str:
    """The return annotation a function's body makes certain, or "".

    Args:
        node (ast.AST): Function without a return annotation.
    Returns:
        str: "None" when it never returns a value; "list", "dict",
            "set", "tuple" or "str" when its only value-returning
            statement is its last one and returns a literal of that
            type, or a name only ever assigned literals of that type.
    Warnings:
        Decorated functions are skipped (decorators may change it).
    """
    if node.decorator_list:
        return ""
    if is_none_returning_bool(node):
        return "None"
    returns_list = [child_node for child_node in ast.walk(node)
                    if isinstance(child_node, (ast.Return, ast.Yield,
                                               ast.YieldFrom, ast.Lambda,
                                               *FUNCTION_NODES_TUPLE))
                    and child_node is not node]
    if len(returns_list) != 1 or returns_list[0] is not node.body[-1]:
        return ""
    return infer_returned_type_str(node)


def add_none_returns_tuple(source_str: str) -> tuple[str, list[str]]:
    """Add certain return annotations (RT402): None, list, dict, ...

    Args:
        source_str (str): Module text.
    Returns:
        tuple[str, list[str]]: New text and annotated function names; the
            input and [] when nothing applies or verification fails.
    Warnings:
        The result must parse to the original tree plus exactly these
        annotations, or it is discarded.
    """
    tree = ast.parse(source_str)
    targets_list = [(node, choose_return_annotation_str(node))
                    for node in ast.walk(tree)
                    if isinstance(node, FUNCTION_NODES_TUPLE)
                    and node.returns is None]
    targets_list = [(node, annotation_str) for node, annotation_str
                    in targets_list if annotation_str]
    if not targets_list:
        return source_str, []
    new_str = insert_annotations_str(source_str, targets_list)
    if not new_str or ast.dump(ast.parse(new_str)) != ast.dump(tree):
        return source_str, []
    return new_str, [f"{node.name} -> {annotation_str}"
                     for node, annotation_str in targets_list]


def insert_annotations_str(source_str: str,
                           targets_list: list[tuple[ast.AST, str]]) -> str:
    """Write return annotations before each function header's colon.

    Args:
        source_str (str): Module text.
        targets_list (list[tuple]): (function node, annotation) pairs;
            each node's returns is set to match, for verification.
    Returns:
        str: The new text, or "" when a header colon is not found.
    Warnings:
        Columns are converted from AST bytes to tokenize characters.
    """
    colons_dict = find_header_colons_dict(source_str)
    lines_list = source_str.splitlines(keepends=True)
    edits_list = []
    for node, annotation_str in targets_list:
        line_str = lines_list[node.lineno - 1]
        column_int = len(line_str.encode("utf-8")[:node.col_offset].decode(
            "utf-8", "ignore"))
        def_column_int = line_str.index("def", column_int)
        colon_tuple = colons_dict.get((node.lineno, def_column_int))
        if colon_tuple is None:
            return ""
        edits_list.append((*colon_tuple, annotation_str))
        node.returns = (ast.Constant(None) if annotation_str == "None"
                        else ast.Name(annotation_str, ast.Load()))
    for line_int, column_int, annotation_str in sorted(edits_list,
                                                       reverse=True):
        line_str = lines_list[line_int - 1]
        lines_list[line_int - 1] = (line_str[:column_int]
                                    + f" -> {annotation_str}"
                                    + line_str[column_int:])
    return "".join(lines_list)


def apply_safe_fixes(
    source_str: str, path_str: str, settings_info: Settings,
) -> FixOutcome:
    """Apply enabled bounded rewrites to one module's text.

    Args:
        source_str (str): Module text (must parse).
        path_str (str): Path used in notes.
        settings_info (Settings): Enabled codes and limits.
    Returns:
        FixOutcome: Fixed text and what was applied.
    Warnings:
        Docstring skeletons contain placeholders that a person must
        replace with real descriptions.
    """
    compile(source_str, path_str, "exec", dont_inherit=True)
    outcome = FixOutcome(source_str)
    outcome.text, renames_list, rename_notes_list = apply_naming_fixes_tuple(
        outcome.text, path_str, is_code_enabled_bool(settings_info, "RT102"),
        is_code_enabled_bool(settings_info, "RT201"))
    if renames_list:
        outcome.applied.append(f"RT102/RT201: renamed "
                               f"{', '.join(renames_list)}")
    outcome.notes += rename_notes_list
    if is_code_enabled_bool(settings_info, "RT504"):
        apply_main_fix_none(outcome, path_str)
    if is_code_enabled_bool(settings_info, "RT402"):
        outcome.text, names_list = add_none_returns_tuple(outcome.text)
        if names_list:
            outcome.applied.append(f"RT402: return types added: "
                                   f"{', '.join(names_list)}")
    apply_funcloom_fixes_none(outcome, path_str, settings_info)
    compile(outcome.text, path_str, "exec", dont_inherit=True)
    return outcome


def apply_main_fix_none(outcome: FixOutcome, path_str: str) -> None:
    """Move a script's top-level code into main() (RT504).

    Args:
        outcome (FixOutcome): Fix state, updated in place.
        path_str (str): The file's path.
    Returns:
        None: Updates outcome.text and applied, or adds a note.
    Warnings:
        Importing the file no longer runs the moved code.
    """
    outcome.text, reason_str = move_into_main_tuple(outcome.text, path_str)
    if reason_str:
        outcome.notes.append(f"RT504: code kept at module level: "
                             f"{reason_str}")
    else:
        outcome.applied.append("RT504: top-level code moved into main(), "
                               "run from a main guard")


def apply_funcloom_fixes_none(
    outcome: FixOutcome, path_str: str, settings_info: Settings,
) -> None:
    """Add docstring skeletons, split long functions and wrap lines.

    Args:
        outcome (FixOutcome): Fix state, updated in place.
        path_str (str): Path used in notes.
        settings_info (Settings): Enabled codes and limits.
    Returns:
        None: Updates outcome.text, applied and notes.
    Warnings:
        FuncLoom runs structural checks; target-project tests remain required.
    """
    refined = refine_source_text(
        outcome.text, path_str, line_length_int=settings_info.line_length,
        function_target_lines_int=settings_info.function_preferred_lines,
        split_functions_bool=is_code_enabled_bool(settings_info, "RT501")
        or is_code_enabled_bool(settings_info, "RT502"),
        document_bool=is_code_enabled_bool(settings_info, "RT301"),
        wrap_lines_bool=is_code_enabled_bool(settings_info, "RT101"))
    if refined.diagnostics:
        outcome.notes += [diagnostic_info.message
                          for diagnostic_info in refined.diagnostics]
        return
    outcome.text = refined.text
    for code_str, names_list in (("RT301: docstrings added to",
                                  refined.documented_functions),
                                 ("RT501/RT502: split",
                                  refined.split_functions)):
        if names_list:
            # FuncLoom prefixes each name with the file; the line already
            # starts with it.
            short_list = [name_str.rsplit(": ", 1)[-1]
                          for name_str in names_list]
            outcome.applied.append(f"{code_str} {', '.join(short_list)}")
    outcome.notes += refined.notes
