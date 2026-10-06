"""Thin commands for independent correctness checking and formatting."""

import argparse
import functools
import difflib
import json
from pathlib import Path
import sys

from refactrail.compat_fixes import FIXABLE_CODES_TUPLE, fix_compat_text
from refactrail.config import load_settings
from refactrail.correctness_batch import (
    check_correctness_paths_list, is_native_selection_bool,
)
from refactrail.discovery import discover_python_files_list
from refactrail.engine import ENGINES_TUPLE, resolve_engine_str
from refactrail.fixing import (
    fix_file, render_diff_str, verify_original_none, write_fix_none,
)
from refactrail.models import Settings
from refactrail.formatting import FormatPlan, plan_format, write_format_none
from refactrail.report import render_json_str, render_text_str
from refactrail.sarif import render_sarif_str


def add_general_commands_none(
    commands_info: argparse._SubParsersAction,
) -> None:
    """Register independent general lint and format commands.

    Args:
        commands_info (argparse._SubParsersAction): Parent command registry.
    Returns:
        None: Adds commands without changing the existing style profile.
    Warnings:
        Format is read-only unless the user supplies --write.
    """
    add_lint_parser_none(commands_info)
    format_parser = commands_info.add_parser(
        "format", help="Preview independent whitespace formatting")
    format_parser.add_argument("paths", nargs="*", default=["."])
    format_parser.add_argument("--line-length", type=int)
    format_parser.add_argument("--notebooks", action="store_true")
    format_parser.add_argument("--bracket-style", default="own-line",
                               choices=("own-line", "hug"))
    mode_group = format_parser.add_mutually_exclusive_group()
    mode_group.add_argument("--write", action="store_true")
    mode_group.add_argument("--check", action="store_true")
    mode_group.add_argument("--diff", action="store_true")
    format_parser.add_argument("--output-format", default="text",
                               choices=("text", "json"))
    format_parser.add_argument("--engine", default="auto",
                               choices=ENGINES_TUPLE,
                               help="auto uses Rust when installed")


def add_lint_parser_none(
    commands_info: argparse._SubParsersAction,
) -> None:
    """Register the lint command and its options.

    Args:
        commands_info (argparse._SubParsersAction): Parent command registry.
    Returns:
        None: Adds the lint command.
    Warnings:
        --fix and --diff are mutually exclusive.
    """
    lint_parser = commands_info.add_parser("lint", help="General correctness")
    lint_parser.add_argument("paths", nargs="*", default=["."])
    lint_parser.add_argument("--select", default="RC")
    lint_parser.add_argument("--ignore", default="")
    lint_parser.add_argument("--jobs", type=int, default=1)
    lint_parser.add_argument("--no-cache", action="store_true")
    lint_parser.add_argument("--output-format", default="text",
                             choices=("text", "json", "sarif"))
    lint_parser.add_argument("--engine", default="auto",
                             choices=ENGINES_TUPLE,
                             help="auto uses Rust when installed")
    fix_group = lint_parser.add_mutually_exclusive_group()
    fix_group.add_argument("--fix", action="store_true",
                           help="apply safe fixes (F401, F541, F632, E703,"
                                " E713, E714), then report what remains")
    fix_group.add_argument("--diff", action="store_true",
                           help="show the safe fixes; change nothing")


def discover_general_files_list(paths_list: list[str],
                                notebooks_bool: bool = False) -> list[str]:
    """Discover an explicit nonempty Python input set.

    Args:
        paths_list (list[str]): User-supplied files or directories.
        notebooks_bool (bool): Discover notebook containers as well.
    Returns:
        list[str]: Sorted paths using existing exclusion/link policies.
    Warnings:
        Notebook discovery is opt-in; explicit paths remain permitted.
    """
    settings_info = load_settings(Path(paths_list[0]))
    suffixes_tuple = (".py", ".pyi", ".ipynb") if notebooks_bool else (
        ".py", ".pyi")
    files_list = discover_python_files_list(paths_list, settings_info,
                                            suffixes_tuple)
    if not files_list:
        raise ValueError("No Python source files were selected")
    if any(Path(path_str).suffix not in (".py", ".pyi", ".ipynb")
           for path_str in files_list):
        raise ValueError(
            "Only .py, .pyi and Python .ipynb files are supported")
    return files_list


