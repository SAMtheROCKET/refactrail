"""Safe fixes for Ruff-compatible lint findings (lint --fix).

Only findings the linter reports (after noqa suppression) are fixed,
and only those whose fix cannot change behaviour beyond what Ruff also
calls safe:

- F401: remove unused imports (not in __init__.py files or .pyi stubs,
  and not in a try body that handles ImportError, where the import is
  a check).
- F541: drop the f prefix of an f-string without placeholders.
- F632: use == / != instead of is / is not with a literal.
- E703: remove a statement's unnecessary trailing semicolon.
- E713 / E714: write `not x in y` as `x not in y` and `not x is y` as
  `x is not y`.

Each pass is verified: the fixed text must compile, and its syntax tree
must equal the original tree with exactly the intended changes. A pass
whose check fails is discarded with a note; the file is never left
half-fixed.
"""

import ast
from dataclasses import dataclass, field
import io
import tokenize

from refactrail.correctness import check_correctness_list
from refactrail.engine import parse_quietly_node
from refactrail.models import Finding

FIXABLE_CODES_TUPLE = ("F401", "F541", "F632", "E703", "E713", "E714")
MAX_PASSES_INT = 4
DEFERRED_NOTE_STR = "overlapping fixes wait for the next pass"
IMPORT_ERROR_NAMES_FROZENSET = frozenset((
    "ImportError", "ModuleNotFoundError", "Exception", "BaseException"))


@dataclass
class CompatFixOutcome:
    """The fixes applied to one file.

    Args:
        text: The fixed text (the input when nothing applied).
        applied: "line:column CODE message" per fixed finding.
        notes: Why a finding or a pass was not fixed.
    Returns:
        CompatFixOutcome: Plain data.
    Warnings:
        Removing an import also removes its import-time side effects,
        as Ruff's fix does.
    """

    text: str
    applied: list[str] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)


@dataclass
class Edit:
    """Replace text[start:end] with replacement.

    Args:
        start: Start offset in the text.
        end: End offset in the text.
        replacement: New text.
        finding: The finding the edit fixes.
        also: Further findings the same edit fixes.
        group: Edits with the same group stand or fall together (0:
            the edit's own finding).
    Returns:
        Edit: Plain data.
    Warnings:
        Edits of one pass must not overlap.
    """

    start: int
    end: int
    replacement: str
    finding: Finding
    also: list[Finding] = field(default_factory=list)
    group: int = 0

    def group_key_int(self) -> int:
        """The key edits that must be applied together share.

        Args:
            None.
        Returns:
            int: The group, or the finding's id for a lone edit.
        Warnings:
            None.
        """
        return self.group or id(self.finding)


class SourceIndex:
    """Offsets, tokens and nodes of one text, for locating edits."""

    def __init__(self, text_str: str) -> None:
        """Index a module's text.

        Args:
            text_str (str): The module text (LF line endings).
        Returns:
            None.
        Warnings:
            The text must parse.
        """
        self.text = text_str
        self.lines_list = text_str.splitlines(keepends=True)
        self.starts_list = [0]
        for line_str in self.lines_list:
            self.starts_list.append(self.starts_list[-1] + len(line_str))
        self.tree = parse_quietly_node(text_str, "<fix>")
        self.tokens_list = list(tokenize.generate_tokens(
            io.StringIO(text_str).readline))

    def find_offset_int(self, line_int: int, column_int: int) -> int:
        """The text offset of a 1-based line and 0-based character column.

        Args:
            line_int (int): 1-based line.
            column_int (int): 0-based column in characters.
        Returns:
            int: Offset into the text.
        Warnings:
            None.
        """
        return self.starts_list[line_int - 1] + column_int

    def find_node_start_int(self, node: ast.AST) -> int:
        """The text offset where a node starts.

        Args:
            node (ast.AST): A node with a position.
        Returns:
            int: Offset into the text.
        Warnings:
            AST columns are UTF-8 byte offsets; they are converted.
        """
        return self.find_byte_offset_int(node.lineno, node.col_offset)

    def find_node_end_int(self, node: ast.AST) -> int:
        """The text offset where a node ends.

        Args:
            node (ast.AST): A node with an end position.
        Returns:
            int: Offset into the text.
        Warnings:
            None.
        """
        return self.find_byte_offset_int(node.end_lineno, node.end_col_offset)

    def find_byte_offset_int(self, line_int: int, byte_int: int) -> int:
        """Convert a line and UTF-8 byte column into a text offset.

        Args:
            line_int (int): 1-based line.
            byte_int (int): Byte column.
        Returns:
            int: Offset into the text.
        Warnings:
            None.
        """
        line_str = self.lines_list[line_int - 1] if line_int <= len(
            self.lines_list) else ""
        prefix_str = line_str.encode("utf-8")[:byte_int].decode(
            "utf-8", "ignore")
        return self.starts_list[line_int - 1] + len(prefix_str)

    def find_token_index_int(self, offset_int: int) -> int:
        """The index of the token starting at an offset, or -1.

        Args:
            offset_int (int): Offset into the text.
        Returns:
            int: Token index, or -1 when no token starts there.
        Warnings:
            None.
        """
        for index_int, token_info in enumerate(self.tokens_list):
            if self.find_offset_int(*token_info.start) == offset_int:
                return index_int
        return -1

    def find_nodes_at_list(self, offset_int: int, node_type: type
                           ) -> list[ast.AST]:
        """Nodes of a type starting at an offset, outermost first.

        Args:
            offset_int (int): Offset into the text.
            node_type (type): The node class wanted.
        Returns:
            list[ast.AST]: Matching nodes.
        Warnings:
            None.
        """
        return [node for node in ast.walk(self.tree)
                if isinstance(node, node_type) and hasattr(node, "lineno")
                and self.find_node_start_int(node) == offset_int]


