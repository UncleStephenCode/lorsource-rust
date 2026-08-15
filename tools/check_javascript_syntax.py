#!/usr/bin/env python3
"""Run ``node --check`` for every tracked JavaScript file under static/js."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path


def tracked_javascript_files(root: Path) -> list[Path]:
    """Return repository-relative tracked ``static/js/**/*.js`` paths.

    Git emits NUL-delimited bytes, so whitespace, quotes and newlines in a
    tracked filename never become shell syntax or split into multiple paths.
    """

    result = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z", "--", "static/js"],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        message = result.stderr.decode(errors="replace").strip()
        raise RuntimeError(f"git ls-files failed: {message}")

    files: list[Path] = []
    for raw_path in result.stdout.split(b"\0"):
        if not raw_path:
            continue
        relative = Path(os.fsdecode(raw_path))
        if relative.suffix == ".js" and relative.parts[:2] == ("static", "js"):
            files.append(relative)
    return sorted(files)


def check_javascript_files(root: Path, node: str, files: list[Path]) -> int:
    failures: list[Path] = []
    for relative in files:
        try:
            result = subprocess.run(
                [node, "--check", os.fspath(relative)],
                cwd=root,
                check=False,
            )
        except FileNotFoundError as error:
            raise RuntimeError(f"JavaScript runtime is unavailable: {node}") from error
        if result.returncode != 0:
            failures.append(relative)

    if failures:
        print(
            "JavaScript syntax check failed for: "
            + ", ".join(os.fspath(path) for path in failures),
            file=sys.stderr,
        )
        return 1
    print(f"JavaScript syntax check passed ({len(files)} tracked files).")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="repository root (defaults to the parent of tools/)",
    )
    parser.add_argument("--node", default="node", help="Node.js executable")
    args = parser.parse_args()

    try:
        files = tracked_javascript_files(args.root.resolve())
        if not files:
            raise RuntimeError("no tracked JavaScript files found under static/js")
        return check_javascript_files(args.root.resolve(), args.node, files)
    except RuntimeError as error:
        print(f"JavaScript syntax check failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
