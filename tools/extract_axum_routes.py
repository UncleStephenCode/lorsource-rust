#!/usr/bin/env python3
"""Extract Axum route declarations from the Rust port.

Besides inline ``get(...).post(...)`` expressions, the application keeps
several compatibility-sensitive method matrices in named ``MethodRouter``
builders.  Treating those calls as ``ANY`` hides accidental extra methods, so
the extractor resolves same-tree ``module::builder()`` calls as well.
"""
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


def capture_block(text: str, open_pos: int) -> tuple[str, int]:
    """Return a brace-delimited Rust block, ignoring braces in strings."""
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
            elif c == "{":
                depth += 1
            elif c == "}":
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


def method_router_builders(route_files: list[Path]) -> dict[tuple[str, str], str]:
    """Index public named MethodRouter builders by Rust module and function."""
    builders: dict[tuple[str, str], str] = {}
    declaration = re.compile(
        r"\bpub(?:\(crate\))?\s+fn\s+(\w+)\s*\([^)]*\)\s*"
        r"->\s*(?:[\w:]+::)?MethodRouter(?:\s*<[^>{}]+>)?\s*\{",
        flags=re.S,
    )
    for file in route_files:
        text = file.read_text(errors="ignore")
        for match in declaration.finditer(text):
            body, _end = capture_block(text, match.end() - 1)
            builders[(file.stem, match.group(1))] = body
    return builders


def resolved_methods(
    expr: str, builders: dict[tuple[str, str], str]
) -> list[str]:
    direct = methods(expr)
    if direct != ["ANY"]:
        return direct
    call = re.match(r"\s*(\w+)::(\w+)\s*\(\s*\)", expr)
    if call is None:
        return direct
    body = builders.get((call.group(1), call.group(2)))
    return methods(body) if body is not None else direct


def split_first_arg(body: str) -> tuple[str, str] | None:
    m = re.match(r'\s*"([^"]+)"\s*,\s*(.*)$', body, flags=re.S)
    if not m:
        return None
    return m.group(1), m.group(2).strip()


def extract_routes(root: Path) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    route_files = sorted((root / "src/routes").glob("*.rs"))
    builders = method_router_builders(route_files)
    for file in route_files:
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
                    "methods": resolved_methods(expr, builders),
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
