"""Independent bounded whitespace formatting with source-preserving writes."""

import ast
from dataclasses import dataclass
from hashlib import sha256
from pathlib import Path
import re
import tokenize

from refactrail.engine import parse_quietly_node
from refactrail.fixing import verify_original_none, write_atomically_none
from refactrail.format_operators import collect_operator_positions_set
from refactrail.format_tokens import (
    build_token_index, collect_line_tokens_dict, collect_literal_tokens_list,
    collect_protected_lines_set,
)
from refactrail.source import decode_source_text
from refactrail.format_wrapping import wrap_source_str

UTF8_BOM_BYTES = b"\xef\xbb\xbf"
LINE_END_PATTERN = re.compile(r"(\r\n|\r|\n)")


@dataclass(frozen=True)
class FormatPlan:
    """Store one compiled whitespace proposal and original byte fingerprint.

    Args:
        path_str: Original source path.
        original_bytes: Immutable input snapshot.
        output_bytes: Validated formatting proposal.
        source_sha256_str: Hash required immediately before a write.
    Returns:
        FormatPlan: Reviewable proposal; constructing it never writes.
    Warnings:
        Compilation and AST equality are not behavior equivalence proofs.
    """

    path_str: str
    original_bytes: bytes
    output_bytes: bytes
    source_sha256_str: str


def get_gap_str(
    left_info: tokenize.TokenInfo, right_info: tokenize.TokenInfo,
    positions_set: set[tuple[int, int]],
) -> str | None:
    """Choose spacing only for supported operators and comma boundaries.

    Args:
        left_info (tokenize.TokenInfo): Token before the whitespace gap.
        right_info (tokenize.TokenInfo): Token after the gap.
        positions_set (set[tuple[int, int]]): Marked operator token starts.
    Returns:
        str | None: Replacement whitespace, or None to preserve the gap.
    Warnings:
        Unary operators and punctuation outside this subset are retained.
    """
    if left_info.start in positions_set or right_info.start in positions_set:
        return " "
    if right_info.string == ",":
        return ""
    if left_info.string == ",":
        return "" if right_info.string in (")", "]", "}") else " "
    return None


def format_line_str(
    line_str: str, tokens_list: list[tokenize.TokenInfo],
    positions_set: set[tuple[int, int]],
) -> str:
    """Edit only whitespace between original tokens on one physical line.

    Args:
        line_str (str): Original line without a separator.
        tokens_list (list[tokenize.TokenInfo]): Tokens on this line.
        positions_set (set[tuple[int, int]]): Spaced operator positions.
    Returns:
        str: Line with supported horizontal whitespace normalized.
    Warnings:
        Protected lines must be excluded by the caller.
    """
    edits_list: list[tuple[int, int, str]] = []
    for left_info, right_info in zip(tokens_list, tokens_list[1:]):
        start_int, end_int = left_info.end[1], right_info.start[1]
        gap_str = line_str[start_int:end_int]
        replacement_str = get_gap_str(left_info, right_info, positions_set)
        if replacement_str is not None and not gap_str.strip(" \t"):
            edits_list.append((start_int, end_int, replacement_str))
    for start_int, end_int, replacement_str in reversed(edits_list):
        line_str = line_str[:start_int] + replacement_str + line_str[end_int:]
    return line_str.rstrip(" \t")


def validate_formatted_none(
    original_str: str, formatted_str: str, path_str: str,
) -> None:
    """Reject proposals that change compiled structure or literal spelling.

    Args:
        original_str (str): Source before formatting, with normalized LF.
        formatted_str (str): Proposed source with normalized LF.
        path_str (str): Label for compilation diagnostics.
    Returns:
        None: Raises ValueError or SyntaxError when validation fails.
    Warnings:
        Targets are never executed; source-position reflection can differ.
    """
    original_str = original_str.replace("\r\n", "\n").replace("\r", "\n")
    formatted_str = formatted_str.replace("\r\n", "\n").replace("\r", "\n")
    parse_quietly_node(formatted_str, path_str)
    original_node = ast.parse(original_str, type_comments=True)
    formatted_node = ast.parse(formatted_str, type_comments=True)
    if ast.dump(original_node) != ast.dump(formatted_node):
        raise ValueError("Formatting would change the source AST")
    if collect_literal_tokens_list(build_token_index(original_str)) != (
        collect_literal_tokens_list(build_token_index(formatted_str))
    ):
        raise ValueError("Formatting would change literal or comment tokens")


