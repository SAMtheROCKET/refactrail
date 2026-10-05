"""Content-aware batch execution for independent correctness checks."""

from concurrent.futures import ProcessPoolExecutor
from hashlib import sha256
from pathlib import Path
import os

from refactrail.compat_scope_checker import SCOPE_CODES_TUPLE
from refactrail.correctness import check_correctness_list
from refactrail.engine import (
    compute_cache_key_str, load_cache_dict, load_rust_core, save_cache_none,
)
from refactrail.models import Finding, Settings


def check_snapshot_list(snapshot_tuple: tuple) -> list[Finding]:
    """Check a byte snapshot in the main process or a worker.

    Args:
        snapshot_tuple (tuple): Path, immutable bytes, select and ignore.
    Returns:
        list[Finding]: Deterministic source observations.
    Warnings:
        No target code is imported or executed.
    """
    return check_correctness_list(*snapshot_tuple)


def prepare_snapshots_tuple(paths_list: list[str], settings_info: Settings,
                             cache_dict: dict) -> tuple:
    """Hash current bytes before reusing location-independent findings.

    Args:
        paths_list (list[str]): Discovered Python files.
        settings_info (Settings): RC selection configuration.
        cache_dict (dict): Validated cache entries.
    Returns:
        tuple: Cache keys, pending immutable snapshots and cached findings.
    Warnings:
        Stub and package-initializer policies form part of the cache key.
    """
    keys_dict, pending_list, findings_list = {}, [], []
    for path_str in paths_list:
        path_info = Path(path_str)
        if path_info.suffix == ".ipynb":
            raise ValueError(
                "Notebook linting needs kernel execution context; "
                "use format for notebooks or lint exported Python")
        raw_bytes = path_info.read_bytes()
        category_str = f"{path_info.suffix}:{path_info.name == '__init__.py'}"
        key_str = sha256(("RC3:" + category_str + compute_cache_key_str(
            raw_bytes, settings_info)).encode()).hexdigest()
        keys_dict[path_str] = key_str
        if key_str in cache_dict:
            findings_list.extend(Finding(path_str, *row_list[1:])
                                 for row_list in cache_dict[key_str])
        else:
            pending_list.append((path_str, raw_bytes, settings_info.select,
                                 settings_info.ignore))
    return keys_dict, pending_list, findings_list


def check_correctness_paths_list(paths_list: list[str],
                                  select_tuple: tuple[str, ...] = ("RC",),
                                  ignore_tuple: tuple[str, ...] = (),
                                  jobs_int: int = 1,
                                  cache_path: Path | None = None,
                                  engine_str: str = "python") -> list:
    """Run independent checks with content and configuration invalidation.

    Args:
        paths_list (list[str]): Python or stub paths.
        select_tuple (tuple[str, ...]): Enabled rule prefixes.
        ignore_tuple (tuple[str, ...]): Disabled ordinary rule prefixes.
        jobs_int (int): Worker count; zero uses available CPUs.
        cache_path (Path | None): Optional advisory cache file.
        engine_str (str): "python" or "rust" (already resolved).
    Returns:
        list: Sorted findings from current immutable source snapshots.
    Warnings:
        Cache entries are local advisory data, not trusted attestations.
    """
    if type(jobs_int) is not int or jobs_int < 0:
        raise ValueError("jobs must be a nonnegative whole number")
    settings_info = Settings(select=select_tuple, ignore=ignore_tuple)
    cache_dict = load_cache_dict(cache_path) if cache_path is not None else {}
    keys_dict, pending_list, findings_list = prepare_snapshots_tuple(
        paths_list, settings_info, cache_dict)
    batches_list = (run_rust_snapshots_list(pending_list, jobs_int)
                    if engine_str == "rust"
                    else run_snapshots_list(pending_list, jobs_int))
    for snapshot_tuple, rows_list in zip(pending_list, batches_list):
        findings_list.extend(rows_list)
        cache_dict[keys_dict[snapshot_tuple[0]]] = [
            [entry.path, entry.line, entry.column, entry.code,
             entry.severity, entry.message] for entry in rows_list]
    if cache_path is not None:
        save_cache_none(cache_path, cache_dict, set(keys_dict.values()))
    return sorted(findings_list)


def is_native_selection_bool(select_tuple: tuple[str, ...]) -> bool:
    """Whether the Rust engine implements every selected code.

    Args:
        select_tuple (tuple[str, ...]): Selected code prefixes.
    Returns:
        bool: True when no prefix selects a scope-based F code.
    Warnings:
        The scope-based Pyflakes-compatible codes (F401 ... F842) run in
        the Python engine until the Rust engine has them too.
    """
    return bool(select_tuple) and all(
        prefix_str.startswith(("RC", "E", "F")) and not any(
            code_str.startswith(prefix_str) for code_str in SCOPE_CODES_TUPLE)
        for prefix_str in select_tuple)


def run_rust_snapshots_list(snapshots_list: list[tuple],
                            jobs_int: int) -> list:
    """Lint snapshots with the Rust engine, threads instead of processes.

    Args:
        snapshots_list (list[tuple]): Immutable pending source snapshots.
        jobs_int (int): Requested threads, zero for one per core.
    Returns:
        list: Finding lists in input order.
    Warnings:
        A file that does not compile is reported by the Python engine, so
        refusals keep CPython's exact message.
    """
    if not snapshots_list:
        return []
    _, _, select_tuple, ignore_tuple = snapshots_list[0]
    results_list = load_rust_core().lint_files(
        [(snapshot_tuple[0], snapshot_tuple[1])
         for snapshot_tuple in snapshots_list],
        list(select_tuple), list(ignore_tuple), jobs_int)
    return [[Finding(*row_tuple) for row_tuple in rows_list]
            if rows_list is not None else check_snapshot_list(snapshot_tuple)
            for snapshot_tuple, rows_list in zip(snapshots_list,
                                                 results_list)]


def run_snapshots_list(snapshots_list: list[tuple], jobs_int: int) -> list:
    """Batch CPU work when enough files justify process startup.

    Args:
        snapshots_list (list[tuple]): Immutable pending source snapshots.
        jobs_int (int): Requested workers, zero for CPU count.
    Returns:
        list: Finding lists in input order.
    Warnings:
        Small batches stay serial; speed claims require measured workloads.
    """
    workers_int = jobs_int or os.cpu_count() or 1
    if workers_int == 1 or len(snapshots_list) < 64:
        return [check_snapshot_list(snapshot_tuple)
                for snapshot_tuple in snapshots_list]
    with ProcessPoolExecutor(max_workers=workers_int) as executor:
        return list(executor.map(check_snapshot_list, snapshots_list,
                                 chunksize=max(1, len(snapshots_list) //
                                               (workers_int * 4))))
