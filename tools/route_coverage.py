#!/usr/bin/env python3
"""Compare Spring and Axum route declarations by normalized path and method.

This report is structural evidence only.  It does not compare parameters,
headers, content negotiation, authentication, responses, database effects or
rendered UI, and therefore must never be presented as semantic parity.
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


def method_relation(original_methods: list[str], rust_methods: list[str]) -> str:
    """Return full/partial/none support for the original declared methods.

    Spring ``ANY`` means unrestricted methods.  A Rust GET declaration is not
    silently treated as full support for that surface.  Likewise, an explicit
    Spring GET+HEAD mapping is only full when both are visible in the inventory.
    Runtime framework behavior (such as implicit HEAD handling) belongs in an
    HTTP compatibility test rather than this source comparison.
    """
    if "ANY" in rust_methods:
        return "full"
    if "ANY" in original_methods:
        return "partial" if rust_methods else "none"
    original = set(original_methods)
    rust = set(rust_methods)
    if original.issubset(rust):
        return "full"
    return "partial" if original & rust else "none"



def expand_original_paths(path: str) -> list[str]:
    sections = ["forum", "news", "polls", "articles", "gallery"]
    variants = [path.replace("/page{page}", "/{page}")]
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
        relations = [(candidate, method_relation(row["methods"], candidate["methods"])) for candidate in candidates]
        matched = [candidate for candidate, relation in relations if relation == "full"]
        partial = [candidate for candidate, relation in relations if relation == "partial"]
        shown = matched or partial or candidates
        rows.append(
            {
                "status": (
                    "method-declared" if matched else
                    "partial-method" if partial else
                    "path-only" if candidates else
                    "missing"
                ),
                "methods": ",".join(row["methods"]),
                "path": row["path"],
                "params": ",".join(row["params"]),
                "headers": ",".join(row.get("headers", [])),
                "consumes": ",".join(row.get("consumes", [])),
                "produces": ",".join(row.get("produces", [])),
                "controller": row["controller"],
                "handler": row["handler"],
                "rust_match": "; ".join(f"{','.join(r['methods'])} {r['path']}" for r in shown),
                "source": f"{row['source']}:{row['line']}",
                "semantic_parity": "not-evaluated",
            }
        )

    if args.json:
        args.json.write_text(json.dumps(rows, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if args.csv:
        with args.csv.open("w", newline="", encoding="utf-8") as fh:
            writer = csv.DictWriter(
                fh, fieldnames=list(rows[0].keys()), lineterminator="\n"
            )
            writer.writeheader(); writer.writerows(rows)
    if args.md:
        declared = sum(1 for r in rows if r["status"] == "method-declared")
        partial = sum(1 for r in rows if r["status"] == "partial-method")
        path_only = sum(1 for r in rows if r["status"] == "path-only")
        missing = sum(1 for r in rows if r["status"] == "missing")
        lines = [
            "# Structural route declaration comparison",
            "",
            "> This is not a semantic parity report. It compares normalized path templates and declared methods only. It does not verify request parameters, headers/content negotiation, authentication/authorization, status/redirects, HTML, database changes or side effects.",
            "",
            f"Expanded original Spring mapping variants: **{len(rows)}**",
            f"Path and all declared methods present in a Rust declaration: **{declared}**",
            f"Path present with only partial/unrestricted-method overlap: **{partial}**",
            f"Path exists but method differs: **{path_only}**",
            f"Missing route declaration: **{missing}**",
            "",
            "Spring `ANY` mappings are intentionally reported as partial unless the Rust inventory also declares `ANY`. Extra Rust methods and Axum's runtime HEAD behavior are not evaluated here.",
            "",
            "| Structural status | Methods | Original path | Mapping conditions | Controller.handler | Rust declaration |",
            "|---|---|---|---|---|---|",
        ]
        for r in rows:
            conditions = "; ".join(
                value for value in (
                    f"params={r['params']}" if r["params"] else "",
                    f"headers={r['headers']}" if r["headers"] else "",
                    f"consumes={r['consumes']}" if r["consumes"] else "",
                    f"produces={r['produces']}" if r["produces"] else "",
                ) if value
            )
            lines.append(
                f"| {r['status']} | `{r['methods']}` | `{r['path']}` | `{conditions}` | `{r['controller']}.{r['handler']}` | `{r['rust_match']}` |"
            )
        args.md.write_text("\n".join(lines) + "\n", encoding="utf-8")
    if not any([args.json, args.csv, args.md]):
        print(json.dumps(rows, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
