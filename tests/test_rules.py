"""Rule-by-rule tests of the Python reference engine against RULES.md.

These cases are also the parity corpus for the Rust engine: both engines
must produce exactly the findings asserted here.
"""

from dataclasses import replace
import unittest


class CorrectnessContractTests(unittest.TestCase):
    def codes(self, source, **options):
        from refactrail.correctness import check_correctness_list
        options.setdefault("select_tuple", ("RC1",))
        return check_correctness_list("example.py", source.encode("utf-8"),
                              **options)

    def test_mutable_defaults_include_kwonly_lambda_and_nested_tuple(self):
        source = ("def build(first=[], *, second=({},)):\n    pass\n"
                  "callback = lambda parameter=set(): parameter\n"
                  "other = lambda parameter={1, 2}: parameter\n")
        rows = self.codes(source)
        self.assertEqual([row.code for row in rows], ["RC101"] * 3)
        self.assertEqual([row.line for row in rows], [1, 1, 4])

    def test_duplicate_keys_use_builtin_equality_and_reset_on_unknown(self):
        rows = self.codes("mapping = {1: 'a', True: 'b', 1.0: 'c'}\n")
        self.assertEqual([row.code for row in rows], ["RC102", "RC102"])
        self.assertEqual(self.codes("mapping = {1: 0, **other, True: 1}\n"), [])
        self.assertEqual(self.codes("mapping = {1: 0, dynamic(): 2, 1: 3}\n"), [])

    def test_identity_literals_exclude_singletons(self):
        rows = self.codes("matched = candidate is 1000\n")
        self.assertEqual([row.code for row in rows], ["RC103"])
        self.assertEqual(self.codes("matched = candidate is None\n"), [])
        self.assertEqual(self.codes("matched = candidate is True\n"), [])

    def test_bare_except_and_nonempty_tuple_assert(self):
        rows = self.codes("try:\n    operate()\nexcept:\n    pass\n"
                         "assert (condition, 'message')\n")
        self.assertEqual([row.code for row in rows], ["RC104", "RC106"])
        self.assertEqual(self.codes("assert (*unknown,)\n"), [])

    def test_unreachable_reports_only_first_direct_successor(self):
        rows = self.codes("def compute():\n    return 1\n"
                         "    print('unreachable')\n    print('also')\n")
        self.assertEqual([(row.code, row.line) for row in rows],
                         [("RC105", 3)])
        self.assertEqual(self.codes("def compute(flag):\n    if flag:\n"
                                    "        return 1\n    return 2\n"), [])

    def test_finally_control_flow_respects_nested_scopes_and_loops(self):
        rows = self.codes("def compute():\n    try:\n        operate()\n"
                         "    finally:\n        return 1\n")
        self.assertEqual([row.code for row in rows], ["RC107"])
        self.assertEqual(self.codes("try:\n    operate()\nfinally:\n"
                                    "    def compute():\n        return 1\n"
                                    "    for element in sequence:\n"
                                    "        break\n"), [])

    def test_unicode_positions_noqa_and_selection(self):
        rows = self.codes("\u00e9 = {1: 2, True: 3}\n")
        self.assertEqual((rows[0].line, rows[0].column), (1, 12))
        self.assertEqual(self.codes("\u00e9 = {1: 2, True: 3} # noqa: RC102\n"), [])
        self.assertEqual(self.codes("mapping={1:2, 1:3}\n", ignore_tuple=("RC1",)), [])
        self.assertEqual(self.codes("mapping={1:2, 1:3}\n", select_tuple=("RC101",)), [])

    def test_compile_errors_cannot_be_suppressed(self):
        rows = self.codes("return 1 # noqa\n", ignore_tuple=("RT", "RC"))
        self.assertEqual([row.code for row in rows], ["RT001"])

    def test_finally_loop_else_exit_targets_the_outer_loop(self):
        rows = self.codes("while outer:\n    try:\n        operate()\n"
                         "    finally:\n        for entry in entries:\n"
                         "            pass\n        else:\n            break\n")
        self.assertEqual([(row.code, row.line) for row in rows], [("RC107", 8)])

    def test_nested_finally_does_not_duplicate_observations(self):
        rows = self.codes("def compute():\n    try:\n        operate()\n"
                         "    finally:\n        try:\n            operate()\n"
                         "        finally:\n            return 1\n")
        self.assertEqual([row.code for row in rows], ["RC107"])

    def test_target_imports_and_calls_are_not_executed(self):
        source = "raise RuntimeError('do not run')\n"
        self.assertEqual(self.codes(source), [])

