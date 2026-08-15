from __future__ import annotations

import subprocess
import sys
import unittest
from contextlib import redirect_stderr
from io import StringIO
from pathlib import Path
from unittest import mock


TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))

import check_javascript_syntax as checker  # noqa: E402


class JavaScriptSyntaxCheckTest(unittest.TestCase):
    @mock.patch.object(checker.subprocess, "run")
    def test_discovers_only_tracked_javascript_below_static_js(self, run) -> None:
        run.return_value = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=(
                b"static/js/root.js\0"
                b"static/js/lor/reactions.js\0"
                b"static/js/lor/readme.txt\0"
                b"frontend/unrelated.js\0"
            ),
            stderr=b"",
        )

        files = checker.tracked_javascript_files(Path("/repository"))

        self.assertEqual(
            [Path("static/js/lor/reactions.js"), Path("static/js/root.js")],
            files,
        )
        run.assert_called_once_with(
            ["git", "-C", "/repository", "ls-files", "-z", "--", "static/js"],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

    @mock.patch.object(checker.subprocess, "run")
    def test_checks_each_path_as_a_separate_argument_without_a_shell(self, run) -> None:
        run.side_effect = [
            subprocess.CompletedProcess(args=[], returncode=0),
            subprocess.CompletedProcess(args=[], returncode=1),
        ]
        files = [Path("static/js/file with spaces.js"), Path("static/js/broken.js")]

        errors = StringIO()
        with redirect_stderr(errors):
            status = checker.check_javascript_files(Path("/repository"), "node", files)

        self.assertEqual(1, status)
        self.assertIn("static/js/broken.js", errors.getvalue())
        self.assertEqual(
            [
                mock.call(
                    ["node", "--check", "static/js/file with spaces.js"],
                    cwd=Path("/repository"),
                    check=False,
                ),
                mock.call(
                    ["node", "--check", "static/js/broken.js"],
                    cwd=Path("/repository"),
                    check=False,
                ),
            ],
            run.call_args_list,
        )


if __name__ == "__main__":
    unittest.main()
