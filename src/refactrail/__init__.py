"""RefacTrail: check and safely refactor Python against a rule profile."""

from refactrail._version import __version__
from refactrail.config import load_settings
from refactrail.correctness import check_correctness_list
from refactrail.engine import check_paths_list, check_text_list
from refactrail.formatting import (
    FormatPlan, format_source_str, plan_format, write_format_none,
)
from refactrail.models import Finding, Settings
from refactrail.lexical import analyze_lexical_dict
from refactrail.project_index import analyze_project_dict
from refactrail.rename import plan_rename_dict
from refactrail.correctness_batch import check_correctness_paths_list

__all__ = [
    "analyze_lexical_dict",
    "analyze_project_dict",
    "plan_rename_dict",
    "check_correctness_paths_list",
    "Finding",
    "FormatPlan",
    "Settings",
    "__version__",
    "check_paths_list",
    "check_text_list",
    "load_settings",
    "check_correctness_list",
    "format_source_str",
    "plan_format",
    "write_format_none",
]