def build_parents_dict(tree: ast.AST) -> dict[int, ast.AST]:
    """Map each node's id to its parent node.

    Args:
        tree (ast.AST): The module tree.
    Returns:
        dict[int, ast.AST]: id(child) -> parent.
    Warnings:
        None.
    """
    parents_dict = {}
    for node in ast.walk(tree):
        for child in ast.iter_child_nodes(node):
            parents_dict[id(child)] = node
    return parents_dict


def edit_semicolon_list(index: SourceIndex, finding: Finding) -> list[Edit]:
    """E703: delete the semicolon (spaces before it stay, as in Ruff).

    Args:
        index (SourceIndex): The indexed text.
        finding (Finding): The E703 finding (at the semicolon).
    Returns:
        list[Edit]: One edit, or [] when the text is unexpected.
    Warnings:
        None.
    """
    offset_int = index.find_offset_int(finding.line, finding.column - 1)
    if index.text[offset_int:offset_int + 1] != ";":
        return []
    return [Edit(offset_int, offset_int + 1, "", finding)]


def edit_fstring_list(index: SourceIndex, finding: Finding) -> list[Edit]:
    """F541: drop the f prefix and undouble the braces.

    Args:
        index (SourceIndex): The indexed text.
        finding (Finding): The F541 finding (at the f-string).
    Returns:
        list[Edit]: One edit, or [] when the f-string has fields.
    Warnings:
        Needs Python 3.12 tokens (FSTRING_START and FSTRING_END).
    """
    offset_int = index.find_offset_int(finding.line, finding.column - 1)
    token_int = index.find_token_index_int(offset_int)
    tokens_list = index.tokens_list
    if token_int < 0 or tokens_list[token_int].type != tokenize.FSTRING_START:
        return []
    depth_int = 0
    for end_int in range(token_int, len(tokens_list)):
        kind_int = tokens_list[end_int].type
        if kind_int == tokenize.FSTRING_START:
            depth_int += 1
        elif kind_int == tokenize.FSTRING_END:
            depth_int -= 1
            if depth_int == 0:
                break
        elif kind_int != tokenize.FSTRING_MIDDLE:
            return []  # a replacement field
    else:
        return []
    start_str = tokens_list[token_int].string
    quote_str = tokens_list[end_int].string
    end_offset_int = index.find_offset_int(*tokens_list[end_int].end)
    body_str = index.text[offset_int + len(start_str):
                          end_offset_int - len(quote_str)]
    prefix_str = "".join(char_str for char_str in start_str
                         if char_str not in "fF")
    body_str = body_str.replace("{{", "{").replace("}}", "}")
    return [Edit(offset_int, end_offset_int,
                 prefix_str + body_str + quote_str, finding)]


def find_operator_span_tuple(index: SourceIndex, left: ast.AST,
                             right: ast.AST) -> tuple[int, int] | None:
    """The text span of the comparison operator between two operands.

    Args:
        index (SourceIndex): The indexed text.
        left (ast.AST): The left operand.
        right (ast.AST): The right operand.
    Returns:
        tuple[int, int] | None: (start, end) of the operator words, or
            None when they cannot be found.
    Warnings:
        Parentheses around the operands are skipped.
    """
    left_end_int = index.find_node_end_int(left)
    right_start_int = index.find_node_start_int(right)
    words_list = [token_info for token_info in index.tokens_list
                  if token_info.type == tokenize.NAME
                  and left_end_int <= index.find_offset_int(*token_info.start)
                  and index.find_offset_int(*token_info.end) <= right_start_int
                  and token_info.string in ("is", "not", "in")]
    if not words_list:
        return None
    return (index.find_offset_int(*words_list[0].start),
            index.find_offset_int(*words_list[-1].end))


