#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ORIGINAL_ROOT=${ORIGINAL_ROOT:-}

if [[ -n "$ORIGINAL_ROOT" ]]; then
  python3 "$ROOT/tools/extract_original_routes.py" "$ORIGINAL_ROOT" \
    --json "$ROOT/docs/generated/original_routes.json" \
    --csv "$ROOT/docs/generated/original_routes.csv" \
    --surface-json "$ROOT/docs/generated/original_surface.json" \
    --surface-csv "$ROOT/docs/generated/original_surface.csv" \
    --summary-md "$ROOT/docs/CONTROLLER_MAP.md"
else
  echo "ORIGINAL_ROOT is not set; using already generated original route inventory" >&2
fi
if [[ -n "$ORIGINAL_ROOT" ]]; then
  LORSOURCE_JAVA_ROOT="$ORIGINAL_ROOT" "$ROOT/scripts/check-db-workflow.sh"
else
  "$ROOT/scripts/check-db-workflow.sh"
fi

python3 "$ROOT/tools/extract_axum_routes.py" "$ROOT" \
  --json "$ROOT/docs/generated/rust_routes.json" \
  --csv "$ROOT/docs/generated/rust_routes.csv"
python3 "$ROOT/tools/route_coverage.py" \
  --original "$ROOT/docs/generated/original_routes.json" \
  --rust "$ROOT/docs/generated/rust_routes.json" \
  --json "$ROOT/docs/generated/route_coverage.json" \
  --csv "$ROOT/docs/generated/route_coverage.csv" \
  --md "$ROOT/docs/ROUTE_COVERAGE.md"
python3 "$ROOT/tools/audit_rust_sql_schema.py" "$ROOT" \
  --schema-contract "$ROOT/compat/java-db/schema-contract.tsv" \
  --java-sql-root "$ROOT/compat/java-db/sql" \
  --json "$ROOT/docs/generated/rust_sql_schema_audit.json" \
  --csv "$ROOT/docs/generated/rust_sql_schema_audit.csv" \
  --md "$ROOT/docs/RUST_SQL_SCHEMA_AUDIT.md"
if [[ "${RUN_HTTP_COMPAT:-0}" == "1" ]]; then
  python3 "$ROOT/compat/test_http_compat.py"
fi

echo "Route inventories, canonical Java DB vendor and static Rust SQL identifiers verified; none alone establishes semantic parity."
