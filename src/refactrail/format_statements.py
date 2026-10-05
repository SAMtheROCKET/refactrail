"""Whole-statement layout: join a long statement, then split it again.

Only gaps between existing tokens change, and a gap only becomes a line
break inside brackets, so the token sequence and syntax tree are kept.
"""

from collections.abc import Callable
from dataclasses import dataclass, field
import io
import keyword
import tokenize

OPENING_BRACKETS_TUPLE = ("(", "[", "{")
CLOSING_BRACKETS_TUPLE = (")", "]", "}")
HEADER_KEYWORDS_TUPLE = ("if", "elif", "while", "for", "with", "def",
                         "class", "async", "except", "match", "case")
COMPARISON_TUPLE = ("<", ">", "==", "!=", "<=", ">=", "in", "is", "not")
ARITHMETIC_TUPLE = ("+", "-", "*", "/", "//", "%", "@", "**", "|", "&", "^",
                    "<<", ">>")
OPERAND_KINDS_TUPLE = ("word", "string", "close")
SKIPPED_TYPES_TUPLE = (tokenize.NL, tokenize.NEWLINE, tokenize.INDENT,
                       tokenize.DEDENT, tokenize.ENDMARKER)
CONTINUATION_INDENT_STR = "    "


@dataclass
class Atom:
    """One indivisible piece of a statement, such as a token or f-string.

    Args:
        text: Original spelling.
        space: Whether a space precedes it when joined on one line.
        kind: "open", "close", "comma", "op", "word" or "string".
        depth: Bracket depth before this atom, within the statement.
    Returns:
        Atom: Layout unit.
    Warnings:
        Keywords are words; the splitter recognises them by text.
    """

    text: str
    space: bool
    kind: str
    depth: int

# f-string and (Python 3.14) t-string boundaries; -1 where not supported.
STRING_START_TYPES_TUPLE = (getattr(tokenize, "FSTRING_START", -1),
                            getattr(tokenize, "TSTRING_START", -1))
STRING_END_TYPES_TUPLE = (getattr(tokenize, "FSTRING_END", -1),
                          getattr(tokenize, "TSTRING_END", -1))


def classify_atom_str(token_info: tokenize.TokenInfo) -> str:
    """Name the layout kind of one token.

    Args:
        token_info (tokenize.TokenInfo): Token from the statement.
    Returns:
        str: "open", "close", "comma", "op", "string" or "word".
    Warnings:
        Numbers and names are both "word".
    """
    if token_info.string in OPENING_BRACKETS_TUPLE:
        return "open"
    if token_info.string in CLOSING_BRACKETS_TUPLE:
        return "close"
    if token_info.string == ",":
        return "comma"
    if token_info.type == tokenize.OP:
        return "op"
    return "string" if token_info.type == tokenize.STRING else "word"


def decide_space_bool(previous_atom: Atom | None, kind_str: str,
                      text_str: str, gap_int: int | None) -> bool:
    """Decide whether a joined atom is preceded by a space.

    Args:
        previous_atom (Atom | None): Atom before it.
        kind_str (str): Kind of the new atom.
        text_str (str): Spelling of the new atom.
        gap_int (int | None): Original same-line gap, or None when the
            original had a line break here.
    Returns:
        bool: True to keep or insert one space.
    Warnings:
        Same-line spacing is copied from the source unchanged.
    """
    if previous_atom is None:
        return False
    if gap_int is not None:
        return gap_int > 0
    return not (previous_atom.kind == "open" or previous_atom.text == "."
                or kind_str in ("close", "comma") or text_str in (".", ":"))


