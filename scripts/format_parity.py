"""Compare `refactrail format` proposals with `refactrail-native format`.

Usage: python scripts/format_parity.py NATIVE_BINARY FOLDER...
       [--line-length=N] [--hug] [--examples=N] [--limit=N] [--suffix=.ipynb]

Per file (.py unless --suffix is given), the Python side is the diff of plan_format (or "ERROR
message"); the native side is the "diff" of `format FILE --output-format
json` (or "ERROR message" from stderr). Identical diffs mean identical
proposals.
"""

from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import subprocess
import sys
import warnings

from refactrail.formatting import plan_format
from refactrail.general_cli import render_format_diff_str


def python_result_str(path_str: str, width_int: int | None, hug_bool: bool) -> str:
    """Return the Python proposal diff or error.

    Args:
        path_str (str): Source file.
        width_int (int | None): Line length, None for spacing only.
        hug_bool (bool): Hug bracket style.
    Returns:
        str: Diff or "ERROR message".
    """
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return render_format_diff_str(plan_format(path_str, width_int, hug_bool))
    except (ValueError, OSError, SyntaxError) as error:
        return f"ERROR {error}"
    except (RecursionError, MemoryError) as error:
        return f"ERROR {type(error).__name__}"


def native_result_str(binary_str: str, path_str: str, width_int: int | None, hug_bool: bool) -> str:
    """Return the native proposal diff or error.

    Args:
        binary_str (str): refactrail-native.
        path_str (str): Source file.
        width_int (int | None): Line length.
        hug_bool (bool): Hug bracket style.
    Returns:
        str: Diff or "ERROR message".
    """
    command_list = [binary_str, "format", path_str, "--output-format", "json"]
    if width_int is not None:
        command_list += ["--line-length", str(width_int)]
    if hug_bool:
        command_list += ["--bracket-style", "hug"]
    result = subprocess.run(command_list, capture_output=True)
    if result.returncode == 2 or not result.stdout:
        error_str = result.stderr.decode("utf-8", "replace").removeprefix("refactrail: error: ").rstrip("\n")
        return f"ERROR {error_str}" if error_str else f"CRASH exit {result.returncode}"
    return json.loads(result.stdout)["files"][0]["diff"]


def main() -> int:
    """Compare and summarize.

    Returns:
        int: 0 when every file matches.
    """
    option = lambda name, default: next((a.split("=", 1)[1] for a in sys.argv  # noqa: E731
                                         if a.startswith(f"--{name}=")), default)
    binary_str = sys.argv[1]
    width_text = option("line-length", None)
    width_int = int(width_text) if width_text is not None else None
    hug_bool = "--hug" in sys.argv
    shown_int = int(option("examples", "3"))
    roots_list = [a for a in sys.argv[2:] if not a.startswith("--")]
    suffix_str = option("suffix", ".py")
    paths_list = sorted(str(path) for root_str in roots_list for path in Path(root_str).rglob(f"*{suffix_str}")
                        if path.is_file() and not path.is_symlink())
    limit_int = int(option("limit", "0"))
    if limit_int:
        paths_list = paths_list[:limit_int]
    with ThreadPoolExecutor(max_workers=8) as executor:
        native_list = list(executor.map(lambda path_str: native_result_str(binary_str, path_str, width_int, hug_bool), paths_list))
    categories, examples, matched_int = Counter(), {}, 0
    for path_str, native_str in zip(paths_list, native_list):
        python_str = python_result_str(path_str, width_int, hug_bool)
        if python_str == native_str:
            matched_int += 1
            continue
        if python_str.startswith("ERROR") or native_str.startswith(("ERROR", "CRASH")):
            key_str = f"py {python_str[:70]!r} | rs {native_str[:70]!r}"
        else:
            key_str = "different diff"
        categories[key_str] += 1
        detail_str = ""
        if key_str == "different diff":
            python_lines, native_lines = python_str.splitlines(), native_str.splitlines()
            detail_str = next((f"py={a!r}\n              rs={b!r}" for a, b in zip(python_lines, native_lines) if a != b),
                              f"lines py={len(python_lines)} rs={len(native_lines)}")
        examples.setdefault(key_str, []).append((path_str, detail_str))
    print(f"{matched_int}/{len(paths_list)} format proposals identical "
          f"(line length {width_int}, {'hug' if hug_bool else 'own-line'})")
    for key_str, number_int in categories.most_common(20):
        print(f"  {number_int:6}  {key_str[:160]}")
        for path_str, detail_str in examples[key_str][:shown_int]:
            print(f"          {path_str}")
            if detail_str:
                print(f"              {detail_str[:400]}")
    return 0 if matched_int == len(paths_list) else 1


if __name__ == "__main__":
    raise SystemExit(main())