def run_lint_int(arguments: argparse.Namespace) -> int:
    """Run correctness rules without imposing a naming or docstring style.

    Args:
        arguments (argparse.Namespace): Parsed lint options.
    Returns:
        int: Zero without findings, one with findings; errors propagate.
    Warnings:
        This first RC implementation uses the independent Python engine.
    """
    files_list = discover_general_files_list(arguments.paths)
    select_tuple = split_lint_codes_tuple(arguments.select)
    ignore_tuple = split_lint_codes_tuple(arguments.ignore)
    cache_path = None if arguments.no_cache else (
        Path.cwd() / ".refactrail_cache" / "correctness-v4.json")
    engine_str = resolve_engine_str(arguments.engine, "lint_files")
    if engine_str == "rust" and not is_native_selection_bool(select_tuple):
        if arguments.engine == "rust":
            raise ValueError("lint checks RC, E and F codes; the Rust "
                             "engine has no other selections")
        engine_str = "python"
    if arguments.fix or arguments.diff:
        known_dict: dict[str, list] = {}
        for finding in check_correctness_paths_list(
                files_list, select_tuple, ignore_tuple, arguments.jobs, None,
                engine_str):
            known_dict.setdefault(finding.path, []).append(finding)
        pending_bool = run_lint_fixes_bool(known_dict, select_tuple,
                                           ignore_tuple, arguments.diff)
        if arguments.diff:
            return int(pending_bool)
    findings_list = check_correctness_paths_list(
        files_list, select_tuple, ignore_tuple, arguments.jobs, cache_path,
        engine_str)
    sys.stdout.write(render_lint_str(findings_list, len(files_list),
                                     arguments.output_format))
    return int(bool(findings_list))


def split_lint_codes_tuple(codes_str: str) -> tuple[str, ...]:
    """Turn "rc,E7" into ("RC", "E7").

    Args:
        codes_str (str): Comma-separated codes.
    Returns:
        tuple[str, ...]: Upper-case codes, blanks dropped.
    Warnings:
        None.
    """
    return tuple(code_str.strip().upper() for code_str
                 in codes_str.split(",") if code_str.strip())


def render_lint_str(findings_list: list, file_count_int: int,
                    format_str: str) -> str:
    """Render lint findings as text, JSON or SARIF.

    Args:
        findings_list (list): Sorted findings.
        file_count_int (int): Number of files checked (text summary).
        format_str (str): "text", "json" or "sarif".
    Returns:
        str: The report.
    Warnings:
        None.
    """
    if format_str == "sarif":
        return render_sarif_str(findings_list)
    if format_str == "json":
        return render_json_str(findings_list)
    return render_text_str(findings_list, file_count_int)


def run_lint_fixes_bool(known_dict: dict[str, list],
                        select_tuple: tuple[str, ...],
                        ignore_tuple: tuple[str, ...],
                        diff_bool: bool) -> bool:
    """Apply (or show) safe fixes for the selected lint findings.

    Args:
        known_dict (dict[str, list]): Each linted file's findings; only
            files with a fixable finding are opened.
        select_tuple (tuple[str, ...]): Selected code prefixes.
        ignore_tuple (tuple[str, ...]): Ignored code prefixes.
        diff_bool (bool): Show a diff instead of writing.
    Returns:
        bool: True when any file has (or had) fixes.
    Warnings:
        Files are changed in place unless diff_bool is set; each file
        is verified before it is written.
    """
    changed_int = sum(
        fix_lint_file_bool(path_str, findings_list, select_tuple,
                           ignore_tuple, diff_bool)
        for path_str, findings_list in sorted(known_dict.items())
        if any(finding.code in FIXABLE_CODES_TUPLE
               for finding in findings_list))
    verb_str = "would be fixed" if diff_bool else "fixed"
    sys.stderr.write(f"{changed_int} file(s) {verb_str}.\n")
    return bool(changed_int)


