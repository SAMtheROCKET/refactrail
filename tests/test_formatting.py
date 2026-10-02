"""Independent formatter evidence: literals, directives and source writes."""

import ast
from dataclasses import replace
from pathlib import Path
import tempfile
import unittest

from refactrail.formatting import format_source_str, plan_format, write_format_none


class FormattingTests(unittest.TestCase):
    def test_assignments_binary_operators_and_commas(self):
        source = "amount=120\nrate =0.1\ntax=amount*rate\nvalues=[1,2, 3]\n"
        expected = "amount = 120\nrate = 0.1\ntax = amount * rate\nvalues = [1, 2, 3]\n"
        self.assertEqual(format_source_str(source), expected)

    def test_unary_call_defaults_and_exponents_preserve_syntax(self):
        source = ("def compute(argument=2):\n    return -argument**2+~argument\n"
                  "result=compute(argument=3)\n")
        output = format_source_str(source)
        self.assertIn("-argument ** 2 + ~argument", output)
        self.assertEqual(ast.dump(ast.parse(source)), ast.dump(ast.parse(output)))
        self.assertEqual(format_source_str(output), output)

    def test_directives_comments_and_disabled_regions_stay_exact(self):
        source = ("value=1 # type: ignore[assignment]\n"
                  "other=2 # ordinary comment\n# fmt: off\n"
                  "total=1+2\n# fmt: on\nnext_value=3\n"
                  "last=4 # fmt: skip\n")
        output = format_source_str(source)
        self.assertEqual(output, source.replace("next_value=3", "next_value = 3"))

    def test_literal_spelling_multiline_fstrings_and_unicode(self):
        source = ('text="a  =  b, c"\nmessage="""first  \n  second   \nend"""\n'
                  'rendered=f"{amount=}, {other + 1}"\n\u00e9=2+3\n')
        output = format_source_str(source)
        self.assertIn('text = "a  =  b, c"', output)
        self.assertIn('message="""first  \n  second   \nend"""', output)
        self.assertIn('rendered=f"{amount=}, {other + 1}"', output)
        self.assertIn('\u00e9 = 2 + 3', output)

    def test_literal_comment_markers_do_not_disable_code(self):
        self.assertEqual(format_source_str('text="# fmt: off"\namount=3\n'),
                         'text = "# fmt: off"\namount = 3\n')

    def test_explicit_continuations_and_line_ending_bytes(self):
        source = "total=1+\\\n    2\nnext_value=3\r\nlast=4\r"
        self.assertEqual(format_source_str(source),
                         "total=1+\\\n    2\nnext_value = 3\r\nlast = 4\r")

    def test_type_ignore_line_numbers_and_type_comments_remain(self):
        source = "value=1 # type: int\nnext_value=2 # type: ignore[assignment]\n"
        output = format_source_str(source)
        self.assertEqual(ast.dump(ast.parse(source, type_comments=True)),
                         ast.dump(ast.parse(output, type_comments=True)))
        self.assertEqual(source, output)

    def test_contextually_invalid_input_is_refused(self):
        for source in ("return 1\n", "continue\n", "broken = (\n"):
            with self.subTest(source=source), self.assertRaises((ValueError, SyntaxError)):
                format_source_str(source)

    def test_source_is_never_executed(self):
        with tempfile.TemporaryDirectory() as folder:
            marker = Path(folder) / "should-not-exist"
            source = f"from pathlib import Path\nPath({str(marker)!r}).touch()\n"
            format_source_str(source)
            self.assertFalse(marker.exists())

    def test_bom_preview_and_stale_writes(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "sample.py"
            original = b"\xef\xbb\xbfamount=1\r\n"
            path.write_bytes(original)
            plan = plan_format(str(path))
            self.assertEqual(path.read_bytes(), original)
            self.assertEqual(plan.output_bytes, b"\xef\xbb\xbfamount = 1\r\n")
            path.write_bytes(b"amount = 999\n")
            with self.assertRaises(ValueError):
                write_format_none(plan)
            self.assertEqual(path.read_bytes(), b"amount = 999\n")

    def test_write_preserves_bom_and_mixed_endings(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "sample.py"
            path.write_bytes(b"\xef\xbb\xbfamount=1\r\nrate=2\nlast=3\r")
            write_format_none(plan_format(str(path)))
            self.assertEqual(path.read_bytes(),
                             b"\xef\xbb\xbfamount = 1\r\nrate = 2\nlast = 3\r")

    def test_non_utf8_cookie_is_refused(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "sample.py"
            path.write_bytes(b"# coding: latin-1\namount=1\n")
            with self.assertRaises(ValueError):
                plan_format(str(path))

    def test_plan_fingerprint_and_replacement_failure_keep_source(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "sample.py"
            original = b"amount=1\n"
            path.write_bytes(original)
            plan = plan_format(str(path))
            with self.assertRaises(ValueError):
                write_format_none(replace(plan, original_bytes=b"amount=2\n"))
            with patch("refactrail.fixing.os.replace", side_effect=OSError), self.assertRaises(OSError):
                write_format_none(plan)
            self.assertEqual(path.read_bytes(), original)
            self.assertEqual(list(Path(folder).glob(".refactrail-*")), [])

    def test_unicode_separators_and_cr_comments_remain_literal(self):
        source = 'text="first\u2028second"\rvalue=1 # type: ignore\rnext_value=2\r'
        output = format_source_str(source)
        self.assertEqual(output, 'text = "first\u2028second"\rvalue=1 # type: ignore\rnext_value = 2\r')

    def test_comparison_chains_walrus_augassign_and_annotations(self):
        source = ('amount:int=1\namount+=2\nflag=0<amount<=9\n'
                  'if (extra:=amount+1):\n    amount=extra\n')
        output = format_source_str(source)
        self.assertIn('amount:int = 1', output)
        self.assertIn('amount += 2', output)
        self.assertIn('0 < amount <= 9', output)
        self.assertIn('extra := amount + 1', output)
        self.assertEqual(format_source_str(output), output)

    def test_linked_file_is_refused(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "source.py"
            path.write_bytes(b"amount=1\n")
            alias = Path(folder) / "alias.py"
            try:
                alias.symlink_to(path)
            except OSError as error:
                self.skipTest(f"Symbolic links unavailable: {error}")
            with self.assertRaises(ValueError):
                plan_format(str(alias))
