"""Parity of the Rust engine (refactrail_core) with the Python engine.

Skipped when refactrail_core is not installed. The cases target what
real code rarely contains: encodings, line endings, # noqa, syntax
errors and unusual positions. RT001 is compared by code only (RULES.md),
since the two parsers place and word syntax errors differently.
"""

from dataclasses import astuple, replace
from pathlib import Path
import tempfile
import unittest

from refactrail.correctness_batch import (
    check_snapshot_list, run_rust_snapshots_list,
)
from refactrail.engine import check_paths_list, check_text_list
from refactrail.formatting import format_document_str, format_engine_str
from refactrail.models import Settings

try:
    import refactrail_core
except ImportError:  # pragma: no cover - depends on the environment
    refactrail_core = None

EDGE_SOURCES_TUPLE = (
    (b"import ast, typing\nSite = tuple[ast.AST, int]\nOpt = Site | None\n"
     b"Deep = typing.Dict[str, int]\nA: typing.TypeAlias = int\n"
     b"type P[T] = list[T]\nLIMIT = 3\nRunner = make()\nX = 3\n"),
    b"\xef\xbb\xbf# coding: latin-1\nx = 1\n",
    b"return 1\n",
    b"break\n",
    b"nonlocal missing\n",
    b'x = "# noqa"\n',
    b'message = """text\n# noqa\nend"""\n',
    b"def compute(count: int) -> float:\n    return count / 2\n",
    b"mask = left < right\n",
    b"mask = left is right\n",

    b"",
    b'"""Only a docstring."""\n',
    b"\xef\xbb\xbfx = 1\n",
    b"# -*- coding: latin-1 -*-\nx = 1\n",
    b"# coding: utf-8\nx = 1\n",
    b"#!/usr/bin/env python\n# vim: set fileencoding=utf8 :\nx = 1\n",
    b"x = '\xe9'\n",
    b"x = 1\ry = 2\r",
    b"x = 1\r\ny = 2\r\n",
    b"def f(:\n    pass\n",
    b"x = (1,\n",
    b"x = 1\x00\n",
    b"def \\\n    run(a):\n    return a\n",
    b"async def fetch(a):\n    return a\n",
    b"async  def fetch(a): return a\n",
    b"class \\\n  Thing:\n    pass\n",
    b"@dec\n@other(1)\ndef f(a, /, b=2, *args, c, **kw):\n    pass\n",
    b"x = 1  # noqa\ny = 2  # noqa: RT201\nz = 3  # NOQA:rt2\nw = 4 # noqa: E501, RT20\n",
    b"try:\n    pass\nexcept* ValueError as e:\n    pass\n",
    b"try:\n    import a\nexcept ImportError as e:\n    a = None\n",
    b"match p:\n    case [x, *rest]:\n        pass\n    case {'k': v}:\n        pass\n",
    b"values = [y := 1 for i in range(3)]\nf = lambda q, r=2: q\n",
    b"def outer():\n    global G\n    G = 1\n    def inner():\n        nonlocal_x = 2\n",
    b"class T:\n    def setUp(self):\n        pass\n    def visit_Name(self, node):\n        pass\n",
    b"def f[T](value: T) -> T:\n    return value\ntype Alias = int\n",
    b"x = f'{a!r:>{width}}' f\"{b}\"\n",
    ("\u00e9t\u00e9 = 1\nname = '\u4e2d\u6587' ; \u00e0 = 2\n".encode()),
    b"from typing import TYPE_CHECKING\nif TYPE_CHECKING:\n    import os\nLIMIT = 3\n",
    b'if "__main__" == __name__:\n    main()\n',
    (b'def run(a, b):\n    """Do.\n\n    Parameters\n    ----------\n    a : int\n'
     b'        First.\n    c : int\n\n    Returns\n    -------\n    int\n    """\n'),
    (b'def run(a, *, b):\n    """Do.\r\n\r\n    Args:\r\n        a (int): x.\r\n'
     b'        *b: y.\r\n    Yields:\r\n        int: z.\r\n    """\r\n    yield a\r\n'),
    b"x = 1\n\x0cdef f():\n\tpass\n",
    b"x = 1\x1c\ny = 2\n",
    b"del x\nfor i, (j, k) in pairs:\n    pass\nwith open(p) as (fh, g):\n    pass\n",
    b"import os.path as p, sys\nfrom . import x as X\nfrom .m import *\n",
    b"class C(Base, metaclass=M):\n    items: list = []\n    a, b = 1, 2\n",
)


def run_python_list(raw_bytes: bytes, settings_info: Settings) -> list[tuple]:
    """Return the Python engine's rows (RT001 reduced to path and code).

    Args:
        raw_bytes (bytes): File contents.
        settings_info (Settings): Active settings.
    Returns:
        list[tuple]: Comparable rows.
    Warnings:
        None.
    """
    return reduce_rows_list([astuple(finding) for finding in
                             check_text_list("t.py", raw_bytes,
                                             settings_info)])


def reduce_rows_list(rows_list: list[tuple]) -> list[tuple]:
    """Reduce RT001 rows to path and code.

    Args:
        rows_list (list[tuple]): Finding rows.
    Returns:
        list[tuple]: Comparable rows.
    Warnings:
        None.
    """
    return [(row_tuple[0], row_tuple[3])
            if row_tuple[3] == "RT001" else tuple(row_tuple)
            for row_tuple in rows_list]


@unittest.skipIf(not hasattr(refactrail_core, "check_source"),
                 "native refactrail_core not installed")
