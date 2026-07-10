#!/usr/bin/env python3
"""Build a route coverage report: original Spring endpoint -> Rust/Axum status.

The matching is intentionally conservative: exact static paths are matched
exactly; parameterized paths are normalized to a comparable shape.
"""
from __future__ import annotations

import argparse
import csv
import json
import re
from pathlib import Path


def route_shape(path: str) -> str:
    path = path.strip() or "/"
    parts = []
    for part in path.strip("/").split("/"):
        if not part:
            continue
        if part.startswith("{") and part.endswith("}"):
            parts.append("{}")
        elif part.startswith(":"):
            parts.append("{}")
        elif re.fullmatch(r"page\{[^}]+\}", part):
            parts.append("page{}")
        elif part.startswith("page:"):
            parts.append("page{}")
        else:
            parts.append(part)
    return "/" + "/".join(parts) if parts else "/"


def method_match(original_methods: list[str], rust_methods: list[str]) -> bool:
    if "ANY" in original_methods or "ANY" in rust_methods:
        return True
    return bool(set(original_methods) & set(rust_methods))



def expand_original_paths(path: str) -> list[str]:
    sections = ["forum", "news", "polls", "articles", "gallery"]
    variants = [path.replace("/page{page}", "/page/{page}")]
    if "{section}" not in path:
        return variants
    return [variant.replace("{section}", section) for variant in variants for section in sections]

def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--original", type=Path, required=True)
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--csv", type=Path)
    parser.add_argument("--md", type=Path)
    args = parser.parse_args()

    original = json.loads(args.original.read_text(encoding="utf-8"))
    rust = json.loads(args.rust.read_text(encoding="utf-8"))
    rust_by_shape: dict[str, list[dict]] = {}
    for row in rust:
        rust_by_shape.setdefault(route_shape(row["path"]), []).append(row)

    rows = []
    for row in original:
        shapes = [route_shape(p) for p in expand_original_paths(row["path"])]
        candidates = []
        for shape in shapes:
            candidates.extend(rust_by_shape.get(shape, []))
        matched = [r for r in candidates if method_match(row["methods"], r["methods"])]
        rows.append(
            {
                "status": "covered" if matched else ("path-only" if candidates else "missing"),
                "methods": ",".join(row["methods"]),
                "path": row["path"],
                "params": ",".join(row["params"]),
                "controller": row["controller"],
                "handler": row["handler"],
                "rust_match": "; ".join(f"{','.join(r['methods'])} {r['path']}" for r in (matched or candidates)),
                "source": f"{row['source']}:{row['line']}",
            }
        )

    if args.json:
        args.json.write_text(json.dumps(rows, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if args.csv:
        with args.csv.open("w", newline="", encoding="utf-8") as fh:
            writer = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
            writer.writeheader(); writer.writerows(rows)
    if args.md:
        covered = sum(1 for r in rows if r["status"] == "covered")
        path_only = sum(1 for r in rows if r["status"] == "path-only")
        missing = sum(1 for r in rows if r["status"] == "missing")
        lines = [
            "# Route coverage report",
            "",
            f"Original routes: **{len(rows)}**",
            f"Covered by Rust route declaration: **{covered}**",
            f"Path exists but method differs: **{path_only}**",
            f"Missing route declaration: **{missing}**",
            "",
            "| Status | Methods | Original path | Controller.handler | Rust match |",
            "|---|---|---|---|---|",
        ]
        for r in rows:
            lines.append(
                f"| {r['status']} | `{r['methods']}` | `{r['path']}` | `{r['controller']}.{r['handler']}` | `{r['rust_match']}` |"
            )
        args.md.write_text("\n".join(lines) + "\n", encoding="utf-8")
    if not any([args.json, args.csv, args.md]):
        print(json.dumps(rows, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
