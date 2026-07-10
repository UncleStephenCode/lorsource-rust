#!/usr/bin/env python3
"""Compare original demo dump schema inventory with Rust migration DDL inventory."""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


def parse_create_tables(text: str) -> dict[str, set[str]]:
    out: dict[str, set[str]] = {}
    for m in re.finditer(r"CREATE TABLE(?: IF NOT EXISTS)?\s+([^\s(]+)\s*\((.*?)\n\);", text, flags=re.S | re.I):
        table = m.group(1).strip('\"')
        cols = set()
        for raw in m.group(2).splitlines():
            line = raw.strip().rstrip(',')
            if not line or line.upper().startswith(("CONSTRAINT ", "PRIMARY KEY", "UNIQUE ", "CHECK ", "FOREIGN KEY")):
                continue
            cm = re.match(r'\"?([A-Za-z_][A-Za-z0-9_]*)\"?\s+', line)
            if cm:
                cols.add(cm.group(1))
        out[table] = cols
    # Account for additive compatibility migrations. This is static inventory,
    # not a SQL executor, so we only need table + column names.
    for m in re.finditer(r"ALTER TABLE\s+([^\s]+)\s+ADD COLUMN IF NOT EXISTS\s+\"?([A-Za-z_][A-Za-z0-9_]*)\"?\s+", text, flags=re.I):
        out.setdefault(m.group(1).strip('\"'), set()).add(m.group(2))
    return out


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--original-json", type=Path, required=True)
    parser.add_argument("--migrations-dir", type=Path, required=True)
    parser.add_argument("--md", type=Path)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    original_list = json.loads(args.original_json.read_text(encoding="utf-8"))
    original = {t["table"]: {c["name"] for c in t["columns"]} for t in original_list}
    migration_text = "\n".join(p.read_text(errors="ignore") for p in sorted(args.migrations_dir.glob("*.sql")))
    rust = parse_create_tables(migration_text)
    rows = []
    for table in sorted(set(original) | set(rust)):
        if table.startswith("jam_") and table in original and table not in rust:
            status = "dropped-upstream"
        else:
            status = "covered" if table in original and table in rust else ("rust-only" if table in rust else "missing")
        rows.append({
            "table": table,
            "status": status,
            "original_columns": sorted(original.get(table, set())),
            "rust_columns": sorted(rust.get(table, set())),
            "missing_columns": sorted(original.get(table, set()) - rust.get(table, set())),
            "extra_columns": sorted(rust.get(table, set()) - original.get(table, set())),
        })
    if args.json:
        args.json.write_text(json.dumps(rows, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if args.md:
        covered = sum(r["status"] == "covered" for r in rows)
        missing = sum(r["status"] == "missing" for r in rows)
        rust_only = sum(r["status"] == "rust-only" for r in rows)
        dropped = sum(r["status"] == "dropped-upstream" for r in rows)
        lines = ["# Schema coverage report", "", f"Tables covered: **{covered}**", f"Missing original tables: **{missing}**", f"Rust-only/current-update tables: **{rust_only}**", f"Dropped upstream legacy tables: **{dropped}**", "", "| Status | Table | Missing columns from Rust migration | Extra Rust columns |", "|---|---|---|---|"]
        for r in rows:
            lines.append(f"| {r['status']} | `{r['table']}` | `{', '.join(r['missing_columns'])}` | `{', '.join(r['extra_columns'])}` |")
        args.md.write_text("\n".join(lines) + "\n", encoding="utf-8")
    if not args.json and not args.md:
        print(json.dumps(rows, ensure_ascii=False, indent=2))

if __name__ == "__main__":
    main()