def fix_lint_file_bool(path_str: str, findings_list: list,
                       select_tuple: tuple[str, ...],
                       ignore_tuple: tuple[str, ...], diff_bool: bool) -> bool:
    """Apply (or show) the safe fixes of one file.

    Args:
        path_str (str): The file.
        findings_list (list): Its findings from the batch lint.
        select_tuple (tuple[str, ...]): Selected code prefixes.
        ignore_tuple (tuple[str, ...]): Ignored code prefixes.
        diff_bool (bool): Show a diff instead of writing.
    Returns:
        bool: True when the file has (or had) fixes.
    Warnings:
        The file is changed in place unless diff_bool is set.
    """
    fixer = functools.partial(fix_compat_text, select_tuple=select_tuple,
                              ignore_tuple=ignore_tuple,
                              known_list=findings_list)
    file_fix = fix_file(path_str, Settings(), fixer)
    if file_fix.skip_reason:
        return False
    for note_str in file_fix.outcome.notes:
        sys.stderr.write(f"{path_str}: note: {note_str}\n")
    if not file_fix.changed_bool:
        return False
    if diff_bool:
        sys.stdout.write(render_diff_str(file_fix))
    else:
        write_fix_none(file_fix)
        sys.stderr.write("".join(f"{path_str}: fixed {line_str}\n"
                                 for line_str in file_fix.outcome.applied))
    return True


def render_format_diff_str(plan_info: FormatPlan) -> str:
    """Render a proposed source change without touching the file.

    Args:
        plan_info (FormatPlan): Original and formatted byte snapshots.
    Returns:
        str: Unified review diff.
    Warnings:
        BOM and line endings remain represented in the proposal bytes.
    """
    return "".join(difflib.unified_diff(
        plan_info.original_bytes.decode("utf-8-sig").splitlines(True),
        plan_info.output_bytes.decode("utf-8-sig").splitlines(True),
        fromfile=f"a/{plan_info.path_str}", tofile=f"b/{plan_info.path_str}"))


def render_format_report_str(
    plans_list: list[FormatPlan], arguments: argparse.Namespace,
) -> str:
    """Describe formatting proposals or completed writes.

    Args:
        plans_list (list[FormatPlan]): Complete validated proposal batch.
        arguments (argparse.Namespace): Output and write mode.
    Returns:
        str: Human-readable text or one JSON document.
    Warnings:
        Formatting supports the bounded contract in docs/RULES.md.
    """
    changed_list = [plan_info for plan_info in plans_list
                    if plan_info.original_bytes != plan_info.output_bytes]
    if arguments.output_format == "json":
        return json.dumps({
            "schema_version": "refactrail-format-1",
            "engine": "refactrail-python", "written": arguments.write,
            "behavior_verified": False,
            "files": [{"path": plan_info.path_str,
                       "source_sha256": plan_info.source_sha256_str,
                       "changed": (plan_info.original_bytes !=
                                   plan_info.output_bytes),
                       "diff": render_format_diff_str(plan_info)}
                      for plan_info in plans_list],
        }, indent=2) + "\n"
    output_str = ""
    if not arguments.write and not arguments.check:
        output_str = "".join(render_format_diff_str(plan_info)
                             for plan_info in changed_list)
    action_str = "formatted" if arguments.write else "would be formatted"
    return output_str + f"{len(changed_list)} file(s) {action_str}.\n"


def run_format_int(arguments: argparse.Namespace) -> int:
    """Compute and validate all proposals before any requested write.

    Args:
        arguments (argparse.Namespace): Paths, output mode and write flag.
    Returns:
        int: Preview/check returns one for changes; successful write is zero.
    Warnings:
        Writes are atomic per file, not a multi-file transaction.
    """
    files_list = discover_general_files_list(arguments.paths,
                                              arguments.notebooks)
    engine_str = resolve_engine_str(arguments.engine, "format_text")
    plans_list = [plan_format(path_str, arguments.line_length,
                              arguments.bracket_style == "hug", engine_str)
                  for path_str in files_list]
    if arguments.write:
        for plan_info in plans_list:
            verify_original_none(Path(plan_info.path_str),
                                 plan_info.source_sha256_str)
        for plan_info in plans_list:
            if plan_info.original_bytes != plan_info.output_bytes:
                write_format_none(plan_info)
    sys.stdout.write(render_format_report_str(plans_list, arguments))
    changed_bool = any(plan_info.original_bytes != plan_info.output_bytes
                       for plan_info in plans_list)
    return int(changed_bool and not arguments.write)
