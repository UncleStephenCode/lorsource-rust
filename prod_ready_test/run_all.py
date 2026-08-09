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
    args = parser.parse_args()

    seed = [
        sys.executable,
        str(HERE / "seed.py"),
        "--confirm",
        CONFIRMATION,
    ]
    if args.start:
        seed.append("--start")
    subprocess.run(seed, check=True)
    subprocess.run(
        [sys.executable, str(HERE / "test_port.py"), "--base", args.base],
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
