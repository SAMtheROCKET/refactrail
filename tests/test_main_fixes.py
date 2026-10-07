"""Tests for RT504: moving a script's top-level code into main()."""

import contextlib
import io
from pathlib import Path
import runpy
import tempfile
import textwrap
import unittest

from refactrail.main_fixes import move_into_main_tuple

SCRIPT = textwrap.dedent('''
    """Count words."""
    import collections

    LIMIT = 2


    def count_words(text):
        return collections.Counter(text.split())


    # Count and report
    counts = count_words("a b a c a b")
    for word, number in counts.most_common(LIMIT):
        print(word, number)
''')


def move(text_str: str, folder: Path | None = None):
    folder = folder or Path(tempfile.mkdtemp())
    path = folder / "words.py"
    path.write_text(text_str, encoding="utf-8")
    return move_into_main_tuple(text_str, str(path)), path


def run_printed(path: Path, text_str: str) -> str:
    # Runs only this test-authored fixture.
    path.write_text(text_str, encoding="utf-8")
    with contextlib.redirect_stdout(io.StringIO()) as output:
        runpy.run_path(str(path), run_name="__main__")
    return output.getvalue()


class MoveIntoMainTests(unittest.TestCase):
    def test_script_code_moves_and_output_is_the_same(self):
        (new_str, reason_str), path = move(SCRIPT)
        self.assertEqual(reason_str, "")
        self.assertIn("def main() -> None:", new_str)
        self.assertIn("    # Count and report\n    counts = count_words(",
                      new_str)
        self.assertTrue(new_str.endswith(
            'if __name__ == "__main__":\n    main()\n'))
        self.assertIn("LIMIT = 2", new_str.split("def main")[0])
        self.assertEqual(run_printed(path, new_str),
                         run_printed(path, SCRIPT))

    def test_unsafe_scripts_are_left_alone(self):
        cases = {
            "a function reads": "def show():\n    print(total)\n\n"
                                "total = sum([1, 2])\nshow()\n",
            "main guard": "x = 1\nif __name__ == '__main__':\n    print(x)\n",
            "multi-line string": "print('''a\nb''')\n",
            "global": "def bump():\n    global n\n    n += 1\n\nn = 0\n"
                      "bump()\n",
            "nothing": "def f():\n    return 1\n",
        }
        for label_str, source_str in cases.items():
            with self.subTest(case=label_str):
                (new_str, reason_str), _ = move(source_str)
                self.assertEqual(new_str, source_str)
                self.assertNotEqual(reason_str, "")

    def test_package_modules_are_left_alone(self):
        folder = Path(tempfile.mkdtemp())
        (folder / "__init__.py").write_text("", encoding="utf-8")
        (new_str, reason_str), _ = move("print(1)\n", folder)
        self.assertEqual(new_str, "print(1)\n")
        self.assertIn("package", reason_str)


if __name__ == "__main__":
    unittest.main()