def build_atoms_list(tokens_list: list, lines_list: list[str]) -> list | None:
    """Turn a statement's tokens into atoms, or refuse the statement.

    Args:
        tokens_list (list): Tokens from statement start to NEWLINE.
        lines_list (list[str]): Source lines for f-string spellings.
    Returns:
        list | None: Atoms, or None for comments, backslashes and
            strings spanning lines.
    Warnings:
        An f-string becomes one atom with its exact original text.
    """
    atoms_list, depth_int, previous_end = [], 0, None
    fstring_start, nesting_int = None, 0
    for token_info in tokens_list:
        if token_info.type == tokenize.COMMENT:
            return None
        if token_info.type in SKIPPED_TYPES_TUPLE:
            continue
        if token_info.type in STRING_START_TYPES_TUPLE:
            nesting_int += 1
            fstring_start = fstring_start or token_info.start
            continue
        if nesting_int:
            if token_info.type in STRING_END_TYPES_TUPLE:
                nesting_int -= 1
            if nesting_int:
                continue
            token_info = slice_fstring_token(fstring_start, token_info,
                                             lines_list)
            fstring_start = None
            if token_info is None:
                return None
        atom_info = make_atom(token_info, atoms_list, previous_end, depth_int)
        if atom_info is None:
            return None
        depth_int += {"open": 1, "close": -1}.get(atom_info.kind, 0)
        atoms_list.append(atom_info)
        previous_end = token_info.end
    return atoms_list


def slice_fstring_token(start_tuple: tuple, end_info: tokenize.TokenInfo,
                        lines_list: list[str]) -> tokenize.TokenInfo | None:
    """Rebuild a single-line f-string as one STRING-like token.

    Args:
        start_tuple (tuple): (line, column) of FSTRING_START.
        end_info (tokenize.TokenInfo): The matching FSTRING_END.
        lines_list (list[str]): Source lines.
    Returns:
        tokenize.TokenInfo | None: Combined token, or None when the
            f-string spans lines.
    Warnings:
        The text is copied from the source, never re-rendered.
    """
    if start_tuple[0] != end_info.end[0]:
        return None
    text_str = lines_list[start_tuple[0] - 1][start_tuple[1]:end_info.end[1]]
    return tokenize.TokenInfo(tokenize.STRING, text_str, start_tuple,
                              end_info.end, end_info.line)


def make_atom(token_info: tokenize.TokenInfo, atoms_list: list,
              previous_end: tuple | None, depth_int: int) -> Atom | None:
    """Create the atom for one token.

    Args:
        token_info (tokenize.TokenInfo): Token to convert.
        atoms_list (list): Atoms so far.
        previous_end (tuple | None): End of the previous token.
        depth_int (int): Bracket depth before the token.
    Returns:
        Atom | None: The atom, or None for strings spanning lines and
            backslash continuations.
    Warnings:
        A line break outside brackets means a backslash continuation.
    """
    if token_info.start[0] != token_info.end[0]:
        return None
    gap_int = None
    if previous_end is not None and previous_end[0] == token_info.start[0]:
        gap_int = token_info.start[1] - previous_end[1]
    elif previous_end is not None and depth_int == 0:
        return None
    kind_str = classify_atom_str(token_info)
    previous_atom = atoms_list[-1] if atoms_list else None
    return Atom(token_info.string, decide_space_bool(
        previous_atom, kind_str, token_info.string, gap_int),
        kind_str, depth_int)


def join_atoms_str(atoms_list: list) -> str:
    """Join atoms on one line.

    Args:
        atoms_list (list): Atoms of a statement or a part of one.
    Returns:
        str: Text without leading indentation.
    Warnings:
        The first atom's leading space is dropped.
    """
    return "".join((" " if atom_info.space and index_int else "")
                   + atom_info.text
                   for index_int, atom_info in enumerate(atoms_list))


def find_pairs_list(atoms_list: list) -> list[tuple[int, int]]:
    """Find outermost bracket pairs of a statement part.

    Args:
        atoms_list (list): Atoms whose own brackets are balanced or not.
    Returns:
        list[tuple[int, int]]: (open index, close index) pairs at the
            lowest depth present, left to right.
    Warnings:
        Brackets closed outside the part are ignored.
    """
    base_int = min(atom_info.depth for atom_info in atoms_list)
    pairs_list, open_int = [], None
    for index_int, atom_info in enumerate(atoms_list):
        if atom_info.kind == "open" and atom_info.depth == base_int:
            open_int = index_int
        elif (atom_info.kind == "close" and atom_info.depth == base_int + 1
              and open_int is not None):
            pairs_list.append((open_int, index_int))
            open_int = None
    return pairs_list


