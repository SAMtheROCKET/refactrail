"""Compare the Python and Rust engines finding by finding.

Usage: python scripts/compare_engines.py FOLDER...
Exit status 0 when every file gives identical findings in both profiles.
RT001 findings are compared by code only (the two parsers word syntax
errors differently).
"""

from dataclasses import astuple, replace
from pathlib import Path
import sys

import refactrail_core

from refactrail.engine import check_text_list
from refactrail.models import Settings

PROFILES_TUPLE = ("standard", "strict")
# Files the Rust engine handed to the Python engine (newer syntax).
FALLBACK_PATHS_SET: set = set()
SHOWN_DIFFERENCES_INT = 40


def list_python_files_list(folders_list: list[str]) -> list[Path]:
    """Collect .py files under the given folders.

    Args:
        folders_list (list[str]): Folders or files.
    Returns:
        list[Path]: Sorted file paths.
    Warnings:
        Symlinks are not followed.
    """
    files_list: list[Path] = []
    for folder_str in folders_list:
        folder_path = Path(folder_str)
        files_list += ([folder_path] if folder_path.is_file()
                       else sorted(folder_path.rglob("*.py")))
    return files_list


def normalize_rows_list(rows_list: list[tuple]) -> list[tuple]:
    """Reduce RT001 rows to their code so parsers' wording may differ.

    Args:
        rows_list (list[tuple]): Finding rows.
    Returns:
        list[tuple]: Comparable rows.
    Warnings:
        None.
    """
    return [row_tuple[:4] if row_tuple[3] == "RT001" else row_tuple
            for row_tuple in rows_list]


def compare_file_list(path: Path, settings_info: Settings) -> list[str]:
    """Return a description of each differing finding in one file.

    Args:
        path (Path): File to check.
        settings_info (Settings): Active settings.
    Returns:
        list[str]: "-" rows only Python gives, "+" rows only Rust gives.
    Warnings:
        None.
    """
    raw_bytes = path.read_bytes()
    python_list = normalize_rows_list([astuple(finding) for finding in
                                       check_text_list(str(path), raw_bytes,
                                                       settings_info)])
    rust_rows = refactrail_core.check_source(str(path), raw_bytes,
                                             settings_info)
    if rust_rows is None:
        FALLBACK_PATHS_SET.add(str(path))
        return []  # newer syntax than the Rust grammar: Python checks it
    rust_list = normalize_rows_list(rust_rows)
    return ([f"- {row_tuple}" for row_tuple in python_list
             if row_tuple not in rust_list]
            + [f"+ {row_tuple}" for row_tuple in rust_list
               if row_tuple not in python_list])


def main() -> int:
    """Compare both engines over the given folders.

    Args:
        None: Reads sys.argv.
    Returns:
        int: 0 when identical, 1 otherwise.
    Warnings:
        None.
    """
    files_list = list_python_files_list(sys.argv[1:])
    differing_int, shown_int = 0, 0
    for profile_str in PROFILES_TUPLE:
        settings_info = replace(Settings(), profile=profile_str)
        for path in files_list:
            differences_list = compare_file_list(path, settings_info)
            if not differences_list:
                continue
            differing_int += 1
            if shown_int < SHOWN_DIFFERENCES_INT:
                shown_int += 1
                print(f"[{profile_str}] {path}")
                print("\n".join(differences_list[:8]))
    print(f"{len(files_list)} files x {len(PROFILES_TUPLE)} profiles; "
          f"{differing_int} differing file checks; "
          f"{len(FALLBACK_PATHS_SET)} files checked by the Python engine.")
    return 1 if differing_int else 0


if __name__ == "__main__":
    sys.exit(main())