def edit_literal_identity_list(index: SourceIndex, finding: Finding,
                               changes_dict: dict) -> list[Edit]:
    """F632: replace is / is not by == / != in a one-operator compare.

    Args:
        index (SourceIndex): The indexed text.
        finding (Finding): The F632 finding (at the comparison).
        changes_dict (dict): id(node) -> change, filled for verification.
    Returns:
        list[Edit]: One edit, or [] for chained comparisons.
    Warnings:
        None.
    """
    offset_int = index.find_offset_int(finding.line, finding.column - 1)
    for node in index.find_nodes_at_list(offset_int, ast.Compare):
        if len(node.ops) != 1 or not isinstance(node.ops[0],
                                                (ast.Is, ast.IsNot)):
            continue
        span_tuple = find_operator_span_tuple(index, node.left,
                                              node.comparators[0])
        if span_tuple is None:
            return []
        changes_dict[id(node)] = ("compare_eq", None, id(node))
        new_str = "==" if isinstance(node.ops[0], ast.Is) else "!="
        return [Edit(*span_tuple, new_str, finding, group=id(node))]
    return []


def edit_negated_test_list(index: SourceIndex, finding: Finding,
                           parents_dict: dict, changes_dict: dict
                           ) -> list[Edit]:
    """E713 / E714: move a leading not into the operator.

    Args:
        index (SourceIndex): The indexed text.
        finding (Finding): The finding (at the compared operand).
        parents_dict (dict): id(child) -> parent.
        changes_dict (dict): id(node) -> change, filled for verification.
    Returns:
        list[Edit]: Two edits (drop "not", rewrite the operator), or [].
    Warnings:
        None.
    """
    offset_int = index.find_offset_int(finding.line, finding.column - 1)
    wanted_type = ast.In if finding.code == "E713" else ast.Is
    for node in index.find_nodes_at_list(offset_int, ast.Compare):
        parent = parents_dict.get(id(node))
        if not (len(node.ops) == 1 and isinstance(node.ops[0], wanted_type)
                and isinstance(parent, ast.UnaryOp)
                and isinstance(parent.op, ast.Not)):
            continue
        not_int = index.find_token_index_int(index.find_node_start_int(parent))
        span_tuple = find_operator_span_tuple(index, node.left,
                                              node.comparators[0])
        if not_int < 0 or span_tuple is None:
            return []
        close_int = find_wrapping_close_int(index, not_int + 1,
                                            index.find_node_end_int(parent))
        drop_int = not_int + (2 if close_int >= 0 else 1)
        while index.tokens_list[drop_int].type == tokenize.NL:
            drop_int += 1
        if close_int >= 0 and index.tokens_list[drop_int].type == (
                tokenize.COMMENT):
            close_int, drop_int = -1, not_int + 1  # keep a comment
        not_end_int = index.find_offset_int(*index.tokens_list[drop_int].start)
        changes_dict[id(parent)] = ("negate", node, id(parent))
        new_str = "not in" if finding.code == "E713" else "is not"
        edits_list = [Edit(index.find_node_start_int(parent), not_end_int, "",
                           finding, group=id(parent)),
                      Edit(*span_tuple, new_str, finding, group=id(parent))]
        if close_int >= 0:
            inner_end_int = index.find_node_end_int(node)
            edits_list.append(Edit(inner_end_int, close_int + 1, "", finding,
                                   group=id(parent)))
        return edits_list
    return []


def find_wrapping_close_int(index: SourceIndex, open_int: int,
                            end_int: int) -> int:
    """The offset of the ) that closes a ( and ends an expression.

    Args:
        index (SourceIndex): The indexed text.
        open_int (int): Index of the token that may be "(".
        end_int (int): Offset where the expression ends.
    Returns:
        int: The ) offset when the ( wraps everything up to end_int
            (so `not (x in y)` loses its brackets), else -1.
    Warnings:
        None.
    """
    tokens_list = index.tokens_list
    if tokens_list[open_int].string != "(":
        return -1
    depth_int = 0
    for token_info in tokens_list[open_int:]:
        if token_info.type == tokenize.OP and token_info.string in "([{":
            depth_int += 1
        elif token_info.type == tokenize.OP and token_info.string in ")]}":
            depth_int -= 1
            if depth_int == 0:
                close_int = index.find_offset_int(*token_info.start)
                return close_int if close_int + 1 == end_int else -1
    return -1


