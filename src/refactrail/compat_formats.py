"""Pyflakes-compatible checks of %-format and str.format() calls (F5xx).

Format strings are parsed here into a summary of their placeholders
(positional count, named keys, starred widths); each check then compares
that summary with the literal operands independently. Nothing is
formatted or executed.
"""

import ast
import string

from refactrail.rules.context import RuleContext
from refactrail.source import locate_node_tuple

PERCENT_FLAGS_STR = "#0- +"
PERCENT_CONVERSIONS_STR = "diouxXeEfFgGcrsab%"
SEQUENCE_NODES_TUPLE = (ast.List, ast.Tuple, ast.Set, ast.ListComp,
                        ast.SetComp, ast.GeneratorExp)
MAPPING_NODES_TUPLE = (ast.Dict, ast.DictComp)
# Right operands that are certainly one value, never a tuple of them.
SINGLE_VALUE_NODES_TUPLE = (ast.List, ast.Set, ast.Dict, ast.ListComp,
                            ast.SetComp, ast.DictComp, ast.GeneratorExp,
                            ast.Constant, ast.JoinedStr)
FORMATTER_INFO = string.Formatter()


class PercentSummary:
    """Placeholders of a %-format string."""

    def __init__(self) -> None:
        """Start with no placeholders.

        Args:
            None.
        Returns:
            None: Initialises the summary.
        Warnings:
            None.
        """
        self.positional_int = 0
        self.keys_set: set[str] = set()
        self.is_starred = False


def report_format_none(context_info: RuleContext, node: ast.AST,
                       code_str: str, message_str: str) -> None:
    """Report a format finding at the expression's start.

    Args:
        context_info (RuleContext): Findings.
        node (ast.AST): The % operation or format() call.
        code_str (str): Rule code.
        message_str (str): Explanation.
    Returns:
        None: Adds the finding.
    Warnings:
        None.
    """
    context_info.report_none(
        code_str, locate_node_tuple(context_info.source_info, node),
        message_str)


def skip_mapping_key_int(format_str: str, index_int: int) -> int:
    """Skip a parenthesised mapping key, allowing nested parentheses.

    Args:
        format_str (str): The format string.
        index_int (int): Position of the opening parenthesis.
    Returns:
        int: Position after the matching closing parenthesis.
    Warnings:
        Raises ValueError when the key is not closed.
    """
    depth_int = 0
    for position_int in range(index_int, len(format_str)):
        if format_str[position_int] == "(":
            depth_int += 1
        elif format_str[position_int] == ")":
            depth_int -= 1
            if depth_int == 0:
                return position_int + 1
    raise ValueError("incomplete mapping key")


def skip_quantity_tuple(format_str: str, index_int: int) -> tuple[int, bool]:
    """Skip a width or precision quantity: * or digits.

    Args:
        format_str (str): The format string.
        index_int (int): Current position.
    Returns:
        tuple[int, bool]: New position and whether it was *.
    Warnings:
        None.
    """
    if format_str.startswith("*", index_int):
        return index_int + 1, True
    while index_int < len(format_str) and format_str[index_int].isdigit():
        index_int += 1
    return index_int, False


def parse_spec_int(format_str: str, index_int: int,
                   summary_info: PercentSummary) -> int:
    """Parse one placeholder after its % sign into the summary.

    Args:
        format_str (str): The format string.
        index_int (int): Position after the % sign.
        summary_info (PercentSummary): Updated in place.
    Returns:
        int: Position after the conversion character.
    Warnings:
        Raises ValueError (incomplete) or LookupError (unsupported
        conversion character).
    """
    key_str = None
    if format_str.startswith("(", index_int):
        end_int = skip_mapping_key_int(format_str, index_int)
        key_str, index_int = format_str[index_int + 1:end_int - 1], end_int
    while index_int < len(format_str) and (
            format_str[index_int] in PERCENT_FLAGS_STR):
        index_int += 1
    index_int, is_star_width_bool = skip_quantity_tuple(format_str, index_int)
    is_star_precision_bool = False
    if format_str.startswith(".", index_int):
        index_int, is_star_precision_bool = skip_quantity_tuple(format_str,
                                                           index_int + 1)
    while index_int < len(format_str) and format_str[index_int] in "hlL":
        index_int += 1
    if index_int >= len(format_str):
        raise ValueError("incomplete format")
    if format_str[index_int] not in PERCENT_CONVERSIONS_STR:
        raise LookupError(format_str[index_int])
    if key_str is None:
        summary_info.positional_int += 1
    else:
        summary_info.keys_set.add(key_str)
    for is_star_bool in (is_star_width_bool, is_star_precision_bool):
        if is_star_bool:
            summary_info.positional_int += 1
            summary_info.is_starred = True
    return index_int + 1


