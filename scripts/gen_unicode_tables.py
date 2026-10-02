"""Generate the Unicode tables the Rust parser needs from CPython's data.

Usage: python3.12 scripts/gen_unicode_tables.py OUTPUT_FOLDER [NameAliases.txt]

Writes:
    nonprintable.txt  ranges "START END" (hex) of code points that
                      str.isprintable() rejects; repr() escapes them.
    names.txt         "NAME;HEX" lines for \\N{...} escapes, sorted by
                      name. CJK unified ideographs and Hangul syllables
                      are computed by the parser instead.
Run it with the Python version whose behaviour the parser mirrors.

Warnings:
    unicodedata cannot list name aliases, so they are read from the
    Unicode Consortium's NameAliases.txt when given; only aliases that
    unicodedata.lookup resolves to the same code point are kept.
"""

from pathlib import Path
import sys
import unicodedata

ALGORITHMIC_PREFIXES_TUPLE = ("CJK UNIFIED IDEOGRAPH-", "HANGUL SYLLABLE ")


def collect_nonprintable_list() -> list:
    """Return (start, end) ranges of code points repr() escapes.

    Args:
        None: Uses str.isprintable.
    Returns:
        list: Inclusive ranges in ascending order.
    """
    ranges_list = []
    for code_int in range(0x110000):
        if chr(code_int).isprintable():
            continue
        if ranges_list and ranges_list[-1][1] == code_int - 1:
            ranges_list[-1][1] = code_int
        else:
            ranges_list.append([code_int, code_int])
    return ranges_list


def collect_names_dict() -> dict:
    """Return name -> code point for every named, non-algorithmic char.

    Args:
        None: Uses unicodedata.name.
    Returns:
        dict: Upper-case names mapped to code points.
    """
    names_dict = {}
    for code_int in range(0x110000):
        name_str = unicodedata.name(chr(code_int), "")
        if name_str and not name_str.startswith(ALGORITHMIC_PREFIXES_TUPLE):
            names_dict[name_str] = code_int
    return names_dict


def collect_aliases_dict(path: Path) -> dict:
    """Return alias -> code point for aliases CPython also resolves.

    Args:
        path (Path): NameAliases.txt ("CODE;ALIAS;TYPE" lines).
    Returns:
        dict: Upper-case aliases mapped to code points.
    """
    aliases_dict = {}
    for line_str in path.read_text(encoding="utf-8").splitlines():
        fields_list = line_str.split("#", 1)[0].strip().split(";")
        if len(fields_list) < 2:
            continue
        code_int, alias_str = int(fields_list[0], 16), fields_list[1].upper()
        try:
            resolved_int = ord(unicodedata.lookup(alias_str))
        except (KeyError, TypeError):
            continue
        if resolved_int == code_int:
            aliases_dict[alias_str] = code_int
    return aliases_dict


def main() -> int:
    """Write both tables to the output folder.

    Args:
        None: Reads sys.argv.
    Returns:
        int: 0 on success.
    """
    folder = Path(sys.argv[1])
    folder.mkdir(parents=True, exist_ok=True)
    lines_list = [f"{start:X} {end:X}" for start, end
                  in collect_nonprintable_list()]
    (folder / "nonprintable.txt").write_text("\n".join(lines_list) + "\n",
                                             encoding="ascii")
    names_dict = collect_names_dict()
    if len(sys.argv) > 2:
        names_dict.update(collect_aliases_dict(Path(sys.argv[2])))
    names_list = [f"{name};{code:X}" for name, code
                  in sorted(names_dict.items())]
    (folder / "names.txt").write_text("\n".join(names_list) + "\n",
                                      encoding="ascii")
    print(f"unicode {unicodedata.unidata_version}: {len(lines_list)} ranges,"
          f" {len(names_list)} names")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