def check_import_guarded_bool(statement: ast.stmt, parents_dict: dict
                              ) -> bool:
    """Whether an import sits in a try body that handles import errors.

    Args:
        statement (ast.stmt): The import statement.
        parents_dict (dict): id(child) -> parent.
    Returns:
        bool: True when an enclosing try catches ImportError (or a
            broader or bare handler), so the import is a check.
    Warnings:
        None.
    """
    node = statement
    while id(node) in parents_dict:
        parent = parents_dict[id(node)]
        if isinstance(parent, (ast.Try, ast.TryStar)) and any(
                node is child for child in parent.body):
            for handler in parent.handlers:
                if handler.type is None or any(
                        isinstance(name_node, ast.Name)
                        and name_node.id in IMPORT_ERROR_NAMES_FROZENSET
                        for name_node in ast.walk(handler.type)):
                    return True
        node = parent
    return False


def check_own_lines_bool(index: SourceIndex, statement: ast.stmt) -> bool:
    """Whether a statement is alone on its lines (comments aside).

    Args:
        index (SourceIndex): The indexed text.
        statement (ast.stmt): The statement.
    Returns:
        bool: True when only spaces precede it and only spaces or a
            comment follow it on its first and last lines.
    Warnings:
        None.
    """
    start_int = index.find_node_start_int(statement)
    end_int = index.find_node_end_int(statement)
    line_start_int = index.starts_list[statement.lineno - 1]
    line_end_int = index.starts_list[statement.end_lineno]
    before_str = index.text[line_start_int:start_int]
    after_str = index.text[end_int:line_end_int].strip()
    return not before_str.strip() and (not after_str
                                       or after_str.startswith("#"))


def edit_imports_list(index: SourceIndex, findings_list: list[Finding],
                      parents_dict: dict, changes_dict: dict,
                      notes_list: list[str]) -> list[Edit]:
    """F401: remove unused names, or whole import statements.

    Args:
        index (SourceIndex): The indexed text.
        findings_list (list[Finding]): The F401 findings of the file.
        parents_dict (dict): id(child) -> parent.
        changes_dict (dict): id(node) -> change, filled for verification.
        notes_list (list[str]): Receives findings left unfixed.
    Returns:
        list[Edit]: Edits removing names or statements.
    Warnings:
        A body left empty receives `pass`.
    """
    by_statement_dict: dict[int, tuple[ast.stmt, list, list]] = {}
    for finding in findings_list:
        offset_int = index.find_offset_int(finding.line, finding.column - 1)
        aliases_list = [
            node for node in ast.walk(index.tree)
            if isinstance(node, ast.alias)
            and index.find_node_start_int(node) <= offset_int
            < index.find_node_end_int(node)]
        statement = parents_dict.get(id(aliases_list[0])) if (
            aliases_list) else None
        if statement is None or check_import_guarded_bool(statement,
                                                          parents_dict):
            notes_list.append(f"{finding.line}:{finding.column} F401 not "
                              "fixed: import is guarded or not found")
            continue
        entry = by_statement_dict.setdefault(id(statement),
                                             (statement, [], []))
        entry[1].append(aliases_list[0])
        entry[2].append(finding)
    edits_list = []
    for statement, unused_list, statement_findings_list in (
            by_statement_dict.values()):
        edits_list.extend(edit_import_statement_list(
            index, statement, unused_list, statement_findings_list,
            parents_dict, changes_dict, notes_list))
    return edits_list


