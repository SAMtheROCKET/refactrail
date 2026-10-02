"""Command line: refactrail check / rules / version."""

import argparse
from pathlib import Path
import sys

from refactrail._version import __version__
from refactrail.analysis_cli import (
    add_analysis_commands_none, run_analysis_int,
)
from refactrail.config import PROFILES_TUPLE, load_settings
from refactrail.correctness import CORRECTNESS_TITLES_DICT
from refactrail.discovery import discover_python_files_list
from refactrail.engine import (
    CACHE_FOLDER_STR, ENGINES_TUPLE, check_paths_list,
)
from refactrail.fixing import fix_file, render_diff_str, write_fix_none
from refactrail.general_cli import (
    add_general_commands_none, run_format_int, run_lint_int,
)
from refactrail.report import (
    OUTPUT_FORMATS_TUPLE, render_github_str, render_json_str,
    render_statistics_str, render_text_str,
)
from refactrail.rules import RULE_TITLES_DICT
from refactrail.sarif import render_sarif_str


def add_settings_options_none(parser: argparse.ArgumentParser) -> None:
    """Add the paths and settings options shared by check and fix.

    Args:
        parser (argparse.ArgumentParser): Sub-command parser.
    Returns:
        None: Adds arguments in place.
    Warnings:
        None.
    """
    parser.add_argument("paths", nargs="*", default=["."])
    parser.add_argument("--profile", choices=PROFILES_TUPLE)
    parser.add_argument("--line-length", type=int)
    parser.add_argument("--select", help="Comma-separated codes")
    parser.add_argument("--ignore", help="Comma-separated codes")


def build_parser() -> argparse.ArgumentParser:
    """Define the command-line interface.

    Args:
        None: The interface is fixed for this version.
    Returns:
        argparse.ArgumentParser: Parser with the check and rules commands.
    Warnings:
        Option names follow Ruff where the meaning is the same.
    """
    parser = argparse.ArgumentParser(
        prog="refactrail", description="Check Python code against a "
        "refactoring rule profile.")
    parser.add_argument("--version", action="version", version=__version__)
    commands = parser.add_subparsers(dest="command", required=True)
    check_parser = commands.add_parser("check", help="Report findings")
    add_settings_options_none(check_parser)
    check_parser.add_argument("--output-format", default="text",
                              choices=OUTPUT_FORMATS_TUPLE)
    check_parser.add_argument("--statistics", action="store_true")
    check_parser.add_argument("--exit-zero", action="store_true")
    check_parser.add_argument("--no-cache", action="store_true")
    check_parser.add_argument("--jobs", type=int, default=0,
                              help="Worker processes (0: one per core)")
    check_parser.add_argument("--engine", default="auto",
                              choices=ENGINES_TUPLE,
                              help="auto uses Rust when installed")
    fix_parser = commands.add_parser(
        "fix", help="Apply bounded refactoring edits")
    add_settings_options_none(fix_parser)
    fix_parser.add_argument("--diff", action="store_true",
                            help="Show the fixes; do not write files")
    commands.add_parser("rules", help="List RT style and RC correctness codes")
    add_general_commands_none(commands)
    add_analysis_commands_none(commands)
    return parser


