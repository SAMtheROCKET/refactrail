"""Load the word lists shared by the Python and Rust engines."""

from pathlib import Path

DATA_FOLDER_PATH = Path(__file__).resolve().parent / "data"


def load_word_set(file_name_str: str) -> frozenset[str]:
    """Read a shared word list, skipping comments and blank lines.

    Args:
        file_name_str (str): File inside the package's data folder.
    Returns:
        frozenset[str]: Words, as written.
    Warnings:
        The Rust engine embeds the same files at build time.
    """
    text_str = (DATA_FOLDER_PATH / file_name_str).read_text(encoding="utf-8")
    return frozenset(line_str.strip() for line_str in text_str.splitlines()
                     if line_str.strip() and not line_str.startswith("#"))
