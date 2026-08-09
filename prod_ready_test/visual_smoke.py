#!/usr/bin/env python3
"""Capture deterministic desktop/mobile screenshots of public fixture pages."""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path


PAGES = {
    "home": "/",
    "news-feed": "/news/",
    "articles-feed": "/articles/",
    "forum-index": "/forum/",
    "forum": "/forum/games/",
    "topic": "/forum/games/9101003",
    "comments-thread": "/news/russia/9101002",
    "gallery-feed": "/gallery/",
    "gallery-archive": "/gallery/archive/",
    "gallery-single": "/gallery/screenshots/9101005",
    "gallery-slider": "/gallery/screenshots/9101006",
    "gallery-queue": "/view-all.jsp?section=3",
    "poll-feed": "/polls/",
    "poll": "/polls/polls/9101007?results=true",
    "poll-queue": "/view-all.jsp?section=5",
    "article": "/articles/development/9101009",
    "profile": "/people/raven1000/profile",
    "tracker-login": "/tracker",
    "search": "/search.jsp",
    "tags": "/tags",
    "tag": "/tag/prod-ready",
    "login": "/login.jsp?from=/forum/",
    "register": "/register.jsp",
    "add-section": "/add-section.jsp",
    "not-found": "/definitely-not-found",
    "rules": "/help/rules.md",
}
EXPECTED_STATUS = {"not-found": 404}
VIEWPORTS = {"desktop": (1440, 1200), "mobile": (390, 844)}


def browser() -> str:
    for candidate in ("google-chrome", "google-chrome-stable", "chromium", "chromium-browser"):
        path = shutil.which(candidate)
        if path:
            return path
    raise RuntimeError("Chrome/Chromium is not installed")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:8181")
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("/tmp/prod_ready_test_artifacts"),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    executable = browser()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    for page_name, path in PAGES.items():
        url = args.base.rstrip("/") + path
        try:
            with urllib.request.urlopen(url, timeout=20) as response:
                status = response.status
        except urllib.error.HTTPError as error:
            status = error.code
        expected = EXPECTED_STATUS.get(page_name, 200)
        if status != expected:
            print(
                f"HTTP preflight failed for {page_name}: expected {expected}, got {status}",
                file=sys.stderr,
            )
            return 1
        print(f"HTTP {status} {page_name}")
    with tempfile.TemporaryDirectory(prefix="prod-ready-chrome-") as profile_root:
        for viewport_name, (width, height) in VIEWPORTS.items():
            for page_name, path in PAGES.items():
                destination = output / f"{page_name}-{viewport_name}.png"
                failure = "unknown browser failure"
                captured = False
                for attempt in range(2):
                    profile = Path(profile_root) / f"{page_name}-{viewport_name}-{attempt}"
                    command = [
                        executable,
                        "--headless=new",
                        "--no-sandbox",
                        "--no-first-run",
                        "--disable-background-networking",
                        "--disable-component-update",
                        "--disable-default-apps",
                        "--disable-dev-shm-usage",
                        "--disable-sync",
                        "--hide-scrollbars",
                        f"--user-data-dir={profile}",
                        f"--window-size={width},{height}",
                        f"--screenshot={destination}",
                        args.base.rstrip("/") + path,
                    ]
                    try:
                        result = subprocess.run(
                            command,
                            stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE,
                            timeout=60,
                        )
                    except subprocess.TimeoutExpired:
                        failure = f"browser timed out (attempt {attempt + 1}/2)"
                        continue
                    if result.returncode == 0 and destination.is_file():
                        captured = True
                        break
                    failure = result.stderr.decode(errors="replace")[-500:]
                if not captured:
                    print(
                        f"screenshot failed for {page_name}/{viewport_name}: {failure}",
                        file=sys.stderr,
                    )
                    return 1
                print(f"CAPTURED {destination}")
    print(f"visual artifacts: {output}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"visual smoke failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
