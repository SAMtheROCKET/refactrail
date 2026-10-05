"""Compare RefacTrail's Ruff-compatible codes with Ruff's own findings.

Usage: python scripts/ruff_oracle.py RUFF_BINARY FOLDER [CODES] [--examples=N]

Ruff is only an external test oracle here: it runs as a separate program
and its JSON findings are compared, code by code, with the findings of
RefacTrail's Python engine for the same codes (default E4,E7,E9,F). No
Ruff code is used by RefacTrail. Prints, per code, how many findings
both tools report at the same line and column and how many only one
side reports, with examples. Exit status 0 when every code matches.
"""

from collections import Counter, defaultdict
import json
from pathlib import Path
import subprocess
import sys

from refactrail.correctness_batch import check_correctness_paths_list


def run_ruff_set(ruff_str: str, folder: Path, codes_str: str) -> set[tuple]:
    """Ruff's findings as (path, line, column, code) tuples.

    Args:
        ruff_str: The Ruff executable.
        folder: The folder to check.
        codes_str: Comma-separated code prefixes.

    Returns:
        The findings; syntax errors (no code) are left out.
    """
    completed = subprocess.run(
        [ruff_str, "check", "--isolated", "--no-cache", "--exit-zero",
         "--output-format", "json", "--select", codes_str,
         "--target-version", "py312", str(folder)],
        capture_output=True, text=True, check=False)
    findings_set = set()
    for entry in json.loads(completed.stdout or "[]"):
        if entry.get("code") and entry["filename"].endswith(".py"):
            findings_set.add((str(Path(entry["filename"]).resolve()),
                              entry["location"]["row"],
                              entry["location"]["column"], entry["code"]))
    return findings_set


def run_refactrail_set(folder: Path, codes_str: str) -> set[tuple]:
    """RefacTrail's findings for the same codes.

    Args:
        folder: The folder to check.
        codes_str: Comma-separated code prefixes.

    Returns:
        (path, line, column, code) tuples.
    """
    paths_list = sorted(str(path) for path in folder.rglob("*.py"))
    select_tuple = tuple(code.strip() for code in codes_str.split(","))
    return {(str(Path(finding.path).resolve()), finding.line,
             finding.column, finding.code)
            for finding in check_correctness_paths_list(
                paths_list, select_tuple, jobs_int=0)
            if finding.code.startswith(select_tuple)}


def main() -> int:
    """Print the comparison.

    Returns:
        0 when both tools report the same findings.
    """
    arguments_list = [a for a in sys.argv[1:] if not a.startswith("--")]
    shown_int = next((int(a.split("=", 1)[1]) for a in sys.argv
                      if a.startswith("--examples=")), 3)
    ruff_str, folder = arguments_list[0], Path(arguments_list[1]).resolve()
    codes_str = arguments_list[2] if len(arguments_list) > 2 else "E4,E7,E9,F"
    ruff_set = run_ruff_set(ruff_str, folder, codes_str)
    ours_set = run_refactrail_set(folder, codes_str)
    both_counter = Counter(row[3] for row in ruff_set & ours_set)
    only_dict = defaultdict(lambda: ([], []))
    for row in ruff_set - ours_set:
        only_dict[row[3]][0].append(row)
    for row in ours_set - ruff_set:
        only_dict[row[3]][1].append(row)
    codes_list = sorted(set(both_counter) | set(only_dict))
    print(f"ruff {len(ruff_set)}, refactrail {len(ours_set)}, "
          f"same {len(ruff_set & ours_set)}")
    for code_str in codes_list:
        ruff_only, ours_only = only_dict[code_str]
        status_str = "ok" if not (ruff_only or ours_only) else "DIFF"
        print(f"{code_str:6} {status_str:4} same {both_counter[code_str]:6}"
              f"  only ruff {len(ruff_only):5}  only refactrail "
              f"{len(ours_only):5}")
        for label_str, rows_list in (("ruff", ruff_only),
                                     ("refactrail", ours_only)):
            for row in sorted(rows_list)[:shown_int]:
                print(f"    only {label_str}: {row[0]}:{row[1]}:{row[2]}")
                if "--show-source" in sys.argv:
                    lines_list = Path(row[0]).read_text(
                        encoding="utf-8", errors="replace").splitlines()
                    print(f"        | {lines_list[row[1] - 1].strip()[:110]}")
    return 0 if ruff_set == ours_set else 1


if __name__ == "__main__":
    raise SystemExit(main())
