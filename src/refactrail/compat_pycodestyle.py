"""pycodestyle-compatible codes (E4, E7) from RefacTrail's own analysis.

The codes and their meaning follow pycodestyle, so existing ``# noqa``
comments and configurations carry over. Everything is computed here from
Python's own tokenizer and syntax tree; no other linter's code is used.
"""

import ast
import io
import tokenize

from refactrail.rules.context import RuleContext
from refactrail.source import (
    locate_definition_name_tuple, locate_node_tuple,
)

COMPOUND_KEYWORDS_FROZENSET = frozenset((
    "if", "elif", "else", "for", "while", "with", "try", "except",
    "finally", "class", "def", "async"))
AMBIGUOUS_NAMES_FROZENSET = frozenset(("l", "O", "I"))
STRING_START_TYPES_TUPLE = tuple(
    getattr(tokenize, name_str) for name_str in ("FSTRING_START",
                                                 "TSTRING_START")
    if hasattr(tokenize, name_str))
STRING_END_TYPES_TUPLE = tuple(
    getattr(tokenize, name_str) for name_str in ("FSTRING_END",
                                                 "TSTRING_END")
    if hasattr(tokenize, name_str))
TRIVIA_TYPES_TUPLE = (tokenize.COMMENT, tokenize.NL)
SETUP_CALLS_FROZENSET = frozenset((
    "sys.path.append", "sys.path.insert", "sys.path.extend",
    "sys.path.remove", "sys.path.pop", "sys.path.clear", "sys.path.reverse",
    "sys.path.sort", "os.putenv", "os.unsetenv", "os.environ.update",
    "os.environ.pop", "os.environ.clear", "os.environ.setdefault",
    "os.environ.popitem", "matplotlib.use", "pytest.importorskip"))
BLOCK_STATEMENTS_TUPLE = (ast.If, ast.Try, ast.TryStar, ast.With,
                          ast.AsyncWith, ast.Match)
BUILTIN_TYPES_FROZENSET = frozenset((
    "bool", "bytearray", "bytes", "classmethod", "complex", "dict",
    "enumerate", "filter", "float", "frozenset", "int", "list", "map",
    "memoryview", "object", "property", "range", "reversed", "set",
    "slice", "staticmethod", "str", "super", "tuple", "type", "zip",
    "ArithmeticError", "AssertionError", "AttributeError", "BaseException",
    "BaseExceptionGroup", "BlockingIOError", "BrokenPipeError",
    "BufferError", "BytesWarning", "ChildProcessError",
    "ConnectionAbortedError", "ConnectionError", "ConnectionRefusedError",
    "ConnectionResetError", "DeprecationWarning", "EOFError",
    "EncodingWarning", "EnvironmentError", "Exception", "ExceptionGroup",
    "FileExistsError", "FileNotFoundError", "FloatingPointError",
    "FutureWarning", "GeneratorExit", "IOError", "ImportError",
    "ImportWarning", "IndentationError", "IndexError", "InterruptedError",
    "IsADirectoryError", "KeyError", "KeyboardInterrupt", "LookupError",
    "MemoryError", "ModuleNotFoundError", "NameError", "NotADirectoryError",
    "NotImplementedError", "OSError", "OverflowError",
    "PendingDeprecationWarning", "PermissionError", "ProcessLookupError",
    "RecursionError", "ReferenceError", "ResourceWarning", "RuntimeError",
    "RuntimeWarning", "StopAsyncIteration", "StopIteration", "SyntaxError",
    "SyntaxWarning", "SystemError", "SystemExit", "TabError",
    "TimeoutError", "TypeError", "UnboundLocalError", "UnicodeDecodeError",
    "UnicodeEncodeError", "UnicodeError", "UnicodeTranslateError",
    "UnicodeWarning", "UserWarning", "ValueError", "Warning",
    "WindowsError", "ZeroDivisionError",
))


