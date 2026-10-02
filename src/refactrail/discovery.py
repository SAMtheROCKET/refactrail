"""Find the Python files to check."""

import os
from pathlib import Path

from refactrail.models import Settings


def is_skipped_folder_bool(folder_path: Path, settings_info: Settings) -> bool:
    """Tell whether discovery should skip a folder.

    Args:
        folder_path (Path): Folder found while walking.
        settings_info (Settings): Excluded folder names.
    Returns:
        bool: True for excluded names, *.egg-info, symbolic links and
            virtual environments (any folder holding pyvenv.cfg).
    Warnings:
        Folders named on the command line are never skipped.
    """
    return (folder_path.name in settings_info.exclude
            or folder_path.name.endswith(".egg-info")
            or folder_path.is_symlink() or folder_path.is_junction()
            or (folder_path / "pyvenv.cfg").is_file())


def discover_python_files_list(
    paths_list: list[str], settings_info: Settings,
    suffixes_tuple: tuple[str, ...] = (".py",),
) -> list[str]:
    """List .py files under the given files and folders.

    Args:
        paths_list (list[str]): Files or folders from the command line.
        settings_info (Settings): Excluded folder names.
        suffixes_tuple (tuple[str, ...]): Discovered source suffixes.
    Returns:
        list[str]: Unique paths, sorted, as strings.
    Warnings:
        Missing paths raise FileNotFoundError naming the path.
    """
    found_set: set[str] = set()
    for path_str in paths_list:
        path = Path(path_str)
        if path.is_symlink() or path.is_junction():
            raise ValueError(f"Linked input is not supported: {path}")
        if path.is_file():
            found_set.add(str(path))
            continue
        if not path.is_dir():
            raise FileNotFoundError(f"No such file or folder: {path_str}")
        for folder_str, folders_list, names_list in os.walk(path):
            folders_list[:] = sorted(
                name_str for name_str in folders_list
                if not is_skipped_folder_bool(Path(folder_str) / name_str,
                                              settings_info))
            found_set.update(str(Path(folder_str) / name_str)
                             for name_str in names_list
                             if name_str.endswith(suffixes_tuple)
                             and not (Path(folder_str) / name_str
                                      ).is_symlink())
    return sorted(found_set)
