"""Source-preserving token boundaries for the independent formatter."""

from dataclasses import dataclass
from io import StringIO
import ast
import re
import tokenize

# f-strings and, from Python 3.14, t-strings: their lines keep exact spacing.
FORMATTED_STRING_NODES_TUPLE = tuple(
    getattr(ast, name_str) for name_str in ("JoinedStr", "TemplateStr")
    if hasattr(ast, name_str))
DIRECTIVE_PATTERN = re.compile(r"#\s*fmt:\s*(off|on|skip)\b")
IGNORED_TYPES_TUPLE = (
    tokenize.INDENT, tokenize.DEDENT, tokenize.NL, tokenize.NEWLINE,
    tokenize.ENDMARKER, tokenize.ENCODING,
)


@dataclass
class TokenIndex:
    """Keep token and source positions in Unicode character coordinates.

    Args:
        tokens_list: Complete tokenizer output.
        starts_list: Ordered token starts for binary searches.
        lines_list: Normalized physical source lines.
    Returns:
        TokenIndex: Index for an immutable source snapshot.
    Warnings:
        AST offsets are UTF-8 byte offsets and need conversion.
    """

    tokens_list: list[tokenize.TokenInfo]
    starts_list: list[tuple[int, int]]
    lines_list: list[str]


def build_token_index(text_str: str) -> TokenIndex:
    """Tokenize a normalized source snapshot without executing it.

    Args:
        text_str (str): Source with LF line separators.
    Returns:
        TokenIndex: Tokens and searchable starts.
    Warnings:
        Tokenization errors are propagated as refusals.
    """
    tokens_list = list(tokenize.generate_tokens(StringIO(text_str).readline))
    return TokenIndex(tokens_list, [entry.start for entry in tokens_list],
                       text_str.split("\n"))


def collect_protected_lines_set(
    index_info: TokenIndex, tree_node: ast.Module,
    fstrings_bool: bool = True,
) -> set[int]:
    """Retain comments, string spans, disabled regions and continuations.
    Args:
        index_info (TokenIndex): Source and lexical boundaries.
        tree_node (ast.Module): Parsed source.
        fstrings_bool (bool): Also protect lines holding f-strings, whose
            inner whitespace the token-gap formatter must not edit.
    Returns:
        set[int]: One-based lines on which whitespace edits are disabled.
    Warnings:
        Entire protected lines retain surrounding code.
    """
    protected_set: set[int] = set()
    directives_dict: dict[int, str] = {}
    for token_info in index_info.tokens_list:
        if token_info.type == tokenize.COMMENT:
            protected_set.add(token_info.start[0])
            match_info = DIRECTIVE_PATTERN.search(token_info.string)
            if match_info:
                directives_dict[token_info.start[0]] = match_info.group(1)
        if token_info.type == tokenize.STRING and (
            token_info.start[0] != token_info.end[0]
        ):
            protected_set.update(range(token_info.start[0],
                                       token_info.end[0] + 1))
    for node in ast.walk(tree_node) if fstrings_bool else ():
        if isinstance(node, FORMATTED_STRING_NODES_TUPLE):
            protected_set.update(range(node.lineno, node.end_lineno + 1))
    return protected_set | collect_directive_lines_set(index_info,
                                                       directives_dict)


def collect_directive_lines_set(index_info: TokenIndex,
                                directives_dict: dict[int, str]) -> set[int]:
    """Protect fmt-disabled regions and explicit line continuations.

    Args:
        index_info (TokenIndex): Source lines.
        directives_dict (dict[int, str]): Line to "off", "on" or "skip".
    Returns:
        set[int]: Lines inside fmt: off/on regions, fmt: skip lines and
            both lines of each backslash continuation.
    Warnings:
        An unmatched fmt: off protects the rest of the file.
    """
    protected_set: set[int] = set()
    disabled_bool = False
    for number_int, line_str in enumerate(index_info.lines_list, 1):
        directive_str = directives_dict.get(number_int)
        if directive_str == "off":
            disabled_bool = True
        if disabled_bool or directive_str == "skip":
            protected_set.add(number_int)
        if directive_str == "on":
            disabled_bool = False
        if line_str.rstrip().endswith("\\"):
            protected_set.update((number_int, number_int + 1))
    return protected_set


def collect_line_tokens_dict(index_info: TokenIndex) -> dict:
    """Group meaningful single-line tokens by physical source row.

    Args:
        index_info (TokenIndex): Complete token stream.
    Returns:
        dict: Line number to tokens in source order.
    Warnings:
        Protected multiline tokens are not reformatted.
    """
    grouped_dict: dict[int, list[tokenize.TokenInfo]] = {}
    for token_info in index_info.tokens_list:
        if token_info.type not in IGNORED_TYPES_TUPLE and (
            token_info.start[0] == token_info.end[0]
        ):
            grouped_dict.setdefault(token_info.start[0], []).append(token_info)
    return grouped_dict


def collect_literal_tokens_list(index_info: TokenIndex) -> list:
    """Capture literal and comment spelling for post-format verification.

    Args:
        index_info (TokenIndex): Tokenized source.
    Returns:
        list: Token kinds and original spelling in source order.
    Warnings:
        F-string lines are independently protected by AST spans.
    """
    return [(token_info.type, token_info.string)
            for token_info in index_info.tokens_list
            if token_info.type in (tokenize.STRING, tokenize.COMMENT)]
