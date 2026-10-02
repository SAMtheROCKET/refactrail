"""Generate the NFKC tables the Rust parser needs from CPython's data.

Usage: python3.12 scripts/gen_nfkc_tables.py OUTPUT_FILE

Writes one Rust module (crates/parser/src/nfkc_tables.rs) with:
    DECOMPOSITIONS  (code point, pool start, length), sorted: the full
                    compatibility decomposition (NFKD) of every code point
                    that has one, except algorithmic Hangul syllables.
    POOL            the decomposed code points.
    COMBINING       (code point, canonical combining class) when nonzero.
    COMPOSITIONS    (first, second, composite), sorted: primary
                    composites that canonical composition produces.
Run it with the Python version whose identifiers the parser mirrors;
CPython normalizes non-ASCII identifiers with NFKC.
"""

from pathlib import Path
import sys
import unicodedata

HANGUL_FIRST_INT, HANGUL_LAST_INT = 0xAC00, 0xD7A3


def collect_decompositions_tuple() -> tuple[list, list]:
    """Return the decomposition index and the pooled code points.

    Returns:
        tuple[list, list]: (code point, start, length) rows and the pool.
    """
    rows_list, pool_list = [], []
    for code_int in range(0x110000):
        if HANGUL_FIRST_INT <= code_int <= HANGUL_LAST_INT or (
            0xD800 <= code_int <= 0xDFFF
        ):
            continue
        decomposed_str = unicodedata.normalize("NFKD", chr(code_int))
        if decomposed_str != chr(code_int):
            rows_list.append((code_int, len(pool_list), len(decomposed_str)))
            pool_list.extend(ord(char_str) for char_str in decomposed_str)
    return rows_list, pool_list


def collect_compositions_list() -> list:
    """Return the canonical pairs that compose to a primary composite.

    Returns:
        list: (first, second, composite) rows sorted by the pair.
    """
    rows_list = []
    for code_int in range(0x110000):
        fields_list = unicodedata.decomposition(chr(code_int)).split()
        if len(fields_list) != 2 or fields_list[0].startswith("<"):
            continue
        first_int, second_int = (int(field_str, 16)
                                 for field_str in fields_list)
        if unicodedata.normalize("NFC", chr(first_int) + chr(second_int)) \
                == chr(code_int):
            rows_list.append((first_int, second_int, code_int))
    return sorted(rows_list)


def render_rows_str(name_str: str, type_str: str, rows_list: list) -> str:
    """Render one static Rust array, several rows per line.

    Args:
        name_str (str): Constant name.
        type_str (str): Element type.
        rows_list (list): Tuples or integers.
    Returns:
        str: Rust source.
    """
    items_list = [f"({', '.join(hex(value) for value in row)})"
                  if isinstance(row, tuple) else hex(row)
                  for row in rows_list]
    lines_list = [", ".join(items_list[index:index + 6])
                  for index in range(0, len(items_list), 6)]
    body_str = "".join(f"    {line_str},\n" for line_str in lines_list)
    return (f"pub static {name_str}: [{type_str}; {len(rows_list)}] = [\n"
            f"{body_str}];\n")


def main() -> None:
    """Write the generated module to the path given on the command line."""
    if unicodedata.unidata_version != "15.0.0":
        raise SystemExit("Run with CPython 3.12 (Unicode 15.0.0)")
    decompositions_list, pool_list = collect_decompositions_tuple()
    combining_list = [(code_int, unicodedata.combining(chr(code_int)))
                      for code_int in range(0x110000)
                      if unicodedata.combining(chr(code_int))]
    text_str = (
        "//! NFKC tables generated from CPython 3.12 (Unicode 15.0) by\n"
        "//! scripts/gen_nfkc_tables.py. Do not edit by hand.\n\n"
        + render_rows_str("DECOMPOSITIONS", "(u32, u32, u32)",
                          decompositions_list) + "\n"
        + render_rows_str("POOL", "u32", pool_list) + "\n"
        + render_rows_str("COMBINING", "(u32, u32)", combining_list) + "\n"
        + render_rows_str("COMPOSITIONS", "(u32, u32, u32)",
                          collect_compositions_list()))
    Path(sys.argv[1]).write_text(text_str, encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