def edit_alias_runs_list(index: SourceIndex, statement: ast.stmt,
                         unused_list: list, findings_list: list
                         ) -> list[Edit]:
    """Cut runs of unused names out of an import, keeping its layout.

    Args:
        index (SourceIndex): The indexed text.
        statement (ast.stmt): The import statement (some names stay).
        unused_list (list): The unused aliases.
        findings_list (list): Their findings, in the same order.
    Returns:
        list[Edit]: One deletion per run of adjacent unused names: up
            to the next kept name, or for a final run from the end of
            the last kept name; [] when a cut would remove a comment.
    Warnings:
        Parentheses, line breaks and trailing commas are kept.
    """
    finding_dict = {id(alias): finding
                    for alias, finding in zip(unused_list, findings_list)}
    names_list = statement.names
    edits_list = []
    index_int = 0
    while index_int < len(names_list):
        if id(names_list[index_int]) not in finding_dict:
            index_int += 1
            continue
        run_end_int = index_int
        while (run_end_int + 1 < len(names_list)
               and id(names_list[run_end_int + 1]) in finding_dict):
            run_end_int += 1
        finding = finding_dict[id(names_list[index_int])]
        if run_end_int + 1 < len(names_list):
            start_int = index.find_node_start_int(names_list[index_int])
            end_int = index.find_node_start_int(names_list[run_end_int + 1])
        else:
            start_int = index.find_node_end_int(names_list[index_int - 1])
            end_int = index.find_node_end_int(names_list[run_end_int])
        if "#" in index.text[start_int:end_int]:
            return []
        edits_list.append(Edit(start_int, end_int, "", finding, [
            finding_dict[id(alias)]
            for alias in names_list[index_int + 1:run_end_int + 1]],
            id(statement)))
        index_int = run_end_int + 1
    return edits_list


def edit_import_statement_list(index: SourceIndex, statement: ast.stmt,
                               unused_list: list, findings_list: list,
                               parents_dict: dict, changes_dict: dict,
                               notes_list: list[str]) -> list[Edit]:
    """Remove unused names from one import statement.

    Args:
        index (SourceIndex): The indexed text.
        statement (ast.stmt): The import statement.
        unused_list (list): Its unused aliases.
        findings_list (list): Their findings.
        parents_dict (dict): id(child) -> parent.
        changes_dict (dict): id(node) -> change, filled for verification.
        notes_list (list[str]): Receives findings left unfixed.
    Returns:
        list[Edit]: The edits, or [] when a whole statement shares its
            line or a cut would remove a comment.
    Warnings:
        None.
    """
    kept_list = [alias for alias in statement.names
                 if all(alias is not unused for unused in unused_list)]
    finding = findings_list[0]
    edits_list = (edit_alias_runs_list(index, statement, unused_list,
                                       findings_list) if kept_list else [])
    if kept_list and edits_list:
        changes_dict[id(statement)] = ("keep_aliases", kept_list,
                                       id(statement))
        return edits_list
    if kept_list or not check_own_lines_bool(index, statement):
        notes_list.extend(f"{open_finding.line}:{open_finding.column} F401 "
                          "not fixed: the statement shares its line or a "
                          "comment would be removed"
                          for open_finding in findings_list)
        return []
    parent = parents_dict[id(statement)]
    field_str = next(name_str for name_str in ("body", "orelse",
                                               "finalbody")
                     if any(body_statement is statement for body_statement in
                            getattr(parent, name_str, None) or []))
    changes_dict[id(statement)] = ("remove", (parent, field_str),
                                   id(statement))
    line_start_int = index.starts_list[statement.lineno - 1]
    line_end_int = index.starts_list[statement.end_lineno]
    return [Edit(line_start_int, line_end_int, "", finding,
                 [*findings_list[1:]], id(statement))]


def build_expected_tree(tree: ast.AST, changes_dict: dict,
                        text_str: str) -> ast.AST:
    """The original tree with exactly the intended changes made.

    Args:
        tree (ast.AST): The original module tree.
        changes_dict (dict): id(original node) -> (change, detail).
        text_str (str): The original text, parsed again for the copy
            (much faster than copy.deepcopy of a large tree).
    Returns:
        ast.AST: A changed copy; the original is not modified.
    Warnings:
        Bodies emptied by removed imports receive `pass`, as the
        edits do.
    """
    clone = parse_quietly_node(text_str, "<fix>")
    mapping_dict = {id(original): copied for original, copied
                    in zip(ast.walk(tree), ast.walk(clone))}
    negated_dict = {}
    for node_id, (change_str, detail, _) in changes_dict.items():
        node = mapping_dict[node_id]
        if change_str == "compare_eq":
            node.ops = [ast.Eq() if isinstance(node.ops[0], ast.Is)
                        else ast.NotEq()]
        elif change_str == "negate":
            compare = mapping_dict[id(detail)]
            compare.ops = [ast.NotIn() if isinstance(compare.ops[0], ast.In)
                           else ast.IsNot()]
            negated_dict[id(node)] = compare
        elif change_str == "keep_aliases":
            node.names = [mapping_dict[id(alias)] for alias in detail]
        elif change_str == "remove":
            parent, field_str = detail
            body_list = getattr(mapping_dict[id(parent)], field_str)
            body_list[:] = [body_statement for body_statement in body_list
                            if body_statement is not node]
            if not body_list and not isinstance(parent, ast.Module):
                body_list.append(ast.Pass())
    return NegateTransformer(negated_dict).visit(clone)


