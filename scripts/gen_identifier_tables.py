"""Generate the identifier tables the Rust lexer needs from CPython's data.

Usage: python3.12 scripts/gen_identifier_tables.py OUTPUT_FILE

Writes one Rust module (crates/lexer/src/identifier_tables.rs) with
inclusive non-ASCII code point ranges:
    XID_START       characters str.isidentifier() accepts first.
    XID_CONTINUE    characters it accepts after the first.
    NONPRINTABLE    characters str.isprintable() rejects.
CPython's tokenizer checks every non-ASCII name with these properties
and reports the first character outside them.
"""

from pathlib import Path
import sys
import unicodedata


def collect_ranges_list(predicate) -> list[tuple[int, int]]:
    """Return inclusive ranges of non-ASCII code points that match.

    Args:
        predicate: Function of one character.
    Returns:
        list[tuple[int, int]]: Ascending ranges.
    """
    ranges_list: list[list[int]] = []
    for code_int in range(0x80, 0x110000):
        if 0xD800 <= code_int <= 0xDFFF or not predicate(chr(code_int)):
            continue
        if ranges_list and ranges_list[-1][1] == code_int - 1:
            ranges_list[-1][1] = code_int
        else:
            ranges_list.append([code_int, code_int])
    return [tuple(range_list) for range_list in ranges_list]


def render_ranges_str(name_str: str, ranges_list: list) -> str:
    """Render one static Rust array of ranges, six per line.

    Args:
        name_str (str): Constant name.
        ranges_list (list): Inclusive ranges.
    Returns:
        str: Rust source.
    """
    items_list = [f"({start:#x}, {end:#x})" for start, end in ranges_list]
    body_str = "".join(f"    {', '.join(items_list[index:index + 6])},\n"
                       for index in range(0, len(items_list), 6))
    return (f"pub static {name_str}: [(u32, u32); {len(ranges_list)}] = [\n"
            f"{body_str}];\n")


def main() -> None:
    """Write the generated module to the path given on the command line."""
    if unicodedata.unidata_version != "15.0.0":
        raise SystemExit("Run with CPython 3.12 (Unicode 15.0.0)")
    text_str = (
        "//! Identifier tables generated from CPython 3.12 (Unicode 15.0) by\n"
        "//! scripts/gen_identifier_tables.py. Do not edit by hand.\n\n"
        + render_ranges_str("XID_START", collect_ranges_list(
            str.isidentifier)) + "\n"
        + render_ranges_str("XID_CONTINUE", collect_ranges_list(
            lambda char_str: ("a" + char_str).isidentifier())) + "\n"
        + render_ranges_str("NONPRINTABLE", collect_ranges_list(
            lambda char_str: not char_str.isprintable())))
    Path(sys.argv[1]).write_text(text_str, encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
