"""Regression evidence for invalid inputs, cache damage and source writes."""

from dataclasses import replace
from pathlib import Path
import json
import tempfile
import unittest
from unittest.mock import patch

from refactrail.config import build_settings
from refactrail.engine import check_paths_list
from refactrail.fixing import fix_file, write_fix_none
from refactrail.models import Settings
from refactrail.fixes import apply_safe_fixes


class ReleaseSafetyTests(unittest.TestCase):
    def test_config_rejects_malformed_lists_and_inconsistent_limits(self):
        for config in ({"ignore": "RT2"}, {"select": [3]},
                       {"line_length": 39}, {"line_length": 201},
                       {"function_preferred_lines": 51}):
            with self.subTest(config=config), self.assertRaises(ValueError):
                build_settings(config)

    def test_direct_api_settings_follow_the_same_limits(self):
        for config in ({"line_length": 39}, {"ignore": "RT2"},
                       {"profile": "anything"}, {"line_length": True}):
            with self.subTest(config=config), self.assertRaises(ValueError):
                Settings(**config)

    def test_corrupt_cache_cannot_crash_the_checker(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "source.py"
            source.write_text("x = 1\n", encoding="utf-8")
            cache = Path(folder) / "cache.json"
            for payload in ([], 1, {"damaged": [[1, 2]]}):
                cache.write_text(json.dumps(payload), encoding="utf-8")
                findings = check_paths_list([str(source)], Settings(),
                                            cache_path=cache, jobs_int=1)
                self.assertTrue(findings)

    def test_fix_refuses_compile_invalid_source(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "broken.py"
            source.write_text("return 1\n", encoding="utf-8")
            proposal = fix_file(str(source), Settings())
            self.assertIsNone(proposal.outcome)
            self.assertIn("syntax error", proposal.skip_reason)

    def test_fix_will_not_overwrite_an_intervening_edit(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "source.py"
            source.write_text("def log_event():\n    print('ok')\n",
                              encoding="utf-8")
            proposal = fix_file(str(source), Settings())
            self.assertTrue(proposal.changed_bool)
            source.write_text("# newer user edit\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                write_fix_none(proposal)
            self.assertEqual(source.read_text(), "# newer user edit\n")

    def test_failed_write_keeps_original_and_cleans_temporary_file(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "source.py"
            original = "def log_event():\n    print('ok')\n"
            source.write_text(original, encoding="utf-8")
            proposal = fix_file(str(source), Settings())
            with patch("refactrail.fixing.os.fsync", side_effect=OSError), (
                self.assertRaises(OSError)
            ):
                write_fix_none(proposal)
            self.assertEqual(source.read_text(), original)
            self.assertEqual(list(Path(folder).glob(".refactrail-*")), [])

    def test_annotations_do_not_change_decorator_observations(self):
        source = ("def inspect(function):\n"
                  "    print(function.__annotations__)\n"
                  "    return function\n"
                  "@inspect\ndef log_event():\n    print('event')\n")
        outcome = apply_safe_fixes(source, "source.py", replace(
            Settings(), select=("RT402",)))
        self.assertEqual(outcome.text, source)
