"""Read source files and translate positions for findings."""

from dataclasses import dataclass
import ast
import re
import tokenize
from io import StringIO

CODING_PATTERN = re.compile(rb"^[ \t\f]*#.*?coding[:=][ \t]*([-\w.]+)")
DEFINITION_KEYWORD_PATTERN = re.compile(r"(?:async\s+)?(?:def|class)\s+")
LINE_BREAK_PATTERN = re.compile(r"\r\n|\r|\n")
NOQA_PATTERN = re.compile(
    r"#\s*noqa(?::\s*(?P<codes>[A-Za-z0-9]+(?:[\s,]+[A-Za-z0-9]+)*))?",
    re.IGNORECASE)
FILE_NOQA_PATTERN = re.compile(
    r"^#\s*(?:flake8|ruff)\s*:\s*noqa\b"
    r"(?::\s*(?P<codes>[A-Za-z0-9]+(?:[\s,]+[A-Za-z0-9]+)*))?",
    re.IGNORECASE)
UTF8_NAMES_TUPLE = ("utf-8", "utf8", "utf_8", "utf-8-sig", "utf8-sig")


@dataclass
class SourceFile:
    """Hold one decoded file with its lines and suppressions.

    Args:
        path: Path as given by the user.
        text: Decoded text without a byte-order mark.
        lines: Physical lines without their line endings.
        noqa: Line number to suppressed codes; None suppresses all.
    Returns:
        SourceFile: Immutable view used by every rule.
    Warnings:
        Lines are split at \\r\\n, \\r and \\n only, like Python itself.
    """

    path: str
    text: str
    lines: list[str]
    noqa: dict[int, frozenset[str] | None]


def decode_source_text(raw_bytes: bytes) -> str | None:
    """Decode file bytes as UTF-8, or refuse other encodings.

    Args:
        raw_bytes (bytes): File contents.
    Returns:
        str | None: Text without a BOM, or None when the bytes are not
            UTF-8 or a coding comment declares another encoding.
    Warnings:
        Only the first two lines may hold a coding comment (PEP 263).
    """
    raw_bytes = raw_bytes.removeprefix(b"\xef\xbb\xbf")
    for line_bytes in raw_bytes.splitlines()[:2]:
        match_info = CODING_PATTERN.match(line_bytes)
        if match_info and match_info.group(1).decode(
            "ascii", "replace").lower() not in UTF8_NAMES_TUPLE:
            return None
    try:
        text_str = raw_bytes.decode("utf-8")
    except UnicodeDecodeError:
        return None
    return text_str.removeprefix("﻿")


def split_lines_list(text_str: str) -> list[str]:
    """Split text into physical lines the way Python's tokenizer does.

    Args:
        text_str (str): Source text.
    Returns:
        list[str]: Lines without endings; no empty line after a final
            line break.
    Warnings:
        str.splitlines is not used: it also splits at form feeds and
        other Unicode separators that Python code treats as ordinary.
    """
    lines_list = LINE_BREAK_PATTERN.split(text_str)
    if lines_list and lines_list[-1] == "":
        lines_list.pop()
    return lines_list


def parse_noqa_dict(lines_list: list[str]) -> dict:
    """Find suppression comments on each line.

    Args:
        lines_list (list[str]): Physical lines.
    Returns:
        dict: Line number to a frozenset of codes, or None for all codes.
        Key 0 holds file-wide exemptions ("# ruff: noqa" or
        "# flake8: noqa", optionally with codes).
    Warnings:
        Markers inside string literals do not suppress findings.
    """
    noqa_dict = {}
    tokens_iterable = tokenize.generate_tokens(
        StringIO("\n".join(lines_list)).readline)
    for token_info in tokens_iterable:
        if token_info.type != tokenize.COMMENT:
            continue
        number_int = token_info.start[0]
        file_info = FILE_NOQA_PATTERN.match(token_info.string)
        if file_info is not None:
            record_file_noqa_none(noqa_dict, file_info.group("codes"))
            continue
        match_info = NOQA_PATTERN.search(token_info.string)
        if match_info is None:
            continue
        codes_str = match_info.group("codes")
        noqa_dict[number_int] = None if codes_str is None else frozenset(
            code_str.upper() for code_str in re.split(r"[\s,]+", codes_str)
            if code_str)
    return noqa_dict


def record_file_noqa_none(noqa_dict: dict, codes_str: str | None) -> None:
    """Merge one file-wide exemption into key 0.

    Args:
        noqa_dict (dict): Suppressions being built (changed in place).
        codes_str (str | None): Listed codes, or None for every code.
    Returns:
        None: A blanket exemption wins; listed codes accumulate.
    Warnings:
        None.
    """
    if codes_str is None or noqa_dict.get(0, frozenset()) is None:
        noqa_dict[0] = None
        return
    noqa_dict[0] = noqa_dict.get(0, frozenset()) | frozenset(
        code_str.upper() for code_str in re.split(r"[\s,]+", codes_str)
        if code_str)


def build_source_file(path_str: str, text_str: str) -> SourceFile:
    """Prepare decoded text for the rules.

    Args:
        path_str (str): Path to report.
        text_str (str): Decoded text.
    Returns:
        SourceFile: Text, lines and suppressions.
    Warnings:
        The text is not parsed here.
    """
    lines_list = split_lines_list(text_str)
    return SourceFile(path_str, text_str, lines_list,
                      parse_noqa_dict(lines_list))


def convert_column_int(line_str: str, byte_offset_int: int) -> int:
    """Convert an AST UTF-8 byte offset into a 1-based character column.

    Args:
        line_str (str): Physical line.
        byte_offset_int (int): 0-based byte offset from the AST.
    Returns:
        int: 1-based column counted in Unicode characters.
    Warnings:
        Offsets past the end of the line are clamped to it.
    """
    return len(line_str.encode("utf-8")[:byte_offset_int].decode(
        "utf-8", "ignore")) + 1


def locate_node_tuple(
    source_info: SourceFile, node: ast.AST,
) -> tuple[int, int]:
    """Return a node's 1-based line and character column.

    Args:
        source_info (SourceFile): File holding the node.
        node (ast.AST): Node with lineno and col_offset.
    Returns:
        tuple[int, int]: Line and column.
    Warnings:
        Uses the node's start, which for ast nodes is never a decorator.
    """
    line_int = node.lineno
    line_str = (source_info.lines[line_int - 1]
                if line_int <= len(source_info.lines) else "")
    return line_int, convert_column_int(line_str, node.col_offset)


def locate_definition_name_tuple(
    source_info: SourceFile, node: ast.AST,
) -> tuple[int, int]:
    """Return the position of the name after 'def' or 'class'.

    Args:
        source_info (SourceFile): File holding the definition.
        node (ast.AST): FunctionDef, AsyncFunctionDef or ClassDef.
    Returns:
        tuple[int, int]: Line and column of the identifier.
    Warnings:
        Falls back to the keyword when the name is on another line.
    """
    line_int, column_int = locate_node_tuple(source_info, node)
    line_str = source_info.lines[line_int - 1]
    match_info = DEFINITION_KEYWORD_PATTERN.match(line_str, column_int - 1)
    if match_info and line_str.startswith(node.name, match_info.end()):
        return line_int, match_info.end() + 1
    return line_int, column_int
