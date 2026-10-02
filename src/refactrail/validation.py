"""Validate compilation without executing source for the native engine."""

import warnings


def compile_error_tuple(text_str: str, path_str: str) -> tuple | None:
    """Describe a contextual compilation failure without running code.

    Args:
        text_str (str): Decoded Python source.
        path_str (str): Source name for diagnostics.
    Returns:
        tuple | None: Line, column and message, or None after compilation.
    Warnings:
        Successful compilation does not verify behavior or dependencies.
    """
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", SyntaxWarning)
            warnings.simplefilter("ignore", DeprecationWarning)
            compile(text_str, path_str, "exec", dont_inherit=True)
    except (SyntaxError, ValueError) as error:
        return (getattr(error, "lineno", None) or 1,
                getattr(error, "offset", None) or 1,
                f"Syntax error: {getattr(error, 'msg', str(error))}")
    return None
