"""Fuzz the Rust lexer against CPython tokenize with broken sources.

Usage: python scripts/lexer_fuzz.py RT_DUMP_TOKENS_BINARY COUNT FOLDER...
       [--examples=N] [--seed=N]

Each sampled file is mutated (truncated, a character deleted, or a
quote/bracket/backslash/newline inserted) and both tokenizers must give
identical dumps, including the error outcome and its line.
"""

from collections import Counter
from pathlib import Path
import random
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dump_tokens import dump_lines_list  # noqa: E402
from lexer_parity import classify_str, rust_dumps_dict  # noqa: E402

INSERTIONS_TUPLE = ("'", '"', "(", ")", "[", "]", "{", "}", "\\", "\n",
                    "#", "\t", " ", 'f"', "'''", "\r", "{{", ":", "!")


def mutate_str(text_str: str, generator: random.Random) -> str:
    """Return a randomly broken copy of a source text.

    Args:
        text_str (str): Original source.
        generator (random.Random): Seeded random source.
    Returns:
        str: Mutated source.
    Warnings:
        Positions are code-point indexes, so text stays valid UTF-8.
    """
    position_int = generator.randrange(len(text_str) + 1)
    choice_int = generator.randrange(3)
    if choice_int == 0:
        return text_str[:position_int]
    if choice_int == 1:
        return text_str[:position_int] + text_str[position_int + 1:]
    return (text_str[:position_int] + generator.choice(INSERTIONS_TUPLE)
            + text_str[position_int:])


def main() -> int:
    """Mutate sampled files and compare both tokenizers on each variant.

    Args:
        None: Reads sys.argv.
    Returns:
        int: 0 when every variant matches.
    Warnings:
        Variants are written to a temporary folder and never executed.
    """
    binary_str, count_int = sys.argv[1], int(sys.argv[2])
    sources_list = sorted(path for root_str in sys.argv[3:]
                          if not root_str.startswith("--")
                          for path in Path(root_str).rglob("*.py"))
    seed_int = int(next((argument.split("=", 1)[1] for argument in sys.argv
                         if argument.startswith("--seed=")), "20261002"))
    generator = random.Random(seed_int)
    keep_root = Path.home() / "lexfuzz"
    keep_root.mkdir(exist_ok=True)
    folder = Path(tempfile.mkdtemp(prefix="lexfuzz_", dir=keep_root))
    variants_list = []
    while len(variants_list) < count_int:
        source = generator.choice(sources_list)
        try:
            text_str = source.read_bytes().decode("utf-8-sig")
        except UnicodeDecodeError:
            continue
        if len(text_str) > 20000:
            text_str = text_str[:20000]
        path = folder / f"variant_{len(variants_list):05d}.py"
        path.write_text(mutate_str(text_str, generator), encoding="utf-8",
                        newline="")
        variants_list.append(path)
    categories, examples, matched_int = Counter(), {}, 0
    for index_int in range(0, len(variants_list), 200):
        batch_list = variants_list[index_int:index_int + 200]
        rust_dict = rust_dumps_dict(binary_str, batch_list)
        for path in batch_list:
            expected_list = dump_lines_list(path.read_bytes().decode("utf-8"))
            actual_list = rust_dict.get(str(path), [])
            if expected_list == actual_list:
                matched_int += 1
                continue
            detail_str = classify_str(expected_list, actual_list)
            key_str = detail_str.split(":", 1)[0]
            categories[key_str] += 1
            examples.setdefault(key_str, []).append((str(path), detail_str))
    shown_int = int(next((argument.split("=", 1)[1] for argument in sys.argv
                          if argument.startswith("--examples=")), "1"))
    print(f"{matched_int}/{len(variants_list)} mutated files identical")
    for key_str, number_int in categories.most_common():
        print(f"  {number_int:6}  {key_str}")
        for path_str, detail_str in examples[key_str][:shown_int]:
            print(f"          {path_str}\n          {detail_str}")
    return 0 if matched_int == len(variants_list) else 1


if __name__ == "__main__":
    raise SystemExit(main())
