"""Compare RefacTrail's Rust lexer with CPython's tokenize, file by file.

Usage: python scripts/lexer_parity.py RT_DUMP_TOKENS_BINARY FOLDER...

Files that CPython cannot decode as UTF-8 are skipped (both engines
report them separately). Exit status 0 only when every file matches.
"""

from collections import Counter
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dump_tokens import dump_lines_list  # noqa: E402

BATCH_INT = 200


def rust_dumps_dict(binary_str: str, paths_list: list[Path]) -> dict[str, list[str]]:
    """Run the Rust dumper on several files and split its output per file.

    Args:
        binary_str (str): Path of rt-dump-tokens.
        paths_list (list[Path]): Files to dump (at least two).
    Returns:
        dict[str, list[str]]: Path to dump lines.
    Warnings:
        Paths must not contain newlines.
    """
    output_str = subprocess.run([binary_str, *map(str, paths_list)],
                                capture_output=True, text=True,
                                encoding="utf-8", check=True).stdout
    dumps_dict: dict[str, list[str]] = {}
    current_list: list[str] = []
    for line_str in output_str.split("\n"):
        if line_str.startswith("=== "):
            current_list = dumps_dict.setdefault(line_str[4:], [])
        elif line_str:
            current_list.append(line_str)
    return dumps_dict


def classify_str(expected_list: list[str], actual_list: list[str]) -> str:
    """Describe the first difference between two dumps.

    Args:
        expected_list (list[str]): CPython dump lines.
        actual_list (list[str]): Rust dump lines.
    Returns:
        str: A short category with both lines.
    Warnings:
        Only the first differing line is shown.
    """
    if expected_list[:1] != actual_list[:1] and (
            expected_list[0].startswith("ERROR")
            or actual_list[0].startswith("ERROR")):
        return f"error outcome: cpython={expected_list[0][:60]!r} rust={actual_list[0][:60]!r}"
    for expected_str, actual_str in zip(expected_list, actual_list):
        if expected_str != actual_str:
            kind_str = expected_str.split(" ", 1)[0]
            return f"{kind_str}: cpython={expected_str[:90]!r} rust={actual_str[:90]!r}"
    return f"length: cpython={len(expected_list)} rust={len(actual_list)}"


def main() -> int:
    """Compare both tokenizers on every .py file under the given folders.

    Args:
        None: Reads sys.argv.
    Returns:
        int: 0 when all files match, 1 otherwise.
    Warnings:
        Never executes the files; only reads them.
    """
    binary_str = sys.argv[1]
    paths_list = sorted({path for root_str in sys.argv[2:]
                         for path in Path(root_str).rglob("*.py")
                         if path.is_file()})
    categories, examples = Counter(), {}
    matched_int = skipped_int = 0
    for index_int in range(0, len(paths_list), BATCH_INT):
        batch_list = paths_list[index_int:index_int + BATCH_INT]
        if len(batch_list) == 1:
            batch_list = batch_list + batch_list
        rust_dict = rust_dumps_dict(binary_str, batch_list)
        for path in dict.fromkeys(batch_list):
            try:
                text_str = path.read_bytes().decode("utf-8-sig")
            except UnicodeDecodeError:
                skipped_int += 1
                continue
            expected_list = dump_lines_list(text_str)
            actual_list = rust_dict.get(str(path), [])
            if expected_list == actual_list:
                matched_int += 1
                continue
            category_str = classify_str(expected_list, actual_list)
            key_str = category_str.split(":", 1)[0]
            categories[key_str] += 1
            examples.setdefault(key_str, (str(path), category_str))
    total_int = len(set(paths_list)) - skipped_int
    print(f"{matched_int}/{total_int} files identical "
          f"({skipped_int} undecodable skipped)")
    for key_str, count_int in categories.most_common():
        path_str, detail_str = examples[key_str]
        print(f"  {count_int:6}  {key_str}\n          e.g. {path_str}\n          {detail_str}")
    return 0 if matched_int == total_int else 1


if __name__ == "__main__":
    raise SystemExit(main())