def report_at_none(context_info: RuleContext, code_str: str,
                   position_tuple: tuple[int, int], message_str: str) -> None:
    """Report a finding at a 1-based line and character column.

    Args:
        context_info (RuleContext): Findings and suppressions.
        code_str (str): Rule code.
        position_tuple (tuple[int, int]): Line and column.
        message_str (str): Explanation.
    Returns:
        None: Adds the finding when enabled and not suppressed.
    Warnings:
        None.
    """
    context_info.report_none(code_str, position_tuple, message_str)


def report_node_at_none(context_info: RuleContext, node: ast.AST,
                        code_str: str, message_str: str) -> None:
    """Report a finding at a node's start.

    Args:
        context_info (RuleContext): Findings and suppressions.
        node (ast.AST): Located node.
        code_str (str): Rule code.
        message_str (str): Explanation.
    Returns:
        None: Adds the finding when enabled and not suppressed.
    Warnings:
        None.
    """
    report_at_none(context_info, code_str,
                   locate_node_tuple(context_info.source_info, node),
                   message_str)


def list_tokens_list(text_str: str) -> list[tokenize.TokenInfo]:
    """Tokenize source text, or return no tokens when it cannot be.

    Args:
        text_str (str): Decoded source that already parsed.
    Returns:
        list[tokenize.TokenInfo]: Tokens; f-string and t-string parts
        are kept so callers can skip their insides.
    Warnings:
        Tokenizer failures (rare after a successful parse) yield [].
    """
    try:
        return list(tokenize.generate_tokens(io.StringIO(text_str).readline))
    except (tokenize.TokenError, SyntaxError, ValueError, MemoryError):
        return []


def find_next_significant_info(tokens_list: list[tokenize.TokenInfo],
                               index_int: int) -> tokenize.TokenInfo | None:
    """The first token after index_int that is not a comment or NL.

    Args:
        tokens_list (list[tokenize.TokenInfo]): All tokens.
        index_int (int): Position to search after.
    Returns:
        tokenize.TokenInfo | None: The token, or None at the end.
    Warnings:
        None.
    """
    for token_info in tokens_list[index_int + 1:]:
        if token_info.type not in TRIVIA_TYPES_TUPLE:
            return token_info
    return None


def collect_soft_compound_set(tree_node: ast.Module) -> set[tuple[int, int]]:
    """Start positions of match statements and case patterns.

    Args:
        tree_node (ast.Module): Parsed module.
    Returns:
        set[tuple[int, int]]: (line, 0-based column) of each match
        statement and of each case pattern.
    Warnings:
        Positions use the syntax tree's UTF-8 columns, compared only
        with ASCII-prefixed keywords.
    """
    positions_set = set()
    for node in ast.walk(tree_node):
        if isinstance(node, ast.Match):
            positions_set.add((node.lineno, node.col_offset))
            positions_set.update((case.pattern.lineno,
                                  case.pattern.col_offset)
                                 for case in node.cases)
    return positions_set


def is_compound_start_bool(tokens_list: list[tokenize.TokenInfo],
                           index_int: int,
                           soft_set: set[tuple[int, int]]) -> bool:
    """Whether the statement starting at a token is a compound header.

    Args:
        tokens_list (list[tokenize.TokenInfo]): All tokens.
        index_int (int): The statement's first token.
        soft_set (set[tuple[int, int]]): match and case positions.
    Returns:
        bool: True for if/for/while/with/try/def/class/... headers and
        for the soft keywords match and case when used as statements.
    Warnings:
        Decorators are not headers; the def or class line after them is.
    """
    token_info = tokens_list[index_int]
    if token_info.type != tokenize.NAME:
        return False
    if token_info.string in COMPOUND_KEYWORDS_FROZENSET:
        return True
    if token_info.string == "match":
        return token_info.start in soft_set
    if token_info.string == "case":
        next_info = find_next_significant_info(tokens_list, index_int)
        return next_info is not None and next_info.start in soft_set
    return False


