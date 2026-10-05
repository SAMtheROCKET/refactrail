"""Write random string-literal sources that stress prefixes and f/t-strings.

Usage: python3 scripts/gen_string_cases.py FOLDER COUNT SEED

Each file combines every prefix letter combination (b, u, r, f, t in any
order and case), quotes, replacement fields with conversions, '=' debug
specifiers, nested format specs, escapes, braces and line breaks. The
files are compared with each Python version's tokenize, ast and compile by
the parity scripts; nothing is executed.
"""

from pathlib import Path
import itertools
import random
import sys

LETTERS_TUPLE = ("b", "u", "r", "f", "t")
PIECES_TUPLE = (
    "x", "{x}", "{x!r}", "{x=}", "{x:>{w}}", "{x:{w}.{p}}", "{{", "}}", "\\n",
    "\\N{BULLET}", "{x:%Y\n}", "{'a'}", '{"a"}', "{x if y else z}", "{f'{x}'}",
    "{t'{x}'}", "{x!s:^10}", "\n", " ", "{lambda: 1}", "{(lambda: 1)()}",
    "{x:{'a'}}", "{x,y}", "{*a}", "{yield}", "{}", "{x!}", "{x!z}", "\\{",
    "{x:{}}", "{ x = }", "{x#}", "\\", "'", '"',
)


def make_prefix_str(generator: random.Random) -> str:
    """Return a random prefix of distinct letters in random case.

    Args:
        generator (random.Random): Seeded generator.
    Returns:
        str: Prefix, possibly empty.
    """
    count_int = generator.choice((0, 1, 1, 1, 2, 2, 3))
    letters_list = generator.sample(LETTERS_TUPLE, count_int)
    return "".join(letter_str.upper() if generator.random() < 0.3
                   else letter_str for letter_str in letters_list)


def make_literal_str(generator: random.Random) -> str:
    """Return one random string literal (often invalid on purpose).

    Args:
        generator (random.Random): Seeded generator.
    Returns:
        str: Source text of the literal.
    """
    quote_str = generator.choice(("'", '"', "'''", '"""'))
    body_str = "".join(generator.choice(PIECES_TUPLE)
                       for _ in range(generator.randint(0, 4)))
    return make_prefix_str(generator) + quote_str + body_str + quote_str


def main() -> None:
    """Write COUNT files into FOLDER."""
    folder = Path(sys.argv[1])
    folder.mkdir(parents=True, exist_ok=True)
    for old_path in folder.glob("*.py"):
        old_path.unlink()
    generator = random.Random(int(sys.argv[3]))
    for index_int in range(int(sys.argv[2])):
        literals_list = [make_literal_str(generator)
                         for _ in range(generator.randint(1, 3))]
        joiner_str = generator.choice((" ", " + ", "\n", ", "))
        source_str = "x = " + joiner_str.join(literals_list) + "\n"
        (folder / f"s_{index_int:05d}.py").write_text(
            source_str, encoding="utf-8", newline="")
    print(len(list(itertools.islice(folder.glob("*.py"), None))), "files")


if __name__ == "__main__":
    main()