def summarise_percent_info(format_str: str) -> PercentSummary:
    """Summarise a %-format string's placeholders.

    Args:
        format_str (str): The literal format string.
    Returns:
        PercentSummary: Positional count, named keys and * use.
    Warnings:
        Raises ValueError or LookupError like parse_spec_int.
    """
    summary_info = PercentSummary()
    index_int = format_str.find("%")
    while index_int >= 0:
        if format_str.startswith("%", index_int + 1):
            index_int = format_str.find("%", index_int + 2)
            continue
        index_int = parse_spec_int(format_str, index_int + 1, summary_info)
        index_int = format_str.find("%", index_int)
    return summary_info


def count_substitutions_int(right: ast.AST,
                            summary_info: PercentSummary) -> int:
    """How many values a % operation's right operand supplies.

    Args:
        right (ast.AST): The right operand.
        summary_info (PercentSummary): Placeholders.
    Returns:
        int: The tuple length, 1 for a value that is never a tuple, and
        1 for any other expression when the string has no positional
        placeholders (anything but a mapping or () is then too many);
        -1 when unknown.
    Warnings:
        None.
    """
    if isinstance(right, ast.Tuple):
        if any(isinstance(element, ast.Starred) for element in right.elts):
            return -1
        return len(right.elts)
    if isinstance(right, SINGLE_VALUE_NODES_TUPLE):
        return 1
    return 1 if summary_info.positional_int == 0 else -1


def check_percent_sequence_none(context_info: RuleContext, node: ast.BinOp,
                                summary_info: PercentSummary) -> None:
    """F502, F506 and F507: placeholders against a sequence operand.

    Args:
        context_info (RuleContext): Findings.
        node (ast.BinOp): The % operation.
        summary_info (PercentSummary): Placeholders.
    Returns:
        None: Adds findings.
    Warnings:
        None.
    """
    right = node.right
    if summary_info.positional_int and summary_info.keys_set:
        report_format_none(context_info, node, "F506",
                           "'...' % ... mixes positional and named "
                           "placeholders.")
    if summary_info.keys_set and isinstance(right, SEQUENCE_NODES_TUPLE):
        report_format_none(context_info, node, "F502",
                           "'...' % ... expected a mapping but got a "
                           "sequence.")
    count_int = count_substitutions_int(right, summary_info)
    if not summary_info.keys_set and count_int >= 0 and (
            summary_info.positional_int != count_int):
        report_format_none(context_info, node, "F507",
                           f"'...' % ... has {summary_info.positional_int} "
                           f"placeholder(s) but {count_int} "
                           "substitution(s).")


def check_percent_mapping_none(context_info: RuleContext, node: ast.BinOp,
                               summary_info: PercentSummary) -> None:
    """F503, F504, F505 and F508: placeholders against a mapping operand.

    Args:
        context_info (RuleContext): Findings.
        node (ast.BinOp): The % operation.
        summary_info (PercentSummary): Placeholders.
    Returns:
        None: Adds findings.
    Warnings:
        Dict displays with ** entries are not compared key by key.
    """
    right = node.right
    if not isinstance(right, MAPPING_NODES_TUPLE):
        return
    if summary_info.positional_int > 1:
        report_format_none(context_info, node, "F503",
                           "'...' % ... expected a sequence but got a "
                           "mapping.")
    if summary_info.is_starred:
        report_format_none(context_info, node, "F508",
                           "'...' % ... with * specifier requires a "
                           "sequence.")
    if summary_info.positional_int or not isinstance(right, ast.Dict):
        return
    keys_list = [key.value for key in right.keys
                 if isinstance(key, ast.Constant)
                 and isinstance(key.value, str)]
    extra_list = sorted(set(keys_list) - summary_info.keys_set)
    if extra_list:
        report_format_none(context_info, node, "F504",
                           "'...' % ... has unused named argument(s): "
                           f"{', '.join(extra_list)}.")
    if len(keys_list) == len(right.keys):
        missing_list = sorted(summary_info.keys_set - set(keys_list))
        if missing_list:
            report_format_none(context_info, node, "F505",
                               "'...' % ... is missing argument(s) for "
                               f"placeholder(s): {', '.join(missing_list)}.")


def check_percent_format_none(context_info: RuleContext,
                              node: ast.BinOp) -> None:
    """F501-F509 for a literal string % operation.

    Args:
        context_info (RuleContext): Findings.
        node (ast.BinOp): Binary operation.
    Returns:
        None: Adds findings.
    Warnings:
        None.
    """
    if not (isinstance(node.op, ast.Mod) and isinstance(node.left,
                                                        ast.Constant)
            and isinstance(node.left.value, (str, bytes))):
        return
    format_str = node.left.value
    if isinstance(format_str, bytes):
        format_str = format_str.decode("latin-1")
    try:
        summary_info = summarise_percent_info(format_str)
    except LookupError as error:
        report_format_none(context_info, node, "F509",
                           "'...' % ... has an unsupported format character "
                           f"{str(error.args[0])!r}.")
        return
    except ValueError as error:
        report_format_none(context_info, node, "F501",
                           f"'...' % ... has an invalid format string: "
                           f"{error}.")
        return
    check_percent_sequence_none(context_info, node, summary_info)
    check_percent_mapping_none(context_info, node, summary_info)