from refactrail.engine import check_text_list
from refactrail.models import Settings

STANDARD_SETTINGS = Settings()
STRICT_SETTINGS = Settings(profile="strict")


def find_codes_list(source_str: str, settings_info: Settings = STANDARD_SETTINGS,
                    select_str: str = "") -> list[tuple]:
    """Check source text and return (line, column, code) triples."""
    if select_str:
        settings_info = replace(settings_info, select=(select_str,))
    return [(finding.line, finding.column, finding.code) for finding in
            check_text_list("sample.py", source_str.encode("utf-8"),
                            settings_info)]


class LayoutRuleTests(unittest.TestCase):
    def test_parse_errors_cannot_be_filtered_out(self):
        self.assertEqual([row[2] for row in find_codes_list(
            "return 1\n", select_str="RT207")], ["RT001"])

    def test_rich_comparisons_have_unknown_result_types(self):
        self.assertEqual(find_codes_list("mask = left < right\n",
                         STRICT_SETTINGS, select_str="RT203"), [])
        self.assertEqual(len(find_codes_list("mask = left is right\n",
                             STRICT_SETTINGS, select_str="RT203")), 1)

    def test_contextual_compile_errors_are_not_clean(self):
        for source in ("return 1\n", "break\n", "nonlocal missing\n"):
            self.assertEqual([row[2] for row in find_codes_list(
                source, select_str="RT001")], ["RT001"])

    def test_noqa_strings_do_not_suppress_findings(self):
        self.assertEqual(find_codes_list('x = "# noqa"\n',
                                         select_str="RT201"),
                         [(1, 1, "RT201")])
        self.assertEqual(find_codes_list('x = "# noqa" # noqa: RT201\n',
                                         select_str="RT201"), [])

    def test_rt101_counts_characters_not_bytes(self):
        source = '"""Doc."""\nNAME_STR = "' + "é" * 70 + '"\n'
        self.assertEqual(find_codes_list(source, select_str="RT101"),
                         [(2, 80, "RT101")])
        short = '"""Doc."""\nNAME_STR = "' + "é" * 60 + '"\n'
        self.assertEqual(find_codes_list(short, select_str="RT101"), [])

    def test_rt102_and_rt103_constants(self):
        source = ('"""Doc."""\nimport os\nlimit = 3\nLABEL = "x"\n\n\n'
                  "def run_job() -> None:\n    pass\n\n\nLATE = 1\n")
        self.assertEqual(find_codes_list(source, select_str="RT10"),
                         [(3, 1, "RT102"), (11, 1, "RT103")])

    def test_rt102_ignores_rebound_and_dunder_names(self):
        source = ('"""Doc."""\ncount = 1\ncount = 2\n__version__ = "1"\n'
                  "_ = 5\n")
        self.assertEqual(find_codes_list(source, select_str="RT102"), [])

    def test_rt504_flags_first_import_time_code_only(self):
        source = ('"""Doc."""\nimport re\nPATTERN = re.compile("a")\n'
                  "items = [1]\nprint(items)\nprint(2)\n")
        self.assertEqual(find_codes_list(source, select_str="RT504"),
                         [(5, 1, "RT504")])
        guarded = ('"""Doc."""\ntry:\n    import json\nexcept ImportError:\n'
                   "    json = None\n\nif __name__ == '__main__':\n"
                   "    print(1)\n")
        self.assertEqual(find_codes_list(guarded, select_str="RT504"), [])


