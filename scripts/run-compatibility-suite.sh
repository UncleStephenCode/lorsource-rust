#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ORIGINAL_ROOT=${ORIGINAL_ROOT:-}

if [[ -n "$ORIGINAL_ROOT" ]]; then
  "$ROOT/tools/extract_original_routes.py" "$ORIGINAL_ROOT" \
    --json "$ROOT/docs/generated/original_routes.json" \
    --csv "$ROOT/docs/generated/original_routes.csv"
else
  echo "ORIGINAL_ROOT is not set; using already generated original route inventory" >&2
fi

"$ROOT/tools/extract_axum_routes.py" "$ROOT" \
  --json "$ROOT/docs/generated/rust_routes.json" \
  --csv "$ROOT/docs/generated/rust_routes.csv"
"$ROOT/tools/route_coverage.py" \
  --original "$ROOT/docs/generated/original_routes.json" \
  --rust "$ROOT/docs/generated/rust_routes.json" \
  --json "$ROOT/docs/generated/route_coverage.json" \
  --csv "$ROOT/docs/generated/route_coverage.csv" \
  --md "$ROOT/docs/ROUTE_COVERAGE.md"
"$ROOT/tools/compare_schema_inventory.py" \
  --original-json "$ROOT/docs/generated/original_demo_schema.json" \
  --migrations-dir "$ROOT/db/migrations" \
  --json "$ROOT/docs/generated/schema_coverage.json" \
  --md "$ROOT/docs/SCHEMA_COVERAGE.md"

if [[ "${RUN_HTTP_COMPAT:-0}" == "1" ]]; then
  python3 "$ROOT/compat/test_http_compat.py"
fi

echo "Compatibility reports regenerated under docs/ and docs/generated/"
