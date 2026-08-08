from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))

import audit_csrf_surface as audit  # noqa: E402


class CsrfSurfaceAuditTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "templates").mkdir()
        (self.root / "src/routes").mkdir(parents=True)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def test_reports_post_form_without_csrf(self) -> None:
        path = self.root / "templates/broken.html"
        path.write_text(
            '<form method="post" action="/save"><input name="value"></form>',
            encoding="utf-8",
        )

        findings = audit.audit_root(self.root)

        self.assertEqual(1, len(findings))
        self.assertEqual(path, findings[0].path)
        self.assertEqual(1, findings[0].line)

    def test_accepts_askama_and_escaped_rust_csrf_fields(self) -> None:
        (self.root / "templates/good.html").write_text(
            '<form method="POST"><input type="hidden" name="csrf" value="{{ csrf_token }}"></form>',
            encoding="utf-8",
        )
        (self.root / "src/routes/good.rs").write_text(
            r'''let html = "<form method=\"post\"><input name=\"csrf\" value=\"{csrf}\"></form>";''',
            encoding="utf-8",
        )

        self.assertEqual([], audit.audit_root(self.root))

    def test_ignores_get_forms(self) -> None:
        (self.root / "templates/search.html").write_text(
            '<form method="get"><input name="q"></form>', encoding="utf-8"
        )

        self.assertEqual([], audit.audit_root(self.root))

    def test_reports_markup_preview_without_csrf(self) -> None:
        javascript = self.root / "static/js/preview.js"
        javascript.parent.mkdir(parents=True)
        javascript.write_text(
            "const body = new URLSearchParams({text: value});\n"
            "fetch('/markup/preview', {method: 'POST', body});\n",
            encoding="utf-8",
        )

        findings = audit.audit_root(self.root)

        self.assertEqual(1, len(findings))
        self.assertIn("/markup/preview", findings[0].message)

    def test_accepts_markup_preview_with_csrf(self) -> None:
        javascript = self.root / "static/js/preview.js"
        javascript.parent.mkdir(parents=True)
        javascript.write_text(
            "const body = new URLSearchParams({text: value});\n"
            "body.set('csrf', token);\n"
            "fetch('/markup/preview', {method: 'POST', body});\n",
            encoding="utf-8",
        )

        self.assertEqual([], audit.audit_root(self.root))


if __name__ == "__main__":
    unittest.main()
