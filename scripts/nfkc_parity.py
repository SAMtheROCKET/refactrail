"""Compare the Rust parser's NFKC with unicodedata.normalize("NFKC").

Usage: python3.12 scripts/nfkc_parity.py PATH/TO/rt-dump-ast [COUNT] [SEED]

Checks every code point alone, then COUNT (default 300000) random strings
mixing starters with compositions, combining marks in any order, Hangul
jamo and syllables, and compatibility characters.
"""

from pathlib import Path
import os
import random
import subprocess
import sys
import tempfile
import unicodedata


def build_pools_list() -> list[list[str]]:
    """Return the character pools random strings draw from.

    Returns:
        list[list[str]]: Starters, marks, Hangul and compatibility pools.
    """
    starters_list, marks_list, compat_list = [], [], []
    for code_int in range(0x110000):
        if 0xD800 <= code_int <= 0xDFFF or code_int == 0x0A:
            continue
        char_str = chr(code_int)
        if unicodedata.combining(char_str):
            marks_list.append(char_str)
        fields_str = unicodedata.decomposition(char_str)
        if fields_str.startswith("<"):
            compat_list.append(char_str)
        elif fields_str:
            starters_list.extend(chr(int(field_str, 16))
                                 for field_str in fields_str.split())
    hangul_list = ([chr(code_int) for code_int in range(0x1100, 0x1200)]
                   + [chr(code_int) for code_int in range(0xAC00, 0xAC60)]
                   + [chr(0xD7A3)])
    return [sorted(set(starters_list)), marks_list, hangul_list, compat_list,
            list("aeiouAEIOU_1")]


def build_cases_list(count_int: int, seed_int: int) -> list[str]:
    """Return every single code point and random mixed strings.

    Args:
        count_int (int): Number of random strings.
        seed_int (int): Random seed.
    Returns:
        list[str]: Lines without a line feed.
    """
    cases_list = [chr(code_int) for code_int in range(0x110000)
                  if not 0xD800 <= code_int <= 0xDFFF and code_int != 0x0A]
    pools_list = build_pools_list()
    generator = random.Random(seed_int)
    for _ in range(count_int):
        cases_list.append("".join(
            generator.choice(generator.choice(pools_list))
            for _ in range(generator.randint(2, 7))))
    return cases_list


def main() -> int:
    """Run the comparison and print a summary.

    Returns:
        int: 0 when every line matches.
    """
    count_int = int(sys.argv[2]) if len(sys.argv) > 2 else 300_000
    seed_int = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    cases_list = build_cases_list(count_int, seed_int)
    with tempfile.TemporaryDirectory() as folder_str:
        path = Path(folder_str) / "cases.txt"
        path.write_bytes("\n".join(cases_list).encode("utf-8",
                                                      "surrogatepass"))
        done = subprocess.run([sys.argv[1], str(path)], capture_output=True,
                              env={**os.environ, "RT_DUMP_NFKC": "1"},
                              check=True)
    outputs_list = done.stdout.decode("utf-8").split("\n")[:-1]
    if len(outputs_list) != len(cases_list):
        print("line count", len(outputs_list), len(cases_list))
        return 1
    failures_int = 0
    for case_str, output_str in zip(cases_list, outputs_list):
        if unicodedata.normalize("NFKC", case_str) != output_str:
            failures_int += 1
            if failures_int <= 10:
                print("DIFF", [hex(ord(char_str)) for char_str in case_str],
                      [hex(ord(char_str)) for char_str in output_str])
    print(f"{len(cases_list) - failures_int}/{len(cases_list)} identical")
    return int(bool(failures_int))


if __name__ == "__main__":
    raise SystemExit(main())