def check_compound_colon_none(context_info: RuleContext,
                              tokens_list: list[tokenize.TokenInfo],
                              index_int: int, keyword_str: str) -> None:
    """E701 when a header colon is followed by a statement.

    Args:
        context_info (RuleContext): Findings.
        tokens_list (list[tokenize.TokenInfo]): All tokens.
        index_int (int): The header colon.
        keyword_str (str): The header's first keyword.
    Returns:
        None: Adds E701 at the colon.
    Warnings:
        def headers are pycodestyle's E704 instead, which is not part
        of this set.
    """
    next_info = find_next_significant_info(tokens_list, index_int)
    if next_info is None or next_info.type in (tokenize.NEWLINE,
                                               tokenize.ENDMARKER):
        return
    if keyword_str == "def":
        return
    if keyword_str == "class" and next_info.string == "...":
        after_info = find_next_significant_info(
            tokens_list, tokens_list.index(next_info))
        if after_info is None or after_info.type in (tokenize.NEWLINE,
                                                     tokenize.ENDMARKER):
            return
    line_int, column_int = tokens_list[index_int].start
    report_at_none(context_info, "E701", (line_int, column_int + 1),
                   "Multiple statements on one line (colon).")


def check_semicolon_none(context_info: RuleContext,
                         tokens_list: list[tokenize.TokenInfo],
                         index_int: int) -> None:
    """E702 or E703 for a statement-separating semicolon.

    Args:
        context_info (RuleContext): Findings.
        tokens_list (list[tokenize.TokenInfo]): All tokens.
        index_int (int): The semicolon.
    Returns:
        None: E703 when nothing follows on the line, else E702.
    Warnings:
        None.
    """
    next_info = find_next_significant_info(tokens_list, index_int)
    line_int, column_int = tokens_list[index_int].start
    if next_info is None or next_info.type in (tokenize.NEWLINE,
                                               tokenize.ENDMARKER):
        report_at_none(context_info, "E703", (line_int, column_int + 1),
                       "Statement ends with an unnecessary semicolon.")
    else:
        report_at_none(context_info, "E702", (line_int, column_int + 1),
                       "Multiple statements on one line (semicolon).")


def list_code_tokens_list(tokens_list: list[tokenize.TokenInfo]
                          ) -> list[tuple[int, tokenize.TokenInfo]]:
    """The tokens outside f-/t-string literals, without comments or NLs.

    Args:
        tokens_list (list[tokenize.TokenInfo]): All tokens.
    Returns:
        list[tuple[int, tokenize.TokenInfo]]: (index, token) pairs.
    Warnings:
        The string start and end tokens themselves are skipped too.
    """
    pairs_list = []
    string_depth_int = 0
    for index_int, token_info in enumerate(tokens_list):
        if token_info.type in STRING_START_TYPES_TUPLE:
            string_depth_int += 1
        elif token_info.type in STRING_END_TYPES_TUPLE:
            string_depth_int -= 1
            continue
        if string_depth_int or token_info.type in TRIVIA_TYPES_TUPLE:
            continue
        pairs_list.append((index_int, token_info))
    return pairs_list


def find_header_keyword_str(tokens_list: list[tokenize.TokenInfo],
                            index_int: int,
                            soft_set: set[tuple[int, int]]) -> str:
    """The keyword of a compound header starting at a token, if any.

    Args:
        tokens_list (list[tokenize.TokenInfo]): All tokens.
        index_int (int): The statement's first token.
        soft_set (set[tuple[int, int]]): match and case positions.
    Returns:
        str: "if", "class", "def", "for" (also for "async for") and so
        on; "" when the statement is not a compound header.
    Warnings:
        None.
    """
    if not is_compound_start_bool(tokens_list, index_int, soft_set):
        return ""
    keyword_str = tokens_list[index_int].string
    if keyword_str == "async":
        next_info = find_next_significant_info(tokens_list, index_int)
        keyword_str = next_info.string if next_info else ""
    return keyword_str


