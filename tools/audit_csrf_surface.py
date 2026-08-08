#!/usr/bin/env python3
"""Fail when a server-rendered POST form omits the CSRF form field.

The Rust port applies its CSRF middleware to every POST except the manually
validated ``/add.jsp`` flow. A form without ``name="csrf"`` is therefore a
broken user-facing workflow, not just a security lint.
"""

from __future__ import annotations

import argparse
import re
from dataclasses import dataclass
from pathlib import Path


FORM_RE = re.compile(r"<form\b(?P<attrs>[^>]*)>(?P<body>.*?)</form>", re.IGNORECASE | re.DOTALL)
POST_RE = re.compile(r"\bmethod\s*=\s*['\"]?post\b", re.IGNORECASE)
CSRF_RE = re.compile(r"\bname\s*=\s*['\"]csrf['\"]", re.IGNORECASE)


@dataclass(frozen=True)
class Finding:
    path: Path
    line: int
    message: str = 'POST form is missing name="csrf"'

    def display(self, root: Path) -> str:
        try:
            relative = self.path.relative_to(root)
        except ValueError:
            relative = self.path
        return f"{relative}:{self.line}: {self.message}"


def _normalized_markup(source: str) -> str:
    # Hand-rendered HTML in ordinary Rust strings contains escaped quotes.
    # Raw strings and Askama templates remain unchanged by this normalization.
    return source.replace(r'\"', '"')


def audit_file(path: Path) -> list[Finding]:
    source = path.read_text(encoding="utf-8")
    normalized = _normalized_markup(source)
    findings: list[Finding] = []
    for match in FORM_RE.finditer(normalized):
        if not POST_RE.search(match.group("attrs")):
            continue
        if CSRF_RE.search(match.group(0)):
            continue
        findings.append(Finding(path=path, line=normalized.count("\n", 0, match.start()) + 1))
    return findings


def audit_root(root: Path) -> list[Finding]:
    findings: list[Finding] = []
    for relative_root, suffix in ((Path("templates"), ".html"), (Path("src/routes"), ".rs")):
        source_root = root / relative_root
        if not source_root.exists():
            continue
        for path in sorted(source_root.rglob(f"*{suffix}")):
            findings.extend(audit_file(path))
    findings.extend(audit_markup_preview_javascript(root))
    return findings


def audit_markup_preview_javascript(root: Path) -> list[Finding]:
    """Check manually assembled preview requests that cannot inherit a form token."""
    findings: list[Finding] = []
    javascript_root = root / "static/js"
    if not javascript_root.exists():
        return findings
    fetch_re = re.compile(r"fetch\(\s*['\"]/?markup/preview['\"]")
    csrf_field_re = re.compile(r"(?:append|set)\(\s*['\"]csrf['\"]")
    for path in sorted(javascript_root.rglob("*.js")):
        source = path.read_text(encoding="utf-8")
        for match in fetch_re.finditer(source):
            # The body is assembled immediately before fetch in both the legacy
            # add-form port and the generic forms integration. Keep the window
            # narrow enough that an unrelated request cannot satisfy the check.
            request_setup = source[max(0, match.start() - 500) : match.start()]
            if csrf_field_re.search(request_setup):
                continue
            findings.append(
                Finding(
                    path=path,
                    line=source.count("\n", 0, match.start()) + 1,
                    message="POST /markup/preview does not add the csrf field",
                )
            )
    return findings


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", nargs="?", type=Path, default=Path.cwd())
    args = parser.parse_args()
    root = args.root.resolve()
    findings = audit_root(root)
    for finding in findings:
        print(finding.display(root))
    if findings:
        print(f"CSRF form audit failed: {len(findings)} form(s) without a token")
        return 1
    print("CSRF form audit passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
