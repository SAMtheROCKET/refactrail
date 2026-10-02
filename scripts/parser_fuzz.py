"""Fuzz the Rust parser against CPython's ast.parse with broken sources.

Usage: python scripts/parser_fuzz.py RT_DUMP_AST_BINARY COUNT FOLDER...
       [--examples=N] [--seed=N]

Sampled files are mutated as in lexer_fuzz.py (plus keyword and operator
insertions). Both parsers must agree on whether a variant is valid; valid
variants must dump identically and rejected ones should fail on the same
line (reported separately). Variants are never executed.
"""

from collections import Counter
from pathlib import Path
import random
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
from lexer_fuzz import INSERTIONS_TUPLE  # noqa: E402
from parser_parity import python_dump_str, rust_dumps_dict  # noqa: E402

EXTRA_INSERTIONS_TUPLE = (" if ", " else ", " for ", " in ", " = ", ", ", "*",
                          "**", " lambda ", " not ", " await ", " yield ",
                          ":=", " as ", "@", " return ", "->", ".", ";",
                          " async ", " del ", "pass\n", "    ", " import ")


def mutate_str(text_str: str, generator: random.Random) -> str:
    """Return a randomly broken copy of a source text.

    Args:
        text_str (str): Original source.
        generator (random.Random): Seeded random source.
    Returns:
        str: Mutated source.
    """
    position_int = generator.randrange(len(text_str) + 1)
    choice_int = generator.randrange(4)
    if choice_int == 0:
        return text_str[:position_int]
    if choice_int == 1:
        end_int = position_int + generator.randrange(1, 4)
        return text_str[:position_int] + text_str[end_int:]
    pool_tuple = INSERTIONS_TUPLE if choice_int == 2 else EXTRA_INSERTIONS_TUPLE
    return (text_str[:position_int] + generator.choice(pool_tuple)
            + text_str[position_int:])


def classify_str(expected_str: str, actual_str: str) -> str:
    """Name the kind of disagreement between two outcomes.

    Args:
        expected_str (str): CPython outcome.
        actual_str (str): Rust outcome.
    Returns:
        str: Category key.
    """
    expected_error = expected_str.startswith("ERROR")
    actual_error = actual_str.startswith("ERROR")
    if actual_str.startswith(("CRASH", "MISSING")):
        return "crash"
    if expected_error and actual_error:
        return "error line"
    if expected_error:
        return "accepted invalid"
    if actual_error:
        return "rejected valid"
    return "different tree"


def main() -> int:
    """Mutate sampled files and compare both parsers on each variant.

    Args:
        None: Reads sys.argv.
    Returns:
        int: 0 when every variant agrees, including error lines.
    """
    binary_str, count_int = sys.argv[1], int(sys.argv[2])
    option = lambda name, default: next((a.split("=", 1)[1] for a in sys.argv  # noqa: E731
                                         if a.startswith(f"--{name}=")), default)
    shown_int, seed_int = int(option("examples", "1")), int(option("seed", "20261002"))
    sources_list = sorted(path for root_str in sys.argv[3:]
                          if not root_str.startswith("--")
                          for path in Path(root_str).rglob("*.py") if path.is_file())
    generator = random.Random(seed_int)
    keep_root = Path.home() / "parsefuzz"
    keep_root.mkdir(exist_ok=True)
    folder = Path(tempfile.mkdtemp(prefix="parsefuzz_", dir=keep_root))
    variants_list = []
    while len(variants_list) < count_int:
        try:
            text_str = generator.choice(sources_list).read_bytes().decode("utf-8-sig")
        except UnicodeDecodeError:
            continue
        text_str = text_str[:20000]
        path = folder / f"variant_{len(variants_list):05d}.py"
        path.write_text(mutate_str(text_str, generator), encoding="utf-8", newline="")
        variants_list.append(path)
    categories, examples, matched_int = Counter(), {}, 0
    for index_int in range(0, len(variants_list), 200):
        batch_list = variants_list[index_int:index_int + 200]
        rust_dict = rust_dumps_dict(binary_str, batch_list, True)
        for path in batch_list:
            expected_str = python_dump_str(path.read_bytes().decode("utf-8"), True)
            actual_str = rust_dict.get(str(path), "MISSING")
            if expected_str == actual_str:
                matched_int += 1
                continue
            key_str = classify_str(expected_str, actual_str)
            categories[key_str] += 1
            examples.setdefault(key_str, []).append(
                (str(path), expected_str[:70], actual_str[:70]))
    print(f"{matched_int}/{len(variants_list)} variants identical"
          f" (folder {folder.name})")
    for key_str, number_int in categories.most_common():
        print(f"  {number_int:6}  {key_str}")
        for path_str, expected_str, actual_str in examples[key_str][:shown_int]:
            print(f"          {Path(path_str).name}: cpython={expected_str!r}\n"
                  f"          {'':{len(Path(path_str).name)}}  rust   ={actual_str!r}")
    return 0 if matched_int == len(variants_list) else 1


if __name__ == "__main__":
    raise SystemExit(main())