@dataclass
class LayoutStyle:
    """Width and bracket style for whole-statement layout.

    Args:
        width: Maximum line length.
        hug: True keeps a closing bracket on the last content line and
            packs elements; False gives it its own line, one element per
            line when exploding (the common formatter style).
    Returns:
        LayoutStyle: Settings shared by the recursive splitter.
    Warnings:
        Neither style adds or removes tokens. The memo is only valid for
        one statement's atoms; create a new style for every statement.
    """

    width: int
    hug: bool
    memo: dict = field(default_factory=dict)


def fetch_layout_list(style_info: LayoutStyle, atoms_list: list,
                       kind_str: str, indent_str: str,
                       compute: Callable[[], list[str] | None],
                       ) -> list[str] | None:
    """Return a memoized layout of one statement part.

    Args:
        style_info (LayoutStyle): Style holding the statement's memo.
        atoms_list (list): The part (a slice of the statement's atoms).
        kind_str (str): Which layout: "statement", "header" or "body".
        indent_str (str): Indentation of its first line.
        compute (Callable[[], list[str] | None]): Computes the layout
            when it is not stored yet.
    Returns:
        list[str] | None: The layout.
    Warnings:
        Slices are identified by their first and last atom objects and
        length, which is unique within one statement. Without the memo,
        heads re-laid out at every split make the search factorial.
    """
    key_tuple = (kind_str, indent_str, id(atoms_list[0]),
                 id(atoms_list[-1]), len(atoms_list))
    if key_tuple not in style_info.memo:
        style_info.memo[key_tuple] = compute()
    return style_info.memo[key_tuple]


def is_binary_operator_bool(atoms_list: list, index_int: int) -> bool:
    """Tell whether an operator atom joins two operands.

    Args:
        atoms_list (list): Atoms of one statement part.
        index_int (int): Index of the operator atom.
    Returns:
        bool: False for unary minus, unpacking stars and similar.
    Warnings:
        Keywords before an operator make it unary.
    """
    if index_int == 0:
        return False
    previous_atom = atoms_list[index_int - 1]
    return previous_atom.kind in OPERAND_KINDS_TUPLE and (
        previous_atom.text not in keyword.kwlist
        or previous_atom.text in ("True", "False", "None"))


def rank_delimiter_int(atoms_list: list, index_int: int,
                       first_for_int: int | None) -> int:
    """Return the split priority at an atom, or 0 when it is no delimiter.

    Args:
        atoms_list (list): Atoms of one statement part.
        index_int (int): Atom index, at the part's own depth.
        first_for_int (int | None): Index of the first comprehension `for`.
    Returns:
        int: Higher splits first: comprehension 20, comma 18, conditional
            16, or 14, and 13, string concatenation 12, comparison 10,
            arithmetic 8.
    Warnings:
        Commas inside lambda parameters are excluded by the caller.
    """
    atom_info = atoms_list[index_int]
    text_str = atom_info.text
    if atom_info.kind == "comma":
        return 18
    if not index_int:
        return 0
    if text_str == "for" or text_str == "async" and (
            index_int + 1 < len(atoms_list)
            and atoms_list[index_int + 1].text == "for"):
        return 20
    if text_str == "if":
        return 20 if first_for_int is not None and (
            index_int > first_for_int) else 16
    if text_str in ("else", "or", "and"):
        return {"else": 16, "or": 14, "and": 13}[text_str]
    if atom_info.kind == "string" and atoms_list[index_int - 1].kind == (
            "string"):
        return 12
    if not is_binary_operator_bool(atoms_list, index_int):
        return 0
    if text_str in COMPARISON_TUPLE:
        return 10
    return 8 if text_str in ARITHMETIC_TUPLE else 0


def collect_delimiters_tuple(atoms_list: list) -> tuple[int, list[int]]:
    """Find the highest-priority split points of a bracket body.

    Args:
        atoms_list (list): Atoms inside one pair of brackets.
    Returns:
        tuple[int, list[int]]: Priority and atom indexes; commas split
            after the comma, every other delimiter before its atom.
    Warnings:
        Only atoms at the body's own depth are considered. A trailing
        comma alone splits nothing, so lower priorities are used then.
    """
    ranked_dict = rank_delimiters_dict(atoms_list)
    for rank_int in sorted(ranked_dict, reverse=True):
        indexes_list = ranked_dict[rank_int]
        if rank_int != 18 or indexes_list != [len(atoms_list) - 1]:
            return rank_int, indexes_list
    return 0, []


