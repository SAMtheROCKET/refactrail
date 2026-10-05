"""Flow-ordered name bindings for the Pyflakes-compatible scope codes.

Statements are followed in execution order: module and class code sees
only names bound before it, while function and lambda bodies are checked
after the enclosing module has finished (so they see later definitions),
first in, first out. Each name read is resolved through the scope chain
(class scopes are invisible from nested functions); imports, definitions
and assignments are judged used or unused when their scope ends. Nothing
is imported or executed.
"""

import ast
import builtins
from dataclasses import dataclass, field

MODULE_SCOPE_STR = "module"
CLASS_SCOPE_STR = "class"
FUNCTION_SCOPE_STR = "function"
COMPREHENSION_SCOPE_STR = "comprehension"
TYPE_SCOPE_STR = "type"

# Binding kinds.
IMPORT_KIND_STR = "import"
FUTURE_KIND_STR = "future"
STAR_KIND_STR = "star"
DEFINITION_KIND_STR = "definition"
CLASS_KIND_STR = "class"
ASSIGNMENT_KIND_STR = "assignment"
ANNOTATION_KIND_STR = "annotation"
ARGUMENT_KIND_STR = "argument"
LOOP_KIND_STR = "loop"
HANDLER_KIND_STR = "handler"
DECLARATION_KIND_STR = "declaration"
UNPACKED_KIND_STR = "unpacked"

MODULE_NAMES_FROZENSET = frozenset((
    "__file__", "__name__", "__doc__", "__package__", "__spec__",
    "__loader__", "__builtins__", "__annotations__", "__path__",
    "__cached__", "__dict__", "__debug__", "WindowsError"))
CLASS_NAMES_FROZENSET = frozenset(("__module__", "__qualname__"))
BUILTIN_NAMES_FROZENSET = frozenset(dir(builtins)) | MODULE_NAMES_FROZENSET


@dataclass(eq=False)
class Binding:
    """One binding of a name in a scope.

    Args:
        name: The bound name.
        kind: One of the *_KIND_STR values.
        node: The node to report at (alias, Name, def, arg...).
        branch: The if/try branches the binding sits in, outermost first,
            as (statement id, branch number) pairs.
        full_name: For imports, the imported dotted name.
        is_used: Whether any read resolved to this binding (or, for
            plain assignments, to an earlier binding of the name).
        is_reexport: For imports written "import x as x".
        linked_list: Submodule imports sharing this binding's usage.
        is_from_import: For imports written "from m import x".
        statement: For imports, the import statement.
        scope: The Scope holding the binding.
        is_typing_only: Bound inside an "if TYPE_CHECKING:" block.
    """

    name: str
    kind: str
    node: ast.AST
    branch: tuple = ()
    full_name: str = ""
    is_used: bool = False
    is_reexport: bool = False
    linked_list: list = field(default_factory=list)
    is_from_import: bool = False
    statement: ast.AST | None = None
    scope: object = None
    is_typing_only: bool = False


@dataclass(eq=False)
class Scope:
    """A module, class, function, comprehension or type scope.

    Args:
        kind: One of the *_SCOPE_STR values.
        bindings_dict: The current binding of each name.
        history_list: Every binding ever made here, in order.
        globals_set: Names declared global or nonlocal here.
        star_modules_list: Modules imported with "from m import *".
        uses_locals: Whether locals() is called here.
        nonlocal_dict: Names declared nonlocal -> the scope owning them.
        local_names_set: For functions, every name the body assigns.
    """

    kind: str
    bindings_dict: dict[str, Binding] = field(default_factory=dict)
    history_list: list[Binding] = field(default_factory=list)
    globals_set: set[str] = field(default_factory=set)
    star_modules_list: list[str] = field(default_factory=list)
    uses_locals: bool = False
    nonlocal_dict: dict = field(default_factory=dict)
    local_names_set: set[str] = field(default_factory=set)


def is_branch_conflict_bool(first_tuple: tuple, second_tuple: tuple) -> bool:
    """Whether two branch paths are in different arms of one statement.

    Args:
        first_tuple (tuple): Branch path of one binding.
        second_tuple (tuple): Branch path of another binding.
    Returns:
        bool: True when they part ways inside the same if or try
        statement, so only one of them runs.
    Warnings:
        None.
    """
    for first_pair, second_pair in zip(first_tuple, second_tuple):
        if first_pair == second_pair:
            continue
        return first_pair[0] == second_pair[0]
    return False