class NamingRuleTests(unittest.TestCase):
    def test_declared_signature_suffixes_are_strict_only(self):
        source = "def compute(count: int) -> float:\n    return count / 2\n"
        for code in ("RT206", "RT207"):
            self.assertEqual(find_codes_list(source, select_str=code), [])
            self.assertEqual(len(find_codes_list(
                source, STRICT_SETTINGS, select_str=code)), 1)
        source = ("def compute_float(count_int: int) -> float:\n"
                  "    return count_int / 2\n")
        self.assertEqual(find_codes_list(source, STRICT_SETTINGS,
                                         select_str="RT20"), [])

    def test_rt201_single_characters(self):
        source = ('"""Doc."""\nimport numpy as n\nfrom os import sep as s\n'
                  "from math import e\nfor _ in range(2):\n    pass\n"
                  "values = [x for x in range(3)]\n")
        self.assertEqual(find_codes_list(source, select_str="RT201"),
                         [(2, 8, "RT201"), (3, 16, "RT201"),
                          (7, 17, "RT201")])

    def test_rt202_verbs_and_exemptions(self):
        source = ('"""Doc."""\n\n\ndef total():\n    pass\n\n\n'
                  "def compute_total():\n    pass\n\n\nclass Shape:\n"
                  "    @property\n    def area(self):\n        pass\n\n"
                  "    def setUp(self):\n        pass\n\n"
                  "    def __init__(self):\n        pass\n")
        self.assertEqual(find_codes_list(source, select_str="RT202"),
                         [(4, 5, "RT202")])

    def test_rt203_strict_dtype_suffixes(self):
        source = ('"""Doc."""\n\n\ndef load_rows() -> None:\n'
                  "    count = 0\n    names_list = []\n    label = f'{count}'\n"
                  "    size = len(names_list)\n    ratio = count / 2\n"
                  "    __all__ = ['x']\n")
        self.assertEqual(find_codes_list(source, STRICT_SETTINGS, "RT203"),
                         [(5, 5, "RT203"), (7, 5, "RT203"),
                          (8, 5, "RT203")])
        self.assertEqual(find_codes_list(source, select_str="RT203"), [])

    def test_rt203_rebound_builtin_is_not_trusted(self):
        source = ('"""Doc."""\n\n\ndef len(value_list):\n    return 3\n\n\n'
                  "size = len([1])\n")
        self.assertEqual(find_codes_list(source, STRICT_SETTINGS, "RT203"),
                         [])

    def test_rt204_conventions(self):
        source = ('"""Doc."""\nMAX_SIZE = 3\n\n\nclass bad_name:\n'
                  "    RED = 1\n\n    def setUp(self):\n        pass\n\n\n"
                  "def RunJob(inputValue):\n    localValue = 1\n")
        self.assertEqual(find_codes_list(source, select_str="RT204"),
                         [(5, 7, "RT204"), (12, 5, "RT204"),
                          (12, 12, "RT204"), (13, 5, "RT204")])

    def test_type_aliases_are_not_variables_or_code(self):
        source = ('"""Doc."""\nimport ast\nfrom typing import TypeAlias\n'
                  "Site = tuple[ast.AST, int]\nMaybeSite = Site | None\n"
                  "Plain: TypeAlias = int\ntype Pair = tuple[int, int]\n"
                  "LIMIT_INT = 3\nRunner = make_runner()\n"
                  "Count = 3\n")
        self.assertEqual(find_codes_list(source, select_str="RT1"),
                         [(10, 1, "RT102"), (10, 1, "RT103")])
        self.assertEqual(find_codes_list(source, select_str="RT204"),
                         [(9, 1, "RT204")])
        self.assertEqual(find_codes_list(source, select_str="RT504"),
                         [(9, 1, "RT504")])

    def test_rt205_generic_names_strict_only(self):
        source = '"""Doc."""\ndata = compute()\nresult_list = []\n'
        self.assertEqual(find_codes_list(source, STRICT_SETTINGS, "RT205"),
                         [(2, 1, "RT205")])
        self.assertEqual(find_codes_list(source, select_str="RT205"), [])


class DocumentationRuleTests(unittest.TestCase):
    def test_rt301_missing_docstrings(self):
        source = ("import os\n\n\nclass Shape:\n    pass\n\n\n"
                  "def compute_area(side):\n    return side\n\n\n"
                  "def stub_method():\n    ...\n")
        self.assertEqual(find_codes_list(source, select_str="RT301"),
                         [(1, 1, "RT301"), (4, 7, "RT301"),
                          (8, 5, "RT301")])

    def test_rt302_sections_by_profile(self):
        source = ('"""Doc."""\n\n\ndef compute_area(side):\n'
                  '    """Compute an area.\n\n    Args:\n'
                  '        side: Length.\n    """\n    return side\n')
        self.assertEqual(find_codes_list(source, select_str="RT302"),
                         [(4, 5, "RT302")])
        self.assertEqual(len(find_codes_list(source, STRICT_SETTINGS,
                                             "RT302")), 2)

    def test_rt302_accepts_numpy_style(self):
        source = ('"""Doc."""\n\n\ndef compute_area(side):\n'
                  '    """Compute an area.\n\n    Parameters\n    ----------\n'
                  "    side : float\n        Length.\n\n    Returns\n"
                  '    -------\n    float\n    """\n    return side\n')
        self.assertEqual(find_codes_list(source, select_str="RT30"), [])

    def test_rt303_argument_mismatch(self):
        source = ('"""Doc."""\n\n\nclass Shape:\n    """Shape."""\n\n'
                  "    def resize(self, width, *args):\n"
                  '        """Resize.\n\n        Args:\n'
                  "            width (int): New width.\n"
                  "                Continued text.\n"
                  "            height: Old parameter.\n"
                  '        Returns:\n            None: Nothing.\n        """\n')
        messages_list = [finding.message for finding in check_text_list(
            "sample.py", source.encode("utf-8"),
            replace(STANDARD_SETTINGS, select=("RT303",)))]
        self.assertEqual(messages_list, [
            "Docstring of 'resize' describes unknown parameter 'height'.",
            "Docstring of 'resize' does not describe parameter 'args'."])


