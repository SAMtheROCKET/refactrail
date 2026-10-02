"""Compare the Rust symbol table with CPython's symtable module.

Usage: python scripts/symtable_parity.py RT_DUMP_AST_BINARY FOLDER...
       [--examples=N]

For each file that compiles, both sides print the symbol table tree in one
line: for each table (pre-order, children in creation order) its type,
name and line, then each symbol with its raw flags (definition bits and
resolved scope), sorted by name (CPython's insertion order of free names
follows set iteration, which varies with the hash seed). The Rust side is rt-dump-ast with
RT_DUMP_SYMTABLE=1. Nothing is executed: symtable only analyses source.
"""

from collections import Counter
import os
from pathlib import Path
import subprocess
import symtable
import sys
import warnings


def dump_table_str(table_info: symtable.SymbolTable) -> str:
    """Serialize one table and its descendants.

    Args:
        table_info (symtable.SymbolTable): Table to dump.
    Returns:
        str: Entries joined by "; ".
    """
    entries_list = []

    def visit(table, depth_int):
        entries_list.append(f"{depth_int} {table.get_type()} {table.get_name()} {table.get_lineno()}")
        raw_symbols = table._table.symbols
        entries_list.extend(f"{depth_int} . {name} {raw_symbols[name]}" for name in sorted(raw_symbols))
        for child in table.get_children():
            visit(child, depth_int + 1)

    visit(table_info, 0)
    return "; ".join(entries_list)


def python_dump_str(text_str: str, path_str: str) -> str:
    """Return CPython's symbol table dump, or SKIP for files that fail.

    Args:
        text_str (str): Decoded source.
        path_str (str): Name used for diagnostics.
    Returns:
        str: Dump line or "SKIP".
    """
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            compile(text_str, path_str, "exec", dont_inherit=True)
            return dump_table_str(symtable.symtable(text_str, path_str, "exec"))
    except (SyntaxError, ValueError, RecursionError, MemoryError):
        return "SKIP"


def rust_dumps_dict(binary_str: str, paths_list: list) -> dict:
    """Run the Rust symbol table dump on a batch of files.

    Args:
        binary_str (str): Path of rt-dump-ast.
        paths_list (list): Files to dump.
    Returns:
        dict: path string -> dump line.
    """
    environment_dict = dict(os.environ, RT_DUMP_SYMTABLE="1")
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
            dumps_dict.setdefault(str(path), "CRASH " + result.stderr[-200:])
    return dumps_dict


def first_difference_str(expected_str: str, actual_str: str) -> str:
    """Describe the first differing entry.

    Args:
        expected_str (str): CPython dump.
        actual_str (str): Rust dump.
    Returns:
        str: "expected | actual" around the first difference.
    """
    expected_list, actual_list = expected_str.split("; "), actual_str.split("; ")
    for index_int, (expected, actual) in enumerate(zip(expected_list, actual_list)):
        if expected != actual:
            return f"#{index_int}: py={expected!r} rs={actual!r}"
    return f"length py={len(expected_list)} rs={len(actual_list)}"


def category_str(expected_str: str, actual_str: str) -> str:
    """Name the kind of disagreement by the first differing entry.

    Args:
        expected_str (str): CPython dump.
        actual_str (str): Rust dump.
    Returns:
        str: Category key.
    """
    if actual_str.startswith(("CRASH", "MISSING", "ERROR")):
        return actual_str.split(" ", 1)[0]
    for expected, actual in zip(expected_str.split("; "), actual_str.split("; ")):
        if expected != actual:
            expected_parts, actual_parts = expected.split(" "), actual.split(" ")
            if expected_parts[1] == "." and actual_parts[1] == "." and expected_parts[2] == actual_parts[2]:
                return f"flags {expected_parts[3]} vs {actual_parts[3]}"
            return "structure"
    return "length"


def main() -> int:
    """Compare dumps and print a summary by category.

    Args:
        None: Reads sys.argv.
    Returns:
        int: 0 when every dump matches.
    """
    option = lambda name, default: next((a.split("=", 1)[1] for a in sys.argv  # noqa: E731
                                         if a.startswith(f"--{name}=")), default)
    binary_str = sys.argv[1]
    roots_list = [a for a in sys.argv[2:] if not a.startswith("--")]
    paths_list = sorted(path for root_str in roots_list
                        for path in Path(root_str).rglob("*.py") if path.is_file())
    shown_int = int(option("examples", "2"))
    categories, examples, matched_int, total_int = Counter(), {}, 0, 0
    for index_int in range(0, len(paths_list), 200):
        batch_list = paths_list[index_int:index_int + 200]
        rust_dict = rust_dumps_dict(binary_str, batch_list)
        for path in batch_list:
            try:
                text_str = path.read_bytes().decode("utf-8-sig")
            except UnicodeDecodeError:
                continue
            expected_str = python_dump_str(text_str, str(path))
            if expected_str == "SKIP":
                continue
            total_int += 1
            actual_str = rust_dict.get(str(path), "MISSING")
            if expected_str == actual_str:
                matched_int += 1
                continue
            key_str = category_str(expected_str, actual_str)
            categories[key_str] += 1
            examples.setdefault(key_str, []).append((str(path), first_difference_str(expected_str, actual_str)))
    print(f"{matched_int}/{total_int} symbol tables identical")
    for key_str, number_int in categories.most_common():
        print(f"  {number_int:6}  {key_str}")
        for name_str, difference_str in examples[key_str][:shown_int]:
            print(f"          {name_str}\n            {difference_str[:300]}")
    return 0 if matched_int == total_int else 1


if __name__ == "__main__":
    raise SystemExit(main())
