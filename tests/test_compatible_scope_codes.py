"""Pyflakes-compatible scope codes (F401-F842), checked against Ruff.

Expected (line, column, code) triples are the ones Ruff 0.16.9 reports
for the same snippets (see scripts/ruff_oracle.py).
"""

import textwrap
import unittest

from refactrail.correctness import check_correctness_list


def findings_of(source_str: str, select_str: str = "F",
                path_str: str = "case.py") -> list:
    """(line, column, code) of the findings for a source text."""
    text_str = textwrap.dedent(source_str).lstrip("\n")
    return [(finding.line, finding.column, finding.code)
            for finding in check_correctness_list(
                path_str, text_str.encode(), tuple(select_str.split(",")))]


class UnusedAndUndefinedTests(unittest.TestCase):
    def test_imports_and_undefined_names(self):
        self.assertEqual(findings_of("""
            import os
            import sys, json
            from typing import List
            from pathlib import Path as Path
            import xml.dom as dom
            print(undefined_name, json)
            def f():
                return later_name
            later_name = 1
            print(before_definition)
            before_definition = 2
        """), [(1, 8, "F401"), (2, 8, "F401"), (3, 20, "F401"),
               (5, 19, "F401"), (6, 7, "F821"), (10, 7, "F821")])

    def test_unused_locals_and_annotations(self):
        self.assertEqual(findings_of("""
            def f(argument):
                unused = 1
                annotated: int
                first, second = pair
                a, b = 1, 2
                _ignored = 3
                total = 0
                for item in argument:
                    total = total + item
                try:
                    pass
                except ValueError as error:
                    pass
                return b
        """), [(2, 5, "F841"), (3, 5, "F842"), (4, 21, "F821"),
               (5, 5, "F841"), (12, 26, "F841")])

    def test_star_imports_and_exports(self):
        self.assertEqual(findings_of("""
            from module import *
            print(maybe)
            __all__ = ["maybe", "len"]
            def f():
                from other import *
        """), [(1, 1, "F403"), (2, 7, "F405"), (3, 12, "F405"),
               (5, 5, "F403"), (5, 5, "F406")])
        self.assertEqual(findings_of("""
            import json
            __all__ = ["json", "missing"]
        """), [(2, 20, "F822")])
        self.assertEqual(findings_of("""
            __all__ = ["submodule"]
        """, path_str="pkg/__init__.py"), [(1, 12, "F822")])


class RedefinitionTests(unittest.TestCase):
    def test_redefinitions_and_shadowing(self):
        self.assertEqual(findings_of("""
            import os
            def os(): pass
            x = 1
            def x(): pass
            import sys
            def k(sys): pass
            if os:
                def g(): pass
            else:
                def g(): pass
            from typing import overload
            @overload
            def h(a: int) -> int: ...
            def h(a): return a
        """, "F811"), [(2, 5, "F811"), (4, 5, "F811"), (6, 7, "F811")])

    def test_loop_variable_shadowing_import(self):
        self.assertEqual(findings_of("""
            import name
            for name in []:
                pass
            values = [name for name in []]
        """, "F402"), [(2, 5, "F402")])

    def test_local_referenced_before_assignment(self):
        self.assertEqual(findings_of("""
            value = 1
            def f():
                print(value)
                value = 2
                return value
        """, "F823"), [(3, 11, "F823")])


class FlowTests(unittest.TestCase):
    def test_guards_globals_and_classes(self):
        self.assertEqual(findings_of("""
            try:
                maybe_defined
            except NameError:
                maybe_defined = None
            def setup():
                global configured
                configured = True
            def use():
                return configured
            class C:
                attribute = 1
                def method(self):
                    return attribute
                values = [attribute for _ in range(3)]
        """, "F821"), [(13, 16, "F821"), (14, 15, "F821")])

    def test_typing_only_and_string_annotations(self):
        self.assertEqual(findings_of("""
            from typing import TYPE_CHECKING, Literal, cast
            if TYPE_CHECKING:
                from pathlib import Path
            def f(value: "Path", mode: Literal["r", "w"]) -> "Missing":
                return cast("Path", value)
        """), [(4, 51, "F821")])

    def test_multi_line_import_noqa_on_first_line(self):
        self.assertEqual(findings_of("""
            from os import (  # noqa: F401
                path,
                sep,
            )
        """), [])


if __name__ == "__main__":
    unittest.main()
