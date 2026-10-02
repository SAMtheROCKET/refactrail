"""Module facts shared by several rules: constants, functions, main block."""

from dataclasses import dataclass
import ast
import re

UPPER_CASE_PATTERN = re.compile(r"^_?[A-Z][A-Z0-9_]*$")
TYPE_ALIAS_NAME_PATTERN = re.compile(r"^_?[A-Z][A-Za-z0-9]*$")
FUNCTION_NODES_TUPLE = (ast.FunctionDef, ast.AsyncFunctionDef)
COMPREHENSION_NODES_TUPLE = (ast.ListComp, ast.SetComp, ast.DictComp,
                             ast.GeneratorExp)


def is_type_expression_bool(node: ast.expr) -> bool:
    """Tell whether an expression has the shape of a type.

    Args:
        node (ast.expr): Assigned value.
    Returns:
        bool: True for dotted names, subscripts of dotted names (such as
            tuple[int, str]) and unions of those joined with |.
    Warnings:
        Shape only: a PascalCase name bound to it is treated as a type
        alias, which is never verified at runtime.
    """
    if isinstance(node, ast.Subscript):
        node = node.value
    elif isinstance(node, ast.BinOp) and isinstance(node.op, ast.BitOr):
        return all(is_type_expression_bool(side_node) or (
            isinstance(side_node, ast.Constant) and side_node.value is None)
            for side_node in (node.left, node.right))
    while isinstance(node, ast.Attribute):
        node = node.value
    return isinstance(node, ast.Name)


def find_type_alias_name_str(node: ast.stmt) -> str:
    """Return the alias a top-level statement defines, or "".

    Args:
        node (ast.stmt): Top-level statement.
    Returns:
        str: The name for type X = ..., X: TypeAlias = ... and a
            PascalCase X = <type expression>; otherwise "".
    Warnings:
        See RULES.md (type aliases) for the exact forms.
    """
    if isinstance(node, ast.TypeAlias):
        return node.name.id
    if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
        annotation_node = node.annotation
        alias_bool = node.value is not None and (
            isinstance(annotation_node, ast.Name)
            and annotation_node.id == "TypeAlias"
            or isinstance(annotation_node, ast.Attribute)
            and annotation_node.attr == "TypeAlias"
            and isinstance(annotation_node.value, ast.Name)
            and annotation_node.value.id == "typing")
        return node.target.id if alias_bool else ""
    if isinstance(node, ast.Assign) and len(node.targets) == 1 and (
        isinstance(node.targets[0], ast.Name)
    ):
        name_str = node.targets[0].id
        if (TYPE_ALIAS_NAME_PATTERN.match(name_str)
                and not UPPER_CASE_PATTERN.match(name_str)
                and is_type_expression_bool(node.value)):
            return name_str
    return ""


def find_type_alias_names_set(tree: ast.Module) -> set[str]:
    """Collect the type aliases defined at module level.

    Args:
        tree (ast.Module): Parsed module.
    Returns:
        set[str]: Alias names.
    Warnings:
        Only direct top-level statements count.
    """
    return {find_type_alias_name_str(node) for node in tree.body} - {""}


@dataclass
class FunctionFact:
    """Describe one function and where it is defined.

    Args:
        node: The def or async def node.
        is_method: True when defined directly in a class body.
        class_name: Owning class name for methods, else "".
    Returns:
        FunctionFact: Read-only description.
    Warnings:
        Nested functions are included, with is_method False.
    """

    node: ast.AST
    is_method: bool
    class_name: str


def is_dunder_bool(name_str: str) -> bool:
    """Tell whether a name starts and ends with two underscores.

    Args:
        name_str (str): Name to test.
    Returns:
        bool: True for names like __init__.
    Warnings:
        "__" alone is not a dunder.
    """
    return len(name_str) > 4 and name_str.startswith("__") and (
        name_str.endswith("__"))


def is_immutable_literal_bool(node: ast.expr) -> bool:
    """Tell whether an expression is an immutable literal constant.

    Args:
        node (ast.expr): Assigned value.
    Returns:
        bool: True for numbers, strings, bytes, bools, None, signed
            numbers, and tuples of these.
    Warnings:
        f-strings are not literals.
    """
    if isinstance(node, ast.Constant):
        return not isinstance(node.value, type(...))
    if isinstance(node, ast.UnaryOp) and isinstance(
        node.op, (ast.USub, ast.UAdd),
    ):
        return isinstance(node.operand, ast.Constant) and isinstance(
            node.operand.value, (int, float, complex)) and not isinstance(
            node.operand.value, bool)
    if isinstance(node, ast.Tuple):
        return all(is_immutable_literal_bool(element_node)
                   for element_node in node.elts)
    return False


def count_module_bindings_dict(tree: ast.Module) -> dict[str, int]:
    """Count how many times each name is bound at module level.

    Args:
        tree (ast.Module): Parsed module.
    Returns:
        dict[str, int]: Name to number of module-level binding sites,
            with names declared global anywhere counted twice.
    Warnings:
        Bindings inside functions and classes are not counted, except
        through global declarations.
    """
    counts_dict: dict[str, int] = {}
    pending_list: list[ast.AST] = list(tree.body)
    while pending_list:
        node = pending_list.pop()
        if isinstance(node, (*FUNCTION_NODES_TUPLE, ast.ClassDef)):
            counts_dict[node.name] = counts_dict.get(node.name, 0) + 1
            continue
        if isinstance(node, COMPREHENSION_NODES_TUPLE):
            for inner_node in ast.walk(node):
                if isinstance(inner_node, ast.NamedExpr):
                    name_str = inner_node.target.id
                    counts_dict[name_str] = counts_dict.get(name_str, 0) + 1
            continue
        if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Store):
            counts_dict[node.id] = counts_dict.get(node.id, 0) + 1
        elif isinstance(node, (ast.Import, ast.ImportFrom)):
            for alias_node in node.names:
                bound_str = alias_node.asname or alias_node.name.split(".")[0]
                counts_dict[bound_str] = counts_dict.get(bound_str, 0) + 1
        if not isinstance(node, ast.Lambda):
            pending_list.extend(ast.iter_child_nodes(node))
    for node in ast.walk(tree):
        if isinstance(node, ast.Global):
            for name_str in node.names:
                counts_dict[name_str] = counts_dict.get(name_str, 0) + 2
    return counts_dict


