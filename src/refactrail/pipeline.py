"""Explicit refactoring stages and unresolved context for local workflows."""

from dataclasses import asdict
from hashlib import sha256
from pathlib import Path

from refactrail._version import __version__
from refactrail.correctness import check_correctness_list
from refactrail.discovery import discover_python_files_list
from refactrail.engine import check_text_list
from refactrail.fixing import (
    FileFix, fix_file, render_diff_str, write_fix_none,
)
from refactrail.formatting import format_source_str
from refactrail.models import Settings

CONTEXT_CODES_TUPLE = (
    "RT201", "RT202", "RT203", "RT204", "RT205", "RT206", "RT207",
    "RT401", "RT402",
)


def collect_context_questions_list(findings_list: list[dict]) -> list[dict]:
    """Turn unresolved names and types into located review questions.

    Args:
        findings_list: Serialized findings from the final source.
    Returns:
        list[dict]: Questions with their source locations and rule codes.
    Warnings:
        Suggestions are not invented and public names are not renamed.
    """
    return [{**finding_dict, "question": (
        "What does this value or operation represent, what is its type, "
        "and is its name part of a public API?"),
        "answer": "", "applied": False}
        for finding_dict in findings_list
        if finding_dict["code"] in CONTEXT_CODES_TUPLE]


def prepare_file_refactoring_dict(
    source_path: Path, root_path: Path, settings_info: Settings,
    write_bool: bool,
) -> dict:
    """Compute fixes and formatting, then check the proposed or written file.

    Args:
        source_path: Regular Python file to process.
        root_path: Root used for portable report paths.
        settings_info: Explicit style profile; no project config is executed.
        write_bool: Whether to apply the computed, hash-checked edits.
    Returns:
        dict: File hashes, diff, applied changes and remaining findings.
    Warnings:
        Workflow callers enable writes only in their newly created copy.
    """
    relative_str = source_path.relative_to(root_path).as_posix()
    file_info = fix_file(str(source_path), settings_info)
    if file_info.skip_reason:
        return {"path": relative_str, "status": "skipped",
                "reason": file_info.skip_reason, "changed": False}
    fixed_str = file_info.outcome.text
    file_info.outcome.text = format_source_str(
        fixed_str, relative_str, settings_info.line_length)
    if fixed_str != file_info.outcome.text:
        file_info.outcome.applied.append("Formatted supported whitespace")
    raw_bytes = file_info.outcome.text.encode("utf-8")
    findings_list = check_text_list(relative_str, raw_bytes, settings_info)
    findings_list += check_correctness_list(
        relative_str, raw_bytes, ("E4", "E7", "E9", "F"))
    diff_str = render_diff_str(file_info).replace(str(source_path),
                                                relative_str)
    if write_bool and file_info.changed_bool:
        write_fix_none(file_info)
    return serialize_file_result_dict(file_info, source_path, relative_str,
                                      write_bool, raw_bytes, findings_list,
                                      diff_str)



def prepare_refactoring_dict(
    root_path: Path, *, write_bool: bool = False,
    settings_info: Settings | None = None,
) -> dict:
    """Run independent fixes, formatting and checks with explicit coverage.

    Args:
        root_path: Project directory; use a fresh copy for workflow writes.
        write_bool: Explicit write request, false by default.
        settings_info: Profile, or the requested strict profile by default.
    Returns:
        dict: Changes, unresolved questions and final check status.
    Warnings:
        Unknown meanings and types need context. Writes are atomic per file,
        not a transaction. Target source is never imported or executed.
    """
    root_path = Path(root_path).absolute()
    if not root_path.is_dir():
        raise ValueError("Refactoring requires a project directory")
    settings_info = settings_info or Settings(profile="strict")
    files_list = discover_python_files_list([str(root_path)], settings_info)
    results_list = []
    for path_str in files_list:
        try:
            results_list.append(prepare_file_refactoring_dict(
                Path(path_str), root_path, settings_info, write_bool))
        except (OSError, ValueError, SyntaxError) as error_info:
            results_list.append({"path": Path(path_str).relative_to(
                root_path).as_posix(), "status": "error", "changed": False,
                "reason": str(error_info).replace(str(root_path), ".")})
    return summarize_refactoring_dict(results_list, settings_info,
                                      write_bool)


def serialize_file_result_dict(
    file_info: FileFix, source_path: Path, relative_str: str, write_bool: bool,
    raw_bytes: bytes, findings_list: list, diff_str: str,
) -> dict:
    """Serialize one checked file without temporary absolute paths.

    Args:
        file_info: Original source and proposed fixes.
        source_path: File whose final bytes can be hashed.
        relative_str: Portable report path.
        write_bool: Whether fixes were explicitly written.
        raw_bytes: Proposed UTF-8 bytes for a preview.
        findings_list: Final style and correctness findings.
        diff_str: Portable unified diff.
    Returns:
        dict: File evidence and remaining findings.
    Warnings:
        Preview hashes describe normalized text; write hashes use disk bytes.
    """
    return {"path": relative_str, "status": "checked",
            "changed": file_info.changed_bool,
            "written": write_bool and file_info.changed_bool,
            "source_sha256": file_info.source_sha256,
            "result_sha256": sha256(source_path.read_bytes()).hexdigest()
            if write_bool else sha256(raw_bytes).hexdigest(),
            "applied": file_info.outcome.applied, "diff": diff_str,
            "notes": [note_str.replace(str(source_path), relative_str)
                      for note_str in file_info.outcome.notes],
            "findings": [asdict(finding_info) for finding_info
                         in sorted(set(findings_list))]}


def summarize_refactoring_dict(
    results_list: list[dict], settings_info: Settings, write_bool: bool,
) -> dict:
    """Keep completed checks distinct from unresolved findings or failures.

    Args:
        results_list: Per-file changes and check outcomes.
        settings_info: Selected style profile.
        write_bool: Explicit writing option.
    Returns:
        dict: Workflow record and context checklist.
    Warnings:
        A passing check does not establish runtime equivalence.
    """
    findings_list = [finding_dict for result_dict in results_list
                     for finding_dict in result_dict.get("findings", [])]
    complete_bool = bool(results_list) and all(
        result_dict["status"] == "checked" for result_dict in results_list)
    return {"schema_version": "refactrail-workflow-1",
            "tool_version": __version__, "profile": settings_info.profile,
            "status": ("incomplete" if not complete_bool else
                       "findings" if findings_list else "passed"),
            "analysis_complete": complete_bool, "source_execution": False,
            "behavior_verified": False, "write_requested": write_bool,
            "changed_files": sum(item_dict["changed"]
                                 for item_dict in results_list),
            "files": results_list, "findings": findings_list,
            "context_questions": collect_context_questions_list(
                findings_list)}
