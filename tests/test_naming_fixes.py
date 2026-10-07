"""Tests for the RT102/RT201 renames and literal return types."""

from pathlib import Path
import tempfile
import textwrap
import unittest

from refactrail.fixes import add_none_returns_tuple
from refactrail.naming_fixes import (
    apply_naming_fixes_tuple, find_singular_str, suggest_loop_name_str)
import ast

SCRIPT = textwrap.dedent('''
    import csv

    threshold = 10

    def load(path):
        rows = []
        with open(path) as f:
            for r in csv.DictReader(f):
                rows.append(r)
        return rows

    def count(rows):
        totals = {}
        for r in rows:
            totals[r["k"]] = totals.get(r["k"], 0) + 1
        return totals

    for i in range(threshold):
        print(i)
''')


def fix_script(text_str: str, folder: Path | None = None):
    folder = folder or Path(tempfile.mkdtemp())
    path = folder / "job.py"
    path.write_text(text_str, encoding="utf-8")
    return apply_naming_fixes_tuple(text_str, str(path), True, True)


class SingularTests(unittest.TestCase):
    def test_plural_names_and_known_calls(self):
        self.assertEqual(find_singular_str("rows"), "row")
        self.assertEqual(find_singular_str("entries"), "entry")
        self.assertEqual(find_singular_str("boxes"), "box")
        self.assertEqual(find_singular_str("raw_data"), "raw_record")
        self.assertEqual(find_singular_str("status"), "")
        call = ast.parse("range(3)", mode="eval").body
        self.assertEqual(suggest_loop_name_str(call), "index")


class RenameTests(unittest.TestCase):
    def test_script_names_become_meaningful(self):
        new_str, applied, notes = fix_script(SCRIPT)
        self.assertEqual(notes, [])
        self.assertIn("THRESHOLD = 10", new_str)
        self.assertIn("for index in range(THRESHOLD):", new_str)
        self.assertIn("with open(path) as file_handle:", new_str)
        self.assertIn("for row in csv.DictReader(file_handle):", new_str)
        self.assertIn('totals[row["k"]]', new_str)
        self.assertIn('"k"', new_str)  # strings are never touched
        self.assertEqual(len(applied), 5)
        compile(new_str, "job.py", "exec")

    def test_collisions_closures_and_dynamic_code_are_refused(self):
        source_str = textwrap.dedent('''
            def outer(rows):
                row = 1
                for r in rows:
                    print(r, row)

            def closure(items):
                for x in items:
                    def show():
                        return x
                    show()
        ''')
        new_str, applied, notes = fix_script(source_str)
        self.assertEqual(new_str, source_str)
        self.assertEqual(applied, [])
        self.assertEqual(len(notes), 2)
        dynamic_str = "for x in items:\n    print(eval('x'))\n"
        self.assertEqual(fix_script(dynamic_str)[0], dynamic_str)

    def test_constants_of_packages_and_shared_names_are_kept(self):
        folder = Path(tempfile.mkdtemp())
        (folder / "__init__.py").write_text("", encoding="utf-8")
        new_str, _, notes = fix_script("limit = 3\nprint(limit)\n", folder)
        self.assertIn("limit = 3", new_str)
        self.assertIn("package", notes[0])
        other = Path(tempfile.mkdtemp())
        (other / "user.py").write_text("from job import limit\n",
                                       encoding="utf-8")
        new_str, _, notes = fix_script("limit = 3\nprint(limit)\n", other)
        self.assertIn("limit = 3", new_str)
        self.assertIn("mentions", notes[0])


class ReturnTypeTests(unittest.TestCase):
    def test_literal_returns_are_annotated(self):
        new_str, names_list = add_none_returns_tuple(SCRIPT)
        self.assertIn("def load(path) -> list:", new_str)
        self.assertIn("def count(rows) -> dict:", new_str)
        self.assertEqual(sorted(names_list),
                         ["count -> dict", "load -> list"])

    def test_uncertain_returns_are_left_alone(self):
        source_str = textwrap.dedent('''
            def pick(flag):
                if flag:
                    return []
                return {}

            def grow(items):
                items = list(items)
                return items
        ''')
        self.assertEqual(add_none_returns_tuple(source_str),
                         (source_str, []))


if __name__ == "__main__":
    unittest.main()
