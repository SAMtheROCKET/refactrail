"""Generate the data tables the Rust lexical analysis reads.

Usage: python3.12 scripts/gen_lexical_tables.py rust/crates/engine/data

Writes, from the running interpreter (run it with CPython 3.12 and the
site module, as the Python engine runs):

- builtins.txt: the names in vars(builtins), one per line;
- word_chars.txt: the code point ranges ("start end" in hex) where
  Python's regular expression \\w matches (str.isalnum() or "_").
"""

import builtins
from pathlib import Path
import sys


def word_ranges_list() -> list[tuple[int, int]]:
    """Return inclusive code point ranges matched by \\w.

    Returns:
        list[tuple[int, int]]: Sorted, merged ranges.
    """
    ranges_list = []
    start_int = None
    for code_int in range(sys.maxunicode + 2):
        matches_bool = code_int <= sys.maxunicode and (chr(code_int).isalnum() or code_int == 0x5F)
        if matches_bool and start_int is None:
            start_int = code_int
        elif not matches_bool and start_int is not None:
            ranges_list.append((start_int, code_int - 1))
            start_int = None
    return ranges_list


def main() -> int:
    """Write both tables.

    Returns:
        int: 0.
    """
    folder = Path(sys.argv[1])
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "builtins.txt").write_text("\n".join(sorted(vars(builtins))) + "\n", encoding="utf-8", newline="\n")
    lines_list = [f"{start:x} {end:x}" for start, end in word_ranges_list()]
    (folder / "word_chars.txt").write_text("\n".join(lines_list) + "\n", encoding="utf-8", newline="\n")
    print(f"builtins: {len(vars(builtins))} names; word ranges: {len(lines_list)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
