"""pycodestyle- and Pyflakes-compatible codes (E4, E7, F syntax rules).

Each case was checked against Ruff 0.16.9 as an external oracle
(scripts/ruff_oracle.py); the expected (line, column, code) triples here
are the ones Ruff reports for the same source.
"""

import textwrap
import unittest

from refactrail.correctness import check_correctness_list
from refactrail.correctness_batch import is_native_selection_bool


def findings_of(source_str: str, select_str: str = "E4,E7,F") -> list:
    """(line, column, code) of the findings for a source text."""
    text_str = textwrap.dedent(source_str).lstrip("\n")
    select_tuple = tuple(select_str.split(","))
    return [(finding.line, finding.column, finding.code)
            for finding in check_correctness_list(
                "case.py", text_str.encode(), select_tuple)]


class PycodestyleCodesTests(unittest.TestCase):
    def test_statement_layout_codes(self):
        self.assertEqual(findings_of("""
            '''Doc.'''
            import os, sys
            x = 1
            import json
            if x: y = 1; z = 2;
            class Stub: ...
            class Real: pass
            def f(): return 1
            async def g():
                async with x: pass
        """, "E4,E7"), [(2, 1, "E401"), (4, 1, "E402"), (5, 5, "E701"),
               (5, 12, "E702"), (5, 19, "E703"), (7, 11, "E701"),
               (10, 17, "E701")])

    def test_import_section_allowances(self):
        self.assertEqual(findings_of("""
            '''Doc.'''
            __all__ = ["x"]
            import sys
            sys.path.insert(0, "lib")
            import os
            os.environ["A"] = "1"
            if sys:
                pass
            try:
                import json
            except ImportError:
                json = None
            import re
            print(re)
            import io
        """, "E402"), [(15, 1, "E402")])

    def test_comparison_codes(self):
        self.assertEqual(findings_of("""
            if x == None or None != x:
                pass
            if x == True or 0 == False:
                pass
            if a is None == b is None:
                pass
            if not x in y or not x is y:
                pass
            if type(x) == int or x.dtype == float or exc == ValueError:
                pass
        """, "E7"), [(1, 9, "E711"), (1, 17, "E711"), (3, 4, "E712"),
                     (7, 8, "E713"), (7, 22, "E714"), (9, 4, "E721"),
                     (9, 42, "E721")])

    def test_either_side_being_a_builtin_type_is_enough(self):
        self.assertEqual(findings_of("""
            def f(type):
                return type(x) == int
            def g(type):
                return type(x) == y
        """, "E721"), [(2, 12, "E721")])

    def test_bare_except_lambda_and_names(self):
        self.assertEqual(findings_of("""
            try:
                pass
            except:
                pass
            try:
                pass
            except:
                raise
            f = lambda: 0
            l = 1
            def I(O): pass
            class O: pass
            for l in []: pass
        """, "E722,E731,E74"), [(3, 1, "E722"), (9, 1, "E731"),
                                (10, 1, "E741"), (11, 5, "E743"),
                                (11, 7, "E741"), (12, 7, "E742"),
                                (13, 5, "E741")])


class PyflakesCodesTests(unittest.TestCase):
    def test_percent_format_codes(self):
        self.assertEqual(findings_of("""
            a = "%s %s" % (1,)
            b = "%(a)s" % (1, 2)
            c = "%(a)s %(b)s" % {"a": 1, "c": 2}
            d = "%s %(a)s" % (1,)
            e = "%y" % 3
            f = "%" % 3
            g = "x" % exc
            h = "%s %s" % exc
            i = "%*d" % {"a": 1}
        """, "F50"), [(1, 5, "F507"), (2, 5, "F502"), (3, 5, "F504"),
                      (3, 5, "F505"), (4, 5, "F502"), (4, 5, "F506"),
                      (5, 5, "F509"), (6, 5, "F501"), (7, 5, "F507"),
                      (9, 5, "F503"), (9, 5, "F507"), (9, 5, "F508")])

    def test_str_format_codes(self):
        self.assertEqual(findings_of("""
            a = "{} {}".format(1)
            b = "{0} {}".format(1, 2)
            c = "{a}".format(a=1, b=2)
            d = "{".format()
            e = "{}".format(*args)
        """, "F52"), [(1, 5, "F524"), (2, 5, "F523"), (2, 5, "F525"),
                      (3, 5, "F522"), (4, 5, "F521")])

    def test_literal_and_shape_codes(self):
        self.assertEqual(findings_of("""
            a = f"plain" f"text"
            b = f"{x:>{w}}"
            c = {"k": 1, "k": 2, x: 1, x: 3}
            if (x, 1):
                pass
            assert (x, "message")
            if x is "s" or x is [] or x is -1:
                pass
            def f() -> "List[":
                raise NotImplemented
        """, "F541,F6,F722,F901"), [
            (1, 5, "F541"), (1, 14, "F541"), (3, 14, "F601"),
            (3, 28, "F602"), (4, 4, "F634"), (6, 1, "F631"),
            (7, 4, "F632"), (7, 16, "F632"), (9, 12, "F722"),
            (10, 11, "F901")])

    def test_compiler_rejected_statements_are_findings(self):
        self.assertEqual(findings_of("""
            break
            def f():
                for x in y:
                    pass
                else:
                    continue
            return 1
            class C:
                yield 1
            try:
                pass
            except:
                pass
            except ValueError:
                pass
        """, "F7"), [(1, 1, "F701"), (6, 9, "F702"), (7, 1, "F706"),
                     (9, 5, "F704"), (12, 1, "F707")])

    def test_suppressed_compile_errors_still_fail(self):
        self.assertEqual(findings_of("return 1  # noqa\n", "F706"),
                         [(1, 1, "RT001")])
        self.assertEqual(findings_of("return 1\n", "RC"), [(1, 1, "RT001")])
        self.assertEqual(findings_of("if x: y = 1\n", "E701"),
                         [(1, 5, "E701")])

    def test_other_syntax_errors_stay_rt001(self):
        codes_list = [code for _, _, code in findings_of("def f(:\n", "F")]
        self.assertEqual(codes_list, ["RT001"])


class SuppressionAndEngineTests(unittest.TestCase):
    def test_file_level_exemptions(self):
        self.assertEqual(findings_of("""
            # flake8: noqa
            import os, sys
        """, "E4"), [])
        self.assertEqual(findings_of("""
            # ruff: noqa: E401
            import os, sys
            x = 1
            import json
        """, "E4"), [(4, 1, "E402")])

    def test_rust_engine_only_for_rc_selections(self):
        self.assertTrue(is_native_selection_bool(("RC",)))
        self.assertTrue(is_native_selection_bool(("RC1", "RC201")))
        self.assertFalse(is_native_selection_bool(("RC", "F")))
        self.assertFalse(is_native_selection_bool(("E4",)))


if __name__ == "__main__":
    unittest.main()