def format_source_str(text_str: str, path_str: str = "<source>",
                      width_int: int | None = None,
                      hug_bool: bool = False) -> str:
    """Format supported whitespace while preserving each source line ending.

    Args:
        text_str (str): Decoded source, without a UTF-8 byte-order mark.
        path_str (str): Source label for errors.
        width_int (int | None): Optional bracket-group wrapping width.
        hug_bool (bool): Keep closing brackets on the last content line.
    Returns:
        str: Validated formatting proposal; no source is written.
    Warnings:
        Protected lines and indivisible expressions can exceed the width.
        Spacing runs again after wrapping: a split can move code off a
        protected line, and one run must reach the fixed point.
    """
    normalized_str = text_str.replace("\r\n", "\n").replace("\r", "\n")
    formatted_str = format_spacing_str(text_str, path_str)
    if width_int is not None:
        formatted_str = format_spacing_str(wrap_source_str(
            formatted_str, width_int, hug_bool), path_str)
    validate_formatted_none(normalized_str, formatted_str.replace(
        "\r\n", "\n").replace("\r", "\n"), path_str)
    return formatted_str


def format_spacing_str(text_str: str, path_str: str) -> str:
    """Normalize operator and comma spacing on unprotected lines.

    Args:
        text_str (str): Decoded source with original line separators.
        path_str (str): Source label for errors.
    Returns:
        str: Source with spacing edits and a final line separator.
    Warnings:
        Only whitespace between tokens changes; validation is the
        caller's job.
    """
    normalized_str = text_str.replace("\r\n", "\n").replace("\r", "\n")
    tree_node = parse_quietly_node(normalized_str, path_str)
    index_info = build_token_index(normalized_str)
    protected_set = collect_protected_lines_set(index_info, tree_node)
    positions_set = collect_operator_positions_set(tree_node, index_info)
    tokens_dict = collect_line_tokens_dict(index_info)
    pieces_list = LINE_END_PATTERN.split(text_str)
    for index_int in range(0, len(pieces_list), 2):
        number_int = index_int // 2 + 1
        if number_int not in protected_set:
            pieces_list[index_int] = format_line_str(
                pieces_list[index_int], tokens_dict.get(number_int, []),
                positions_set,
            )
    formatted_str = "".join(pieces_list)
    if formatted_str and not formatted_str.endswith(("\r", "\n")):
        formatted_str += pieces_list[1] if len(pieces_list) > 1 else "\n"
    return formatted_str


def plan_format(path_str: str, width_int: int | None = None,
                hug_bool: bool = False,
                engine_str: str = "python") -> FormatPlan:
    """Read a regular UTF-8 file and prepare a validated formatting proposal.

    Args:
        path_str (str): Source path.
        width_int (int | None): Optional wrapping width.
        hug_bool (bool): Keep closing brackets on the last content line.
        engine_str (str): "python" or "rust" (already resolved).
    Returns:
        FormatPlan: Original and proposed bytes with the source fingerprint.
    Warnings:
        Linked files and unsupported encodings are refused.
    """
    source_path = Path(path_str)
    if source_path.is_symlink() or source_path.is_junction():
        raise ValueError(f"Linked source is not supported: {source_path}")
    raw_bytes = source_path.read_bytes()
    text_str = decode_source_text(raw_bytes)
    if text_str is None:
        raise ValueError("Source is not UTF-8 or declares another encoding")
    try:
        formatted_str = format_engine_str(text_str, path_str, width_int,
                                          hug_bool, engine_str)
    except (SyntaxError, ValueError, tokenize.TokenError) as error:
        raise ValueError(f"Cannot format {path_str}: {error}") from error
    prefix_bytes = (UTF8_BOM_BYTES
                    if raw_bytes.startswith(UTF8_BOM_BYTES) else b"")
    return FormatPlan(path_str, raw_bytes, prefix_bytes + formatted_str.encode(
        "utf-8"), sha256(raw_bytes).hexdigest())