class EngineParityTests(unittest.TestCase):
    """Both engines must give identical findings on every edge case."""

    def test_edge_sources_match_in_both_profiles(self) -> None:
        """Compare every edge source under several settings.

        Returns:
            None: Fails on the first differing case.
        """
        settings_list = [Settings(), Settings(profile="strict"),
                         replace(Settings(), ignore=("RT3",),
                                 line_length=40)]
        for raw_bytes in EDGE_SOURCES_TUPLE:
            for settings_info in settings_list:
                with self.subTest(source=raw_bytes[:40],
                                  profile=settings_info.profile):
                    rust_rows = refactrail_core.check_source(
                        "t.py", raw_bytes, settings_info)
                    if rust_rows is None:
                        # Newer syntax than the Rust grammar; the Python
                        # engine checks the file (see NewSyntaxTests).
                        continue
                    self.assertEqual(reduce_rows_list(rust_rows),
                                     run_python_list(raw_bytes,
                                                     settings_info))


NEW_SYNTAX_SOURCES_TUPLE = (
    b'name = "x"\nmessage = t"hello {name}"\n',
    b"try:\n    pass\nexcept ValueError, TypeError:\n    pass\n",
    b"def first[T = int](items: list[T]) -> T:\n    return items[0]\n",
    b"class Box[*Ts = *tuple[int], **P = [int]]:\n    pass\n",
    b"type Pair[T = str] = tuple[T, T]\n",
    b"def f():\n    try:\n        pass\n    finally:\n        return 1\n",
)


@unittest.skipIf(not hasattr(refactrail_core, "check_files"),
                 "native refactrail_core not installed")
class NewSyntaxTests(unittest.TestCase):
    """Syntax from Python 3.13 and 3.14 gives the same findings in both
    engines on whatever interpreter runs the tests."""

    def test_new_syntax_matches_python_engine(self) -> None:
        """Compare check results for each new-syntax source.

        Returns:
            None: Fails on the first differing source.
        """
        settings_info = Settings(profile="strict")
        with tempfile.TemporaryDirectory() as folder_str:
            for index_int, raw_bytes in enumerate(NEW_SYNTAX_SOURCES_TUPLE):
                path = Path(folder_str) / f"new_{index_int}.py"
                path.write_bytes(raw_bytes)
                with self.subTest(source=raw_bytes[:40]):
                    self.assertEqual(
                        check_paths_list([str(path)], settings_info, 1,
                                         None, "rust"),
                        check_paths_list([str(path)], settings_info, 1,
                                         None, "python"))


LINT_SOURCES_TUPLE = (
    b"if x == None:\n    pass\nd = {1: 'a', 1.0: 'b', True: 'c'}\n",
    b"assert (value, 'message')\nresult = value is 1\n",
    b"def f(items=[]):\n    return items\n",
    b"try:\n    run()\nexcept:\n    pass\n",
    b"def f(:\n    pass\n",
    b"x = 1\x00\n",
    b"\xef\xbb\xbf# coding: latin-1\nx = '\xe9'\n",
)

FORMAT_SOURCES_TUPLE = (
    ("t.py", "x=1\nif x :\n    y = [ 1,2 ]\n"),
    ("t.py", "values = call(first_argument, second_argument, third, fourth)"
             "\n"),
    ("t.py", "x = 1\r\ny = (2,\r\n     3)\r\n"),
    ("t.pyi", "def f(a:int)->int: ...\n"),
    ("t.ipynb", '{"cells": [{"cell_type": "code", "metadata": {}, '
                '"source": ["x=1\\n", "y = ( 2 )"], "outputs": [], '
                '"execution_count": null}], "metadata": {}, '
                '"nbformat": 4, "nbformat_minor": 5}\n'),
)


@unittest.skipIf(not hasattr(refactrail_core, "lint_files"),
                 "refactrail_core lint/format not installed")
class GeneralEngineParityTests(unittest.TestCase):
    """--engine rust must equal --engine python for lint and format."""

    def test_lint_snapshots_match(self) -> None:
        """Compare RC findings, including compile-failure fallbacks.

        Returns:
            None: Fails on the first differing source.
        """
        for raw_bytes in LINT_SOURCES_TUPLE:
            snapshot_tuple = ("t.py", raw_bytes, ("RC",), ())
            with self.subTest(source=raw_bytes[:40]):
                self.assertEqual(
                    run_rust_snapshots_list([snapshot_tuple], 1),
                    [check_snapshot_list(snapshot_tuple)])

    def test_format_documents_match(self) -> None:
        """Compare formatted documents in spacing, wrap and hug modes.

        Returns:
            None: Fails on the first differing document.
        """
        for path_str, text_str in FORMAT_SOURCES_TUPLE:
            for width_int, hug_bool in ((None, False), (40, False),
                                        (40, True)):
                with self.subTest(path=path_str, width=width_int,
                                  hug=hug_bool, source=text_str[:30]):
                    self.assertEqual(
                        format_engine_str(text_str, path_str, width_int,
                                          hug_bool, "rust"),
                        format_document_str(text_str, path_str, width_int,
                                            hug_bool))

    def test_format_refusal_uses_python_message(self) -> None:
        """A source the formatter refuses raises the same error.

        Returns:
            None: Fails when the messages differ.
        """
        for engine_str in ("python", "rust"):
            with self.subTest(engine=engine_str), \
                    self.assertRaisesRegex(SyntaxError, "t.py"):
                format_engine_str("def f(:\n", "t.py", None, False,
                                  engine_str)

if __name__ == "__main__":
    unittest.main()