def run_fix_int(arguments: argparse.Namespace) -> int:
    """Run the fix command: write fixes, or print them with --diff.

    Args:
        arguments (argparse.Namespace): Parsed options.
    Returns:
        int: With --diff, 1 when changes are pending; otherwise 1 when
            findings remain after fixing, else 0.
    Warnings:
        Files are changed in place unless --diff is given.
    """
    if getattr(arguments, "jobs", 0) < 0:
        raise ValueError("jobs must be nonnegative")
    settings_info = load_settings(Path(arguments.paths[0]), {
        "profile": arguments.profile, "line_length": arguments.line_length,
        "select": split_codes_tuple(arguments.select),
        "ignore": split_codes_tuple(arguments.ignore)})
    files_list = discover_python_files_list(arguments.paths, settings_info)
    fixes_list = [fix_file(path_str, settings_info)
                  for path_str in files_list]
    for file_fix in fixes_list:
        if file_fix.skip_reason:
            sys.stderr.write(f"{file_fix.path}: skipped: "
                             f"{file_fix.skip_reason}\n")
        elif arguments.diff:
            sys.stdout.write(render_diff_str(file_fix))
        elif file_fix.changed_bool:
            write_fix_none(file_fix)
            sys.stdout.write("".join(f"{file_fix.path}: {line_str}\n" for
                                     line_str in file_fix.outcome.applied))
    changed_int = sum(file_fix.changed_bool for file_fix in fixes_list)
    skipped_bool = any(fix_info.skip_reason for fix_info in fixes_list)
    if arguments.diff:
        sys.stdout.write(f"{changed_int} file(s) would be changed.\n")
        return int(bool(changed_int) or skipped_bool)
    remaining_list = check_paths_list(files_list, settings_info)
    sys.stdout.write(f"Fixed {changed_int} file(s); {len(remaining_list)} "
                     "finding(s) remain for the selected rules (run "
                     "'refactrail check').\n")
    return int(bool(remaining_list) or skipped_bool)


def split_codes_tuple(codes_str: str | None) -> tuple[str, ...] | None:
    """Turn "RT1,RT201" into a tuple of codes.

    Args:
        codes_str (str | None): Comma-separated codes, or None.
    Returns:
        tuple[str, ...] | None: Codes, or None when not given.
    Warnings:
        Codes are upper-cased.
    """
    if codes_str is None:
        return None
    return tuple(code_str.strip().upper() for code_str in codes_str.split(",")
                 if code_str.strip())


def run_check_int(arguments: argparse.Namespace) -> int:
    """Run the check command and print its report.

    Args:
        arguments (argparse.Namespace): Parsed options.
    Returns:
        int: 0 without findings (or with --exit-zero), 1 with findings.
    Warnings:
        Settings come from the pyproject.toml nearest the first path.
    """
    if getattr(arguments, "jobs", 0) < 0:
        raise ValueError("jobs must be nonnegative")
    settings_info = load_settings(Path(arguments.paths[0]), {
        "profile": arguments.profile, "line_length": arguments.line_length,
        "select": split_codes_tuple(arguments.select),
        "ignore": split_codes_tuple(arguments.ignore)})
    files_list = discover_python_files_list(arguments.paths, settings_info)
    cache_path = None if arguments.no_cache else (
        Path.cwd() / CACHE_FOLDER_STR / "results-v1.json")
    findings_list = check_paths_list(files_list, settings_info,
                                     arguments.jobs, cache_path,
                                     arguments.engine)
    renderers_dict = {
        "text": lambda: render_text_str(findings_list, len(files_list)),
        "json": lambda: render_json_str(findings_list),
        "github": lambda: render_github_str(findings_list),
        "sarif": lambda: render_sarif_str(findings_list),
    }
    sys.stdout.write(renderers_dict[arguments.output_format]())
    if arguments.statistics:
        sys.stderr.write(render_statistics_str(findings_list))
    return 0 if arguments.exit_zero or not findings_list else 1


def main(argv_list: list[str] | None = None) -> int:
    """Run the command line and return the exit status.

    Args:
        argv_list (list[str] | None): Arguments; None reads sys.argv.
    Returns:
        int: 0 success, 1 findings, 2 usage or configuration error.
    Warnings:
        Configuration and path errors are printed, not raised.
    """
    arguments = build_parser().parse_args(argv_list)
    try:
        if arguments.command in ("scope", "index", "rename"):
            return run_analysis_int(arguments)
        if arguments.command == "lint":
            return run_lint_int(arguments)
        if arguments.command == "format":
            return run_format_int(arguments)
        if arguments.command == "fix":
            return run_fix_int(arguments)
        if arguments.command == "rules":
            sys.stdout.write("".join(f"{code_str}  {title_str}\n" for
                                     code_str, title_str in
                                     {**RULE_TITLES_DICT,
                                      **CORRECTNESS_TITLES_DICT}.items()))
            return 0
        return run_check_int(arguments)
    except (ValueError, OSError, SyntaxError) as error:
        sys.stderr.write(f"refactrail: error: {error}\n")
        return 2
