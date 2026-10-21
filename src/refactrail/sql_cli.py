"""Draft SQL resources through FuncLoom's public, separately tested API."""

import argparse
from dataclasses import asdict
import json

from funcloom import draft_sql_project_report


def add_sql_parser_none(commands_info: argparse._SubParsersAction) -> None:
    """Register the explicit SQL extraction command.

    Args:
        commands_info (argparse._SubParsersAction): CLI command registry.
    Returns:
        None: Adds sql and its preview/output options.
    Warnings:
        This Python workflow does not change RT/RC native lint behavior.
    """
    parser_info = commands_info.add_parser(
        "sql", help="Draft separate SQL resources in a new copy")
    parser_info.add_argument("path", help="Python source or project folder")
    parser_info.add_argument("--output", help="New output folder")
    parser_info.add_argument("--format", default="text",
                             choices=("text", "json"))


def run_sql_int(arguments_info: argparse.Namespace) -> int:
    """Show a SQL draft or create its new project copy.

    Args:
        arguments_info (argparse.Namespace): Source and output settings.
    Returns:
        int: One on refusal, zero on a completed preview or write.
    Warnings:
        Query semantics and runtime file I/O require user review.
    """
    report_info = draft_sql_project_report(
        arguments_info.path, arguments_info.output)
    if arguments_info.format == "json":
        print(json.dumps(asdict(report_info), indent=2))
    else:
        print(f"SQL resources: {report_info.status}; "
              f"{len(report_info.assets)} extracted")
        for asset_info in report_info.assets:
            print(f"{asset_info.source}:{asset_info.line} -> "
                  f"{asset_info.path}")
        for issue_info in report_info.diagnostics:
            print(f"{issue_info.path}:{issue_info.line}: {issue_info.code} "
                  f"{issue_info.message}")
        for note_str in report_info.notes:
            print(f"Review: {note_str}")
        if report_info.output:
            print(f"Output: {report_info.output}")
    return int(report_info.status == "refused")
