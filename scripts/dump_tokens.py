"""Dump CPython's token stream in the format the Rust lexer must match.

Usage: python scripts/dump_tokens.py FILE

One line per token: KIND START_ROW,START_COL-END_ROW,END_COL "TEXT".
Operators print as OP; text is JSON-escaped (non-ASCII kept). A
tokenizer error prints a single line: ERROR <row> (row may be "?").
Columns are code-point offsets, rows are 1-based, as tokenize reports.
"""

import io
import json
import sys
import tokenize
import warnings

# CPython 3.14's tokenize module raises MemoryError on some malformed
# t-strings that compile() reports correctly; such inputs have no defined
# token oracle and are skipped.
ORACLE_CRASH_STR = "ORACLE-CRASH"


def dump_lines_list(text_str: str) -> list[str]:
    """Return the token dump of one decoded source text.

    Args:
        text_str (str): Source text with its original line endings.
    Returns:
        list[str]: Dump lines, or a single ERROR line.
    Warnings:
        Tokens before an error are discarded; only the outcome compares.
    """
    lines_list = []
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            tokens_list = list(tokenize.generate_tokens(io.StringIO(
                text_str, newline="").readline))
    except MemoryError:
        return [ORACLE_CRASH_STR]
    except (tokenize.TokenError, SyntaxError) as error:
        row_obj = getattr(error, "lineno", None)
        if row_obj is None and len(getattr(error, "args", ())) > 1:
            row_obj = error.args[1][0]
        return [f"ERROR {row_obj if row_obj is not None else '?'}"]
    try:
        for token_info in tokens_list:
            name_str = tokenize.tok_name[token_info.type]
            (start_row, start_col), (end_row, end_col) = (token_info.start,
                                                          token_info.end)
            lines_list.append(
                f"{name_str} {start_row},{start_col}-{end_row},{end_col} "
                f"{json.dumps(token_info.string, ensure_ascii=False)}")
    except (tokenize.TokenError, SyntaxError) as error:
        row_obj = getattr(error, "lineno", None)
        if row_obj is None and len(getattr(error, "args", ())) > 1:
            row_obj = error.args[1][0]
        return [f"ERROR {row_obj if row_obj is not None else '?'}"]
    return lines_list


def main() -> int:
    """Print the dump for the file named on the command line.

    Args:
        None: Reads sys.argv.
    Returns:
        int: Zero.
    Warnings:
        The file must be UTF-8.
    """
    with open(sys.argv[1], encoding="utf-8-sig", newline="") as handle:
        text_str = handle.read()
    sys.stdout.write("\n".join(dump_lines_list(text_str)) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
