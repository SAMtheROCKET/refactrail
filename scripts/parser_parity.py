"""Compare the Rust parser's ast.dump output with CPython's ast.parse.

Usage: python scripts/parser_parity.py RT_DUMP_AST_BINARY FOLDER...
       [--examples=N] [--show=N] [--no-attributes]

Every *.py file under the folders is parsed by both; outputs must be
identical (a syntax error must be reported by both, on the same line).
Files are read, never imported or executed.
"""

from collections import Counter
import ast
import os
from pathlib import Path
import subprocess
import sys
import warnings

BATCH_INT = 200


def python_dump_str(text_str: str, attributes_bool: bool) -> str:
    """Return CPython's dump of a source, or "ERROR line".

    Args:
        text_str (str): Decoded source without a BOM.
        attributes_bool (bool): Whether to include positions.
    Returns:
        str: ast.dump text or the error line.
    """
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        try:
            tree = ast.parse(text_str)
        except (SyntaxError, ValueError) as error:
            return f"ERROR {getattr(error, 'lineno', 0)}"
        except (RecursionError, MemoryError):
            return "ERROR recursion"
    return ast.dump(tree, include_attributes=attributes_bool)


def rust_dumps_dict(binary_str: str, paths_list: list, attributes_bool: bool) -> dict:
    """Run the Rust dumper on a batch of files.

    Args:
        binary_str (str): Path of rt-dump-ast.
        paths_list (list): Files to parse.
        attributes_bool (bool): Whether positions are included.
    Returns:
        dict: path string -> dump line.
    """
    environment_dict = dict(os.environ)
    if not attributes_bool:
        environment_dict["RT_DUMP_NO_ATTRIBUTES"] = "1"
    arguments_list = [binary_str] + [str(path) for path in paths_list]
    if len(paths_list) == 1:
        arguments_list.append(os.devnull)
    result = subprocess.run(arguments_list, capture_output=True, text=True,
                            encoding="utf-8", env=environment_dict)
    dumps_dict, current_str = {}, None
    for line_str in result.stdout.split("\n"):
        if line_str.startswith("=== "):
            current_str = line_str[4:]
        elif current_str is not None and current_str not in dumps_dict:
            dumps_dict[current_str] = line_str
    if result.returncode != 0:
        for path in paths_list:
            dumps_dict.setdefault(str(path), "CRASH " + result.stderr[-300:])
    return dumps_dict


def first_difference_str(expected_str: str, actual_str: str) -> str:
    """Describe where two dumps diverge.

    Args:
        expected_str (str): CPython dump.
        actual_str (str): Rust dump.
    Returns:
        str: Category and context of the first differing character.
    """
    if expected_str.startswith("ERROR") or actual_str.startswith(("ERROR", "CRASH")):
        return f"error outcome: cpython={expected_str[:60]!r} rust={actual_str[:200]!r}"
    index_int = next((i for i, (a, b) in enumerate(zip(expected_str, actual_str))
                      if a != b), min(len(expected_str), len(actual_str)))
    node_int = expected_str.rfind("(", 0, index_int)
    node_start_int = max(expected_str.rfind(" ", 0, node_int),
                         expected_str.rfind("[", 0, node_int),
                         expected_str.rfind("=", 0, node_int)) + 1
    category_str = expected_str[node_start_int:node_int] or "?"
    window_str = expected_str[max(0, index_int - 80):index_int + 80]
    other_str = actual_str[max(0, index_int - 80):index_int + 80]
    return f"{category_str}: cpython=...{window_str}...\n          rust  =...{other_str}..."


def main() -> int:
    """Compare both parsers over every Python file in the folders.

    Args:
        None: Reads sys.argv.
    Returns:
        int: 0 when every file matches.
    """
    binary_str = sys.argv[1]
    attributes_bool = "--no-attributes" not in sys.argv
    shown_int = int(next((a.split("=", 1)[1] for a in sys.argv
                          if a.startswith("--examples=")), "1"))
    paths_list = sorted(path for root_str in sys.argv[2:]
                        if not root_str.startswith("--")
                        for path in Path(root_str).rglob("*.py") if path.is_file())
    categories, examples, matched_int, skipped_int = Counter(), {}, 0, 0
    for index_int in range(0, len(paths_list), BATCH_INT):
        batch_list = paths_list[index_int:index_int + BATCH_INT]
        rust_dict = rust_dumps_dict(binary_str, batch_list, attributes_bool)
        for path in batch_list:
            try:
                text_str = path.read_bytes().decode("utf-8-sig")
            except (UnicodeDecodeError, OSError):
                skipped_int += 1
                continue
            expected_str = python_dump_str(text_str, attributes_bool)
            actual_str = rust_dict.get(str(path), "MISSING")
            if expected_str == actual_str:
                matched_int += 1
                continue
            detail_str = first_difference_str(expected_str, actual_str)
            key_str = detail_str.split(":", 1)[0]
            categories[key_str] += 1
            examples.setdefault(key_str, []).append((str(path), detail_str))
    total_int = len(paths_list) - skipped_int
    print(f"{matched_int}/{total_int} files identical ({skipped_int} undecodable skipped)")
    for key_str, number_int in categories.most_common():
        print(f"  {number_int:6}  {key_str}")
        for path_str, detail_str in examples[key_str][:shown_int]:
            print(f"          {path_str}\n          {detail_str}")
    return 0 if matched_int == total_int else 1


if __name__ == "__main__":
    raise SystemExit(main())
