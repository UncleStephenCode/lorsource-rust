#!/usr/bin/env python3
"""Seed the disposable stack, run regressions and optionally take screenshots."""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


HERE = Path(__file__).resolve().parent
CONFIRMATION = "seed-disposable-compose-lor"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:8181")
    parser.add_argument("--visual", action="store_true")
    parser.add_argument("--start", action="store_true")
    parser.add_argument(
        "--browser-seed",
        action="store_true",
        help="seed accounts only, then create and verify content through Playwright",
    )
    parser.add_argument(
        "--seven-day-benchmark",
        action="store_true",
        help="load seven days of history, run HTTP tests, browser activities and load benchmark",
    )
    args = parser.parse_args()

    if args.browser_seed and args.seven_day_benchmark:
        parser.error("--browser-seed and --seven-day-benchmark are mutually exclusive")

    seed = [
        sys.executable,
        str(HERE / "seed.py"),
        "--confirm",
        CONFIRMATION,
    ]
    if args.start:
        seed.append("--start")
    if args.browser_seed:
        seed.append("--accounts-only")
    subprocess.run(seed, check=True)
    if args.browser_seed:
        subprocess.run(
            [
                sys.executable,
                str(HERE / "browser_seed.py"),
                "--base",
                args.base,
                "--restart",
            ],
            check=True,
        )
        return 0
    subprocess.run(
        [sys.executable, str(HERE / "test_port.py"), "--base", args.base],
        check=True,
    )
    if args.seven_day_benchmark:
        subprocess.run(
            [
                sys.executable,
                str(HERE / "browser_seed.py"),
                "--base",
                args.base,
                "--restart",
            ],
            check=True,
        )
        subprocess.run(
            [
                sys.executable,
                str(HERE / "benchmark_verify.py"),
                "--base",
                args.base,
            ],
            check=True,
        )
    if args.visual:
        subprocess.run(
            [sys.executable, str(HERE / "visual_smoke.py"), "--base", args.base],
            check=True,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
