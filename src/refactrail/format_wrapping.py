"""Line splitting at existing brackets, in the style of a right-hand split."""

import ast
import io
import re
import tokenize

from refactrail.format_statements import LayoutStyle, plan_layouts_list
from refactrail.format_tokens import (
    build_token_index, collect_line_tokens_dict, collect_protected_lines_set,
)

DEFINITION_HEADER_PATTERN = re.compile(r"\s*(?:async\s+)?(?:def|class)\b")
OPENING_BRACKETS_TUPLE = ("(", "[", "{")
CLOSING_BRACKETS_TUPLE = (")", "]", "}")
CONTINUATION_INDENT_STR = "    "


def collect_groups_list(tokens_list: list[tokenize.TokenInfo]) -> list:
    """Find non-empty bracket groups wholly contained on one line.

    Args:
        tokens_list (list[tokenize.TokenInfo]): Meaningful line tokens.
    Returns:
        list: (opening, closing, direct commas, opaque, depth) per group.
            Opaque groups hold a comprehension or lambda, whose commas
            are not element separators.
    Warnings:
        Depth counts brackets opened on this line only.
    """
    stack_list = []
    groups_list = []
    for token_info in tokens_list:
        spelling_str = token_info.string
        if spelling_str in OPENING_BRACKETS_TUPLE:
            stack_list.append([token_info, [], False])
        elif spelling_str in CLOSING_BRACKETS_TUPLE and stack_list:
            opening_info, commas_list, opaque_bool = stack_list.pop()
            if token_info.start[1] > opening_info.end[1]:
                groups_list.append((opening_info, token_info, commas_list,
                                    opaque_bool, len(stack_list)))
        elif stack_list:
            if spelling_str == ",":
                stack_list[-1][1].append(token_info)
            if spelling_str in ("for", "lambda"):
                stack_list[-1][2] = True
    return groups_list


def order_groups_list(groups_list: list, header_bool: bool) -> list:
    """Order candidate groups: outermost first, then by side.

    Args:
        groups_list (list): Groups from collect_groups_list.
        header_bool (bool): Whether the line starts a def or class.
    Returns:
        list: Groups in the order splits are tried: rightmost first,
            except leftmost first for definition headers so that the
            parameter list splits before a return annotation.
    Warnings:
        This mirrors the usual right-hand / left-hand split preference.
    """
    side_int = 1 if header_bool else -1
    return sorted(groups_list, key=lambda group_tuple: (
        group_tuple[4], side_int * group_tuple[0].start[1]))


def collect_single_lines_set(normalized_str: str) -> set[int]:
    """Find physical lines that hold one complete logical line.

    Args:
        normalized_str (str): Source with LF line endings.
    Returns:
        set[int]: 1-based numbers of lines where a statement (or compound
            statement header) starts and ends.
    Warnings:
        Continuation lines of multi-line statements are excluded; they
        need whole-statement layout, which this splitter does not do.
    """
    skipped_tuple = (tokenize.NL, tokenize.COMMENT, tokenize.INDENT,
                     tokenize.DEDENT, tokenize.ENCODING)
    single_lines_set, start_int = set(), None
    for token_info in tokenize.generate_tokens(
            io.StringIO(normalized_str).readline):
        if token_info.type in skipped_tuple:
            continue
        if start_int is None:
            start_int = token_info.start[0]
        if token_info.type == tokenize.NEWLINE:
            if token_info.start[0] == start_int:
                single_lines_set.add(start_int)
            start_int = None
    return single_lines_set


