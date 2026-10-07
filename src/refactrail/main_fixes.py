"""RT504: move a script's top-level code into main() behind a main guard.

Only the trailing block of executable statements after the last import,
function and class moves; constants (names bound once to a literal) and
the module docstring stay at module level. The fix is refused when:

- the file is part of a package (an __init__.py beside it), or already
  has a main() or a main guard;
- a function or class reads a name the moved code binds (it would stop
  being a module global);
- the module uses global, nonlocal, eval, exec, globals(), locals(),
  vars() or a star import;
- a moved statement holds a multi-line string (indenting would change
  its value) or there is nothing to move.

The result must parse to the original statements plus the new main()
holding exactly the moved statements and the guard calling it.
"""

import ast
from pathlib import Path
import symtable

from refactrail.facts import find_constant_candidates_dict
from refactrail.naming_fixes import check_dynamic_bool, list_tables_list

DEFINITION_NODES_TUPLE = (ast.FunctionDef, ast.AsyncFunctionDef,
                          ast.ClassDef, ast.Import, ast.ImportFrom)
MAIN_BLOCK_STR = '''

def main() -> None:
    """Run the script.

    Returns:
        None: The script's results are its printed output and files.
    """
{body}


if __name__ == "__main__":
    main()
'''


def find_moved_statements_list(tree_node: ast.Module) -> list[ast.stmt]:
    """The trailing executable statements that may move into main().

    Args:
        tree_node (ast.Module): The module.
    Returns:
        list[ast.stmt]: Statements after the last import, function or
            class, without leading constants; [] when a statement before
            them is executable too (the code is interleaved).
    Warnings:
        Constants are recognised as RT102 recognises them.
    """
    body_list = tree_node.body
    if body_list and isinstance(body_list[0], ast.Expr) and isinstance(
            body_list[0].value, ast.Constant):
        body_list = body_list[1:]
    last_definition_int = max(
        (index_int for index_int, node in enumerate(body_list)
         if isinstance(node, DEFINITION_NODES_TUPLE)), default=-1)
    constants_set = set(find_constant_candidates_dict(tree_node).values())
    before_list = body_list[:last_definition_int + 1]
    if any(not isinstance(node, DEFINITION_NODES_TUPLE)
           and node not in constants_set for node in before_list):
        return []
    moved_list = list(body_list[last_definition_int + 1:])
    while moved_list and moved_list[0] in constants_set:
        moved_list.pop(0)
    return moved_list


def find_refusal_str(tree_node: ast.Module, text_str: str, path_str: str,
                     moved_list: list[ast.stmt]) -> str:
    """Why the move would be unsafe, or "".

    Args:
        tree_node (ast.Module): The module.
        text_str (str): Its text.
        path_str (str): Its path.
        moved_list (list[ast.stmt]): The statements to move.
    Returns:
        str: The reason; "" when the move is safe.
    Warnings:
        Readers in other files are not searched; package modules are
        refused instead.
    """
    if not moved_list:
        return "no trailing top-level code to move"
    if (Path(path_str).parent / "__init__.py").exists():
        return "the file is part of a package"
    if any(isinstance(node, ast.If) and "__main__" in ast.unparse(node.test)
           or getattr(node, "name", "") == "main" for node in tree_node.body):
        return "the file already has main() or a main guard"
    if check_dynamic_bool(tree_node):
        return "the module uses global, eval, globals() or a star import"
    if any(isinstance(node, (ast.Constant, ast.JoinedStr))
           and node.end_lineno != node.lineno
           for statement in moved_list for node in ast.walk(statement)):
        return "a moved statement holds a multi-line string"
    shared_set = find_shared_names_set(text_str, moved_list)
    if shared_set:
        return (f"a function reads {', '.join(sorted(shared_set))}, which "
                "the moved code sets")
    return ""


def find_shared_names_set(text_str: str,
                          moved_list: list[ast.stmt]) -> set[str]:
    """Names the moved code sets that a function or class reads.

    Args:
        text_str (str): Module text.
        moved_list (list[ast.stmt]): The statements to move.
    Returns:
        set[str]: Names that would stop being module globals.
    Warnings:
        Python's own symbol tables decide what each scope reads.
    """
    bound_set = {node.id for statement in moved_list
                 for node in ast.walk(statement)
                 if isinstance(node, ast.Name)
                 and isinstance(node.ctx, ast.Store)}
    table_info = symtable.symtable(text_str, "<main>", "exec")
    return bound_set & {
        name_str for child_info in list_tables_list(table_info)[1:]
        for name_str in child_info.get_identifiers()
        if not child_info.lookup(name_str).is_local()}


def move_into_main_tuple(text_str: str, path_str: str
                         ) -> tuple[str, str]:
    """Move a script's trailing top-level code into main() (RT504).

    Args:
        text_str (str): Module text with "\\n" line endings.
        path_str (str): Its path.
    Returns:
        tuple: The new text and "" when applied; the input and the
            reason when refused.
    Warnings:
        Importing the module no longer runs that code; running it as a
        script does, through the guard.
    """
    tree_node = ast.parse(text_str)
    moved_list = find_moved_statements_list(tree_node)
    reason_str = find_refusal_str(tree_node, text_str, path_str, moved_list)
    if reason_str:
        return text_str, reason_str
    lines_list = text_str.splitlines(keepends=True)
    first_int = moved_list[0].lineno - 1
    while first_int > 0 and lines_list[first_int - 1].lstrip().startswith(
            "#"):
        first_int -= 1
    body_str = "".join(("    " + line_str) if line_str.strip() else line_str
                       for line_str in lines_list[first_int:]).rstrip()
    head_str = "".join(lines_list[:first_int]).rstrip() + "\n"
    new_str = head_str + MAIN_BLOCK_STR.format(body=body_str)
    if not is_moved_exactly_bool(tree_node, new_str, moved_list):
        return text_str, "the check after moving failed"
    return new_str, ""


def is_moved_exactly_bool(old_tree: ast.Module, new_str: str,
                          moved_list: list[ast.stmt]) -> bool:
    """Whether the new text is the old module with the code in main().

    Args:
        old_tree (ast.Module): The original module.
        new_str (str): The rewritten text.
        moved_list (list[ast.stmt]): The statements that moved.
    Returns:
        bool: True when the module keeps every other statement in order,
            main() holds exactly the moved statements (after its
            docstring) and a guard calls it.
    Warnings:
        None.
    """
    try:
        new_tree = ast.parse(new_str)
        compile(new_tree, "<main>", "exec", dont_inherit=True)
    except SyntaxError:
        return False
    kept_list = old_tree.body[:len(old_tree.body) - len(moved_list)]
    main_node, guard_node = new_tree.body[-2], new_tree.body[-1]
    return (isinstance(main_node, ast.FunctionDef)
            and [ast.dump(node) for node in new_tree.body[:-2]]
            == [ast.dump(node) for node in kept_list]
            and [ast.dump(node) for node in main_node.body[1:]]
            == [ast.dump(node) for node in moved_list]
            and ast.unparse(guard_node) == (
                "if __name__ == '__main__':\n    main()"))
