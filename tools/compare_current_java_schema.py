#!/usr/bin/env python3
"""Compare current Java/Liquibase schema surface with Rust migrations.

This is a static inventory checker. It understands the original demo dump plus
common Liquibase create/add/drop/rename operations well enough to catch table and
column-name drift that breaks migration from the Java app to the Rust app.
"""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

DDL_SKIP = ("CONSTRAINT", "PRIMARY", "UNIQUE", "CHECK", "FOREIGN")


def _split_columns(body: str) -> list[str]:
    out: list[str] = []
    depth = 0
    cur: list[str] = []
    for ch in body:
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        if ch == "," and depth == 0:
            out.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    if cur:
        out.append("".join(cur))
    return out


def _add_create_table(schema: dict[str, set[str]], table: str, body: str) -> None:
    schema.setdefault(table, set())
    for raw in _split_columns(body):
        line = raw.strip().rstrip(",")
        if not line or line.upper().startswith(DDL_SKIP):
            continue
        col = line.split()[0].strip('"')
        if re.match(r"^[A-Za-z_][A-Za-z0-9_]*$", col):
            schema[table].add(col)


def parse_java_schema(root: Path) -> dict[str, set[str]]:
    schema: dict[str, set[str]] = {}
    dump = root / "sql" / "demo.db"
    if dump.exists():
        text = dump.read_text(errors="ignore")
        for m in re.finditer(r"CREATE TABLE ([A-Za-z0-9_]+) \((.*?)\);", text, re.S):
            table = m.group(1)
            if not table.startswith("jam_"):
                _add_create_table(schema, table, m.group(2))

    for path in sorted((root / "sql" / "updates").glob("*.xml")):
        text = path.read_text(errors="ignore")
        for rn in re.finditer(r'<renameTable[^>]*oldTableName="([^"]+)"[^>]*newTableName="([^"]+)"', text):
            old, new = rn.groups()
            schema[new] = schema.pop(old, set())
        for ct in re.finditer(r'<createTable[^>]*tableName="([^"]+)"[^>]*>(.*?)</createTable>', text, re.S):
            table = ct.group(1)
            schema.setdefault(table, set())
            for col in re.finditer(r'<column[^>]*name="([^"]+)"', ct.group(2)):
                schema[table].add(col.group(1))
        for ac in re.finditer(r'<addColumn[^>]*tableName="([^"]+)"[^>]*>(.*?)</addColumn>', text, re.S):
            table = ac.group(1)
            schema.setdefault(table, set())
            for col in re.finditer(r'<column[^>]*name="([^"]+)"', ac.group(2)):
                schema[table].add(col.group(1))
        for dc in re.finditer(r'<dropColumn[^>]*columnName="([^"]+)"[^>]*tableName="([^"]+)"', text):
            col, table = dc.groups()
            schema.setdefault(table, set()).discard(col)
        for m in re.finditer(r"create\s+table\s+(?:if\s+not\s+exists\s+)?([A-Za-z0-9_]+)\s*\((.*?)\)\s*;", text, re.I | re.S):
            _add_create_table(schema, m.group(1), m.group(2))
        for m in re.finditer(r"ALTER\s+TABLE\s+([A-Za-z0-9_]+)\s+ADD\s+COLUMN\s+(?:IF\s+NOT\s+EXISTS\s+)?([A-Za-z0-9_]+)", text, re.I):
            schema.setdefault(m.group(1), set()).add(m.group(2))
        for m in re.finditer(r"ALTER\s+TABLE\s+([A-Za-z0-9_]+)\s+DROP\s+COLUMN\s+(?:IF\s+EXISTS\s+)?([A-Za-z0-9_]+)", text, re.I):
            schema.setdefault(m.group(1), set()).discard(m.group(2))
    return {k: v for k, v in schema.items() if not k.startswith("jam_")}


def parse_rust_schema(migrations: Path) -> dict[str, set[str]]:
    schema: dict[str, set[str]] = {}
    for path in sorted(migrations.glob("*.sql")):
        text = path.read_text(errors="ignore")
        for m in re.finditer(r"CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?([A-Za-z0-9_]+)\s*\((.*?)\);", text, re.I | re.S):
            _add_create_table(schema, m.group(1), m.group(2))
        for m in re.finditer(r"ALTER\s+TABLE\s+([A-Za-z0-9_]+)\s+ADD\s+COLUMN\s+(?:IF\s+NOT\s+EXISTS\s+)?([A-Za-z0-9_]+)", text, re.I):
            schema.setdefault(m.group(1), set()).add(m.group(2))
        for m in re.finditer(r"ALTER\s+TABLE\s+([A-Za-z0-9_]+)\s+DROP\s+COLUMN\s+(?:IF\s+EXISTS\s+)?([A-Za-z0-9_]+)", text, re.I):
            schema.setdefault(m.group(1), set()).discard(m.group(2))
    return schema


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--java-root", type=Path, required=True)
    ap.add_argument("--migrations-dir", type=Path, default=Path("db/migrations"))
    ap.add_argument("--json", type=Path)
    ap.add_argument("--md", type=Path)
    args = ap.parse_args()

    java = parse_java_schema(args.java_root)
    rust = parse_rust_schema(args.migrations_dir)
    rows = []
    for table in sorted(set(java) | set(rust)):
        rows.append({
            "table": table,
            "status": "covered" if table in java and table in rust else ("rust-only" if table in rust else "missing"),
            "java_columns": sorted(java.get(table, set())),
            "rust_columns": sorted(rust.get(table, set())),
            "missing_columns": sorted(java.get(table, set()) - rust.get(table, set())),
            "extra_columns": sorted(rust.get(table, set()) - java.get(table, set())),
        })
    if args.json:
        args.json.write_text(json.dumps(rows, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if args.md:
        missing_tables = [r for r in rows if r["status"] == "missing"]
        missing_columns = [r for r in rows if r["missing_columns"]]
        lines = [
            "# Current Java schema compatibility report", "",
            f"Java tables without Rust migration table: **{len(missing_tables)}**",
            f"Java tables with missing Rust columns: **{len(missing_columns)}**", "",
            "| Status | Table | Missing Java columns in Rust | Extra Rust compatibility columns |",
            "|---|---|---|---|",
        ]
        for r in rows:
            lines.append(f"| {r['status']} | `{r['table']}` | `{', '.join(r['missing_columns'])}` | `{', '.join(r['extra_columns'])}` |")
        args.md.write_text("\n".join(lines) + "\n", encoding="utf-8")
    if not args.json and not args.md:
        print(json.dumps(rows, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