def collect_open_lines_set(normalized_str: str) -> set[int]:
    """Find lines that start inside a bracket opened on an earlier line.

    Args:
        normalized_str (str): Source with LF line endings.
    Returns:
        set[int]: 1-based numbers of such continuation lines.
    Warnings:
        Inside brackets a line break after a comma never changes tokens.
    """
    open_lines_set, depth_int, line_int = set(), 0, 0
    for token_info in tokenize.generate_tokens(
            io.StringIO(normalized_str).readline):
        if token_info.start[0] != line_int:
            line_int = token_info.start[0]
            if depth_int:
                open_lines_set.add(line_int)
        if token_info.type == tokenize.OP:
            if token_info.string in OPENING_BRACKETS_TUPLE:
                depth_int += 1
            elif token_info.string in CLOSING_BRACKETS_TUPLE:
                depth_int = max(depth_int - 1, 0)
    return open_lines_set


def pack_commas_list(line_str: str, tokens_list: list,
                     width_int: int) -> list[str]:
    """Re-pack a continuation line at its own top-level commas.

    Args:
        line_str (str): Continuation line starting inside brackets.
        tokens_list (list): Meaningful tokens on the line.
        width_int (int): Requested line width.
    Returns:
        list[str]: Lines at the original indentation, each filled with as
            many comma-separated elements as fit; [line_str] if none.
    Warnings:
        Only commas outside brackets opened on this line are used, and
        never one followed by a closing bracket from an earlier line.
    """
    indent_str = line_str[:len(line_str) - len(line_str.lstrip(" "))]
    depth_int, cuts_list = 0, []
    for token_info in tokens_list:
        if token_info.string in OPENING_BRACKETS_TUPLE:
            depth_int += 1
        elif token_info.string in CLOSING_BRACKETS_TUPLE:
            depth_int -= 1
            if depth_int < 0:
                break
        elif token_info.string == "," and depth_int == 0:
            cuts_list.append(token_info.end[1])
    pieces_list, start_int = [], len(indent_str)
    for cut_int in [*cuts_list, len(line_str)]:
        piece_str = line_str[start_int:cut_int].strip()
        if piece_str:
            pieces_list.append(piece_str)
        start_int = cut_int
    lines_list = [indent_str + pieces_list[0]]
    for piece_str in pieces_list[1:]:
        if len(lines_list[-1]) + 1 + len(piece_str) <= width_int:
            lines_list[-1] += " " + piece_str
        else:
            lines_list.append(indent_str + piece_str)
    return lines_list if max(map(len, lines_list)) < len(line_str) else [
        line_str]


def split_group_list(line_str: str, group_tuple: tuple,
                     width_int: int) -> list[str]:
    """Split a bracket group without adding or removing syntax tokens.

    Args:
        line_str (str): Physical code line without its separator.
        group_tuple (tuple): Group from collect_groups_list.
        width_int (int): Requested line width.
    Returns:
        list[str]: The head ending at the opening bracket, the content on
            one indented line when it fits (else one element per line),
            and the closing bracket with the rest of the line.
    Warnings:
        Returns the line unchanged when it is indented with tabs.
    """
    opening_info, closing_info, commas_list, opaque_bool, _ = group_tuple
    indent_str = line_str[:len(line_str) - len(line_str.lstrip(" \t"))]
    if "\t" in indent_str:
        return [line_str]
    inner_str = indent_str + CONTINUATION_INDENT_STR
    head_str = line_str[:opening_info.end[1]].rstrip()
    tail_str = indent_str + line_str[closing_info.start[1]:]
    content_str = inner_str + line_str[
        opening_info.end[1]:closing_info.start[1]].strip()
    if len(content_str) <= width_int or not commas_list or opaque_bool:
        return [head_str, content_str, tail_str]
    segments_list = [head_str]
    start_int = opening_info.end[1]
    for comma_info in commas_list:
        segments_list.append(
            inner_str + line_str[start_int:comma_info.end[1]].strip())
        start_int = comma_info.end[1]
    remainder_str = line_str[start_int:closing_info.start[1]].strip()
    if remainder_str:
        segments_list.append(inner_str + remainder_str)
    return [*segments_list, tail_str]