def check_statement_tokens_none(context_info: RuleContext) -> None:
    """E701, E702 and E703 from the token stream.

    Args:
        context_info (RuleContext): Findings, source and tree.
    Returns:
        None: Adds findings at the offending colon or semicolon.
    Warnings:
        f-string and t-string insides are skipped.
    """
    tokens_list = context_info.tokens_list
    soft_set = collect_soft_compound_set(context_info.tree)
    depth_int = lambda_int = 0
    keyword_str, is_start_bool = "", True
    for index_int, token_info in list_code_tokens_list(tokens_list):
        if token_info.type in (tokenize.NEWLINE, tokenize.INDENT,
                               tokenize.DEDENT):
            keyword_str, is_start_bool = "", True
            continue
        if is_start_bool:
            keyword_str = find_header_keyword_str(tokens_list, index_int,
                                                  soft_set)
            is_start_bool, lambda_int = False, 0
        if token_info.type != tokenize.OP:
            lambda_int += token_info.string == "lambda" and depth_int == 0
            continue
        if token_info.string in "([{":
            depth_int += 1
        elif token_info.string in ")]}":
            depth_int = max(0, depth_int - 1)
        elif depth_int == 0 and token_info.string == ";":
            check_semicolon_none(context_info, tokens_list, index_int)
            is_start_bool = True
        elif depth_int == 0 and token_info.string == ":" and lambda_int:
            lambda_int -= 1
        elif depth_int == 0 and token_info.string == ":" and keyword_str:
            check_compound_colon_none(context_info, tokens_list, index_int,
                                      keyword_str)
            keyword_str, is_start_bool = "", True


def is_singleton_bool(node: ast.AST, values_tuple: tuple) -> bool:
    """Whether a node is one of the given constant singletons.

    Args:
        node (ast.AST): Expression.
        values_tuple (tuple): Allowed values, compared by identity.
    Returns:
        bool: True for None, True or False constants as requested.
    Warnings:
        None.
    """
    return isinstance(node, ast.Constant) and any(
        node.value is singleton_value for singleton_value in values_tuple)


def is_plain_constant_bool(node: ast.AST) -> bool:
    """Whether a node is a constant other than None, True and False.

    Args:
        node (ast.AST): Expression.
    Returns:
        bool: True for numbers, strings, bytes and Ellipsis.
    Warnings:
        Comparing two constants (0 == False) is left alone.
    """
    return isinstance(node, ast.Constant) and not is_singleton_bool(
        node, (None, True, False))


def check_literal_comparison_none(context_info: RuleContext,
                                  node: ast.Compare) -> None:
    """E711 and E712: == or != against None, True or False.

    Args:
        context_info (RuleContext): Findings.
        node (ast.Compare): Comparison.
    Returns:
        None: Adds findings at the singleton.
    Warnings:
        None.
    """
    operands_list = [node.left, *node.comparators]
    for index_int, operator in enumerate(node.ops):
        left, right = operands_list[index_int:index_int + 2]
        if not isinstance(operator, (ast.Eq, ast.NotEq)):
            continue
        checked_list = [right] if not is_plain_constant_bool(left) else []
        if index_int == 0 and not is_plain_constant_bool(right):
            checked_list.insert(0, left)
        for operand in checked_list:
            if is_singleton_bool(operand, (None,)):
                report_node_at_none(context_info, operand, "E711",
                                    "Comparison to None should use "
                                    "'is' or 'is not'.")
            elif is_singleton_bool(operand, (True, False)):
                report_node_at_none(context_info, node, "E712",
                                    "Avoid equality comparisons to True "
                                    "or False; test the value itself.")


def is_builtin_read_bool(context_info: RuleContext,
                         node: ast.Name) -> bool:
    """Whether a name read resolves to the builtin of that name.

    Args:
        context_info (RuleContext): Lexical facts.
        node (ast.Name): The read.
    Returns:
        bool: True when lexical scopes resolve it to a builtin; without
        a lexical report, when the file never binds the name.
    Warnings:
        None.
    """
    resolutions_dict = context_info.read_resolutions_dict
    if not resolutions_dict:
        return node.id not in context_info.bound_names_set
    return resolutions_dict.get(locate_node_tuple(
        context_info.source_info, node)) == "builtin_or_implicit"