class FormatSummary:
    """Placeholders of a str.format() string."""

    def __init__(self) -> None:
        """Start with no placeholders.

        Args:
            None.
        Returns:
            None: Initialises the summary.
        Warnings:
            None.
        """
        self.automatic_int = 0
        self.indices_set: set[int] = set()
        self.keys_set: set[str] = set()

    def add_field_none(self, field_str: str | None) -> None:
        """Record one replacement field.

        Args:
            field_str (str | None): The field name, None for plain text.
        Returns:
            None: Counts automatic, numbered or named fields.
        Warnings:
            Attribute and index parts (a.b, a[0]) are ignored.
        """
        if field_str is None:
            return
        key_str = field_str.partition(".")[0].partition("[")[0]
        if key_str == "":
            self.automatic_int += 1
        elif key_str.isascii() and key_str.isdigit():
            self.indices_set.add(int(key_str))
        else:
            self.keys_set.add(key_str)


def summarise_format_info(format_str: str) -> FormatSummary:
    """Summarise a str.format() string, nested specs included.

    Args:
        format_str (str): The literal format string.
    Returns:
        FormatSummary: Automatic, numbered and named fields.
    Warnings:
        Raises ValueError for an invalid format string.
    """
    summary_info = FormatSummary()
    for _, field_str, spec_str, _ in FORMATTER_INFO.parse(format_str):
        summary_info.add_field_none(field_str)
        if not spec_str:
            continue
        for _, inner_str, inner_spec_str, _ in FORMATTER_INFO.parse(spec_str):
            if inner_spec_str and "{" in inner_spec_str:
                raise ValueError("Max string recursion exceeded")
            summary_info.add_field_none(inner_str)
    return summary_info


def check_format_call_none(context_info: RuleContext, node: ast.Call) -> None:
    """F521-F525 for a literal string's .format() call.

    Args:
        context_info (RuleContext): Findings.
        node (ast.Call): Call.
    Returns:
        None: Adds findings.
    Warnings:
        None.
    """
    func = node.func
    if not (isinstance(func, ast.Attribute) and func.attr == "format"
            and isinstance(func.value, ast.Constant)
            and isinstance(func.value.value, str)):
        return
    try:
        summary_info = summarise_format_info(func.value.value)
    except ValueError as error:
        report_format_none(context_info, node, "F521",
                           f"'...'.format(...) has an invalid format "
                           f"string: {error}.")
        return
    if summary_info.automatic_int and summary_info.indices_set:
        report_format_none(context_info, node, "F525",
                           "'...'.format(...) mixes automatic and manual "
                           "numbering.")
    check_format_arguments_none(context_info, node, summary_info)


def check_format_arguments_none(context_info: RuleContext, node: ast.Call,
                                summary_info: FormatSummary) -> None:
    """F522-F524: unused or missing format() arguments.

    Args:
        context_info (RuleContext): Findings.
        node (ast.Call): The format() call.
        summary_info (FormatSummary): The string's placeholders.
    Returns:
        None: Adds findings.
    Warnings:
        Missing arguments are not judged when *args or **kwargs are
        passed.
    """
    used_set = set(range(summary_info.automatic_int)) | (
        summary_info.indices_set)
    extra_named_list = sorted(
        keyword.arg for keyword in node.keywords
        if keyword.arg is not None and keyword.arg not in
        summary_info.keys_set)
    extra_positional_list = [
        index_int for index_int, argument in enumerate(node.args)
        if not isinstance(argument, ast.Starred) and index_int not in used_set]
    if extra_named_list:
        report_format_none(context_info, node, "F522",
                           "'...'.format(...) has unused named argument(s): "
                           f"{', '.join(extra_named_list)}.")
    if extra_positional_list:
        report_format_none(context_info, node, "F523",
                           "'...'.format(...) has unused positional "
                           "argument(s): "
                           f"{', '.join(map(str, extra_positional_list))}.")
    if any(isinstance(argument, ast.Starred) for argument in node.args) or (
            any(keyword.arg is None for keyword in node.keywords)):
        return
    given_set = {keyword.arg for keyword in node.keywords}
    missing_list = sorted(str(index_int) for index_int in used_set
                          if index_int >= len(node.args)) + sorted(
        summary_info.keys_set - given_set)
    if missing_list:
        report_format_none(context_info, node, "F524",
                           "'...'.format(...) is missing argument(s) for "
                           f"placeholder(s): {', '.join(missing_list)}.")