class NegateTransformer(ast.NodeTransformer):
    """Replace chosen `not` operations by their prepared comparisons."""

    def __init__(self, negated_dict: dict) -> None:
        """Remember the replacements.

        Args:
            negated_dict (dict): id(UnaryOp) -> Compare to use instead.
        Returns:
            None.
        Warnings:
            None.
        """
        self.negated_dict = negated_dict

    def visit_UnaryOp(self, node: ast.UnaryOp) -> ast.AST:
        """Replace a chosen `not` by its comparison.

        Args:
            node (ast.UnaryOp): The operation.
        Returns:
            ast.AST: The replacement or the visited node.
        Warnings:
            None.
        """
        self.generic_visit(node)
        return self.negated_dict.get(id(node), node)


class UnifyTransformer(ast.NodeTransformer):
    """Unify string forms that denote the same value."""

    def visit_JoinedStr(self, node: ast.JoinedStr) -> ast.AST:
        """Turn an f-string of constants into the string it equals.

        Args:
            node (ast.JoinedStr): The f-string.
        Returns:
            ast.AST: A Constant, or the visited node.
        Warnings:
            None.
        """
        self.generic_visit(node)
        if all(isinstance(part, ast.Constant) for part in node.values):
            return ast.Constant("".join(part.value for part in node.values))
        return node

    def visit_Constant(self, node: ast.Constant) -> ast.AST:
        """Ignore the u prefix kind of string constants.

        Args:
            node (ast.Constant): The constant.
        Returns:
            ast.AST: The constant without a kind.
        Warnings:
            None.
        """
        node.kind = None
        return node


def find_removed_groups_dict(index: SourceIndex, changes_dict: dict
                             ) -> dict[tuple[int, str], list]:
    """Group removed statements by the body they are removed from.

    Args:
        index (SourceIndex): The indexed text.
        changes_dict (dict): id(node) -> change.
    Returns:
        dict: (id(parent), field) -> [parent, removed statements...].
    Warnings:
        None.
    """
    removed_dict: dict[tuple[int, str], list] = {}
    for node in ast.walk(index.tree):
        change = changes_dict.get(id(node))
        if change and change[0] == "remove":
            parent, field_str = change[1]
            removed_dict.setdefault((id(parent), field_str),
                                    [parent]).append(node)
    return removed_dict


def fill_empty_bodies_none(index: SourceIndex, edits_list: list[Edit],
                           changes_dict: dict) -> None:
    """Give `pass` to a block whose every statement is removed.

    Args:
        index (SourceIndex): The indexed text.
        edits_list (list[Edit]): The pass's edits (changed in place).
        changes_dict (dict): id(node) -> change.
    Returns:
        None.
    Warnings:
        Module bodies may become empty and get nothing.
    """
    for (_, field_str), group_list in find_removed_groups_dict(
            index, changes_dict).items():
        parent, removed_list = group_list[0], group_list[1:]
        body_list = getattr(parent, field_str)
        if isinstance(parent, ast.Module) or len(removed_list) < len(
                body_list):
            continue
        last = body_list[-1]
        line_start_int = index.starts_list[last.lineno - 1]
        line_end_int = index.starts_list[last.end_lineno]
        replacement_str = (
            index.text[line_start_int:index.find_node_start_int(last)] + "pass"
            + index.text[index.find_node_end_int(last):line_end_int])
        for edit in edits_list:
            if edit.start == line_start_int:
                edit.replacement = replacement_str


def build_edits_list(index: SourceIndex, findings_list: list[Finding],
                     changes_dict: dict, notes_list: list[str]
                     ) -> list[Edit]:
    """The edits fixing one pass's findings.

    Args:
        index (SourceIndex): The indexed text.
        findings_list (list[Finding]): Fixable findings.
        changes_dict (dict): id(node) -> change, filled for verification.
        notes_list (list[str]): Receives findings left unfixed.
    Returns:
        list[Edit]: The edits, in text order; overlapping edits make
            the caller discard the pass.
    Warnings:
        None.
    """
    parents_dict = build_parents_dict(index.tree)
    edits_list = edit_imports_list(
        index, [import_finding for import_finding in findings_list
                if import_finding.code == "F401"],
        parents_dict, changes_dict, notes_list)
    for finding in findings_list:
        if finding.code == "E703":
            edits_list += edit_semicolon_list(index, finding)
        elif finding.code == "F541":
            edits_list += edit_fstring_list(index, finding)
        elif finding.code == "F632":
            edits_list += edit_literal_identity_list(index, finding,
                                                     changes_dict)
        elif finding.code in ("E713", "E714"):
            edits_list += edit_negated_test_list(index, finding,
                                                 parents_dict, changes_dict)
    fill_empty_bodies_none(index, edits_list, changes_dict)
    kept_list = drop_overlaps_list(edits_list, changes_dict)
    if len(kept_list) < len(edits_list):
        notes_list.append(DEFERRED_NOTE_STR)
    return kept_list