def write_format_none(plan_info: FormatPlan) -> None:
    """Write a reviewed format plan using stale-source checks and replacement.

    Args:
        plan_info (FormatPlan): Compiled proposal for one source snapshot.
    Returns:
        None: Replaces that file atomically if still unchanged.
    Warnings:
        Multiple file writes are not a transaction; review diffs first.
    """
    source_path = Path(plan_info.path_str)
    if sha256(plan_info.original_bytes).hexdigest() != (
        plan_info.source_sha256_str
    ):
        raise ValueError("Format plan fingerprint does not match its source")
    verify_original_none(source_path, plan_info.source_sha256_str)
    original_str = decode_source_text(plan_info.original_bytes)
    output_str = decode_source_text(plan_info.output_bytes)
    if original_str is None or output_str is None:
        raise ValueError("Format plan contains unsupported encoding")
    validate_document_none(original_str, output_str, plan_info.path_str)
    write_atomically_none(source_path, plan_info.output_bytes,
                          plan_info.source_sha256_str)


def format_engine_str(text_str: str, path_str: str, width_int: int | None,
                      hug_bool: bool, engine_str: str) -> str:
    """Format a document with the chosen engine.

    Args:
        text_str (str): Decoded original document.
        path_str (str): Python, stub or notebook path.
        width_int (int | None): Optional line width.
        hug_bool (bool): Keep closing brackets on the last content line.
        engine_str (str): "python" or "rust".
    Returns:
        str: Validated complete proposed document.
    Warnings:
        When the Rust engine cannot format a document, the Python engine
        runs and reports its exact refusal.
    """
    if engine_str == "rust":
        from refactrail.engine import load_rust_core
        formatted_str = load_rust_core().format_text(
            text_str, path_str, width_int, hug_bool,
            Path(path_str).suffix == ".ipynb")
        if formatted_str is not None:
            return formatted_str
    return format_document_str(text_str, path_str, width_int, hug_bool)


def format_document_str(text_str: str, path_str: str,
                         width_int: int | None = None,
                         hug_bool: bool = False) -> str:
    """Select source or notebook formatting from the input extension.

    Args:
        text_str (str): Decoded original document.
        path_str (str): Python, stub or notebook path.
        width_int (int | None): Optional line width.
        hug_bool (bool): Keep closing brackets on the last content line.
    Returns:
        str: Validated complete proposed document.
    Warnings:
        All notebook code cells must be supported before any write.
    """
    if Path(path_str).suffix == ".ipynb":
        from refactrail.notebooks import format_notebook_str
        return format_notebook_str(text_str, path_str, width_int, hug_bool)
    return format_source_str(text_str, path_str, width_int, hug_bool)


def validate_document_none(original_str: str, output_str: str,
                            path_str: str) -> None:
    """Validate Python or notebook bytes again immediately before writing.

    Args:
        original_str (str): Original decoded document.
        output_str (str): Proposed decoded document.
        path_str (str): Input filename.
    Returns:
        None: Raises on unsupported or structurally changed documents.
    Warnings:
        Validation never imports or executes target code.
    """
    if Path(path_str).suffix == ".ipynb":
        from refactrail.notebooks import validate_notebook_none
        validate_notebook_none(original_str, output_str, path_str)
    else:
        validate_formatted_none(original_str, output_str, path_str)
