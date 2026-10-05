"""Source evidence, cache invalidation and bounded rename regressions."""

from contextlib import redirect_stdout, redirect_stderr
from io import StringIO
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from refactrail.cli import main
from refactrail.correctness_batch import check_correctness_paths_list
from refactrail.project_index import analyze_project_dict
from refactrail.rename import plan_rename_dict


class ProjectTests(unittest.TestCase):
    def create_project(self, root):
        files = {'pkg/__init__.py': 'from .worker import calculate\n',
                 'pkg/worker.py': 'from .constants import BASE\ndef calculate():\n    return BASE\n',
                 'pkg/constants.py': 'BASE = 120\n',
                 'main.py': 'from pkg import calculate\nprint(calculate())\n',
                 'unknown.py': 'from thirdparty import external\n'}
        for name, source in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(source)
        return files

    def test_relative_imports_reexports_and_transitive_impact(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            original = self.create_project(root)
            report = analyze_project_dict(folder, ['pkg/constants.py'])
            self.assertEqual(report['impact'], ['main.py', 'pkg/__init__.py', 'pkg/constants.py', 'pkg/worker.py'])
            self.assertTrue(any(edge['resolution'] == 'unresolved' for edge in report['edges']))
            worker = next(f for f in report['files'] if f['path'] == 'pkg/worker.py')
            self.assertEqual(worker['symbols'][0]['name'], 'calculate')
            self.assertFalse(report['behavior_verified'])
            self.assertEqual(report, analyze_project_dict(folder, ['pkg/constants.py']))
            for name, source in original.items(): self.assertEqual((root / name).read_text(), source)

    def test_cycles_stubs_and_changed_content(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'a.py').write_text('import b\n')
            (root / 'b.py').write_text('import a\n')
            (root / 'b.pyi').write_text('def operate() -> None: ...\n')
            before = analyze_project_dict(folder, ['b.py'])
            self.assertEqual(before['impact'], ['a.py', 'b.py'])
            self.assertIn('b.pyi', before['edges'][0]['candidates'])
            (root / 'a.py').write_text('import other\n')
            after = analyze_project_dict(folder, ['b.py'])
            self.assertEqual(after['impact'], ['b.py'])
            self.assertNotEqual(before['files'][0]['source_sha256'], after['files'][0]['source_sha256'])

    def test_deleted_module_and_outside_root(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'main.py').write_text('import deleted\n')
            report = analyze_project_dict(folder, ['deleted.py'])
            self.assertEqual(report['impact'], ['deleted.py', 'main.py'])
            with self.assertRaises(ValueError): analyze_project_dict(folder, ['../outside.py'])

    def test_invalid_source_and_empty_project_are_refused(self):
        with tempfile.TemporaryDirectory() as folder:
            with self.assertRaises(ValueError): analyze_project_dict(folder)
            (Path(folder) / 'broken.py').write_text('return 1\n')
            with self.assertRaisesRegex(ValueError, 'broken.py'): analyze_project_dict(folder)


class CacheTests(unittest.TestCase):
    def test_warm_cache_content_config_and_path_category(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            path = root / 'sample.py'
            path.write_text('import math\n')
            cache = root / 'cache' / 'correctness.json'
            before = check_correctness_paths_list([str(path)], cache_path=cache)
            self.assertEqual(before[0].code, 'RC202')
            with patch('refactrail.correctness_batch.check_snapshot_list', side_effect=AssertionError('cache miss')):
                self.assertEqual(check_correctness_paths_list([str(path)], cache_path=cache), before)
            self.assertEqual(check_correctness_paths_list([str(path)], ('RC1',), cache_path=cache), [])
            path.write_text('import math\nprint(math.pi)\n')
            self.assertEqual(check_correctness_paths_list([str(path)], cache_path=cache), [])
            path.write_text('import math\n')
            check_correctness_paths_list([str(path)], cache_path=cache)
            initializer = root / '__init__.py'
            initializer.write_text('import math\n')
            self.assertEqual(check_correctness_paths_list([str(initializer)], cache_path=cache), [])

    def test_corrupt_cache_and_worker_count(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'sample.py'
            path.write_text('print(missing)\n')
            cache = Path(folder) / 'cache.json'
            for source in ('not json', '[]', '{"bad": [[null, 0, 0]]}'):
                cache.write_text(source)
                self.assertEqual(check_correctness_paths_list([str(path)], cache_path=cache)[0].code, 'RC201')
            for jobs in (-1, True):
                with self.assertRaises(ValueError): check_correctness_paths_list([str(path)], jobs_int=jobs)

    def test_parallel_results_match_serial_snapshots(self):
        with tempfile.TemporaryDirectory() as folder:
            paths = []
            for index in range(64):
                path = Path(folder) / f'module_{index}.py'
                path.write_text(f'def compute(value=[]):\n    return value + missing_{index}\n')
                paths.append(str(path))
            serial = check_correctness_paths_list(paths)
            self.assertEqual(check_correctness_paths_list(paths, jobs_int=2), serial)


class RenameTests(unittest.TestCase):
    def proposal(self, source, old='amount', new='base_amount_int'):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'sample.py'
            path.write_bytes(source.encode())
            proposal = plan_rename_dict(str(path), 'calculate', old, new)
            self.assertEqual(path.read_bytes(), source.encode())
            self.assertFalse(proposal['can_apply'])
            self.assertFalse(proposal['behavior_verified'])
            return proposal

    def test_local_rename_preserves_attributes_keywords_and_comments(self):
        source = ('def calculate(receiver):\n    amount = 120\n'
                  '    # amount is the base\n    receiver.amount = amount\n'
                  '    return receiver.compute(amount=amount)\n')
        output = self.proposal(source)['proposed_source']
        self.assertIn('receiver.amount = base_amount_int', output)
        self.assertIn('compute(amount=base_amount_int)', output)
        self.assertIn('# amount is the base', output)

    def test_unicode_crlf_and_authored_behavior(self):
        source = 'def calculate():\r\n    amount = "é"\r\n    return amount + amount\r\n'
        output = self.proposal(source)['proposed_source']
        self.assertNotIn('\n', output.replace('\r\n', ''))
        original_namespace, output_namespace = {}, {}
        # Only these authored fixtures are executed, never customer projects.
        exec(source, original_namespace)
        exec(output, output_namespace)
        self.assertEqual(original_namespace['calculate'](), output_namespace['calculate']())

    def test_refusals_parameters_captures_patterns_and_collisions(self):
        sources = [
            'def calculate(amount):\n    return amount\n',
            'amount=1\ndef calculate():\n    global amount\n    return amount\n',
            'def calculate():\n    amount=1\n    def nested():\n        return amount\n    return nested\n',
            'def calculate():\n    amount=1\n    return locals()\n',
            'def calculate():\n    amount=1\n    base_amount_int=2\n    return amount\n',
            'def calculate():\n    import math as amount\n    return amount\n',
            'def calculate():\n    match {}:\n        case {**amount}: return amount\n',
        ]
        for source in sources:
            with self.subTest(source=source), self.assertRaises(ValueError): self.proposal(source)

    def test_normalized_identifier_and_invalid_source(self):
        for replacement in ('class', 'not valid', '\uff21'):
            with self.assertRaises(ValueError): self.proposal('def calculate():\n    amount=1\n    return amount\n', new=replacement)
        with self.assertRaises(SyntaxError): self.proposal('return 1\n')


class AnalysisCliTests(unittest.TestCase):
    def test_scope_index_and_rename_are_json_and_read_only(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'example.py'
            source = 'def calculate():\n    amount=1\n    return amount\n'
            path.write_text(source)
            commands = [['scope', str(path)], ['index', folder, '--changed', 'example.py'],
                        ['rename', str(path), '--function', 'calculate', '--old', 'amount', '--new', 'amount_int']]
            for command in commands:
                output, error = StringIO(), StringIO()
                with redirect_stdout(output), redirect_stderr(error):
                    self.assertEqual(main(command), 0)
                self.assertTrue(json.loads(output.getvalue())['schema_version'])
                self.assertEqual(path.read_text(), source)
            path.write_text('return 1\n')
            with redirect_stdout(StringIO()), redirect_stderr(StringIO()):
                self.assertEqual(main(['scope', str(path)]), 2)