def drop_overlaps_list(edits_list: list[Edit], changes_dict: dict
                       ) -> list[Edit]:
    """Leave overlapping fixes for the next pass.

    Args:
        edits_list (list[Edit]): All edits of the pass.
        changes_dict (dict): id(node) -> change (changed in place: the
            changes of dropped groups are removed).
    Returns:
        list[Edit]: Non-overlapping edits in text order; when two
            groups overlap, the later one waits for the next pass.
    Warnings:
        None.
    """
    dropped_set: set[int] = set()
    while True:
        kept_list = sorted((edit for edit in edits_list
                            if edit.group_key_int() not in dropped_set),
                           key=lambda candidate_edit: (
                               candidate_edit.start, candidate_edit.end))
        clash_list = [later for earlier, later in zip(kept_list,
                                                      kept_list[1:])
                      if later.start < earlier.end]
        if not clash_list:
            break
        dropped_set.add(clash_list[0].group_key_int())
    for node_id in [node_id for node_id, change in changes_dict.items()
                    if change[2] in dropped_set]:
        del changes_dict[node_id]
    return kept_list


def apply_edits_str(text_str: str, edits_list: list[Edit]) -> str:
    """Apply non-overlapping edits.

    Args:
        text_str (str): The text.
        edits_list (list[Edit]): Edits in text order.
    Returns:
        str: The edited text.
    Warnings:
        None.
    """
    for edit in reversed(edits_list):
        text_str = (text_str[:edit.start] + edit.replacement
                    + text_str[edit.end:])
    return text_str


def check_fixed_text_bool(index: SourceIndex, changes_dict: dict,
                          new_str: str, path_str: str) -> bool:
    """Whether fixed text compiles to exactly the expected tree.

    Args:
        index (SourceIndex): The original text and tree.
        changes_dict (dict): The intended changes.
        new_str (str): The fixed text.
        path_str (str): The file path, for compile().
    Returns:
        bool: True when it compiles and its normalised tree equals the
            expected one.
    Warnings:
        None.
    """
    try:
        new_tree = parse_quietly_node(new_str, path_str)
    except (SyntaxError, ValueError):
        return False
    expected = build_expected_tree(index.tree, changes_dict, index.text)
    unify = UnifyTransformer()
    return (ast.dump(unify.visit(expected))
            == ast.dump(unify.visit(new_tree)))


def keep_verified_edits_list(index: SourceIndex, edits_list: list[Edit],
                             changes_dict: dict, path_str: str
                             ) -> list[Edit]:
    """The edits whose result passes the tree check.

    Args:
        index (SourceIndex): The indexed text.
        edits_list (list[Edit]): The pass's non-overlapping edits.
        changes_dict (dict): Their intended changes.
        path_str (str): The file path, for compile().
    Returns:
        list[Edit]: All edits when together they pass; otherwise the
            groups that pass one at a time, applied together only if
            that passes too, else the first passing group alone.
    Warnings:
        A group that fails on its own is never applied.
    """
    if not edits_list or check_fixed_text_bool(
            index, changes_dict, apply_edits_str(index.text,
                                                      edits_list), path_str):
        return edits_list
    groups_dict: dict[int, list[Edit]] = {}
    for edit in edits_list:
        groups_dict.setdefault(edit.group_key_int(), []).append(edit)
    passing_list = []
    for group_int, group_edits_list in groups_dict.items():
        changes_part_dict = {node_id: change for node_id, change
                             in changes_dict.items() if change[2] == group_int}
        if check_fixed_text_bool(
                index, changes_part_dict,
                apply_edits_str(index.text, group_edits_list), path_str):
            passing_list.append((group_edits_list, changes_part_dict))
    combined_list = sorted((edit for group_edits_list, _ in passing_list
                            for edit in group_edits_list),
                           key=lambda candidate_edit: (
                               candidate_edit.start, candidate_edit.end))
    combined_dict = {node_id: change for _, part_dict in passing_list
                     for node_id, change in part_dict.items()}
    if combined_list and check_fixed_text_bool(
            index, combined_dict,
            apply_edits_str(index.text, combined_list), path_str):
        return combined_list
    return passing_list[0][0] if passing_list else []