def rank_delimiters_dict(atoms_list: list) -> dict[int, list[int]]:
    """Group a body's split points by priority.

    Args:
        atoms_list (list): Atoms inside one pair of brackets.
    Returns:
        dict[int, list[int]]: Priority to atom indexes.
    Warnings:
        Commas inside lambda parameters are not split points.
    """
    base_int = min(atom_info.depth for atom_info in atoms_list)
    level_list = [index_int for index_int, atom_info in enumerate(atoms_list)
                  if atom_info.depth == base_int]
    first_for_int = next((index_int for index_int in level_list
                          if atoms_list[index_int].text == "for"), None)
    ranked_dict: dict[int, list[int]] = {}
    lambda_bool = False
    for index_int in level_list:
        text_str = atoms_list[index_int].text
        if text_str == "lambda":
            lambda_bool = True
        elif text_str == ":" and lambda_bool:
            lambda_bool = False
        elif not (lambda_bool and atoms_list[index_int].kind == "comma"):
            rank_int = rank_delimiter_int(atoms_list, index_int,
                                          first_for_int)
            if rank_int:
                ranked_dict.setdefault(rank_int, []).append(index_int)
    return ranked_dict


def cut_pieces_list(atoms_list: list, rank_int: int,
                    indexes_list: list[int]) -> list[list]:
    """Cut a body into pieces at its delimiters.

    Args:
        atoms_list (list): Body atoms.
        rank_int (int): Delimiter priority (18 means commas).
        indexes_list (list[int]): Delimiter atom indexes.
    Returns:
        list[list]: Non-empty atom pieces in order.
    Warnings:
        Commas stay at the end of their piece; other delimiters start
        the next piece.
    """
    cuts_list = [index_int + 1 if rank_int == 18 else index_int
                 for index_int in indexes_list]
    pieces_list, start_int = [], 0
    for cut_int in [*cuts_list, len(atoms_list)]:
        if atoms_list[start_int:cut_int]:
            pieces_list.append(atoms_list[start_int:cut_int])
        start_int = cut_int
    return pieces_list


def render_body_list(atoms_list: list, indent_str: str,
                     style_info: LayoutStyle) -> list[str]:
    """Lay out the atoms inside a bracket pair, once per statement part.

    Args:
        atoms_list (list): Body atoms.
        indent_str (str): Indentation for body lines.
        style_info (LayoutStyle): Width, bracket style and memo.
    Returns:
        list[str]: Body lines.
    Warnings:
        See compute_body_list for the layout rules.
    """
    return fetch_layout_list(style_info, atoms_list, "body", indent_str,
                              lambda: compute_body_list(
                                  atoms_list, indent_str, style_info))


def compute_body_list(atoms_list: list, indent_str: str,
                      style_info: LayoutStyle) -> list[str]:
    """Lay out the atoms inside a bracket pair.

    Args:
        atoms_list (list): Body atoms.
        indent_str (str): Indentation for body lines.
        style_info (LayoutStyle): Width and bracket style.
    Returns:
        list[str]: Body lines; long pieces are split recursively.
    Warnings:
        Unsplittable pieces may remain longer than the width. The hug
        style packs pieces onto a line while they fit.
    """
    line_str = indent_str + join_atoms_str(atoms_list)
    if len(line_str) <= style_info.width:
        return [line_str]
    rank_int, indexes_list = collect_delimiters_tuple(atoms_list)
    pieces_list = cut_pieces_list(atoms_list, rank_int, indexes_list)
    if len(pieces_list) < 2:
        return render_statement_list(atoms_list, indent_str, style_info,
                                     False) or [line_str]
    lines_list: list[str] = []
    for piece_list in pieces_list:
        piece_lines_list = render_body_list(piece_list, indent_str,
                                            style_info)
        joined_str = (lines_list[-1] + " " + piece_lines_list[0].lstrip()
                      if lines_list else "")
        if (style_info.hug and len(piece_lines_list) == 1 and lines_list
                and len(joined_str) <= style_info.width):
            lines_list[-1] = joined_str
        else:
            lines_list += piece_lines_list
    return lines_list


