#!/usr/bin/env python3
"""Extract .route("...", get(...).post(...)) declarations from the Rust port."""
from __future__ import annotations

import argparse
import csv
import json
import re
from pathlib import Path


def capture_call(text: str, open_pos: int) -> tuple[str, int]:
    depth = 0
    quote = False
    esc = False
    for i in range(open_pos, len(text)):
        c = text[i]
        if quote:
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                quote = False
        else:
            if c == '"':
                quote = True
            elif c == '(':
                depth += 1
            elif c == ')':
                depth -= 1
                if depth == 0:
                    return text[open_pos + 1 : i], i + 1
    return text[open_pos + 1 :], len(text)


def methods(expr: str) -> list[str]:
    result = []
    for name, verb in [("get", "GET"), ("post", "POST"), ("put", "PUT"), ("delete", "DELETE"), ("patch", "PATCH")]:
        if re.search(rf"\b{name}\s*\(", expr) or re.search(rf"\.{name}\s*\(", expr):
            result.append(verb)
    return result or ["ANY"]


def split_first_arg(body: str) -> tuple[str, str] | None:
    m = re.match(r'\s*"([^"]+)"\s*,\s*(.*)$', body, flags=re.S)
    if not m:
        return None
    return m.group(1), m.group(2).strip()


def extract_routes(root: Path) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    for file in sorted((root / "src/routes").glob("*.rs")):
        text = file.read_text(errors="ignore")
        for m in re.finditer(r"\.route\s*\(", text):
            body, _end = capture_call(text, m.end() - 1)
            parsed = split_first_arg(body)
            if not parsed:
                continue
            path, expr = parsed
            rows.append(
                {
                    "path": path,
                    "methods": methods(expr),
                    "handler": re.sub(r"\s+", " ", expr),
                    "source": str(file.relative_to(root)),
                    "line": text.count("\n", 0, m.start()) + 1,
                }
            )
    return rows


def write_csv(rows: list[dict[str, object]], output: Path) -> None:
    with output.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(
            fh,
            fieldnames=["methods", "path", "handler", "source", "line"],
            lineterminator="\n",
        )
        writer.writeheader()
        for row in rows:
            writer.writerow({**row, "methods": ",".join(row["methods"])})


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path, nargs="?", default=Path.cwd())
    parser.add_argument("--json", type=Path)
    parser.add_argument("--csv", type=Path)
    args = parser.parse_args()
    routes = extract_routes(args.root)
    if args.json:
        args.json.write_text(json.dumps(routes, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if args.csv:
        write_csv(routes, args.csv)
    if not args.json and not args.csv:
        print(json.dumps(routes, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
