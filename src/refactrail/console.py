"""Keep diagnostics printable on non-Unicode Windows terminals."""

import sys


def configure_console_none() -> None:
    """Escape only characters the selected output encoding cannot represent.

    Args:
        None: Uses the process output streams.
    Returns:
        None: Configures text error handling when supported.
    Warnings:
        Use Python -X utf8 for lossless redirected diffs and editor output.
        File writes never pass through this console setting.
    """
    for stream_info in (sys.stdout, sys.stderr):
        if hasattr(stream_info, "reconfigure"):
            stream_info.reconfigure(errors="backslashreplace")
