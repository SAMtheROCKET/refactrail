"""Compare two `lint --output-format json` documents finding by finding.

Usage: python scripts/compare_lint.py PYTHON_JSON NATIVE_JSON [--examples=N]

Prints the number of findings on each side and, by rule code, the
findings only one side reported. Exit status 0 when both are identical.
"""

from collections import Counter
import json
import sys


def load_set(path_str: str) -> list[tuple]:
    """Read findings as comparable tuples.

    Args:
        path_str (str): JSON document path.
    Returns:
        list[tuple]: (path, line, column, code, severity, message) rows.
    """
    with open(path_str, encoding="utf-8") as handle:
        return [tuple(entry[key] for key in ("path", "line", "column", "code", "severity", "message"))
                for entry in json.load(handle)]


def main() -> int:
    """Print the comparison.

    Returns:
        int: 0 when identical.
    """
    shown_int = next((int(a.split("=", 1)[1]) for a in sys.argv if a.startswith("--examples=")), 3)
    python_list, native_list = load_set(sys.argv[1]), load_set(sys.argv[2])
    python_set, native_set = set(python_list), set(native_list)
    print(f"python {len(python_list)} findings, native {len(native_list)}; "
          f"{'identical' if python_list == native_list else 'DIFFERENT'}")
    for label_str, only_set in (("only python", python_set - native_set), ("only native", native_set - python_set)):
        codes = Counter(row[3] for row in only_set)
        for code_str, count_int in codes.most_common():
            print(f"  {label_str} {code_str}: {count_int}")
            for row in sorted(row for row in only_set if row[3] == code_str)[:shown_int]:
                print(f"      {row[0]}:{row[1]}:{row[2]} {row[5][:120]}")
    return 0 if python_list == native_list else 1


if __name__ == "__main__":
    raise SystemExit(main())
