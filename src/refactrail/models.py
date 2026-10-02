"""Findings and settings shared by every RefacTrail engine and command."""

from dataclasses import dataclass, field

ERROR_SEVERITY_STR = "error"
WARNING_SEVERITY_STR = "warning"
STRICT_ONLY_CODES_TUPLE = ("RT203", "RT205", "RT206", "RT207")


@dataclass(frozen=True, order=True)
class Finding:
    """Record one rule violation at a source position.

    Args:
        path: File path as given on the command line.
        line: 1-based line number.
        column: 1-based column in Unicode characters.
        code: Rule code such as "RT101".
        severity: "error" or "warning".
        message: Human-readable explanation.
    Returns:
        Finding: Instances sort by path, line, column and code.
    Warnings:
        Both engines must produce byte-identical findings for parity.
    """

    path: str
    line: int
    column: int
    code: str
    severity: str
    message: str


@dataclass(frozen=True)
class Settings:
    """Hold the rule profile and limits for one run.

    Args:
        profile: "standard" or "strict".
        line_length: Maximum characters per line.
        function_preferred_lines: Span that triggers RT501.
        function_max_lines: Span that triggers RT502.
        main_max_lines: Span of the main block that triggers RT503.
        select: Enabled code prefixes.
        ignore: Disabled code prefixes.
        exclude: Folder names skipped during discovery.
    Returns:
        Settings: Immutable configuration.
    Warnings:
        Strict-only rules stay off in the standard profile even when
        selected.
    """

    profile: str = "standard"
    line_length: int = 79
    function_preferred_lines: int = 40
    function_max_lines: int = 50
    main_max_lines: int = 100
    select: tuple[str, ...] = ("RT",)
    ignore: tuple[str, ...] = ()
    exclude: tuple[str, ...] = field(default_factory=lambda: (
        ".git", ".hg", ".venv", "venv", "__pycache__", ".tox", ".nox",
        ".mypy_cache", ".pytest_cache", ".ruff_cache", "build", "dist",
        "node_modules", ".refactrail_cache",
    ))

    def __post_init__(self) -> None:
        """Validate settings constructed directly through the Python API.

        Args:
            None: Reads the frozen settings fields.
        Returns:
            None: Raises ValueError for invalid configuration.
        Warnings:
            API and CLI settings share the same limits.
        """
        validate_settings_none(self)

    @property
    def strict_bool(self) -> bool:
        """Tell whether the strict profile is active.

        Args:
            None: Reads the profile name.
        Returns:
            bool: True for the strict profile.
        Warnings:
            Unknown profile names are rejected when settings are built.
        """
        return self.profile == "strict"


def is_code_enabled_bool(settings_info: Settings, code_str: str) -> bool:
    """Decide whether a rule code runs under the given settings.

    Args:
        settings_info (Settings): Active settings.
        code_str (str): Rule code such as "RT203".
    Returns:
        bool: True when selected, not ignored, and allowed by the profile.
    Warnings:
        Prefix matching mirrors Ruff: "RT2" selects every RT2xx rule.
    """
    if code_str in ("RT000", "RT001", "RT002"):
        return True
    if code_str in STRICT_ONLY_CODES_TUPLE and not settings_info.strict_bool:
        return False
    selected_bool = any(code_str.startswith(prefix_str)
                        for prefix_str in settings_info.select)
    ignored_bool = any(code_str.startswith(prefix_str)
                       for prefix_str in settings_info.ignore)
    return selected_bool and not ignored_bool


def validate_settings_none(settings_info: Settings) -> None:
    """Validate direct API settings before either engine receives them.

    Args:
        settings_info (Settings): Immutable options to validate.
    Returns:
        None: Raises ValueError for an unsupported value or combination.
    Warnings:
        Configuration loaders convert lists to tuples before this check.
    """
    if settings_info.profile not in ("standard", "strict"):
        raise ValueError("profile must be standard or strict")
    for name_str in ("line_length", "function_preferred_lines",
                     "function_max_lines", "main_max_lines"):
        setting_int = getattr(settings_info, name_str)
        if type(setting_int) is not int or setting_int < 1:
            raise ValueError(f"{name_str} must be a positive integer")
    if not 40 <= settings_info.line_length <= 200:
        raise ValueError("line_length must be from 40 to 200")
    if settings_info.function_preferred_lines > (
        settings_info.function_max_lines
    ):
        raise ValueError("Preferred function size exceeds maximum")
    for name_str in ("select", "ignore", "exclude"):
        entries_tuple = getattr(settings_info, name_str)
        if not isinstance(entries_tuple, tuple) or not all(
            isinstance(entry_str, str) and entry_str.strip()
            for entry_str in entries_tuple
        ):
            raise ValueError(f"{name_str} must contain nonempty strings")