def find_constant_candidates_dict(tree: ast.Module) -> dict[str, ast.stmt]:
    """Find module-level names bound once to an immutable literal.

    Args:
        tree (ast.Module): Parsed module.
    Returns:
        dict[str, ast.stmt]: Name to its assignment statement.
    Warnings:
        Dunders and "_" are never constant candidates.
    """
    counts_dict = count_module_bindings_dict(tree)
    candidates_dict = {}
    for statement_node in tree.body:
        target_node = find_constant_target_node(statement_node)
        if target_node is None or counts_dict.get(target_node.id) != 1:
            continue
        if target_node.id == "_" or is_dunder_bool(target_node.id):
            continue
        candidates_dict[target_node.id] = statement_node
    return candidates_dict


def find_constant_target_node(statement_node: ast.stmt) -> ast.Name | None:
    """Return the name of a NAME = literal statement, if it is one.

    Args:
        statement_node (ast.stmt): Top-level statement.
    Returns:
        ast.Name | None: The single plain-name target, or None.
    Warnings:
        Chained and unpacking assignments are not constants.
    """
    if isinstance(statement_node, ast.Assign) and len(
        statement_node.targets) == 1:
        target_node = statement_node.targets[0]
    elif isinstance(statement_node, ast.AnnAssign) and (
        statement_node.value is not None
    ):
        target_node = statement_node.target
    else:
        return None
    if isinstance(target_node, ast.Name) and is_immutable_literal_bool(
        statement_node.value,
    ):
        return target_node
    return None


def collect_function_facts_list(tree: ast.Module) -> list[FunctionFact]:
    """List every function with its method status, in source order.

    Args:
        tree (ast.Module): Parsed module.
    Returns:
        list[FunctionFact]: Functions at any depth.
    Warnings:
        Order is by (line, column) of the def.
    """
    facts_list = []
    pending_list: list[tuple[ast.AST, str]] = [(tree, "")]
    while pending_list:
        node, class_name_str = pending_list.pop()
        for child_node in ast.iter_child_nodes(node):
            if isinstance(child_node, FUNCTION_NODES_TUPLE):
                facts_list.append(FunctionFact(
                    child_node, bool(class_name_str), class_name_str))
            owner_str = (child_node.name if isinstance(child_node,
                                                       ast.ClassDef) else "")
            pending_list.append((child_node, owner_str))
    return sorted(facts_list, key=lambda fact: (fact.node.lineno,
                                                fact.node.col_offset))


def is_main_block_bool(node: ast.stmt) -> bool:
    """Tell whether a statement is if __name__ == "__main__":.

    Args:
        node (ast.stmt): Top-level statement.
    Returns:
        bool: True for the main guard with either operand order.
    Warnings:
        Only == is accepted.
    """
    if not isinstance(node, ast.If) or not isinstance(node.test,
                                                      ast.Compare):
        return False
    test_node = node.test
    if len(test_node.ops) != 1 or not isinstance(test_node.ops[0], ast.Eq):
        return False
    sides_list = [test_node.left, test_node.comparators[0]]
    names_list = [side_node.id for side_node in sides_list
                  if isinstance(side_node, ast.Name)]
    constants_list = [side_node.value for side_node in sides_list
                      if isinstance(side_node, ast.Constant)]
    return names_list == ["__name__"] and constants_list == ["__main__"]


def read_docstring_str(node: ast.AST) -> str | None:
    """Return a module, class or function docstring, if present.

    Args:
        node (ast.AST): Module, ClassDef or function node.
    Returns:
        str | None: Raw docstring value (not cleaned), or None.
    Warnings:
        Only a plain string literal as the first statement counts.
    """
    body_list = getattr(node, "body", [])
    if body_list and isinstance(body_list[0], ast.Expr) and isinstance(
        body_list[0].value, ast.Constant,
    ) and isinstance(body_list[0].value.value, str):
        return body_list[0].value.value
    return None


def read_decorator_names_list(node: ast.AST) -> list[str]:
    """Return decorator names as dotted text without call arguments.

    Args:
        node (ast.AST): Function or class node.
    Returns:
        list[str]: Names such as "property" or "value.setter".
    Warnings:
        Complex decorator expressions give an empty string.
    """
    names_list = []
    for decorator_node in node.decorator_list:
        target_node = (decorator_node.func if isinstance(decorator_node,
                                                         ast.Call)
                       else decorator_node)
        parts_list = []
        while isinstance(target_node, ast.Attribute):
            parts_list.insert(0, target_node.attr)
            target_node = target_node.value
        if isinstance(target_node, ast.Name):
            parts_list.insert(0, target_node.id)
            names_list.append(".".join(parts_list))
        else:
            names_list.append("")
    return names_list
