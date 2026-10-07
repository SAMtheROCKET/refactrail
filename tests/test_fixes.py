"""Tests for safe fixes and the fix command. Fixtures are authored here."""

from dataclasses import replace
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from refactrail.cli import main
from refactrail.fixes import add_none_returns_tuple, apply_safe_fixes
from refactrail.models import Settings

NONE_RETURN_SOURCE = '''"""Doc."""
import abc


def log_value(value: int):
    print(value)


async def notify(
    target: str,
    retries: int = 3,
):
    print(target, retries)


def produce():
    yield 1


def compute() -> int:
    return 1


def early_exit(flag: bool):
    if flag:
        return
    print("done")


def stub():
    ...


class Base(abc.ABC):
    @abc.abstractmethod
    def area(self):
        raise NotImplementedError

    @staticmethod
    def show(text: str):
        def inner():
            return text
        print(inner())
'''


class NoneReturnTests(unittest.TestCase):
    def test_only_certain_none_functions_are_annotated(self):
        new_text, names_list = add_none_returns_tuple(NONE_RETURN_SOURCE)
        self.assertEqual(sorted(names_list),
                         ["early_exit -> None", "log_value -> None",
                          "notify -> None"])
        self.assertIn("def log_value(value: int) -> None:", new_text)
        self.assertIn("    retries: int = 3,\n) -> None:", new_text)
        self.assertIn("def produce():", new_text)
        self.assertIn("    def area(self):", new_text)
        self.assertIn("        def inner():", new_text)

    def test_second_run_changes_nothing(self):
        once_text, _ = add_none_returns_tuple(NONE_RETURN_SOURCE)
        twice_text, names_list = add_none_returns_tuple(once_text)
        self.assertEqual((twice_text, names_list), (once_text, []))


class SafeFixTests(unittest.TestCase):
    def test_fixed_code_behaves_the_same(self):
        source = ('"""Doc."""\n\n\ndef report(values):\n    total = 0\n'
                  + "".join(f"    total = total + {index}\n"
                            for index in range(45))
                  + "    print(total, values)\n")
        outcome = apply_safe_fixes(source, "sample.py", Settings())
        self.assertIn('"""Report.', outcome.text)
        self.assertIn("def report(values) -> None:", outcome.text)
        outputs_list = []
        for text_str in (source, outcome.text):
            namespace_dict: dict = {}
            exec(compile(text_str, "fixture", "exec"), namespace_dict)
            with tempfile.TemporaryFile("w+") as stream:
                sys_stdout = sys.stdout
                sys.stdout = stream
                try:
                    namespace_dict["report"]([1])
                finally:
                    sys.stdout = sys_stdout
                stream.seek(0)
                outputs_list.append(stream.read())
        self.assertEqual(outputs_list[0], outputs_list[1])

    def test_selected_codes_limit_the_fixes(self):
        source = '"""Doc."""\n\n\ndef log_value(value):\n    print(value)\n'
        only_returns = replace(Settings(), select=("RT402",))
        outcome = apply_safe_fixes(source, "sample.py", only_returns)
        self.assertEqual(outcome.text, source.replace(
            "(value):", "(value) -> None:"))


class FixCommandTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def run_cli(self, *arguments):
        return subprocess.run([sys.executable, "-m", "refactrail",
                               *arguments], capture_output=True, text=True,
                              cwd=self.root, timeout=60)

    def test_diff_writes_nothing_and_fix_preserves_crlf_and_bom(self):
        path = self.root / "module.py"
        original_bytes = (b"\xef\xbb\xbf\"\"\"Doc.\"\"\"\r\n\r\n\r\n"
                          b"def log_value(value: int):\r\n"
                          b"    print(value)\r\n")
        path.write_bytes(original_bytes)
        diff_result = self.run_cli("fix", "module.py", "--select", "RT402",
                                   "--diff")
        self.assertEqual(diff_result.returncode, 1, diff_result.stderr)
        self.assertIn("+def log_value(value: int) -> None:",
                      diff_result.stdout)
        self.assertEqual(path.read_bytes(), original_bytes)
        fix_result = self.run_cli("fix", "module.py", "--select", "RT402")
        self.assertEqual(fix_result.returncode, 0, fix_result.stdout)
        self.assertEqual(path.read_bytes(), original_bytes.replace(
            b"int):", b"int) -> None:"))

    def test_broken_files_are_skipped(self):
        (self.root / "broken.py").write_text("def broken(:\n")
        result = self.run_cli("fix", "broken.py")
        self.assertIn("skipped: syntax error", result.stderr)
        self.assertEqual((self.root / "broken.py").read_text(),
                         "def broken(:\n")

    def test_main_entry_point_accepts_arguments(self):
        (self.root / "clean.py").write_text('"""Doc."""\n')
        self.assertEqual(main(["check", str(self.root / "clean.py"),
                               "--no-cache"]), 0)


if __name__ == "__main__":
    unittest.main()
