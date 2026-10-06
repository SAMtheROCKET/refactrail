"""Compare `refactrail rename` with `refactrail-native rename`.

Usage: python scripts/rename_parity.py NATIVE_BINARY FOLDER... [--limit=N]

For every top-level function in the .py files under the folders, the
script asks both engines for renames of its first local names: to a new
name (a candidate), to a keyword, to a name that is not NFKC-normalized,
to another local (a collision), of a parameter, of an unknown name, and
in a missing function. The Python result is json.dumps(plan_rename_dict
..., indent=2) or "refactrail: error: message" with exit status 2; the
native binary runs with REFACTRAIL_PYTHON_VERSION set to this Python's
version. Stdout, stderr and exit status must be identical.
"""

import ast
from collections import Counter
import json
import os
from pathlib import Path
import subprocess
import symtable
import sys
import warnings

from refactrail.rename import plan_rename_dict


def run_python_tuple(path_str: str, request_tuple: tuple) -> tuple:
    """Return the Python engine's exit status, stdout and stderr.

    Args:
        path_str (str): Source file.
        request_tuple (tuple): Function, old name, new name.
    Returns:
        tuple: (status, stdout, stderr) as the CLI would print them.
    """
    try:
        document_dict = plan_rename_dict(path_str, *request_tuple)
        return 0, json.dumps(document_dict, indent=2) + "\n", ""
    except (ValueError, OSError, SyntaxError) as error:
        return 2, "", f"refactrail: error: {error}\n"


def run_native_tuple(native_str: str, path_str: str,
                     request_tuple: tuple) -> tuple:
    """Return the native binary's exit status, stdout and stderr.

    Args:
        native_str (str): refactrail-native binary.
        path_str (str): Source file.
        request_tuple (tuple): Function, old name, new name.
    Returns:
        tuple: (status, stdout, stderr).
    """
    function_str, old_str, new_str = request_tuple
    version_str = f"{sys.version_info.major}.{sys.version_info.minor}"
    done = subprocess.run(
        [native_str, "rename", path_str, "--function", function_str,
         "--old", old_str, "--new", new_str], capture_output=True,
        env=dict(os.environ, REFACTRAIL_PYTHON_VERSION=version_str))
    return (done.returncode, done.stdout.decode("utf-8", "replace"),
            done.stderr.decode("utf-8", "replace"))


def list_function_tables_list(text_str: str) -> list:
    """Pair each top-level function with its symbol table.

    Args:
        text_str (str): Source text.
    Returns:
        list: (function name, symbol table) pairs; empty when the source
            does not compile.
    """
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            tree_node = ast.parse(text_str)
            table_info = symtable.symtable(text_str, "<parity>", "exec")
    except (SyntaxError, ValueError):
        return []
    children_dict = {(child.get_name(), child.get_lineno()): child
                     for child in table_info.get_children()}
    return [(node.name, children_dict[(node.name, node.lineno)])
            for node in tree_node.body
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
            and (node.name, node.lineno) in children_dict]


def build_requests_list(path_str: str) -> list:
    """List the rename requests to compare for one file.

    Args:
        path_str (str): Source file.
    Returns:
        list: (function, old, new) requests.
    """
    try:
        text_str = Path(path_str).read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return []
    requests_list = []
    for name_str, table_info in list_function_tables_list(text_str):
        symbols_list = table_info.get_symbols()
        locals_list = [symbol.get_name() for symbol in symbols_list
                       if symbol.is_local() and not symbol.is_parameter()]
        params_list = [symbol.get_name() for symbol in symbols_list
                       if symbol.is_parameter()]
        for old_str in locals_list[:2]:
            requests_list.extend((name_str, old_str, new_str) for new_str in (
                f"renamed_{old_str}", "class", "ﬁle", locals_list[-1]))
        requests_list.extend((name_str, param_str, "renamed_param")
                             for param_str in params_list[:1])
        requests_list.append((name_str, "no_such_local", "other_name"))
        requests_list.append((f"{name_str}_missing", "x", "y"))
    return requests_list


def main() -> None:
    """Compare both engines and print a summary and the first mismatches.

    Returns:
        None: Exits 1 when any request differs.
    """
    limit_int = next((int(argument.split("=", 1)[1])
                      for argument in sys.argv if argument.startswith(
                          "--limit=")), 6000)
    native_str, *folders_list = [argument for argument in sys.argv[1:]
                                 if not argument.startswith("--")]
    counts_info, mismatches_list = Counter(), []
    for path in sorted(path for folder in folders_list
                       for path in Path(folder).rglob("*.py")):
        if counts_info["requests"] >= limit_int:
            break
        for request_tuple in build_requests_list(str(path)):
            counts_info["requests"] += 1
            expected = run_python_tuple(str(path), request_tuple)
            actual = run_native_tuple(native_str, str(path), request_tuple)
            counts_info["ok" if expected[0] == 0
                        else expected[2].split(":")[2].split()[0]] += 1
            if expected != actual:
                mismatches_list.append((str(path), request_tuple))
    print(dict(counts_info))
    print(f"{len(mismatches_list)} mismatches", *mismatches_list[:10],
          sep="\n")
    sys.exit(1 if mismatches_list else 0)


if __name__ == "__main__":
    main()