def is_type_expression_bool(context_info: RuleContext,
                            node: ast.AST) -> bool:
    """Whether an expression is a type() call or a builtin type name.

    Args:
        context_info (RuleContext): Lexical facts.
        node (ast.AST): Expression.
    Returns:
        bool: True for type(...) and builtin classes such as int, str or
        ValueError, when the name is the builtin.
    Warnings:
        None.
    """
    if isinstance(node, ast.Call):
        node = node.func
        return (isinstance(node, ast.Name) and node.id == "type"
                and is_builtin_read_bool(context_info, node))
    return (isinstance(node, ast.Name) and node.id in BUILTIN_TYPES_FROZENSET
            and is_builtin_read_bool(context_info, node))


def is_dtype_expression_bool(node: ast.AST) -> bool:
    """Whether an expression is a NumPy dtype (x.dtype or np.dtype(...)).

    Args:
        node (ast.AST): Expression.
    Returns:
        bool: True for attribute access named dtype and dtype() calls.
    Warnings:
        None.
    """
    if isinstance(node, ast.Call):
        node = node.func
        return isinstance(node, ast.Attribute) and node.attr == "dtype"
    return isinstance(node, ast.Attribute) and node.attr == "dtype"


def check_type_comparison_none(context_info: RuleContext,
                               node: ast.Compare) -> None:
    """E721: comparing types with == or != instead of is or isinstance.

    Args:
        context_info (RuleContext): Findings.
        node (ast.Compare): Comparison.
    Returns:
        None: Adds one finding at the comparison.
    Warnings:
        None.
    """
    operands_list = [node.left, *node.comparators]
    for index_int, operator in enumerate(node.ops):
        left, right = operands_list[index_int:index_int + 2]
        if isinstance(operator, (ast.Eq, ast.NotEq)) and (
                is_type_expression_bool(context_info, left)
                or is_type_expression_bool(context_info, right)) and not (
                is_dtype_expression_bool(left)
                or is_dtype_expression_bool(right)):
            report_node_at_none(context_info, node, "E721",
                                "Use 'is' or isinstance() to compare "
                                "types.")
            return


def check_negated_test_none(context_info: RuleContext,
                            node: ast.UnaryOp) -> None:
    """E713 and E714: 'not x in y' and 'not x is y'.

    Args:
        context_info (RuleContext): Findings.
        node (ast.UnaryOp): Unary operation.
    Returns:
        None: Adds findings at the comparison.
    Warnings:
        None.
    """
    operand = node.operand
    if not (isinstance(node.op, ast.Not) and isinstance(operand, ast.Compare)
            and len(operand.ops) == 1):
        return
    if isinstance(operand.ops[0], ast.In):
        report_node_at_none(context_info, operand, "E713",
                            "Membership tests should use 'not in'.")
    elif isinstance(operand.ops[0], ast.Is):
        report_node_at_none(context_info, operand, "E714",
                            "Identity tests should use 'is not'.")


def check_lambda_assignment_none(context_info: RuleContext,
                                 node: ast.Assign | ast.AnnAssign) -> None:
    """E731: a lambda assigned to a name instead of a def.

    Args:
        context_info (RuleContext): Findings.
        node (ast.Assign | ast.AnnAssign): Assignment.
    Returns:
        None: Adds a finding at the statement.
    Warnings:
        None.
    """
    targets_list = (node.targets if isinstance(node, ast.Assign)
                    else [node.target])
    if (isinstance(node.value, ast.Lambda) and len(targets_list) == 1
            and isinstance(targets_list[0], ast.Name)):
        report_node_at_none(context_info, node, "E731",
                            "Do not assign a lambda expression; use a def.")