def list_fixable_findings_list(text_str: str, path_str: str,
                               select_tuple: tuple[str, ...],
                               ignore_tuple: tuple[str, ...]
                               ) -> list[Finding]:
    """The reported findings this module can fix.

    Args:
        text_str (str): The module text.
        path_str (str): The file path.
        select_tuple (tuple[str, ...]): Selected code prefixes.
        ignore_tuple (tuple[str, ...]): Ignored code prefixes.
    Returns:
        list[Finding]: Fixable findings; F401 is left out in
            __init__.py files and .pyi stubs, where imports may be
            re-exports.
    Warnings:
        None.
    """
    return filter_fixable_list(check_correctness_list(
        path_str, text_str.encode("utf-8"), select_tuple, ignore_tuple),
        path_str)


def filter_fixable_list(findings_list: list[Finding], path_str: str
                        ) -> list[Finding]:
    """The findings this module can fix.

    Args:
        findings_list (list[Finding]): A file's findings.
        path_str (str): The file path.
    Returns:
        list[Finding]: Fixable codes only; F401 is left out in
            __init__.py files and .pyi stubs.
    Warnings:
        None.
    """
    is_init = path_str.replace("\\", "/").endswith(("__init__.py", ".pyi"))
    return [finding for finding in findings_list
            if finding.code in FIXABLE_CODES_TUPLE
            and not (is_init and finding.code == "F401")]


def fix_compat_text(text_str: str, path_str: str,
                    select_tuple: tuple[str, ...],
                    ignore_tuple: tuple[str, ...] = (),
                    known_list: list[Finding] | None = None
                    ) -> CompatFixOutcome:
    """Apply safe fixes for the selected findings of one file.

    Args:
        text_str (str): The module text (LF line endings; must parse).
        path_str (str): The file path.
        select_tuple (tuple[str, ...]): Selected code prefixes.
        ignore_tuple (tuple[str, ...]): Ignored code prefixes.
        known_list (list[Finding] | None): The file's findings when the
            caller already linted this exact text (saves the first lint).
    Returns:
        CompatFixOutcome: The fixed text, applied fixes and notes.
    Warnings:
        Each pass must pass the tree check, or it is discarded.
    """
    outcome = CompatFixOutcome(text_str)
    for pass_int in range(MAX_PASSES_INT):
        findings_list = (
            filter_fixable_list(known_list, path_str)
            if pass_int == 0 and known_list is not None
            else list_fixable_findings_list(outcome.text, path_str,
                                            select_tuple, ignore_tuple))
        if not findings_list or not run_fix_pass_bool(outcome,
                                                      findings_list,
                                                      path_str):
            break
    return outcome


def run_fix_pass_bool(outcome: CompatFixOutcome,
                      findings_list: list[Finding], path_str: str) -> bool:
    """Apply one verified pass of fixes.

    Args:
        outcome (CompatFixOutcome): The state so far (changed in place).
        findings_list (list[Finding]): Fixable findings of outcome.text.
        path_str (str): The file path, for compile().
    Returns:
        bool: True when another pass may fix more: fixes were deferred
            because they overlapped, or imports were removed (which can
            leave another import of the same module unused).
    Warnings:
        None.
    """
    index = SourceIndex(outcome.text)
    changes_dict: dict = {}
    notes_list: list[str] = []
    edits_list = build_edits_list(index, findings_list, changes_dict,
                                  notes_list)
    outcome.notes = notes_list  # findings still open after this pass
    verified_list = keep_verified_edits_list(index, edits_list,
                                             changes_dict, path_str)
    if len(verified_list) < len(edits_list):
        outcome.notes.append("some fixes left out: the fixed code did "
                             "not match the expected syntax tree")
    if not verified_list:
        return False
    outcome.text = apply_edits_str(outcome.text, verified_list)
    fixed_list = sorted(dict.fromkeys(
        fixed_finding for edit in verified_list
        for fixed_finding in [edit.finding, *edit.also]))
    outcome.applied += [f"{fixed_finding.line}:{fixed_finding.column} "
                        f"{fixed_finding.code} {fixed_finding.message}"
                        for fixed_finding in fixed_list]
    return DEFERRED_NOTE_STR in notes_list or any(
        fixed_finding.code == "F401" for fixed_finding in fixed_list)
