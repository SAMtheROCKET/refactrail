"""Safe fixes for Ruff-compatible findings (lint --fix and --diff).

Expected texts were checked against Ruff 0.16.9's own --fix output where
Ruff fixes the same finding; deliberate differences are noted.
"""

import contextlib
import io
from pathlib import Path
import tempfile
import textwrap
import unittest

from refactrail.cli import main
from refactrail.compat_fixes import fix_compat_text

SELECT_TUPLE = ("E4", "E7", "F")


def fix_str(text_str: str, path_str: str = "module.py") -> str:
    """Fix dedented text and return the result."""
    return fix_compat_text(textwrap.dedent(text_str), path_str,
                           SELECT_TUPLE).text


class SimpleFixTests(unittest.TestCase):
    def test_semicolon_fstring_and_literal_comparisons(self):
        self.assertEqual(fix_str("""\
            x = 1;
            y = f"plain {{x}}" + F"abc" + rf"\\d"
            if x is "a" or x is not 3:
                pass
            """), textwrap.dedent("""\
            x = 1
            y = "plain {x}" + "abc" + r"\\d"
            if x == "a" or x != 3:
                pass
            """))

    def test_semicolon_keeps_the_spaces_before_it(self):
        self.assertEqual(fix_str("x = 1 ;\n"), "x = 1 \n")

    def test_negated_membership_and_identity(self):
        self.assertEqual(fix_str("""\
            x = 1
            if not x in [1] and not x is None:
                pass
            if not (x in [2]):
                pass
            if (x == 1) and not (
                x is None
            ):
                pass
            if not (  # keep
                x in [3]
            ):
                pass
            """), textwrap.dedent("""\
            x = 1
            if x not in [1] and x is not None:
                pass
            if x not in [2]:
                pass
            if (x == 1) and x is not None:
                pass
            if (  # keep
                x not in [3]
            ):
                pass
            """))

    def test_overlapping_fixes_finish_in_a_later_pass(self):
        self.assertEqual(fix_str('x = 1\nif not x is "":\n    pass\n'),
                         'x = 1\nif x != "":\n    pass\n')

    def test_fields_and_noqa_are_left_alone(self):
        text_str = ('x = 1\ny = f"{x}"\nimport os  # noqa: F401\n'
                    'z = f"a"  # noqa: F541\n')
        self.assertEqual(fix_str(text_str), text_str)


class ImportFixTests(unittest.TestCase):
    def test_whole_statements_and_names(self):
        self.assertEqual(fix_str("""\
            import os, sys
            import json
            import math as m
            from io import (SEEK_SET, SEEK_CUR, SEEK_END)
            from typing import (
                List,
                Dict,
            )
            from m import (A, B,
                           C, D,
                E)
            print(sys, SEEK_SET, Dict, C)
            """), textwrap.dedent("""\
            import sys
            from io import (SEEK_SET)
            from typing import (
                Dict,
            )
            from m import (C)
            print(sys, SEEK_SET, Dict, C)
            """))

    def test_emptied_block_gets_pass_with_the_last_comment(self):
        self.assertEqual(fix_str("""\
            import typing
            if typing.TYPE_CHECKING:
                from a import b
                from c import d  # type: ignore
            def f():
                import re
            """), textwrap.dedent("""\
            import typing
            if typing.TYPE_CHECKING:
                pass  # type: ignore
            def f():
                pass
            """))

    def test_guarded_shared_commented_init_and_stub_imports_stay(self):
        guarded_str = ("try:\n    import yaml\nexcept ImportError:\n"
                       "    pass\n")
        self.assertEqual(fix_str(guarded_str), guarded_str)
        shared_str = "import os; import io\nprint(io)\n"
        self.assertEqual(fix_str(shared_str), shared_str)
        comment_str = "from n import (\n    F,  # keep\n    G,\n)\nG\n"
        self.assertEqual(fix_str(comment_str), comment_str)
        self.assertEqual(fix_str("import os\n", "pkg/__init__.py"),
                         "import os\n")
        self.assertEqual(fix_str("import os\n", "stub.pyi"), "import os\n")

    def test_notes_name_what_was_left(self):
        outcome = fix_compat_text("import os; import io\nprint(io)\n",
                                  "module.py", SELECT_TUPLE)
        self.assertEqual(len(outcome.notes), 1)
        self.assertIn("F401 not fixed", outcome.notes[0])


class CommandLineFixTests(unittest.TestCase):
    def run_cli(self, *arguments: str) -> tuple[int, str]:
        output = io.StringIO()
        with contextlib.redirect_stdout(output), \
                contextlib.redirect_stderr(output):
            code = main(list(arguments))
        return code, output.getvalue()

    def test_diff_changes_nothing_and_fix_writes(self):
        folder = Path(tempfile.mkdtemp())
        path = folder / "sample.py"
        original_bytes = b"import os\r\nx = 1;\r\nprint(x)\r\n"
        path.write_bytes(original_bytes)
        code, output = self.run_cli("lint", str(path), "--select", "E7,F",
                                    "--no-cache", "--diff")
        self.assertEqual(code, 1)
        self.assertIn("-import os", output)
        self.assertEqual(path.read_bytes(), original_bytes)
        code, output = self.run_cli("lint", str(path), "--select", "E7,F",
                                    "--no-cache", "--fix")
        self.assertEqual(code, 0, output)
        self.assertEqual(path.read_bytes(), b"x = 1\r\nprint(x)\r\n")
        self.assertIn("fixed 1:8 F401", output)

    def test_only_selected_codes_are_fixed(self):
        folder = Path(tempfile.mkdtemp())
        path = folder / "sample.py"
        path.write_text("import os\nx = 1;\n", encoding="utf-8")
        code, _ = self.run_cli("lint", str(path), "--select", "E7",
                               "--no-cache", "--fix")
        self.assertEqual(code, 0)
        self.assertEqual(path.read_text(encoding="utf-8"),
                         "import os\nx = 1\n")


if __name__ == "__main__":
    unittest.main()
