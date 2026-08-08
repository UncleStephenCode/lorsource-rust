#!/usr/bin/env python3
"""Extract table/column inventory from the original sql/demo.db PostgreSQL dump."""
from __future__ import annotations

import argparse
import csv
import json
import re
from pathlib import Path


def extract_schema(dump: Path) -> list[dict[str, object]]:
    text = dump.read_text(errors="ignore")
    tables = []
    for m in re.finditer(r"CREATE TABLE\s+([^\s(]+)\s*\((.*?)\n\);", text, flags=re.S):
        table = m.group(1).strip('"')
        body = m.group(2)
        cols = []
        for raw in body.splitlines():
            line = raw.strip().rstrip(',')
            if not line or line.upper().startswith(("CONSTRAINT ", "PRIMARY KEY", "UNIQUE ", "CHECK ", "FOREIGN KEY")):
                continue
            cm = re.match(r'"?([A-Za-z_][A-Za-z0-9_]*)"?\s+(.+)$', line)
            if not cm:
                continue
            name, rest = cm.group(1), cm.group(2)
            nullable = "NOT NULL" not in rest.upper()
            default = None
            dm = re.search(r"\s+DEFAULT\s+(.+?)(?:\s+NOT NULL|$)", rest, flags=re.I)
            if dm:
                default = dm.group(1).strip()
            coltype = re.split(r"\s+DEFAULT\s+|\s+NOT NULL\s*|\s+COLLATE\s+", rest, flags=re.I)[0].strip()
            cols.append({"name": name, "type": coltype, "nullable": nullable, "default": default})
        tables.append({"table": table, "columns": cols})
    return tables


def write_csv(tables: list[dict[str, object]], output: Path) -> None:
    with output.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(fh, fieldnames=["table", "column", "type", "nullable", "default"])
        writer.writeheader()
        for table in tables:
            for col in table["columns"]:
                writer.writerow({"table": table["table"], "column": col["name"], "type": col["type"], "nullable": col["nullable"], "default": col["default"]})


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("dump", type=Path)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--csv", type=Path)
    parser.add_argument("--md", type=Path)
    args = parser.parse_args()
    tables = extract_schema(args.dump)
    if args.json:
        args.json.write_text(json.dumps(tables, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if args.csv:
        write_csv(tables, args.csv)
    if args.md:
        lines = ["# Original demo DB schema inventory", "", f"Tables in `sql/demo.db`: **{len(tables)}**", ""]
        for table in tables:
            lines += [f"## `{table['table']}`", "", "| Column | Type | Nullable | Default |", "|---|---|---:|---|"]
            for col in table["columns"]:
                lines.append(f"| `{col['name']}` | `{col['type']}` | `{col['nullable']}` | `{col['default'] or ''}` |")
            lines.append("")
        args.md.write_text("\n".join(lines), encoding="utf-8")
    if not any([args.json, args.csv, args.md]):
        print(json.dumps(tables, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
