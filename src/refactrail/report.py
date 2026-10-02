"""Render findings as text, JSON or GitHub annotations."""

from collections import Counter
from json.encoder import encode_basestring_ascii

from refactrail.models import ERROR_SEVERITY_STR, Finding

OUTPUT_FORMATS_TUPLE = ("text", "json", "github", "sarif")


def render_text_str(findings_list: list[Finding], files_int: int) -> str:
    """Render findings one per line, with a summary.

    Args:
        findings_list (list[Finding]): Sorted findings.
        files_int (int): Number of files checked.
    Returns:
        str: Report text ending with a newline.
    Warnings:
        None.
    """
    lines_list = [f"{finding.path}:{finding.line}:{finding.column}: "
                  f"{finding.code} {finding.message}"
                  for finding in findings_list]
    errors_int = sum(finding.severity == ERROR_SEVERITY_STR
                     for finding in findings_list)
    if findings_list:
        lines_list.append(
            f"Found {len(findings_list)} finding(s) in {files_int} file(s): "
            f"{errors_int} error(s), {len(findings_list) - errors_int} "
            "warning(s).")
    else:
        lines_list.append(f"All checks passed ({files_int} file(s)).")
    return "\n".join(lines_list) + "\n"


def render_json_str(findings_list: list[Finding]) -> str:
    """Render findings as a JSON array of objects.

    Args:
        findings_list (list[Finding]): Sorted findings.
    Returns:
        str: JSON text ending with a newline.
    Warnings:
        Field names are part of the public output contract. The text
        is exactly json.dumps(..., indent=2), built with its C string
        escaper because the indenting encoder is slow for big reports.
    """
    if not findings_list:
        return "[]\n"
    escape = encode_basestring_ascii
    objects_list = [
        f'  {{\n    "path": {escape(finding.path)},\n'
        f'    "line": {finding.line},\n'
        f'    "column": {finding.column},\n'
        f'    "code": {escape(finding.code)},\n'
        f'    "severity": {escape(finding.severity)},\n'
        f'    "message": {escape(finding.message)}\n  }}'
        for finding in findings_list]
    return "[\n" + ",\n".join(objects_list) + "\n]\n"


def render_github_str(findings_list: list[Finding]) -> str:
    """Render findings as GitHub Actions workflow annotations.

    Args:
        findings_list (list[Finding]): Sorted findings.
    Returns:
        str: One ::error or ::warning command per finding.
    Warnings:
        Newlines in messages are escaped as GitHub requires.
    """
    lines_list = []
    for finding in findings_list:
        message_str = finding.message.replace("%", "%25").replace(
            "\r", "%0D").replace("\n", "%0A")
        lines_list.append(
            f"::{finding.severity} file={finding.path},"
            f"line={finding.line},col={finding.column},"
            f"title={finding.code}::{finding.code} "
            f"{message_str}")
    return "\n".join(lines_list) + ("\n" if lines_list else "")


def render_statistics_str(findings_list: list[Finding]) -> str:
    """Count findings per code, most frequent first.

    Args:
        findings_list (list[Finding]): Findings.
    Returns:
        str: Lines such as "  12  RT401".
    Warnings:
        None.
    """
    counts_list = Counter(finding.code
                          for finding in findings_list).most_common()
    return "".join(f"{count_int:5d}  {code_str}\n"
                   for code_str, count_int in counts_list)