def split_pair_list(atoms_list: list, pair_tuple: tuple[int, int],
                    indent_str: str, header_bool: bool,
                    style_info: LayoutStyle) -> list[str]:
    """Split a statement part at one bracket pair.

    Args:
        atoms_list (list): Atoms of the part.
        pair_tuple (tuple[int, int]): Opening and closing indexes.
        indent_str (str): Indentation of the part's first line.
        header_bool (bool): Whether it is a compound statement header.
        style_info (LayoutStyle): Width and bracket style.
    Returns:
        list[str]: Head lines ending with the opening bracket, body lines
            and the closing bracket with the rest.
    Warnings:
        Headers of compound statements get a deeper body indentation in
        the hug style, so that the block below stays distinguishable.
    """
    open_int, close_int = pair_tuple
    extra_str = CONTINUATION_INDENT_STR * (
        2 if header_bool and style_info.hug else 1)
    head_atoms_list = atoms_list[:open_int + 1]
    head_list = render_statement_list(head_atoms_list, indent_str,
                                      style_info, header_bool) or [
        indent_str + join_atoms_str(head_atoms_list)]
    body_list = render_body_list(atoms_list[open_int + 1:close_int],
                                 indent_str + extra_str, style_info)
    tail_str = join_atoms_str(atoms_list[close_int:])
    if style_info.hug and len(body_list[-1] + tail_str) <= style_info.width:
        return head_list + body_list[:-1] + [body_list[-1] + tail_str]
    tail_list = render_statement_list(atoms_list[close_int:], indent_str,
                                      style_info, header_bool) or [
        indent_str + tail_str]
    return head_list + body_list + tail_list


def unwrap_single_pair_tuple(atoms_list: list,
                             pair_tuple: tuple[int, int]) -> tuple[int, int]:
    """Use the inner pair when a bracket holds one bracketed element.

    Args:
        atoms_list (list): Atoms of a statement part.
        pair_tuple (tuple[int, int]): Outer opening and closing indexes.
    Returns:
        tuple[int, int]: The innermost pair reached through bodies such
            as `(*(...))` or `([...])`, else pair_tuple itself.
    Warnings:
        Only operator atoms (such as `*`) may precede the inner bracket.
    """
    open_int, close_int = pair_tuple
    start_int = open_int + 1
    while start_int < close_int and atoms_list[start_int].kind == "op":
        start_int += 1
    if start_int >= close_int or atoms_list[start_int].kind != "open":
        return pair_tuple
    inner_list = find_pairs_list(atoms_list[start_int:close_int])
    if len(inner_list) != 1 or inner_list[0] != (0, close_int - start_int - 1):
        return pair_tuple
    if close_int - start_int - 1 <= 1:
        return pair_tuple
    return unwrap_single_pair_tuple(atoms_list, (start_int, close_int - 1))


def render_statement_list(atoms_list: list, indent_str: str,
                          style_info: LayoutStyle,
                          header_bool: bool) -> list[str] | None:
    """Lay out a statement part, computing each distinct part once.

    Args:
        atoms_list (list): Atoms to lay out.
        indent_str (str): Indentation of the first line.
        style_info (LayoutStyle): Width, bracket style and memo.
        header_bool (bool): Whether it is a compound statement header.
    Returns:
        list[str] | None: Lines, or None when no bracket split helps.
    Warnings:
        See compute_statement_list for the layout rules.
    """
    return fetch_layout_list(
        style_info, atoms_list, "header" if header_bool else "statement",
        indent_str, lambda: compute_statement_list(
            atoms_list, indent_str, style_info, header_bool))


