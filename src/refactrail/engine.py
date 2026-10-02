"""The Python reference engine: check text, files and whole folders."""

from concurrent.futures import ProcessPoolExecutor
from functools import cache
from hashlib import sha256
from operator import attrgetter
import ast
import json
import os
import sys
import warnings
from pathlib import Path
from types import ModuleType

from refactrail._version import __version__
from refactrail.models import (
    ERROR_SEVERITY_STR, Finding, Settings, is_code_enabled_bool,
)
from refactrail.rules import RULE_CHECKS_TUPLE
from refactrail.rules.context import RuleContext
from refactrail.source import build_source_file, decode_source_text

PARALLEL_MINIMUM_FILES_INT = 64
CACHE_FOLDER_STR = ".refactrail_cache"
ENGINES_TUPLE = ("auto", "python", "rust")
RULE_CONTRACT_VERSION_INT = 3
FINDING_ORDER_KEY = attrgetter("path", "line", "column", "code",
                               "severity", "message")


def load_rust_core() -> ModuleType | None:
    """Import the optional Rust engine, refactrail_core.

    Args:
        None: Tries the import.
    Returns:
        module | None: The extension module, or None when not installed.
    Warnings:
        Both engines share a contract; speed depends on the workload.
    """
    try:
        import refactrail_core
    except ImportError:
        return None
    return refactrail_core


def resolve_engine_str(engine_str: str,
                       operation_str: str = "check_files") -> str:
    """Choose the engine that will run: "python" or "rust".

    Args:
        engine_str (str): "auto", "python" or "rust".
        operation_str (str): Rust core function the operation needs.
    Returns:
        str: "rust" when requested or when auto finds it, else "python".
    Warnings:
        Raises ValueError when "rust" is requested but not installed.
    """
    if engine_str == "python":
        return "python"
    core_module = load_rust_core()
    if core_module is not None and getattr(
        core_module, "RULE_CONTRACT_VERSION", None,
    ) == RULE_CONTRACT_VERSION_INT and hasattr(core_module, operation_str):
        return "rust"
    if engine_str == "rust":
        raise ValueError("The Rust engine is not available on this system; "
                         "use --engine python (same results).")
    return "python"


def parse_quietly_node(text_str: str, path_str: str) -> ast.Module:
    """Parse source without printing the parser's warnings.

    Args:
        text_str (str): Decoded source.
        path_str (str): Path for error messages.
    Returns:
        ast.Module: Parsed module.
    Warnings:
        Raises SyntaxError or ValueError like ast.parse; SyntaxWarning
        and DeprecationWarning (e.g. invalid escapes) are silenced.
    """
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", SyntaxWarning)
        warnings.simplefilter("ignore", DeprecationWarning)
        compile(text_str, path_str, "exec", dont_inherit=True)
        return ast.parse(text_str, filename=path_str)


def check_text_list(
    path_str: str, raw_bytes: bytes, settings_info: Settings,
) -> list[Finding]:
    """Check one file's bytes and return its sorted findings.

    Args:
        path_str (str): Path to report.
        raw_bytes (bytes): File contents.
        settings_info (Settings): Active settings.
    Returns:
        list[Finding]: Findings sorted by position and code.
    Warnings:
        Undecodable or unparsable files give a single RT002 or RT001.
    """
    text_str = decode_source_text(raw_bytes)
    if text_str is None:
        return [Finding(path_str, 1, 1, "RT002", ERROR_SEVERITY_STR,
                        "File is not UTF-8 or declares another encoding.")
                ] if is_code_enabled_bool(settings_info, "RT002") else []
    try:
        tree = parse_quietly_node(text_str, path_str)
    except (SyntaxError, ValueError) as error:
        return [Finding(path_str, getattr(error, "lineno", None) or 1,
                        getattr(error, "offset", None) or 1, "RT001",
                        ERROR_SEVERITY_STR, f"Syntax error: {error.msg}"
                        if isinstance(error, SyntaxError) else str(error))
                ] if is_code_enabled_bool(settings_info, "RT001") else []
    context = RuleContext(build_source_file(path_str, text_str), tree,
                          settings_info)
    for check_rule in RULE_CHECKS_TUPLE:
        try:
            check_rule(context)
        except Exception as error:  # noqa: BLE001 - reported, not hidden
            context.findings_list.append(Finding(
                path_str, 1, 1, "RT000", ERROR_SEVERITY_STR,
                f"Internal error in {check_rule.__name__}: "
                f"{type(error).__name__}: {error}. Please report it."))
    return sorted(context.findings_list)


