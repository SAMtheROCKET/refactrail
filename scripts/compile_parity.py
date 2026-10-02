"""Compare the Rust compile check with CPython's compile() outcome.

Usage: python scripts/compile_parity.py RT_DUMP_AST_BINARY FOLDER...
       [--examples=N] [--fuzz=COUNT] [--seed=N]

For each file, CPython's outcome is what refactrail.validation reports:
"OK", or "ERROR line offset message" from compile(text, path, "exec").
The Rust side (rt-dump-ast with RT_DUMP_COMPILE=1) must print the same.
With --fuzz, COUNT mutated variants of the files are compared instead.
Nothing is executed: compile() only builds code objects.
"""

from collections import Counter
import os
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import warnings

sys.path.insert(0, str(Path(__file__).resolve().parent))
from parser_fuzz import mutate_str  # noqa: E402


def python_outcome_str(text_str: str, path_str: str) -> str:
    """Return CPython's compile outcome for one source.

    Args:
        text_str (str): Decoded source.
        path_str (str): Name used for diagnostics.
    Returns:
        str: "OK" or "ERROR line offset message".
    """
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            compile(text_str, path_str, "exec", dont_inherit=True)
    except (SyntaxError, ValueError) as error:
        line_int = getattr(error, "lineno", None) or 1
        offset_int = getattr(error, "offset", None) or 1
        return f"ERROR {line_int} {offset_int} {getattr(error, 'msg', str(error))}"
    except (RecursionError, MemoryError):
        return "ERROR recursion"
    return "OK"


def rust_outcomes_dict(binary_str: str, paths_list: list) -> dict:
    """Run the Rust compile check on a batch of files.

    Args:
        binary_str (str): Path of rt-dump-ast.
        paths_list (list): Files to check.
    Returns:
        dict: path string -> outcome line.
    """
    environment_dict = dict(os.environ, RT_DUMP_COMPILE="1")
    arguments_list = [binary_str] + [str(path) for path in paths_list]
    if len(paths_list) == 1:
        arguments_list.append(os.devnull)
    result = subprocess.run(arguments_list, capture_output=True, text=True,
                            encoding="utf-8", env=environment_dict)
    outcomes_dict, current_str = {}, None
    for line_str in result.stdout.split("\n"):
        if line_str.startswith("=== "):
            current_str = line_str[4:]
        elif current_str is not None and current_str not in outcomes_dict:
            outcomes_dict[current_str] = line_str
    if result.returncode != 0:
        for path in paths_list:
            outcomes_dict.setdefault(str(path), "CRASH " + result.stderr[-200:])
    return outcomes_dict


def collect_paths_list(roots_list: list, fuzz_int: int, seed_int: int) -> list:
    """Return the files to compare, writing fuzz variants when asked.

    Args:
        roots_list (list): Folders to scan.
        fuzz_int (int): Number of variants, 0 for the files themselves.
        seed_int (int): Random seed for variants.
    Returns:
        list: Paths to compare.
    """
    sources_list = sorted(path for root_str in roots_list
                          for path in Path(root_str).rglob("*.py") if path.is_file())
    if not fuzz_int:
        return sources_list
    generator = random.Random(seed_int)
    keep_root = Path.home() / "compilefuzz"
    keep_root.mkdir(exist_ok=True)
    folder = Path(tempfile.mkdtemp(prefix="compilefuzz_", dir=keep_root))
    variants_list = []
    while len(variants_list) < fuzz_int:
        try:
            text_str = generator.choice(sources_list).read_bytes().decode("utf-8-sig")
        except UnicodeDecodeError:
            continue
        path = folder / f"variant_{len(variants_list):05d}.py"
        path.write_text(mutate_str(text_str[:20000], generator), encoding="utf-8", newline="")
        variants_list.append(path)
    return variants_list


def category_str(expected_str: str, actual_str: str) -> str:
    """Name the kind of disagreement.

    Args:
        expected_str (str): CPython outcome.
        actual_str (str): Rust outcome.
    Returns:
        str: Category key.
    """
    if not expected_str.startswith("ERROR"):
        return "rejected valid"
    if not actual_str.startswith("ERROR"):
        return "accepted invalid: " + expected_str.split(" ", 3)[-1][:50]
    expected_list, actual_list = expected_str.split(" ", 3), actual_str.split(" ", 3)
    if expected_list[1] != actual_list[1]:
        return "line: " + expected_list[-1][:50]
    if expected_list[-1] != actual_list[-1]:
        return "message: " + expected_list[-1][:50]
    return "offset: " + expected_list[-1][:50]


def main() -> int:
    """Compare outcomes and print a summary by category.

    Args:
        None: Reads sys.argv.
    Returns:
        int: 0 when every outcome matches.
    """
    option = lambda name, default: next((a.split("=", 1)[1] for a in sys.argv  # noqa: E731
                                         if a.startswith(f"--{name}=")), default)
    binary_str = sys.argv[1]
    roots_list = [a for a in sys.argv[2:] if not a.startswith("--")]
    paths_list = collect_paths_list(roots_list, int(option("fuzz", "0")), int(option("seed", "1")))
    shown_int = int(option("examples", "2"))
    categories, examples, matched_int, total_int = Counter(), {}, 0, 0
    for index_int in range(0, len(paths_list), 200):
        batch_list = paths_list[index_int:index_int + 200]
        rust_dict = rust_outcomes_dict(binary_str, batch_list)
        for path in batch_list:
            try:
                text_str = path.read_bytes().decode("utf-8-sig")
            except UnicodeDecodeError:
                continue
            total_int += 1
            expected_str = python_outcome_str(text_str, str(path))
            actual_str = rust_dict.get(str(path), "MISSING")
            if expected_str == actual_str:
                matched_int += 1
                continue
            key_str = category_str(expected_str, actual_str)
            categories[key_str] += 1
            examples.setdefault(key_str, []).append((path.name, expected_str[:110], actual_str[:110]))
    print(f"{matched_int}/{total_int} outcomes identical")
    for key_str, number_int in categories.most_common():
        print(f"  {number_int:6}  {key_str}")
        for name_str, expected_str, actual_str in examples[key_str][:shown_int]:
            print(f"          {name_str}: py={expected_str!r}\n          {'':{len(name_str)}}  rs={actual_str!r}")
    return 0 if matched_int == total_int else 1


if __name__ == "__main__":
    raise SystemExit(main())
