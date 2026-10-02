"""Portable SARIF 2.1.0 output for RefacTrail findings."""

import json
from pathlib import Path

from refactrail._version import __version__
from refactrail.models import Finding

SARIF_SCHEMA_STR = (
    "https://docs.oasis-open.org/sarif/sarif/v2.1.0/os/schemas/"
    "sarif-schema-2.1.0.json"
)


def build_result_dict(finding_info: Finding) -> dict:
    """Map a finding into a SARIF result with an escaped absolute file URI.

    Args:
        finding_info (Finding): Located diagnostic.
    Returns:
        dict: SARIF result with a one-based region.
    Warnings:
        Source coordinates count Unicode code points, not UTF-16 units.
    """
    return {
        "ruleId": finding_info.code, "level": finding_info.severity,
        "message": {"text": finding_info.message},
        "locations": [{"physicalLocation": {
            "artifactLocation": {
                "uri": Path(finding_info.path).absolute().as_uri(),
            },
            "region": {"startLine": finding_info.line,
                       "startColumn": finding_info.column},
        }}],
    }


def render_sarif_str(findings_list: list[Finding]) -> str:
    """Serialize findings with engine version and explicit analysis status.

    Args:
        findings_list (list[Finding]): Sorted diagnostics.
    Returns:
        str: A SARIF 2.1.0 document followed by a newline.
    Warnings:
        A successful invocation does not prove behavioral correctness.
    """
    failed_bool = any(finding_info.code in ("RT000", "RT001", "RT002")
                      for finding_info in findings_list)
    document_dict = {
        "$schema": SARIF_SCHEMA_STR, "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {"name": "RefacTrail",
                                "version": __version__}},
            "columnKind": "unicodeCodePoints",
            "invocations": [{"executionSuccessful": not failed_bool}],
            "results": [build_result_dict(finding_info)
                        for finding_info in findings_list],
        }],
    }
    return json.dumps(document_dict, indent=2, ensure_ascii=True) + "\n"