def check_file_list(
    path_str: str, settings_info: Settings,
) -> list[Finding]:
    """Read a file and check it.

    Args:
        path_str (str): File path.
        settings_info (Settings): Active settings.
    Returns:
        list[Finding]: Sorted findings.
    Warnings:
        Unreadable files raise OSError to the caller.
    """
    return check_text_list(path_str, Path(path_str).read_bytes(),
                           settings_info)


def check_file_batch_list(
    batch_tuple: tuple[list[str], Settings],
) -> list[list[Finding]]:
    """Check several files in a worker process.

    Args:
        batch_tuple (tuple[list[str], Settings]): Paths and settings.
    Returns:
        list[list[Finding]]: Findings per path, in the same order.
    Warnings:
        Must stay a module-level function so worker processes can use it.
    """
    paths_list, settings_info = batch_tuple
    return [check_file_list(path_str, settings_info)
            for path_str in paths_list]


def compute_cache_key_str(raw_bytes: bytes, settings_info: Settings) -> str:
    """Hash file contents together with everything that affects findings.

    Args:
        raw_bytes (bytes): File contents.
        settings_info (Settings): Active settings.
    Returns:
        str: Hex digest.
    Warnings:
        Any change to RefacTrail's own code or word lists, not only a
        new version, invalidates every cached result.
    """
    digest = sha256(raw_bytes)
    identity_tuple = (__version__, compute_engine_fingerprint_str(),
                      sys.version_info[:3], settings_info)
    digest.update(repr(identity_tuple).encode("utf-8"))
    return digest.hexdigest()


@cache
def compute_engine_fingerprint_str() -> str:
    """Hash the installed RefacTrail sources and data files once.

    Args:
        None: Reads the package folder.
    Returns:
        str: Hex digest of every .py and data .txt file, path-sorted.
    Warnings:
        Computed once per process; edits during a run are not seen.
    """
    package_path = Path(__file__).resolve().parent
    digest = sha256()
    for file_path in sorted([*package_path.rglob("*.py"),
                             *package_path.rglob("data/*.txt")]):
        digest.update(file_path.relative_to(package_path).as_posix().encode())
        digest.update(file_path.read_bytes())
    return digest.hexdigest()


def load_cache_dict(cache_path: Path) -> dict:
    """Read the result cache, or start an empty one.

    Args:
        cache_path (Path): Cache file.
    Returns:
        dict: Cache key to serialized findings.
    Warnings:
        A damaged cache is ignored, never trusted.
    """
    try:
        cache_dict = json.loads(cache_path.read_text(encoding="utf-8"))
        return cache_dict if is_cache_valid_bool(cache_dict) else {}
    except (OSError, ValueError):
        return {}


def sort_findings_list(findings_list: list[Finding]) -> list[Finding]:
    """Sort findings in their natural order, quickly.

    Args:
        findings_list (list[Finding]): Findings in any order.
    Returns:
        list[Finding]: Sorted by path, line, column, code, severity
            and message, the same order as sorted(findings_list).
    Warnings:
        Uses a C-level key; dataclass comparison builds tuples.
    """
    return sorted(findings_list, key=FINDING_ORDER_KEY)


def split_cached_tuple(
    paths_list: list[str], settings_info: Settings, cache_dict: dict,
) -> tuple[dict[str, str], list[str], list[Finding]]:
    """Separate files with cached results from files needing a check.

    Args:
        paths_list (list[str]): Files to check.
        settings_info (Settings): Active settings.
        cache_dict (dict): Cache key to serialized findings.
    Returns:
        tuple: Cache key per path, paths to check, and cached findings.
    Warnings:
        Reads every file once to hash it.
    """
    keys_dict, pending_list, findings_list = {}, [], []
    for path_str in paths_list:
        key_str = compute_cache_key_str(Path(path_str).read_bytes(),
                                        settings_info)
        keys_dict[path_str] = key_str
        if key_str in cache_dict:
            findings_list += [Finding(path_str, *cached_row[1:])
                              for cached_row in cache_dict[key_str]]
        else:
            pending_list.append(path_str)
    return keys_dict, pending_list, findings_list


