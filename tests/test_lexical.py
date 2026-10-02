"""Compiler scope evidence, independent of runtime execution."""

from pathlib import Path
import tempfile
import unittest

from refactrail.correctness import check_correctness_list
from refactrail.lexical import analyze_lexical_dict


class LexicalTests(unittest.TestCase):
    def findings(self, source, path="example.py"):
        return check_correctness_list(path, source.encode(), ("RC2",))

    def test_missing_global_has_exact_unicode_location(self):
        rows = self.findings("é = 1\nanswer = é + absent\n")
        self.assertEqual([(r.code, r.line, r.column) for r in rows],
                         [("RC201", 2, 14)])

    def test_closure_global_nonlocal_and_later_bindings(self):
        source = ("def outer(argument):\n    saved = argument\n"
                  "    def inner():\n        nonlocal saved\n"
                  "        saved += global_value\n        return saved\n"
                  "    return inner\nglobal_value = 1\n")
        self.assertEqual(self.findings(source), [])

    def test_comprehension_first_iterable_and_walrus(self):
        self.assertEqual(self.findings(
            "values = [1, 2]\nfound = [last := n for n in values if n]\n"
            "print(last)\n"), [])
        rows = self.findings("values = [n for n in missing]\nprint(n)\n")
        self.assertEqual([r.code for r in rows], ["RC201", "RC201"])

    def test_defaults_and_decorators_use_enclosing_scope(self):
        rows = self.findings("@decorate\ndef compute(value=missing):\n"
                             "    return value\n")
        self.assertEqual([r.line for r in rows], [1, 2])

    def test_class_namespace_is_not_a_method_closure(self):
        rows = self.findings("class Shape:\n    size = 1\n"
                             "    def compute(self):\n        return size\n")
        self.assertEqual([(r.line, r.code) for r in rows], [(4, "RC201")])
        self.assertEqual(self.findings("class Shape:\n    __size = 1\n"
            "    alias = __size\n    def compute(self):\n"
            "        return __class__.__name__\n"), [])

    def test_import_usage_closures_reexports_and_strings(self):
        self.assertEqual(self.findings("import math\ndef outer():\n"
            "    def inner():\n        return math.pi\n    return inner\n"), [])
        self.assertEqual(self.findings("from library import thing as thing\n"
            "import typing\n__all__ = ['typing']\n"), [])
        self.assertEqual(self.findings("import typing\ndef compute() -> 'typing.Any':\n"
                                       "    pass\n"), [])
        rows = self.findings("import math\nfrom os import path\n")
        self.assertEqual([r.code for r in rows], ["RC202", "RC202"])
        self.assertEqual(self.findings("import math\n", "__init__.py"), [])
        self.assertEqual(self.findings("import math\n", "types.pyi"), [])

    def test_future_annotations_are_not_missing_runtime_globals(self):
        self.assertEqual(self.findings("from __future__ import annotations\n"
            "def compute(argument: Missing) -> Other:\n    return argument\n"), [])

    def test_dynamic_namespaces_are_explicitly_incomplete(self):
        for source in ("from library import *\nprint(dynamic)\n",
                       "exec('dynamic = 1')\nprint(dynamic)\n",
                       "namespace = globals()\nprint(dynamic)\n",
                       "type Alias[T] = list[T]\n"):
            with self.subTest(source=source):
                report = analyze_lexical_dict(source, "example.py")
                self.assertEqual(report["coverage"], "partial")
                self.assertTrue(report["limitations"])
                self.assertEqual(self.findings(source), [])

    def test_names_only_stored_are_not_dynamic_namespace_access(self):
        source = ("from dataclasses import dataclass\nimport math\n"
                  "@dataclass\nclass Plan:\n    locals: set\n    vars: int = 0\n")
        report = analyze_lexical_dict(source, "example.py")
        self.assertFalse(any("Dynamic" in limit
                             for limit in report["limitations"]))
        self.assertEqual([r.code for r in self.findings(source)], ["RC202"])

    def test_noqa_and_selection(self):
        self.assertEqual(self.findings("print(missing) # noqa: RC201\n"), [])
        self.assertEqual(self.findings("import math # noqa: RC202\n"), [])

    def test_binding_existence_does_not_claim_initialization(self):
        source = "if condition:\n    maybe = 1\nprint(maybe)\n"
        rows = self.findings(source)
        self.assertEqual(len(rows), 1)
        self.assertIn("condition", rows[0].message)
        report = analyze_lexical_dict(source, "example.py")
        self.assertFalse(report["initialization_verified"])

    def test_target_code_never_runs(self):
        with tempfile.TemporaryDirectory() as folder:
            marker = Path(folder) / "executed"
            self.findings(f"from pathlib import Path\nPath({str(marker)!r}).touch()\n")
            self.assertFalse(marker.exists())


    def test_type_comments_are_import_usage_evidence(self):
        self.assertEqual(self.findings('import typing\nvalues = [] # type: typing.List[int]\n'), [])
        self.assertEqual(self.findings('import typing\rvalues = [] # type: typing.List[int]\r'), [])

    def test_generator_scopes_private_parameters_and_nested_comprehensions(self):
        sources = [
            'def compute(values):\n    return (number for number in values)\n',
            'class Shape:\n    def compute(self, __amount):\n        return __amount\n',
            'values = [1]\nresult = [[inner + outer for inner in values] for outer in values]\n',
        ]
        for source in sources:
            self.assertEqual(self.findings(source), [])
            self.assertTrue(analyze_lexical_dict(source)['diagnostics_supported'])

    def test_deeply_nested_valid_expression_is_analyzed(self):
        # Valid code nesting deeper than the default recursion limit once
        # crashed the whole lint run with RecursionError.
        source = "total = " + "1 + " * 1200 + "missing\n"
        rows = self.findings(source)
        self.assertEqual([(r.code, r.line) for r in rows], [("RC201", 1)])
        self.assertTrue(analyze_lexical_dict(source)["diagnostics_supported"])
