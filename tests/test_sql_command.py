"""Keep RefacTrail's SQL entry point aligned with FuncLoom's public API."""

from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import tempfile
import unittest

from refactrail.cli import main


class SqlCommandTests(unittest.TestCase):
    def test_preview_then_new_copy(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root/'report.py'
            source.write_text('query_str = "SELECT ?"\n')
            stream = io.StringIO()
            with redirect_stdout(stream):
                status = main(['sql', str(source), '--format', 'json'])
            self.assertEqual(status, 0)
            report = json.loads(stream.getvalue())
            self.assertEqual(report['schema_version'], 'sql-assets-1')
            self.assertEqual(report['status'], 'planned')
            self.assertFalse(report['behavior_verified'])
            self.assertEqual(len(list(root.iterdir())), 1)
            with redirect_stdout(io.StringIO()):
                self.assertEqual(main(['sql', str(source), '--output',
                                       str(root/'out')]), 0)
            self.assertTrue((root/'out/src/report.py').is_file())
            self.assertEqual(source.read_text(), 'query_str = "SELECT ?"\n')

    def test_invalid_python_refuses_output(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root/'bad.py'
            source.write_text('query_str = "SELECT 1"\nreturn 3\n')
            with redirect_stdout(io.StringIO()):
                self.assertEqual(main(['sql', str(source), '--output',
                                       str(root/'out')]), 1)
            self.assertFalse((root/'out').exists())
