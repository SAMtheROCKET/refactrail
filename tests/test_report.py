"""Tests of the report renderers and finding order."""

from dataclasses import asdict
import json
import random
import unittest

from refactrail.engine import sort_findings_list
from refactrail.models import Finding
from refactrail.report import (
    render_github_str, render_json_str, render_text_str,
)

SAMPLE_FINDINGS_LIST = [
    Finding("src/a.py", 3, 5, "RT201", "warning",
            "Name 'x' is a single character; use a meaningful name."),
    Finding("src/été.py", 1, 1, "RT002", "error",
            'Quote " backslash \\ tab \t newline \n percent % 中'),
]


class ReportTests(unittest.TestCase):
    """Renderers keep their output contracts."""

    def test_json_matches_standard_library_indent(self) -> None:
        """The fast JSON renderer equals json.dumps(indent=2)."""
        for findings_list in ([], SAMPLE_FINDINGS_LIST[:1],
                              SAMPLE_FINDINGS_LIST):
            expected_str = json.dumps([asdict(finding) for finding in
                                       findings_list], indent=2) + "\n"
            self.assertEqual(render_json_str(findings_list), expected_str)

    def test_github_annotations_render(self) -> None:
        """Each finding becomes one escaped workflow command."""
        lines_list = render_github_str(SAMPLE_FINDINGS_LIST).splitlines()
        self.assertEqual(lines_list[0], (
            "::warning file=src/a.py,line=3,col=5,title=RT201::RT201 "
            "Name 'x' is a single character; use a meaningful name."))
        self.assertIn("newline %0A percent %25", lines_list[1])
        self.assertEqual(render_github_str([]), "")

    def test_text_summary(self) -> None:
        """The text report ends with a count of errors and warnings."""
        self.assertTrue(render_text_str(SAMPLE_FINDINGS_LIST, 2).endswith(
            "Found 2 finding(s) in 2 file(s): 1 error(s), 1 warning(s).\n"))

    def test_fast_sort_matches_natural_order(self) -> None:
        """sort_findings_list gives the dataclass order."""
        random_generator = random.Random(7)
        findings_list = [Finding(random_generator.choice("ab"),
                                 random_generator.randint(1, 3),
                                 random_generator.randint(1, 3),
                                 random_generator.choice(("RT1", "RT2")),
                                 "warning", random_generator.choice("xy"))
                         for _ in range(300)]
        self.assertEqual(sort_findings_list(findings_list),
                         sorted(findings_list))


if __name__ == "__main__":
    unittest.main()