def compute_statement_list(atoms_list: list, indent_str: str,
                           style_info: LayoutStyle,
                           header_bool: bool) -> list[str] | None:
    """Lay out a statement (or part of one) within the width if possible.

    Args:
        atoms_list (list): Atoms to lay out.
        indent_str (str): Indentation of the first line.
        style_info (LayoutStyle): Width and bracket style.
        header_bool (bool): Whether it is a compound statement header.
    Returns:
        list[str] | None: Lines, or None when no bracket split helps.
    Warnings:
        Tries every outermost bracket pair and keeps the fitting layout
        with the fewest lines (ties: rightmost pair, leftmost for
        headers), else the layout with the shortest longest line.
    """
    line_str = indent_str + join_atoms_str(atoms_list)
    if len(line_str) <= style_info.width:
        return [line_str]
    pairs_list = [pair_tuple for pair_tuple in find_pairs_list(atoms_list)
                  if pair_tuple[1] > pair_tuple[0] + 1]
    if style_info.hug:
        pairs_list = [unwrap_single_pair_tuple(atoms_list, pair_tuple)
                      for pair_tuple in pairs_list]
    layouts_list = [split_pair_list(atoms_list, pair_tuple, indent_str,
                                    header_bool, style_info)
                    for pair_tuple in (pairs_list if header_bool
                                       else pairs_list[::-1])]
    fitting_list = [lines_list for lines_list in layouts_list
                    if max(map(len, lines_list)) <= style_info.width]
    if fitting_list:
        return min(fitting_list, key=len)
    best_list = min(layouts_list, key=lambda lines_list: max(
        map(len, lines_list)), default=None)
    if best_list is not None and max(map(len, best_list)) < len(line_str):
        return best_list
    return None


def collect_statements_list(tokens_list: list) -> list[list]:
    """Group tokens into logical lines.

    Args:
        tokens_list (list): Tokens of a whole module.
    Returns:
        list[list]: Tokens from each statement's first token to its
            NEWLINE, comments inside included.
    Warnings:
        Comment-only lines before a statement are not part of it.
    """
    statements_list, current_list = [], []
    for token_info in tokens_list:
        if not current_list and token_info.type in (
                *SKIPPED_TYPES_TUPLE, tokenize.COMMENT):
            continue
        current_list.append(token_info)
        if token_info.type == tokenize.NEWLINE:
            statements_list.append(current_list)
            current_list = []
    return statements_list


def layout_statement_list(tokens_list: list, lines_list: list[str],
                          style_info: LayoutStyle) -> list[str] | None:
    """Propose new lines for one over-long statement.

    Args:
        tokens_list (list): The statement's tokens.
        lines_list (list[str]): Source lines (LF, without separators).
        style_info (LayoutStyle): Width and bracket style.
    Returns:
        list[str] | None: Replacement lines, or None to keep it as is.
    Warnings:
        A proposal is used only when its longest line is shorter.
    """
    first_int, last_int = tokens_list[0].start[0], tokens_list[-1].start[0]
    old_list = lines_list[first_int - 1:last_int]
    if max(map(len, old_list)) <= style_info.width:
        return None
    first_str = old_list[0]
    indent_str = first_str[:len(first_str) - len(first_str.lstrip(" \t"))]
    atoms_list = build_atoms_list(tokens_list, lines_list)
    if not atoms_list or "\t" in indent_str:
        return None
    new_list = render_statement_list(
        atoms_list, indent_str, LayoutStyle(style_info.width, style_info.hug),
        atoms_list[0].text in HEADER_KEYWORDS_TUPLE)
    if not new_list or new_list == old_list or max(map(len, new_list)) >= max(
            map(len, old_list)):
        return None
    return new_list


def plan_layouts_list(text_str: str, protected_set: set[int],
                      style_info: LayoutStyle) -> list[tuple]:
    """Plan new layouts for every over-long statement of a module.

    Args:
        text_str (str): Compilable source with LF line separators.
        protected_set (set[int]): Lines that must not change.
        style_info (LayoutStyle): Width and bracket style.
    Returns:
        list[tuple]: (first line, last line, new lines) per statement,
            from the bottom of the module upwards.
    Warnings:
        Statements touching protected lines are skipped entirely.
    """
    lines_list = text_str.split("\n")
    tokens_list = list(tokenize.generate_tokens(
        io.StringIO(text_str).readline))
    plans_list = []
    for statement_list in reversed(collect_statements_list(tokens_list)):
        first_int = statement_list[0].start[0]
        last_int = statement_list[-1].start[0]
        if protected_set & set(range(first_int, last_int + 1)):
            continue
        new_list = layout_statement_list(statement_list, lines_list,
                                         style_info)
        if new_list is not None:
            plans_list.append((first_int, last_int, new_list))
    return plans_list
