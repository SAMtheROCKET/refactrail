"""Public command behavior for independent linting and formatting."""

from contextlib import redirect_stderr, redirect_stdout
from io import StringIO
import json
from pathlib import Path
import tempfile
import unittest

from refactrail.cli import main
from refactrail.models import Finding
from refactrail.sarif import render_sarif_str


class GeneralCliTests(unittest.TestCase):
    def invoke(self, arguments):
        stdout, stderr = StringIO(), StringIO()
        with redirect_stdout(stdout), redirect_stderr(stderr):
            code = main(arguments)
        return code, stdout.getvalue(), stderr.getvalue()

    def test_no_arguments_show_the_quick_start(self):
        code, output, error = self.invoke([])
        self.assertEqual((code, error), (0, ""))
        for command in ("refactrail lint", "refactrail format --diff",
                        "refactrail check", "refactrail fix --diff"):
            self.assertIn(command, output)

    def test_lint_is_independent_of_strict_owner_naming(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "sample.py"
            path.write_text("a=1\nb=2\ntotal=a+b\n", encoding="utf-8")
            code, output, error = self.invoke(["lint", str(path), "--output-format", "json"])
            self.assertEqual((code, json.loads(output), error), (0, [], ""))

    def test_lint_json_and_sarif_report_real_findings(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "with spaces.py"
            path.write_text("def compute(default=[]):\n    return default\n", encoding="utf-8")
            code, output, error = self.invoke(["lint", str(path), "--output-format", "json"])
            self.assertEqual(code, 1)
            self.assertEqual(json.loads(output)[0]["code"], "RC101")
            code, output, error = self.invoke(["lint", str(path), "--output-format", "sarif"])
            document = json.loads(output)
            self.assertEqual(document["version"], "2.1.0")
            run = document["runs"][0]
            self.assertEqual(run["columnKind"], "unicodeCodePoints")
            self.assertEqual(run["results"][0]["ruleId"], "RC101")
            self.assertIn("with%20spaces.py", run["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"])

    def test_format_preview_check_and_write(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "sample.py"
            path.write_bytes(b"total=1+2\n")
            code, output, error = self.invoke(["format", str(path)])
            self.assertEqual(code, 1)
            self.assertIn("+total = 1 + 2", output)
            self.assertEqual(path.read_bytes(), b"total=1+2\n")
            code, output, error = self.invoke(["format", str(path), "--check", "--output-format", "json"])
            self.assertFalse(json.loads(output)["written"])
            self.assertEqual(code, 1)
            code, output, error = self.invoke(["format", str(path), "--write"])
            self.assertEqual(code, 0)
            self.assertEqual(path.read_bytes(), b"total = 1 + 2\n")
            self.assertEqual(self.invoke(["format", str(path), "--check"])[0], 0)

    def test_bad_input_never_reports_success_or_partially_writes_batch(self):
        with tempfile.TemporaryDirectory() as folder:
            good = Path(folder) / "a.py"
            bad = Path(folder) / "z.py"
            good.write_bytes(b"amount=1\n")
            bad.write_bytes(b"return 1\n")
            code, output, error = self.invoke(["format", folder, "--write"])
            self.assertEqual(code, 2)
            self.assertIn("z.py", error)
            self.assertEqual(good.read_bytes(), b"amount=1\n")

    def test_empty_and_notebook_targets_are_explicitly_refused(self):
        with tempfile.TemporaryDirectory() as folder:
            self.assertEqual(self.invoke(["lint", folder])[0], 2)
            notebook = Path(folder) / "sample.ipynb"
            notebook.write_text('{"cells":[]}', encoding="utf-8")
            self.assertEqual(self.invoke(["format", str(notebook)])[0], 2)

    def test_sarif_marks_compile_failure_as_incomplete_analysis(self):
        document = json.loads(render_sarif_str([
            Finding("bad.py", 1, 1, "RT001", "error", "Invalid source")]))
        self.assertFalse(document["runs"][0]["invocations"][0]["executionSuccessful"])

    def test_statistics_do_not_corrupt_structured_style_output(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "sample.py"
            path.write_bytes(b"amount = 1\n")
            code, output, error = self.invoke([
                "check", str(path), "--output-format", "json", "--statistics", "--no-cache"])
            self.assertIsInstance(json.loads(output), list)
