"""Identify operator tokens from AST operands without rebuilding source."""

import ast
from bisect import bisect_left

from refactrail.format_tokens import TokenIndex

OPERATOR_NAMES_DICT = {
    ast.Add: "+", ast.Sub: "-", ast.Mult: "*", ast.MatMult: "@",
    ast.Div: "/", ast.FloorDiv: "//", ast.Mod: "%", ast.Pow: "**",
    ast.LShift: "<<", ast.RShift: ">>", ast.BitOr: "|",
    ast.BitXor: "^", ast.BitAnd: "&", ast.Eq: "==", ast.NotEq: "!=",
    ast.Lt: "<", ast.LtE: "<=", ast.Gt: ">", ast.GtE: ">=",
    ast.Is: "is", ast.IsNot: "is not", ast.In: "in", ast.NotIn: "not in",
}


def locate_boundary_tuple(
    node: ast.AST, index_info: TokenIndex, end_bool: bool,
) -> tuple[int, int]:
    """Translate a node boundary from UTF-8 bytes to Unicode characters.

    Args:
        node (ast.AST): Node with parser positions.
        index_info (TokenIndex): Source lines.
        end_bool (bool): Choose exclusive end rather than start.
    Returns:
        tuple[int, int]: One-based line and zero-based character column.
    Warnings:
        This conversion is required for non-ASCII identifiers.
    """
    line_int = node.end_lineno if end_bool else node.lineno
    column_int = node.end_col_offset if end_bool else node.col_offset
    line_bytes = index_info.lines_list[line_int - 1].encode("utf-8")
    return line_int, len(line_bytes[:column_int].decode("utf-8"))


def collect_node_gaps_list(node: ast.AST) -> list:
    """Describe operand gaps that contain an operator requiring spacing.

    Args:
        node (ast.AST): Current syntax node.
    Returns:
        list: Left operand, right operand and operator spelling triples.
    Warnings:
        Unary operators and argument defaults are left unchanged.
    """
    if isinstance(node, ast.BinOp):
        return [(node.left, node.right, OPERATOR_NAMES_DICT[type(node.op)])]
    if isinstance(node, ast.Compare):
        operands_list = [node.left, *node.comparators]
        return [(operands_list[index_int], operands_list[index_int + 1],
                 OPERATOR_NAMES_DICT[type(operator_node)])
                for index_int, operator_node in enumerate(node.ops)]
    if isinstance(node, ast.Assign):
        operands_list = [*node.targets, node.value]
        return [(left_node, right_node, "=") for left_node, right_node
                in zip(operands_list, operands_list[1:])]
    if isinstance(node, ast.AnnAssign) and node.value is not None:
        return [(node.annotation, node.value, "=")]
    if isinstance(node, ast.AugAssign):
        return [(node.target, node.value,
                 OPERATOR_NAMES_DICT[type(node.op)] + "=")]
    if isinstance(node, ast.NamedExpr):
        return [(node.target, node.value, ":=")]
    return []


def collect_operator_positions_set(
    tree_node: ast.Module, index_info: TokenIndex,
) -> set[tuple[int, int]]:
    """Locate operator tokens using indexed operand boundaries.

    Args:
        tree_node (ast.Module): Parsed immutable source.
        index_info (TokenIndex): Lexical index of the same source.
    Returns:
        set[tuple[int, int]]: Token starts requiring surrounding spaces.
    Warnings:
        Only whitespace gaps are changed; line breaks are retained.
    """
    positions_set: set[tuple[int, int]] = set()
    for node in ast.walk(tree_node):
        for left_node, right_node, operator_str in collect_node_gaps_list(
            node
        ):
            start_tuple = locate_boundary_tuple(left_node, index_info, True)
            end_tuple = locate_boundary_tuple(right_node, index_info, False)
            first_int = bisect_left(index_info.starts_list, start_tuple)
            last_int = bisect_left(index_info.starts_list, end_tuple)
            names_list = operator_str.split()
            positions_set.update(
                token_info.start for token_info in
                index_info.tokens_list[first_int:last_int]
                if token_info.string in names_list
            )
    return positions_set
