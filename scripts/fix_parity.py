"""Compare `refactrail lint --diff` with `refactrail-native lint --diff`.

Usage: python scripts/fix_parity.py NATIVE_BINARY FOLDER... [--select=CODES]

Each folder is linted with safe fixes previewed (`--diff`, nothing is
written) by the Python engine and by the native binary, which runs with
REFACTRAIL_PYTHON_VERSION set to this Python's version. Exit status,
stdout (the diffs) and stderr (notes and the summary) must be identical.
"""

import difflib
import os
import subprocess
import sys


def run_tuple(command_list: list[str], env_dict: dict) -> tuple:
    """Run one command and return its exit status, stdout and stderr.

    Args:
        command_list (list[str]): The command.
        env_dict (dict): Its environment.
    Returns:
        tuple: (status, stdout bytes, stderr bytes).
    """
    done = subprocess.run(command_list, capture_output=True, env=env_dict)
    return done.returncode, done.stdout, done.stderr


def print_difference_none(name_str: str, left: bytes, right: bytes) -> None:
    """Print the first lines where two outputs differ.

    Args:
        name_str (str): "stdout" or "stderr".
        left (bytes): Python engine output.
        right (bytes): Native output.
    Returns:
        None.
    """
    lines_list = list(difflib.unified_diff(
        left.decode("utf-8", "replace").splitlines(),
        right.decode("utf-8", "replace").splitlines(),
        "python", "native", lineterm="", n=2))
    print(f"--- {name_str} differs", *lines_list[:60], sep="\n")


def main() -> None:
    """Compare both engines on every folder.

    Returns:
        None: Exits 1 when any folder differs.
    """
    select_str = next((argument.split("=", 1)[1] for argument in sys.argv
                       if argument.startswith("--select=")), "E4,E7,F")
    native_str, *folders_list = [argument for argument in sys.argv[1:]
                                 if not argument.startswith("--")]
    version_str = f"{sys.version_info.major}.{sys.version_info.minor}"
    env_dict = dict(os.environ, REFACTRAIL_PYTHON_VERSION=version_str)
    failures_int = 0
    for folder_str in folders_list:
        arguments_list = ["lint", folder_str, "--diff", "--select",
                          select_str]
        expected = run_tuple([sys.executable, "-m", "refactrail",
                              *arguments_list, "--no-cache"], env_dict)
        actual = run_tuple([native_str, *arguments_list], env_dict)
        files_int = expected[1].count(b"\n+++ b/")
        print(f"{folder_str}: {files_int} files with fixes, "
              f"{'identical' if expected == actual else 'DIFFERENT'}")
        for name_str, position_int in (("stdout", 1), ("stderr", 2)):
            if expected[position_int] != actual[position_int]:
                print_difference_none(name_str, expected[position_int],
                                      actual[position_int])
        failures_int += expected != actual
    sys.exit(1 if failures_int else 0)


if __name__ == "__main__":
    main()
