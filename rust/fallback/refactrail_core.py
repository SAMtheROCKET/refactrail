"""Fallback for systems without a native refactrail-core wheel.

pip installs this pure-Python wheel only when no native wheel matches the
system (for example 32-bit or ARM Windows, PyPy or free-threaded CPython).
It provides no engine, so RefacTrail uses its Python engine, which gives
the same results.
"""

NATIVE = False
__version__ = "0.5.0a0"