def check_paths_list(
    paths_list: list[str], settings_info: Settings, jobs_int: int = 0,
    cache_path: Path | None = None, engine_str: str = "auto",
) -> list[Finding]:
    """Check many files, in parallel and with a result cache.

    Args:
        paths_list (list[str]): Files to check.
        settings_info (Settings): Active settings.
        jobs_int (int): Worker processes; 0 means one per CPU core.
        cache_path (Path | None): Cache file, or None to disable caching.
        engine_str (str): "auto", "python" or "rust".
    Returns:
        list[Finding]: All findings, sorted.
    Warnings:
        Results for unchanged files come from the cache.
    """
    engine_str = resolve_engine_str(engine_str)
    if cache_path is None:
        results_dict = run_checks_dict(paths_list, settings_info,
                                       jobs_int, engine_str)
        return sort_findings_list([
            finding for path_findings_list in results_dict.values()
            for finding in path_findings_list])
    cache_dict = load_cache_dict(cache_path)
    keys_dict, pending_list, findings_list = split_cached_tuple(
        paths_list, settings_info, cache_dict)
    new_dict = run_checks_dict(pending_list, settings_info, jobs_int,
                               engine_str)
    for path_str, path_findings_list in new_dict.items():
        findings_list += path_findings_list
        cache_dict[keys_dict[path_str]] = [
            [finding.path, finding.line, finding.column, finding.code,
             finding.severity, finding.message]
            for finding in path_findings_list]
    save_cache_none(cache_path, cache_dict, set(keys_dict.values()))
    return sort_findings_list(findings_list)


def run_checks_dict(
    paths_list: list[str], settings_info: Settings, jobs_int: int,
    engine_str: str = "python",
) -> dict[str, list[Finding]]:
    """Check files serially or across worker processes or threads.

    Args:
        paths_list (list[str]): Files needing a fresh check.
        settings_info (Settings): Active settings.
        jobs_int (int): Workers; 0 means one per CPU core.
        engine_str (str): "python" or "rust" (already resolved).
    Returns:
        dict[str, list[Finding]]: Findings per path.
    Warnings:
        Small batches run in this process; starting workers costs time.
    """
    if engine_str == "rust":
        rows_list = load_rust_core().check_files(paths_list, settings_info,
                                                 jobs_int)
        return {path_str: [Finding(*row_tuple) for row_tuple in path_rows]
                for path_str, path_rows in zip(paths_list, rows_list)}
    workers_int = jobs_int or os.cpu_count() or 1
    if workers_int == 1 or len(paths_list) < PARALLEL_MINIMUM_FILES_INT:
        return {path_str: check_file_list(path_str, settings_info)
                for path_str in paths_list}
    chunk_int = max(1, len(paths_list) // (workers_int * 4))
    batches_list = [(paths_list[index_int:index_int + chunk_int],
                     settings_info)
                    for index_int in range(0, len(paths_list), chunk_int)]
    results_dict = {}
    with ProcessPoolExecutor(max_workers=workers_int) as executor:
        for batch_tuple, batch_results_list in zip(
            batches_list, executor.map(check_file_batch_list, batches_list),
        ):
            results_dict.update(zip(batch_tuple[0], batch_results_list))
    return results_dict


def save_cache_none(
    cache_path: Path, cache_dict: dict, live_keys_set: set[str],
) -> None:
    """Write the cache, keeping only entries for the files just checked.

    Args:
        cache_path (Path): Cache file.
        cache_dict (dict): All entries.
        live_keys_set (set[str]): Keys of the current files.
    Returns:
        None: Writes the file; failures are ignored.
    Warnings:
        The cache is an optimisation; losing it only costs time.
    """
    try:
        cache_path.parent.mkdir(parents=True, exist_ok=True)
        (cache_path.parent / ".gitignore").write_text("*\n",
                                                      encoding="utf-8")
        cache_path.write_text(json.dumps(
            {key_str: rows_list for key_str, rows_list in cache_dict.items()
             if key_str in live_keys_set}), encoding="utf-8")
    except OSError:
        pass


def is_cache_valid_bool(cache_dict: object) -> bool:
    """Check cache structure before constructing cached findings.

    Args:
        cache_dict (object): Decoded untrusted cache data.
    Returns:
        bool: Whether every entry follows the finding schema.
    Warnings:
        Invalid data is discarded and checked again from source.
    """
    if not isinstance(cache_dict, dict):
        return False
    for key_str, rows_list in cache_dict.items():
        if not isinstance(key_str, str) or not isinstance(rows_list, list):
            return False
        for row_list in rows_list:
            if not isinstance(row_list, list) or len(row_list) != 6:
                return False
            if not all(isinstance(row_list[index_int], str)
                       for index_int in (0, 3, 4, 5)):
                return False
            if not all(type(row_list[index_int]) is int
                       and row_list[index_int] > 0 for index_int in (1, 2)):
                return False
    return True