def split_line_list(line_str: str, groups_list: list,
                    width_int: int) -> list[str]:
    """Split a long line at the first group that shortens it.

    Args:
        line_str (str): Physical code line without its separator.
        groups_list (list): Groups found on the line.
        width_int (int): Requested line width.
    Returns:
        list[str]: Replacement lines, or [line_str] when no split helps.
    Warnings:
        A split must make the longest resulting line shorter.
    """
    header_bool = bool(DEFINITION_HEADER_PATTERN.match(line_str))
    for group_tuple in order_groups_list(groups_list, header_bool):
        lines_list = split_group_list(line_str, group_tuple, width_int)
        if max(map(len, lines_list)) < len(line_str):
            return lines_list
    return [line_str]


def parenthesize_import_list(line_str: str, tokens_list: list,
                             width_int: int) -> list[str]:
    """Wrap a long `from module import a, b` line in parentheses.

    Args:
        line_str (str): One complete logical line.
        tokens_list (list): Meaningful tokens on the line.
        width_int (int): Requested line width.
    Returns:
        list[str]: `from module import (`, packed names, `)`; or
            [line_str] when the line is not such an import.
    Warnings:
        Adds two bracket tokens; the syntax tree is unchanged. Star
        imports and already parenthesized imports are left alone.
    """
    spellings_list = [token_info.string for token_info in tokens_list]
    if (not spellings_list or spellings_list[0] != "from"
            or "import" not in spellings_list
            or "(" in spellings_list or "*" in spellings_list):
        return [line_str]
    import_info = tokens_list[spellings_list.index("import")]
    indent_str = line_str[:len(line_str) - len(line_str.lstrip(" "))]
    names_str = (indent_str + CONTINUATION_INDENT_STR
                 + line_str[import_info.end[1]:].strip())
    packed_list = pack_commas_list(names_str, collect_line_tokens_list(
        names_str), width_int)
    return [line_str[:import_info.end[1]] + " (", *packed_list,
            indent_str + ")"]


def collect_line_tokens_list(line_str: str) -> list:
    """Tokenize one line of import names for comma packing.

    Args:
        line_str (str): Indented names such as "    a, b as c".
    Returns:
        list: Meaningful tokens with columns on this line.
    Warnings:
        Only used for names after `import`, which cannot hold brackets.
    """
    offset_int = len(line_str) - len(line_str.lstrip())
    return [token_info._replace(
        start=(1, token_info.start[1] + offset_int),
        end=(1, token_info.end[1] + offset_int))
        for token_info in tokenize.generate_tokens(
            io.StringIO(line_str.lstrip() + "\n").readline)
        if token_info.type in (tokenize.NAME, tokenize.OP)]


def wrap_once_str(text_str: str, width_int: int,
                  hug_bool: bool = False) -> str:
    """Wrap eligible long lines once, keeping original line separators.

    Args:
        text_str (str): Compilable Python source.
        width_int (int): Requested line width from 40 through 200.
        hug_bool (bool): Leave single-line statements (other than
            imports) to the whole-statement layout.
    Returns:
        str: Source with bounded bracket-group splitting.
    Warnings:
        Protected lines and unsplittable expressions may exceed the width.
    """
    if type(width_int) is not int or not 40 <= width_int <= 200:
        raise ValueError("Line length must be a whole number from 40 to 200")
    normalized_str = text_str.replace("\r\n", "\n").replace("\r", "\n")
    tree_node = ast.parse(normalized_str, type_comments=True)
    if tree_node.type_ignores:
        return text_str
    index_info = build_token_index(normalized_str)
    protected_set = collect_protected_lines_set(index_info, tree_node)
    tokens_dict = collect_line_tokens_dict(index_info)
    kinds_tuple = (collect_single_lines_set(normalized_str),
                   collect_open_lines_set(normalized_str))
    pieces_list = re.split(r"(\r\n|\r|\n)", text_str)
    for index_int in range(0, len(pieces_list), 2):
        line_str = pieces_list[index_int]
        number_int = index_int // 2 + 1
        if len(line_str) <= width_int or number_int in protected_set:
            continue
        ending_str = (pieces_list[index_int + 1]
                      if index_int + 1 < len(pieces_list) else "\n")
        pieces_list[index_int] = ending_str.join(split_long_line_list(
            line_str, (number_int, tokens_dict.get(number_int, [])),
            kinds_tuple, width_int, hug_bool))
    return "".join(pieces_list)


