"""Regressions reproduced from the cross-computer user transcript."""

import ast
from contextlib import redirect_stderr, redirect_stdout
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from refactrail import prepare_refactoring_dict
from refactrail.cli import main
from refactrail.fixes import apply_safe_fixes
from refactrail.models import Settings


class FeedbackTests(unittest.TestCase):
    def test_cp932_diff_does_not_crash_or_write_source(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'sample.py'
            source = 'def show_value():\n    """Show \u2014 value."""\n    print(1)\n'
            path.write_text(source, encoding='utf-8')
            env = {**os.environ, 'PYTHONIOENCODING': 'cp932:strict',
                   'PYTHONUTF8': '0'}
            result = subprocess.run([sys.executable, '-m', 'refactrail',
                'fix', str(path), '--diff'], env=env, capture_output=True)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn(b'\\u2014', result.stdout)
            self.assertEqual(path.read_text('utf-8'), source)

    def test_fix_completes_existing_docs_without_fabricating_types(self):
        source = 'def add_metrics(df):\n    """Compute revenue metrics."""\n    return df.copy()\n'
        result = apply_safe_fixes(source, 'sample.py', Settings())
        function = ast.parse(result.text).body[0]
        self.assertTrue(ast.get_docstring(function).startswith(
            'Compute revenue metrics.'))
        for section in ('Args:', 'Returns:', 'Warnings:'):
            self.assertIn(section, ast.get_docstring(function))
        self.assertIsNone(function.args.args[0].annotation)
        self.assertIsNone(function.returns)
        self.assertEqual(apply_safe_fixes(result.text, 'sample.py',
                                         Settings()).text, result.text)

    def test_rT302_selection_completes_only_existing_documentation(self):
        source = ('def read_value():\n    """Read value."""\n    return 1\n'
                  'def read_other():\n    return 2\n')
        result = apply_safe_fixes(source, 'sample.py',
                                 Settings(select=('RT302',)))
        functions = ast.parse(result.text).body
        self.assertIn('Returns:', ast.get_docstring(functions[0]))
        self.assertIsNone(ast.get_docstring(functions[1]))

    def test_declined_fixes_are_visible_even_when_nothing_changes(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'sample.py'
            path.write_text('import os\nos.chdir(".")\nimport sys\n',
                            encoding='utf-8')
            output = io.StringIO()
            with redirect_stdout(output), redirect_stderr(output):
                main(['fix', str(path), '--select', 'RT504', '--diff'])
            self.assertIn('note:', output.getvalue())
            self.assertIn('kept at module level', output.getvalue())

    def test_project_stage_is_read_only_unless_explicitly_requested(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            path = root / 'sample.py'
            original = b'def calculate_value(df):\n    """Keep meaning."""\n    m=df.copy()\n    return m\n'
            path.write_bytes(original)
            preview = prepare_refactoring_dict(root)
            self.assertEqual(path.read_bytes(), original)
            self.assertFalse(preview['write_requested'])
            self.assertEqual(preview['status'], 'findings')
            self.assertTrue(preview['context_questions'])
            self.assertTrue(all(item['answer'] == '' and not item['applied']
                                for item in preview['context_questions']))
            applied = prepare_refactoring_dict(root, write_bool=True)
            self.assertNotEqual(path.read_bytes(), original)
            self.assertIn('    m = df.copy()', path.read_text('utf-8'))
            self.assertFalse(applied['behavior_verified'])
            self.assertEqual(prepare_refactoring_dict(root)['changed_files'], 0)

    def test_invalid_source_is_incomplete_and_stays_untouched(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            path = root / 'invalid.py'
            path.write_bytes(b'def broken(\n')
            result = prepare_refactoring_dict(root, write_bool=True)
            self.assertEqual(result['status'], 'incomplete')
            self.assertEqual(path.read_bytes(), b'def broken(\n')
