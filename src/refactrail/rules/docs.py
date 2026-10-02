"""Documentation rules: docstrings, their sections and argument entries."""

import ast
import re

from refactrail.facts import (
    FUNCTION_NODES_TUPLE, read_decorator_names_list, read_docstring_str,
)
from refactrail.rules.context import RuleContext
from refactrail.source import locate_definition_name_tuple

SECTION_ALIASES_DICT = {
    "Args:": "Args", "Arguments:": "Args", "Parameters:": "Args",
    "Returns:": "Returns", "Yields:": "Yields", "Warnings:": "Warnings",
    "Raises:": "Raises",
}
NUMPY_SECTIONS_DICT = {"Parameters": "Args", "Returns": "Returns",
                       "Yields": "Yields", "Warnings": "Warnings"}
ENTRY_PATTERN = re.compile(r"^\*{0,2}([A-Za-z_][A-Za-z0-9_]*)\s*(\(|:|$)")
DASHES_PATTERN = re.compile(r"^-{3,}$")


def is_stub_function_bool(node: ast.AST) -> bool:
    """Tell whether a function body is only ... or pass.

    Args:
        node (ast.AST): Function node.
    Returns:
        bool: True for protocol and abstract stubs.
    Warnings:
        A docstring makes the body more than a stub.
    """
    return len(node.body) == 1 and (
        isinstance(node.body[0], ast.Pass)
        or isinstance(node.body[0], ast.Expr)
        and isinstance(node.body[0].value, ast.Constant)
        and node.body[0].value.value is Ellipsis)


def check_missing_docstrings_none(context: RuleContext) -> None:
    """Report modules, classes and functions without docstrings (RT301).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds findings to the context.
    Warnings:
        Overloads and stub functions are exempt.
    """
    tree = context.tree
    if tree.body and read_docstring_str(tree) is None:
        context.report_none("RT301", (1, 1), "Missing docstring in module.")
    for node in ast.walk(tree):
        if not isinstance(node, (*FUNCTION_NODES_TUPLE, ast.ClassDef)):
            continue
        if read_docstring_str(node) is not None:
            continue
        kind_str = "class" if isinstance(node, ast.ClassDef) else "function"
        if kind_str == "function" and (
            is_stub_function_bool(node)
            or "overload" in read_decorator_names_list(node)
            or "typing.overload" in read_decorator_names_list(node)
        ):
            continue
        context.report_none("RT301", locate_definition_name_tuple(
            context.source_info, node),
            f"Missing docstring in {kind_str} '{node.name}'.")


def find_sections_dict(docstring_str: str) -> dict[str, int]:
    """Locate section headings in a docstring.

    Args:
        docstring_str (str): Raw docstring value.
    Returns:
        dict[str, int]: Canonical section name to its line index.
    Warnings:
        The first occurrence of each section wins.
    """
    lines_list = docstring_str.split("\n")
    sections_dict: dict[str, int] = {}
    for index_int, line_str in enumerate(lines_list):
        stripped_str = line_str.strip()
        section_str = SECTION_ALIASES_DICT.get(stripped_str)
        if section_str is None and stripped_str in NUMPY_SECTIONS_DICT and (
            index_int + 1 < len(lines_list)
            and DASHES_PATTERN.match(lines_list[index_int + 1].strip())
        ):
            section_str = NUMPY_SECTIONS_DICT[stripped_str]
        if section_str is not None:
            sections_dict.setdefault(section_str, index_int)
    return sections_dict


def list_parameters_list(node: ast.AST, is_method_bool: bool) -> list[str]:
    """List a function's parameter names, without the implicit one.

    Args:
        node (ast.AST): Function node.
        is_method_bool (bool): Whether it is defined in a class body.
    Returns:
        list[str]: Parameter names in declaration order.
    Warnings:
        The implicit parameter is dropped unless staticmethod applies.
    """
    arguments_node = node.args
    positional_list = [*arguments_node.posonlyargs, *arguments_node.args]
    names_list = [parameter_node.arg for parameter_node in [
        *positional_list, arguments_node.vararg,
        *arguments_node.kwonlyargs, arguments_node.kwarg] if parameter_node]
    if is_method_bool and positional_list and (
        "staticmethod" not in read_decorator_names_list(node)
    ):
        names_list.remove(positional_list[0].arg)
    return names_list


def is_generator_bool(node: ast.AST) -> bool:
    """Tell whether a function yields in its own body.

    Args:
        node (ast.AST): Function node.
    Returns:
        bool: True when a yield or yield from belongs to this function.
    Warnings:
        Nested functions and lambdas are not searched.
    """
    pending_list: list[ast.AST] = list(node.body)
    while pending_list:
        current_node = pending_list.pop()
        if isinstance(current_node, (ast.Yield, ast.YieldFrom)):
            return True
        if not isinstance(current_node, (*FUNCTION_NODES_TUPLE,
                                         ast.ClassDef, ast.Lambda)):
            pending_list.extend(ast.iter_child_nodes(current_node))
    return False


