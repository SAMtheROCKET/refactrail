"""Load RefacTrail settings from pyproject.toml and command-line options."""

from dataclasses import fields, replace
from pathlib import Path
import tomllib

from refactrail.models import Settings

PROFILES_TUPLE = ("standard", "strict")
INTEGER_KEYS_TUPLE = (
    "line_length", "function_preferred_lines", "function_max_lines",
    "main_max_lines",
)
LIST_KEYS_TUPLE = ("select", "ignore", "exclude")


def find_pyproject_path(start_path: Path) -> Path | None:
    """Find the nearest pyproject.toml at or above a folder.

    Args:
        start_path (Path): File or folder where the search begins.
    Returns:
        Path | None: The configuration file, or None when there is none.
    Warnings:
        The search stops at the file system root.
    """
    folder_path = start_path.resolve()
    if folder_path.is_file():
        folder_path = folder_path.parent
    for candidate_path in (folder_path, *folder_path.parents):
        pyproject_path = candidate_path / "pyproject.toml"
        if pyproject_path.is_file():
            return pyproject_path
    return None


def read_tool_table_dict(pyproject_path: Path | None) -> dict:
    """Read the [tool.refactrail] table of a pyproject.toml file.

    Args:
        pyproject_path (Path | None): File to read, or None.
    Returns:
        dict: Table with dashes turned into underscores; empty if absent.
    Warnings:
        Invalid TOML raises ValueError with the file name.
    """
    if pyproject_path is None:
        return {}
    try:
        document_dict = tomllib.loads(
            pyproject_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as error:
        raise ValueError(f"{pyproject_path}: invalid TOML: {error}") from error
    table_dict = document_dict.get("tool", {}).get("refactrail", {})
    if not isinstance(table_dict, dict):
        raise ValueError("tool.refactrail must be a table")
    return {key_str.replace("-", "_"): setting_value
            for key_str, setting_value in table_dict.items()}


def build_settings(
    table_dict: dict, overrides_dict: dict | None = None,
) -> Settings:
    """Validate configuration values and build Settings.
    Args:
        table_dict (dict): Values from pyproject.toml.
        overrides_dict (dict | None): Command-line values, which win.
    Returns:
        Settings: Validated settings.
    Warnings:
        Unknown keys and invalid values raise ValueError naming them.
    """
    given_dict = {key_str: setting_value for key_str, setting_value
                  in (overrides_dict or {}).items()
                  if setting_value is not None}
    merged_dict = {**table_dict, **given_dict}
    known_set = {field_info.name for field_info in fields(Settings)}
    unknown_list = sorted(set(merged_dict) - known_set)
    if unknown_list:
        raise ValueError(f"Unknown setting(s): {', '.join(unknown_list)}")
    if merged_dict.get("profile", "standard") not in PROFILES_TUPLE:
        raise ValueError(f"Profile must be one of {PROFILES_TUPLE}")
    for key_str in INTEGER_KEYS_TUPLE:
        setting_value = merged_dict.get(key_str)
        if setting_value is not None and (
            not isinstance(setting_value, int)
            or isinstance(setting_value, bool) or setting_value < 1
        ):
            raise ValueError(f"{key_str} must be a positive integer")
    validate_limits_none(merged_dict)
    for key_str in LIST_KEYS_TUPLE:
        if key_str in merged_dict:
            values_list = merged_dict[key_str]
            if not isinstance(values_list, (list, tuple)) or not all(
                isinstance(entry_str, str) and entry_str.strip()
                for entry_str in values_list
            ):
                raise ValueError(f"{key_str} must be a list of strings")
            merged_dict[key_str] = tuple(values_list)
    return replace(Settings(), **merged_dict)


def load_settings(
    start_path: Path, overrides_dict: dict | None = None,
) -> Settings:
    """Load settings for a path from its nearest pyproject.toml.

    Args:
        start_path (Path): File or folder being checked.
        overrides_dict (dict | None): Command-line values.
    Returns:
        Settings: Validated settings.
    Warnings:
        Command-line values override the file.
    """
    return build_settings(
        read_tool_table_dict(find_pyproject_path(start_path)),
        overrides_dict)


def validate_limits_none(settings_dict: dict) -> None:
    """Validate related size limits before constructing settings.

    Args:
        settings_dict (dict): Validated integer settings.
    Returns:
        None: Raises ValueError for inconsistent limits.
    Warnings:
        Width matches the FuncLoom API used by the fix command.
    """
    if not 40 <= settings_dict.get("line_length", 79) <= 200:
        raise ValueError("line_length must be from 40 to 200")
    if settings_dict.get("function_preferred_lines", 40) > (
        settings_dict.get("function_max_lines", 50)
    ):
        raise ValueError("Preferred function size exceeds maximum")