def find_identifier_tuple(context_info: RuleContext, node: ast.AST,
                          name_str: str) -> tuple[int, int]:
    """The position of an identifier that has no node of its own.

    Args:
        context_info (RuleContext): Source access.
        node (ast.AST): The statement or handler holding the name.
        name_str (str): The identifier.
    Returns:
        tuple[int, int]: Line and column of the first whole-word
        occurrence after the node's start, else the node's start.
    Warnings:
        Used for global, nonlocal and except ... as names.
    """
    line_int, column_int = locate_node_tuple(context_info.source_info, node)
    lines_list = context_info.source_info.lines
    for offset_int in range(getattr(node, "end_lineno", line_int)
                            - line_int + 1):
        line_str = lines_list[line_int - 1 + offset_int]
        start_int = column_int - 1 if offset_int == 0 else 0
        found_int = line_str.find(name_str, start_int)
        while found_int >= 0:
            before_str = line_str[found_int - 1:found_int]
            after_str = line_str[found_int + len(name_str):
                                 found_int + len(name_str) + 1]
            if not (before_str.isidentifier() or before_str.isdigit()
                    or after_str.isidentifier() or after_str.isdigit()):
                return line_int + offset_int, found_int + 1
            found_int = line_str.find(name_str, found_int + 1)
    return line_int, column_int


def check_ambiguous_name_none(context_info: RuleContext,
                              node: ast.AST) -> None:
    """E741, E742 and E743: the names l, O and I.

    Args:
        context_info (RuleContext): Findings.
        node (ast.AST): Any node.
    Returns:
        None: Adds findings at the binding names.
    Warnings:
        None.
    """
    message_str = "Ambiguous variable name; l, O and I look like 1 and 0."
    if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Store):
        if node.id in AMBIGUOUS_NAMES_FROZENSET:
            report_node_at_none(context_info, node, "E741", message_str)
    elif isinstance(node, ast.arg) and node.arg in AMBIGUOUS_NAMES_FROZENSET:
        report_node_at_none(context_info, node, "E741", message_str)
    elif isinstance(node, (ast.Global, ast.Nonlocal)):
        for name_str in node.names:
            if name_str in AMBIGUOUS_NAMES_FROZENSET:
                report_at_none(context_info, "E741", find_identifier_tuple(
                    context_info, node, name_str), message_str)
    elif isinstance(node, ast.ExceptHandler) and (
            node.name in AMBIGUOUS_NAMES_FROZENSET):
        report_at_none(context_info, "E741", find_identifier_tuple(
            context_info, node, node.name), message_str)
    elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef,
                           ast.ClassDef)) and (
            node.name in AMBIGUOUS_NAMES_FROZENSET):
        code_str = "E742" if isinstance(node, ast.ClassDef) else "E743"
        report_at_none(context_info, code_str, locate_definition_name_tuple(
            context_info.source_info, node), message_str)


def check_pycodestyle_node_none(context_info: RuleContext,
                                node: ast.AST) -> None:
    """Dispatch the node-level pycodestyle-compatible checks.

    Args:
        context_info (RuleContext): Findings.
        node (ast.AST): Current node of the shared traversal.
    Returns:
        None: Adds E401, E711-E714, E721, E722, E731 and E741-E743.
    Warnings:
        None.
    """
    if isinstance(node, ast.Import) and len(node.names) > 1:
        report_node_at_none(context_info, node, "E401",
                            "Multiple imports on one line.")
    elif isinstance(node, ast.Compare):
        check_literal_comparison_none(context_info, node)
        check_type_comparison_none(context_info, node)
    elif isinstance(node, ast.UnaryOp):
        check_negated_test_none(context_info, node)
    elif isinstance(node, (ast.Assign, ast.AnnAssign)):
        check_lambda_assignment_none(context_info, node)
    elif isinstance(node, ast.ExceptHandler) and node.type is None and (
            not any(isinstance(statement, ast.Raise) and statement.exc is None
                    for statement in node.body)):
        report_node_at_none(context_info, node, "E722",
                            "Do not use a bare except.")
    check_ambiguous_name_none(context_info, node)


