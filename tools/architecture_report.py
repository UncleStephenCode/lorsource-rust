#!/usr/bin/env python3
from __future__ import annotations

import json
import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"

PAT_SQL = re.compile(r"sqlx::query(?:_as|_scalar)?|query_as::<|query_scalar\(")
PAT_STRUCT = re.compile(r"^\s*pub\s+struct\s+(\w+)", re.M)
PAT_FN = re.compile(r"^\s*pub\s+(?:async\s+)?fn\s+(\w+)", re.M)


def count_sql(path: Path) -> int:
    return len(PAT_SQL.findall(path.read_text(errors="ignore")))


def main() -> None:
    toolchain = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())
    rust_version = toolchain["toolchain"]["channel"]
    files = sorted(SRC.rglob("*.rs"))
    sql_by_area: dict[str, int] = {}
    direct_route_sql: list[dict[str, object]] = []
    structs: list[str] = []
    public_fns: list[str] = []

    for file in files:
        rel = file.relative_to(ROOT).as_posix()
        text = file.read_text(errors="ignore")
        area = rel.split("/")[1] if rel.startswith("src/") and "/" in rel[4:] else "root"
        n_sql = count_sql(file)
        sql_by_area[area] = sql_by_area.get(area, 0) + n_sql
        if rel.startswith("src/routes/") and n_sql:
            direct_route_sql.append({"file": rel, "sql_calls": n_sql})
        structs.extend(PAT_STRUCT.findall(text))
        public_fns.extend(PAT_FN.findall(text))

    report = {
        "rust_edition": "2024",
        "rust_version": rust_version,
        "architecture_layers": ["bootstrap", "config", "domain", "application", "infra", "routes"],
        "sql_calls_by_area": sql_by_area,
        "direct_sql_in_routes": direct_route_sql,
        "hungarian_structs": sorted([s for s in structs if s.startswith(("St", "C", "Tr", "Ty"))]),
        "non_hungarian_public_structs": sorted([s for s in structs if not s.startswith(("St", "C", "Tr", "Ty"))]),
        "hungarian_public_methods": sorted([f for f in public_fns if re.match(r"^(st|vec|opt|v|i|b|s|o)[A-Z_].*", f)]),
    }
    out = ROOT / "docs" / "generated" / "architecture_report_v9.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
