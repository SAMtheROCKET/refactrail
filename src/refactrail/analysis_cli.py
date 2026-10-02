"""Thin interfaces for partial scope, project and local rename evidence."""

import argparse
from hashlib import sha256
import json
from pathlib import Path
import sys

from refactrail.lexical import analyze_lexical_dict
from refactrail.project_index import analyze_project_dict
from refactrail.rename import plan_rename_dict
from refactrail.source import decode_source_text


def add_analysis_commands_none(commands_info: argparse._SubParsersAction
                                ) -> None:
    """Register source-linked analysis and read-only rename commands.

    Args:
        commands_info (argparse._SubParsersAction): Main command registry.
    Returns:
        None: Adds scope, index and rename subcommands.
    Warnings:
        The rename command produces a review proposal without applying it.
    """
    scope_parser = commands_info.add_parser(
        "scope", help="Inspect lexical evidence")
    scope_parser.add_argument("path")
    index_parser = commands_info.add_parser(
        "index", help="Inspect project import impact")
    index_parser.add_argument(
        "root", help="Directory used as Python import root")
    index_parser.add_argument("--changed", action="append", default=[])
    rename_parser = commands_info.add_parser(
        "rename", help="Preview a local variable rename")
    rename_parser.add_argument("path")
    rename_parser.add_argument("--function", required=True)
    rename_parser.add_argument("--old", required=True)
    rename_parser.add_argument("--new", required=True)


def read_scope_report_dict(path_str: str) -> dict:
    """Read compiler evidence with an original-source fingerprint.

    Args:
        path_str (str): Regular Python source path.
    Returns:
        dict: Versioned partial scope report.
    Warnings:
        Notebook kernel state and unsupported encodings are not inferred.
    """
    path_info = Path(path_str)
    if path_info.is_symlink() or path_info.is_junction():
        raise ValueError("Linked scope inputs are unsupported")
    raw_bytes = path_info.read_bytes()
    text_str = decode_source_text(raw_bytes)
    if text_str is None:
        raise ValueError("Scope input must be UTF-8 Python source")
    report_dict = analyze_lexical_dict(text_str, path_str)
    return {"schema_version": "refactrail-scope-1", "path": path_str,
        "source_sha256": sha256(raw_bytes).hexdigest(), **report_dict}


def run_analysis_int(arguments: argparse.Namespace) -> int:
    """Render exactly one JSON document from the selected analysis API.

    Args:
        arguments (argparse.Namespace): Parsed source and query options.
    Returns:
        int: Zero for completed partial analysis; errors propagate.
    Warnings:
        Coverage and unresolved semantics are explicit report fields.
    """
    if arguments.command == "scope":
        report_dict = read_scope_report_dict(arguments.path)
    elif arguments.command == "index":
        report_dict = analyze_project_dict(arguments.root, arguments.changed)
    else:
        report_dict = plan_rename_dict(arguments.path, arguments.function,
                                       arguments.old, arguments.new)
    sys.stdout.write(json.dumps(report_dict, indent=2) + "\n")
    return 0