def check_docstring_sections_none(context: RuleContext) -> None:
    """Report docstrings without a summary or required sections (RT302).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds one finding per missing part.
    Warnings:
        Warnings sections are required only in the strict profile.
    """
    for fact in context.functions_list:
        node = fact.node
        docstring_str = read_docstring_str(node)
        if docstring_str is None:
            continue
        position_tuple = locate_definition_name_tuple(context.source_info,
                                                      node)
        missing_list = list_missing_parts_list(
            node, docstring_str, fact.is_method,
            context.settings_info.strict_bool)
        for part_str in missing_list:
            context.report_none("RT302", position_tuple, (
                f"Docstring of '{node.name}' has no {part_str}."))


def list_missing_parts_list(
    node: ast.AST, docstring_str: str, is_method_bool: bool,
    strict_bool: bool,
) -> list[str]:
    """List the parts a function docstring lacks.

    Args:
        node (ast.AST): Function node.
        docstring_str (str): Its docstring.
        is_method_bool (bool): Whether it is a method.
        strict_bool (bool): Whether the strict profile is active.
    Returns:
        list[str]: Such as "summary line" or "Args section", in order.
    Warnings:
        Generators may use Yields instead of Returns.
    """
    sections_dict = find_sections_dict(docstring_str)
    missing_list = []
    if not docstring_str.strip():
        missing_list.append("summary line")
    if list_parameters_list(node, is_method_bool) and (
        "Args" not in sections_dict
    ):
        missing_list.append("Args section")
    returns_bool = "Returns" in sections_dict or (
        "Yields" in sections_dict and is_generator_bool(node))
    if node.name != "__init__" and not returns_bool:
        missing_list.append("Returns section")
    if strict_bool and "Warnings" not in sections_dict:
        missing_list.append("Warnings section")
    return missing_list


def read_documented_names_list(docstring_str: str) -> list[str] | None:
    """Return the parameter names an Args section describes.

    Args:
        docstring_str (str): Raw docstring value.
    Returns:
        list[str] | None: Documented names, or None without an Args
            section.
    Warnings:
        Continuation lines, indented deeper than entries, are skipped.
        NumPy-style entries sit at the heading's own indentation.
    """
    sections_dict = find_sections_dict(docstring_str)
    if "Args" not in sections_dict:
        return None
    heading_int = sections_dict["Args"]
    body_list, entry_indent_int = collect_section_lines_tuple(
        docstring_str.split("\n"), heading_int,
        set(sections_dict.values()) - {heading_int})
    if entry_indent_int is None and body_list:
        entry_indent_int = body_list[0][0]
    return [match_info.group(1) for indent_int, text_str in body_list
            if indent_int == entry_indent_int
            and (match_info := ENTRY_PATTERN.match(text_str))
            and match_info.group(1) != "None"]


def collect_section_lines_tuple(
    lines_list: list[str], heading_int: int, other_headings_set: set[int],
) -> tuple[list[tuple[int, str]], int | None]:
    """Collect the non-blank lines of a docstring section.

    Args:
        lines_list (list[str]): Docstring lines.
        heading_int (int): Index of the section heading.
        other_headings_set (set[int]): Indexes of other headings.
    Returns:
        tuple: (indentation, stripped text) per line, and the entry
            indentation for NumPy style (else None, to be inferred).
    Warnings:
        Google-style sections end at the first line not indented deeper
        than the heading; NumPy-style ones at a line indented less.
    """
    heading_indent_int = len(lines_list[heading_int]) - len(
        lines_list[heading_int].lstrip())
    start_int = heading_int + 1
    numpy_bool = start_int < len(lines_list) and bool(DASHES_PATTERN.match(
        lines_list[start_int].strip()))
    body_list = []
    for index_int in range(start_int + numpy_bool, len(lines_list)):
        line_str = lines_list[index_int]
        if not line_str.strip():
            continue
        indent_int = len(line_str) - len(line_str.lstrip())
        if index_int in other_headings_set or indent_int < (
            heading_indent_int + (not numpy_bool)
        ):
            break
        body_list.append((indent_int, line_str.strip()))
    return body_list, heading_indent_int if numpy_bool else None


def check_documented_arguments_none(context: RuleContext) -> None:
    """Report Args entries that do not match the parameters (RT303).

    Args:
        context (RuleContext): File being checked.
    Returns:
        None: Adds findings to the context.
    Warnings:
        Only functions whose docstring has an Args section are checked.
    """
    for fact in context.functions_list:
        documented_list = read_documented_names_list(
            read_docstring_str(fact.node) or "")
        if documented_list is None:
            continue
        parameters_list = list_parameters_list(fact.node, fact.is_method)
        position_tuple = locate_definition_name_tuple(context.source_info,
                                                      fact.node)
        name_str = fact.node.name
        for parameter_str in parameters_list:
            if parameter_str not in documented_list:
                context.report_none("RT303", position_tuple, (
                    f"Docstring of '{name_str}' does not describe "
                    f"parameter '{parameter_str}'."))
        for entry_str in documented_list:
            if entry_str not in parameters_list:
                context.report_none("RT303", position_tuple, (
                    f"Docstring of '{name_str}' describes unknown "
                    f"parameter '{entry_str}'."))
