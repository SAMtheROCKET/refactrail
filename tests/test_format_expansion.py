"""Notebook preservation, wrapping and discovery contracts."""

import ast
from dataclasses import replace
import json
from pathlib import Path
import tempfile
import unittest

from refactrail.formatting import format_source_str, plan_format, write_format_none
from refactrail.notebooks import format_notebook_str
from refactrail.general_cli import discover_general_files_list


class ExpandedFormattingTests(unittest.TestCase):
    def test_wrap_call_signature_import_and_container(self):
        cases = [
            'answer = calculate(first_argument, second_argument, third_argument)\n',
            'def compute(first_argument: int, second_argument: int, third_argument: int):\n    return first_argument\n',
            'from module import (first_argument, second_argument, third_argument)\n',
            'values = [first_argument, second_argument, third_argument]\n',
            'values = {"first_argument": 1, "second_argument": 2, "third_argument": 3}\n',
        ]
        for source in cases:
            with self.subTest(source=source):
                output = format_source_str(source, width_int=40)
                self.assertGreater(output.count('\n'), source.count('\n'))
                self.assertEqual(ast.dump(ast.parse(source)), ast.dump(ast.parse(output)))
                self.assertEqual(format_source_str(output, width_int=40), output)
                self.assertTrue(all(len(line) <= 40 for line in output.splitlines()))

    def test_wrap_layout_hugs_then_explodes(self):
        hugged = format_source_str(
            "total_value = compute_total(first_amount, second_amount)\n",
            width_int=40)
        self.assertEqual(hugged, "total_value = compute_total(\n"
                                 "    first_amount, second_amount\n)\n")
        header = format_source_str(
            "def read(path_str: str, size_int: int) -> tuple[int, int]:\n"
            "    return 1, 2\n", width_int=50)
        self.assertTrue(header.startswith(
            "def read(\n    path_str: str, size_int: int\n"
            ") -> tuple[int, int]:\n"))

    def test_layout_search_stays_polynomial_on_many_brackets(self):
        import time
        calls = ", ".join(f"g{index}(alpha_{index}, beta_{index}(gamma))"
                          for index in range(60))
        source = (f"result_value = f({calls}) + "
                  + " + ".join(f"term_{index}(a, b)" for index in range(40))
                  + "\n")
        started = time.perf_counter()
        output = format_source_str(source, width_int=79)
        self.assertLess(time.perf_counter() - started, 5)
        self.assertEqual(ast.dump(ast.parse(output)), ast.dump(ast.parse(source)))

    def test_layout_then_spacing_reaches_fixed_point_in_one_run(self):
        source = ("table = str.maketrans({value: f'{value:02x}' for value in "
                  "chain(range(0x20), range(0x7f,0xa0))})\n")
        output = format_source_str(source, width_int=50)
        self.assertEqual(format_source_str(output, width_int=50), output)
        self.assertIn("range(0x7f, 0xa0)", output)
        self.assertIn("f'{value:02x}'", output)

    def test_wrap_parenthesizes_long_from_imports(self):
        source = ("from package.module import first_name, second_name, "
                  "third_name\nfrom package.module import *\n")
        output = format_source_str(source, width_int=40)
        self.assertEqual(output, "from package.module import (\n"
                                 "    first_name, second_name, third_name\n"
                                 ")\nfrom package.module import *\n")
        self.assertEqual(format_source_str(output, width_int=40), output)

    def test_wrap_packs_continuation_lines_and_skips_inner_groups(self):
        source = ("from package.module import (\n"
                  "    first_name, second_name, third_name, fourth_name,\n"
                  "    fifth_name,\n)\n")
        output = format_source_str(source, width_int=40)
        self.assertEqual(output, "from package.module import (\n"
                                 "    first_name, second_name, third_name,\n"
                                 "    fourth_name,\n    fifth_name,\n)\n")
        self.assertEqual(format_source_str(output, width_int=40), output)
        inner = ("values = {item for item in collection\n"
                 "          if isinstance(item, first_kind)} | rest(x)\n")
        laid_out = format_source_str(inner, width_int=40)
        self.assertEqual(laid_out, "values = {\n    item\n"
                                   "    for item in collection\n"
                                   "    if isinstance(item, first_kind)\n"
                                   "} | rest(x)\n")
        hugged = format_source_str(inner, width_int=40, hug_bool=True)
        self.assertEqual(hugged, "values = {\n    item for item in collection\n"
                                 "    if isinstance(item, first_kind)\n"
                                 "} | rest(x)\n")
        for output in (laid_out, hugged):
            self.assertEqual(ast.dump(ast.parse(output)), ast.dump(ast.parse(inner)))

    def test_wrapping_protects_comments_types_literals_and_line_endings(self):
        source = 'answer = calculate(first_argument, second_argument, third_argument)\r\n'
        output = format_source_str(source, width_int=40)
        self.assertNotIn('\n', output.replace('\r\n', ''))
        for tail in (' # noqa\n', ' # type: ignore\n'):
            protected = source.rstrip() + tail
            self.assertEqual(format_source_str(protected, width_int=40), protected)
        protected = source + 'next_value = 1 # type: ignore\r\n'
        self.assertEqual(format_source_str(protected, width_int=40), protected)
        literal = 'text = "' + 'a' * 100 + '"\n'
        self.assertEqual(format_source_str(literal, width_int=40), literal)
        for width in (39, 201, True):
            with self.assertRaises(ValueError):
                format_source_str('answer=1\n', width_int=width)

    def notebook(self, sources):
        return {'nbformat': 4, 'nbformat_minor': 5,
                'metadata': {'kernelspec': {'language': 'python'}, 'custom': [1, 2]},
                'cells': [{'cell_type': 'markdown', 'metadata': {}, 'source': ['# Keep me\n']},
                          *[{'cell_type': 'code', 'source': source,
                             'metadata': {'tags': ['keep']}, 'execution_count': 7,
                             'outputs': [{'output_type': 'stream', 'name': 'stdout', 'text': ['120\n']}]}
                            for source in sources]]}

    def test_notebook_changes_only_source_spans(self):
        document = self.notebook([['amount=120\n', 'rate=0.1\n'], 'answer=1+2'])
        source = json.dumps(document, indent=3, ensure_ascii=True) + '\n'
        output = format_notebook_str(source, 'example.ipynb')
        parsed = json.loads(output)
        self.assertEqual(parsed['cells'][1]['source'], ['amount = 120\n', 'rate = 0.1\n'])
        self.assertEqual(parsed['cells'][2]['source'], 'answer = 1 + 2\n')
        for index in (1, 2):
            parsed['cells'][index]['source'] = document['cells'][index]['source']
        self.assertEqual(parsed, document)
        from refactrail.json_spans import collect_json_spans_dict
        for text in (source, output):
            for start, end in sorted(collect_json_spans_dict(text).values(), reverse=True):
                text = text[:start] + '<SOURCE>' + text[end:]
            if 'before' not in locals(): before = text
            else: self.assertEqual(text, before)
        self.assertEqual(format_notebook_str(output, 'example.ipynb'), output)

    def test_notebook_bom_stale_and_forged_metadata(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'example.ipynb'
            original = b'\xef\xbb\xbf' + json.dumps(self.notebook(['answer=1'])).encode()
            path.write_bytes(original)
            plan = plan_format(str(path))
            self.assertEqual(path.read_bytes(), original)
            changed = json.loads(plan.output_bytes.decode('utf-8-sig'))
            changed['cells'][1]['outputs'] = []
            with self.assertRaises(ValueError):
                write_format_none(replace(plan, output_bytes=json.dumps(changed).encode()))
            write_format_none(plan)
            self.assertTrue(path.read_bytes().startswith(b'\xef\xbb\xbf'))
            with self.assertRaises(ValueError): write_format_none(plan)

    def test_magics_refuse_whole_notebook_and_strings_remain(self):
        document = self.notebook(['amount=1', '%matplotlib inline\n'])
        with self.assertRaisesRegex(ValueError, 'cell=3'):
            format_notebook_str(json.dumps(document), 'example.ipynb')
        source = json.dumps(self.notebook(['text="""\n%literal\n!literal\n"""\n']))
        self.assertEqual(format_notebook_str(source, 'example.ipynb'), source)

    def test_invalid_json_shapes_languages_and_duplicate_keys(self):
        for source in ('{"nbformat":4,"cells":[],"cells":[]}', '{}',
                       json.dumps({'nbformat': 4, 'cells': [{'cell_type': 'code', 'source': [1]}]})):
            with self.assertRaises(ValueError): format_notebook_str(source, 'example.ipynb')
        document = self.notebook(['answer=1'])
        document['metadata']['kernelspec']['language'] = 'julia'
        with self.assertRaises(ValueError): format_notebook_str(json.dumps(document), 'example.ipynb')

    def test_discovery_includes_stubs_and_opt_in_notebooks(self):
        with tempfile.TemporaryDirectory() as folder:
            for name in ('a.py', 'b.pyi', 'c.ipynb', 'readme.txt'):
                (Path(folder) / name).write_text('')
            self.assertEqual([Path(p).name for p in discover_general_files_list([folder])], ['a.py', 'b.pyi'])
            self.assertEqual(len(discover_general_files_list([folder], True)), 3)


    def test_nested_long_groups_reach_one_idempotent_result(self):
        source = 'answer = calculate(first_argument, nested(first_argument, second_argument, third_argument), third_argument)\n'
        output = format_source_str(source, width_int=40)
        self.assertEqual(format_source_str(output, width_int=40), output)
        self.assertEqual(ast.dump(ast.parse(source)), ast.dump(ast.parse(output)))

    def test_invalid_metadata_values_are_clear_refusals(self):
        for language in (None, 42, []):
            document = self.notebook(['amount=1'])
            document['metadata']['kernelspec']['language'] = language
            with self.assertRaises(ValueError):
                format_notebook_str(json.dumps(document), 'example.ipynb')
        with self.assertRaises(ValueError):
            format_notebook_str('{"nbformat":4,"cells":[],"metadata":{"number":NaN}}', 'example.ipynb')
