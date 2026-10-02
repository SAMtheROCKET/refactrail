"""Verify the installed RefacTrail package using trusted regression fixtures."""

from pathlib import Path
import subprocess
import sys

PROJECT_PATH = Path(__file__).resolve().parents[1]


def main() -> int:
    """Run the CLI, regression suite and strict self-check.

    Args:
        None: Uses the interpreter which launched this script.
    Returns:
        int: Zero when every command succeeds.
    Warnings:
        Native parity tests skip when the optional core is unavailable.
    """
    commands_tuple = (
        ("Installed version", ["-m", "refactrail", "--version"]),
        ("Regression suite", ["-m", "unittest", "discover", "-s", "tests",
                              "-v"]),
        ("Strict source rules", ["-m", "refactrail", "check", "src",
                                 "--profile", "strict", "--no-cache"]),
        ("General correctness", ["-m", "refactrail", "lint", "src"]),
        ("Independent formatting", ["-m", "refactrail", "format", "src",
                                    "--check"]),
        ("Rule inventory", ["-m", "refactrail", "rules"]),
    )
    for title_str, arguments_list in commands_tuple:
        print(f"\n[{title_str}]", flush=True)
        result = subprocess.run([sys.executable, *arguments_list],
                                cwd=PROJECT_PATH, check=False)
        if result.returncode:
            return 1
    print("PASS: all local verification steps completed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