def split_long_line_list(line_str: str, line_tuple: tuple,
                         kinds_tuple: tuple, width_int: int,
                         hug_bool: bool) -> list[str]:
    """Choose how one over-long physical line is split.

    Args:
        line_str (str): The line without its separator.
        line_tuple (tuple): (1-based line number, tokens on the line).
        kinds_tuple (tuple): (single-statement lines, continuation lines).
        width_int (int): Requested line width.
        hug_bool (bool): Leave statements to whole-statement layout.
    Returns:
        list[str]: Replacement lines, or [line_str] to keep it.
    Warnings:
        Imports may gain parentheses; other lines only change gaps.
    """
    number_int, tokens_list = line_tuple
    single_lines_set, open_lines_set = kinds_tuple
    if number_int in single_lines_set:
        lines_list = parenthesize_import_list(line_str, tokens_list,
                                              width_int)
        if lines_list == [line_str] and not hug_bool:
            lines_list = split_line_list(
                line_str, collect_groups_list(tokens_list), width_int)
        return lines_list
    if number_int in open_lines_set:
        return pack_commas_list(line_str, tokens_list, width_int)
    return [line_str]


def wrap_source_str(text_str: str, width_int: int,
                    hug_bool: bool = False) -> str:
    """Split long lines, then lay out still-long statements as a whole.

    Args:
        text_str (str): Compilable source with original line separators.
        width_int (int): Valid requested line width.
        hug_bool (bool): Keep closing brackets on the last content line
            and pack elements, instead of the own-line style.
    Returns:
        str: Idempotent wrapping proposal for supported bracket groups.
    Warnings:
        Exceeding the iteration cap refuses the proposal without writing it.
    """
    for iteration_int in range(200):
        output_str = wrap_once_str(text_str, width_int, hug_bool)
        if output_str == text_str:
            return layout_long_statements_str(output_str, width_int,
                                              hug_bool)
        text_str = output_str
    raise ValueError("Bracket wrapping exceeded its 200-pass resource limit")


def layout_long_statements_str(text_str: str, width_int: int,
                               hug_bool: bool) -> str:
    """Apply whole-statement layout, keeping original line separators.

    Args:
        text_str (str): Compilable source with original line separators.
        width_int (int): Valid requested line width.
        hug_bool (bool): Bracket style passed to the layout engine.
    Returns:
        str: Source whose over-long statements were laid out again.
    Warnings:
        Files with type-ignore directives keep their line structure. New
        lines use the separator of their statement's first line.
    """
    pieces_list = re.split(r"(\r\n|\r|\n)", text_str)
    lines_list, endings_list = pieces_list[0::2], pieces_list[1::2] + [""]
    normalized_str = "\n".join(lines_list)
    tree_node = ast.parse(normalized_str, type_comments=True)
    if tree_node.type_ignores:
        return text_str
    protected_set = collect_protected_lines_set(
        build_token_index(normalized_str), tree_node, fstrings_bool=False)
    for first_int, last_int, new_list in plan_layouts_list(
            normalized_str, protected_set, LayoutStyle(width_int, hug_bool)):
        ending_str = endings_list[first_int - 1] or "\n"
        lines_list[first_int - 1:last_int] = new_list
        endings_list[first_int - 1:last_int] = (
            [ending_str] * (len(new_list) - 1) + [endings_list[last_int - 1]])
    return "".join(line_str + ending_str for line_str, ending_str
                   in zip(lines_list, endings_list))
