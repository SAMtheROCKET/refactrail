"""The fix command: apply safe fixes in place or show them as a diff."""

from dataclasses import dataclass
from hashlib import sha256
import os
import stat
import tempfile
import difflib
from pathlib import Path

from refactrail.fixes import FixOutcome, apply_safe_fixes
from refactrail.models import Settings
from refactrail.source import decode_source_text

UTF8_BOM_BYTES = b"\xef\xbb\xbf"


@dataclass
class FileFix:
    """Hold one file's original and fixed text.

    Args:
        path: File path.
        original: Text as read (line endings normalized to LF).
        outcome: Fix result, or None when the file was skipped.
        line_ending: The file's own line ending, restored on write.
        has_bom: Whether the file started with a UTF-8 BOM.
        skip_reason: Why the file was not fixed, if it was skipped.
    Returns:
        FileFix: Plain data.
    Warnings:
        None.
    """

    path: str
    original: str
    outcome: FixOutcome | None
    line_ending: str = "\n"
    has_bom: bool = False
    skip_reason: str = ""
    source_sha256: str = ""

    @property
    def changed_bool(self) -> bool:
        """Tell whether fixing changed the text.

        Args:
            None: Compares original and fixed text.
        Returns:
            bool: True when the fixed text differs.
        Warnings:
            None.
        """
        return self.outcome is not None and self.outcome.text != (
            self.original)


def detect_line_ending_str(text_str: str) -> str:
    """Return the line ending a file mostly uses.

    Args:
        text_str (str): Decoded text.
    Returns:
        str: "\\r\\n", "\\r" or "\\n".
    Warnings:
        Mixed files are written back with their first line ending.
    """
    for ending_str in ("\r\n", "\r", "\n"):
        if ending_str in text_str:
            return ending_str
    return "\n"


def fix_file(path_str: str, settings_info: Settings) -> FileFix:
    """Compute the fixes for one file without writing anything.

    Args:
        path_str (str): File path.
        settings_info (Settings): Enabled codes and limits.
    Returns:
        FileFix: Original text, fix outcome and how to write it back.
    Warnings:
        Files that are not UTF-8 or do not parse are skipped.
    """
    source_path = Path(path_str)
    if source_path.is_symlink() or source_path.is_junction():
        raise ValueError(f"Linked source is not writable: {source_path}")
    raw_bytes = source_path.read_bytes()
    text_str = decode_source_text(raw_bytes)
    if text_str is None:
        return FileFix(path_str, "", None,
                       skip_reason="not UTF-8 or declares another encoding")
    ending_str = detect_line_ending_str(text_str)
    normalized_str = text_str.replace("\r\n", "\n").replace("\r", "\n")
    try:
        compile(normalized_str, path_str, "exec", dont_inherit=True)
    except (SyntaxError, ValueError) as error:
        return FileFix(path_str, normalized_str, None, ending_str,
                       skip_reason=f"syntax error: {error}")
    return FileFix(path_str, normalized_str,
                   apply_safe_fixes(normalized_str, path_str, settings_info),
                   ending_str, raw_bytes.startswith(UTF8_BOM_BYTES),
                   source_sha256=sha256(raw_bytes).hexdigest())


def render_diff_str(file_fix: FileFix) -> str:
    """Render a unified diff of one file's fixes.

    Args:
        file_fix (FileFix): Computed fix.
    Returns:
        str: Diff text, empty when nothing changed.
    Warnings:
        Shown with LF line endings.
    """
    if not file_fix.changed_bool:
        return ""
    return "".join(difflib.unified_diff(
        file_fix.original.splitlines(keepends=True),
        file_fix.outcome.text.splitlines(keepends=True),
        fromfile=f"a/{file_fix.path}", tofile=f"b/{file_fix.path}"))


def write_fix_none(file_fix: FileFix) -> None:
    """Write a fixed file back with its own line endings and BOM.

    Args:
        file_fix (FileFix): Computed, changed fix.
    Returns:
        None: Replaces the file's contents.
    Warnings:
        The file is changed in place, as formatters do; use --diff to
        preview first.
    """
    if file_fix.outcome is None:
        raise ValueError("No validated fix is available")
    source_path = Path(file_fix.path)
    verify_original_none(source_path, file_fix.source_sha256)
    compile(file_fix.outcome.text, file_fix.path, "exec", dont_inherit=True)
    text_str = file_fix.outcome.text.replace("\n", file_fix.line_ending)
    output_bytes = ((UTF8_BOM_BYTES if file_fix.has_bom else b"")
                    + text_str.encode("utf-8"))
    write_atomically_none(source_path, output_bytes, file_fix.source_sha256)


def verify_original_none(source_path: Path, digest_str: str) -> None:
    """Reject a linked file or edits made after a proposal was computed.

    Args:
        source_path (Path): Original file.
        digest_str (str): SHA-256 of the bytes originally read.
    Returns:
        None: Raises ValueError instead of overwriting a newer edit.
    Warnings:
        This is a local consistency check, not a hostile filesystem lock.
    """
    if source_path.is_symlink() or source_path.is_junction() or (
        sha256(source_path.read_bytes()).hexdigest() != digest_str
    ):
        raise ValueError(f"Source changed or is linked: {source_path}")


def write_atomically_none(
    source_path: Path, output_bytes: bytes, digest_str: str,
) -> None:
    """Replace a verified source through a temporary file beside it.

    Args:
        source_path (Path): File to replace.
        output_bytes (bytes): Encoded and compiled proposal.
        digest_str (str): Original content fingerprint.
    Returns:
        None: Replaces the file atomically and retains permission bits.
    Warnings:
        Concurrent edits detected before replacement cancel the write.
    """
    mode_int = stat.S_IMODE(source_path.stat().st_mode)
    descriptor_int, temporary_str = tempfile.mkstemp(
        dir=source_path.parent, prefix=".refactrail-")
    temporary_path = Path(temporary_str)
    try:
        with os.fdopen(descriptor_int, "wb") as stream:
            stream.write(output_bytes)
            stream.flush()
            os.fsync(stream.fileno())
        temporary_path.chmod(mode_int)
        verify_original_none(source_path, digest_str)
        os.replace(temporary_path, source_path)
    finally:
        temporary_path.unlink(missing_ok=True)
