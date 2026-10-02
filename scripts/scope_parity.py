"""Compare `refactrail scope` documents with `refactrail-native scope`.

Usage: python scripts/scope_parity.py NATIVE_BINARY FOLDER... [--examples=N]

For every .py file, the Python document is json.dumps(report, indent=2)
of read_scope_report_dict (or "ERROR message" when it raises); the native
document is the binary's output (or "ERROR message" from its stderr).
"""

from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import subprocess
import sys
import warnings

from refactrail.analysis_cli import read_scope_report_dict


def python_document_str(path_str: str) -> str:
    """Return the Python engine's document or error.

    Args:
        path_str (str): Source file.
    Returns:
        str: Document text or "ERROR message".
    """
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return json.dumps(read_scope_report_dict(path_str), indent=2) + "\n"
    except (ValueError, OSError, SyntaxError) as error:
        return f"ERROR {error}"
    except (RecursionError, MemoryError):
        return "ERROR recursion"


def native_document_str(binary_str: str, path_str: str) -> str:
    """Return the native document or error.

    Args:
        binary_str (str): refactrail-native.
        path_str (str): Source file.
    Returns:
        str: Document text or "ERROR message".
    """
    result = subprocess.run([binary_str, "scope", path_str], capture_output=True)
    if result.returncode != 0:
        return "ERROR " + result.stderr.decode("utf-8", "replace").removeprefix("refactrail: error: ").rstrip("\n")
    return result.stdout.decode("utf-8")


def first_difference_str(expected_str: str, actual_str: str) -> str:
    """Show the first differing line.

    Args:
        expected_str (str): Python text.
        actual_str (str): Native text.
    Returns:
        str: Description.
    """
    for index_int, (expected, actual) in enumerate(zip(expected_str.split("\n"), actual_str.split("\n"))):
        if expected != actual:
            return f"line {index_int + 1}: py={expected[:140]!r} rs={actual[:140]!r}"
    return f"length py={len(expected_str)} rs={len(actual_str)}"


def main() -> int:
    """Compare documents and summarize.

    Returns:
        int: 0 when all match.
    """
    shown_int = next((int(a.split("=", 1)[1]) for a in sys.argv if a.startswith("--examples=")), 3)
    binary_str = sys.argv[1]
    roots_list = [a for a in sys.argv[2:] if not a.startswith("--")]
    paths_list = sorted(str(path) for root_str in roots_list for path in Path(root_str).rglob("*.py")
                        if path.is_file() and not path.is_symlink())
    with ThreadPoolExecutor(max_workers=8) as executor:
        native_list = list(executor.map(lambda path_str: native_document_str(binary_str, path_str), paths_list))
    categories, examples, matched_int = Counter(), {}, 0
    for path_str, native_str in zip(paths_list, native_list):
        python_str = python_document_str(path_str)
        if python_str == native_str:
            matched_int += 1
            continue
        if python_str.startswith("ERROR") or native_str.startswith("ERROR"):
            key_str = "error: " + python_str[:60] if python_str.startswith("ERROR") else "native error: " + native_str[:60]
        else:
            key_str = first_difference_str(python_str, native_str).split(":", 1)[1][:70]
        categories[key_str] += 1
        examples.setdefault(key_str, []).append((path_str, first_difference_str(python_str, native_str)))
    print(f"{matched_int}/{len(paths_list)} scope documents identical")
    for key_str, number_int in categories.most_common(25):
        print(f"  {number_int:6}  {key_str}")
        for path_str, difference_str in examples[key_str][:shown_int]:
            print(f"          {path_str}\n            {difference_str[:300]}")
    return 0 if matched_int == len(paths_list) else 1


if __name__ == "__main__":
    raise SystemExit(main())