class AnnotationAndSizeRuleTests(unittest.TestCase):
    def test_rt401_and_rt402(self):
        source = ('"""Doc."""\n\n\nclass Shape:\n    """Shape."""\n\n'
                  "    def __init__(self, side: int):\n        pass\n\n"
                  "    @staticmethod\n    def build(side, *parts: int):\n"
                  "        pass\n")
        self.assertEqual(find_codes_list(source, select_str="RT40"),
                         [(11, 9, "RT402"), (11, 15, "RT401")])
        self.assertIn((7, 9, "RT402"), find_codes_list(
            source, STRICT_SETTINGS, "RT40"))

    def test_rt501_rt502_rt503(self):
        def build_function_str(lines_int):
            return ('"""Doc."""\n\n\n@staticmethod\ndef run_long():\n'
                    + "".join(f"    step_{index} = {index}\n"
                              for index in range(lines_int - 2)))

        self.assertEqual(find_codes_list(build_function_str(41),
                                         select_str="RT50"),
                         [(5, 5, "RT501")])
        self.assertEqual(find_codes_list(build_function_str(40),
                                         select_str="RT50"), [])
        self.assertEqual(find_codes_list(build_function_str(51),
                                         select_str="RT50"),
                         [(5, 5, "RT502")])
        main_str = ('"""Doc."""\nif __name__ == "__main__":\n'
                    + "    print(1)\n" * 100)
        self.assertEqual(find_codes_list(main_str, select_str="RT50"),
                         [(2, 1, "RT503")])
        self.assertEqual(find_codes_list(main_str.replace(
            "    print(1)\n", "", 1), select_str="RT50"), [])


class EngineBehaviourTests(unittest.TestCase):
    def test_noqa_suppresses_listed_codes_only(self):
        source = ('"""Doc."""\nx = compute()  # noqa: RT201\n'
                  "y = compute()  # noqa\nz = compute()  # NOQA: RT3\n")
        self.assertEqual(find_codes_list(source, select_str="RT201"),
                         [(4, 1, "RT201")])

    def test_syntax_and_encoding_errors(self):
        self.assertEqual([code for _, _, code in
                          find_codes_list("def broken(:\n")], ["RT001"])
        findings = check_text_list("legacy.py", b"# coding: latin-1\nx = 1\n",
                                   STANDARD_SETTINGS)
        self.assertEqual([(finding.line, finding.code)
                          for finding in findings], [(1, "RT002")])
        invalid = check_text_list("bad.py", b"x = '\xff'\n",
                                  STANDARD_SETTINGS)
        self.assertEqual([finding.code for finding in invalid], ["RT002"])

    def test_select_and_ignore_prefixes(self):
        source = "def f(x):\n    return x\n"
        ignored = replace(STANDARD_SETTINGS, ignore=("RT2", "RT3", "RT4"))
        codes_set = {code for _, _, code in find_codes_list(source, ignored)}
        self.assertEqual(codes_set, set())



class NativeRulesTableTests(unittest.TestCase):
    def test_native_rules_table_matches_the_python_command(self):
        import contextlib
        import io
        from pathlib import Path

        from refactrail.cli import main

        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            main(["rules"])
        table_path = (Path(__file__).resolve().parents[1] / "rust" / "crates"
                      / "cli" / "data" / "rules.txt")
        if not table_path.exists():
            self.skipTest("Rust sources are not part of this checkout")
        self.assertEqual(table_path.read_text(encoding="utf-8"),
                         output.getvalue())

if __name__ == "__main__":
    unittest.main()