def resolve_dotted_str(node: ast.AST, aliases_dict: dict[str, str]) -> str:
    """The imported dotted name an expression refers to, if any.

    Args:
        node (ast.AST): A name or attribute chain.
        aliases_dict (dict[str, str]): Local name -> imported dotted name.
    Returns:
        str: For example "sys.path.append", or "" when the chain does
        not start with an imported name.
    Warnings:
        Only imports seen before the statement count.
    """
    parts_list = []
    while isinstance(node, ast.Attribute):
        parts_list.append(node.attr)
        node = node.value
    if not isinstance(node, ast.Name) or node.id not in aliases_dict:
        return ""
    return ".".join([aliases_dict[node.id], *reversed(parts_list)])


def record_aliases_none(node: ast.Import | ast.ImportFrom,
                        aliases_dict: dict[str, str]) -> None:
    """Record the dotted names an import statement binds.

    Args:
        node (ast.Import | ast.ImportFrom): Import statement.
        aliases_dict (dict[str, str]): Updated in place.
    Returns:
        None: Adds local name -> dotted name entries.
    Warnings:
        Relative and star imports bind nothing resolvable here.
    """
    for alias in node.names:
        if isinstance(node, ast.Import):
            local_str = alias.asname or alias.name.split(".")[0]
            aliases_dict[local_str] = (alias.name if alias.asname
                                       else local_str)
        elif node.level == 0 and node.module and alias.name != "*":
            aliases_dict[alias.asname or alias.name] = (
                f"{node.module}.{alias.name}")


def is_environ_target_bool(node: ast.AST, aliases_dict: dict) -> bool:
    """Whether a target is os.environ[...].

    Args:
        node (ast.AST): Assignment or deletion target.
        aliases_dict (dict): Imported names.
    Returns:
        bool: True for subscripts of os.environ.
    Warnings:
        None.
    """
    return isinstance(node, ast.Subscript) and resolve_dotted_str(
        node.value, aliases_dict) == "os.environ"


def is_import_preamble_bool(node: ast.stmt, aliases_dict: dict) -> bool:
    """Whether a module statement may come before imports.

    Args:
        node (ast.stmt): Module-level statement (not an import).
        aliases_dict (dict): Imported names so far.
    Returns:
        bool: True for dunder assignments, if/try/with/match blocks and
        path, environment, backend and importorskip setup.
    Warnings:
        None.
    """
    if isinstance(node, BLOCK_STATEMENTS_TUPLE):
        return True
    value_node = getattr(node, "value", None)
    if isinstance(value_node, ast.Call) and isinstance(
            node, (ast.Expr, ast.Assign, ast.AnnAssign)) and (
            resolve_dotted_str(value_node.func, aliases_dict)
            in SETUP_CALLS_FROZENSET):
        return True
    targets_list = (node.targets if isinstance(node, (ast.Assign,
                                                      ast.Delete))
                    else [node.target] if isinstance(
                        node, (ast.AugAssign, ast.AnnAssign)) else [])
    if any(is_environ_target_bool(target, aliases_dict)
           for target in targets_list):
        return True
    return isinstance(node, (ast.Assign, ast.AnnAssign)) and all(
        isinstance(target, ast.Name) and target.id.startswith("__")
        and target.id.endswith("__") for target in targets_list)


def check_import_position_none(context_info: RuleContext) -> None:
    """E402: module-level imports after other code.

    Args:
        context_info (RuleContext): Findings and tree.
    Returns:
        None: Adds a finding at each late top-level import.
    Warnings:
        Imports nested in if, try or with blocks are not checked.
    """
    aliases_dict: dict[str, str] = {}
    has_boundary_bool = False
    for index_int, node in enumerate(context_info.tree.body):
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            if has_boundary_bool:
                report_node_at_none(context_info, node, "E402",
                                    "Module-level import not at the top "
                                    "of the file.")
            record_aliases_none(node, aliases_dict)
            continue
        is_docstring = index_int == 0 and isinstance(node, ast.Expr) and (
            isinstance(node.value, ast.Constant)
            and isinstance(node.value.value, str))
        if not (is_docstring or is_import_preamble_bool(node,
                                                        aliases_dict)):
            has_boundary_bool = True
