#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ORIGINAL_ROOT=${ORIGINAL_ROOT:-}

if [[ -n "$ORIGINAL_ROOT" ]]; then
  python3 "$ROOT/tools/extract_original_routes.py" "$ORIGINAL_ROOT" \
    --json "$ROOT/docs/generated/original_routes.json" \
    --csv "$ROOT/docs/generated/original_routes.csv"
else
  echo "ORIGINAL_ROOT is not set; using already generated original route inventory" >&2
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
if [[ -n "$ORIGINAL_ROOT" ]]; then
  python3 "$ROOT/tools/compare_current_java_schema.py" \
    --java-root "$ORIGINAL_ROOT" \
    --migrations-dir "$ROOT/db/migrations" \
    --json "$ROOT/docs/generated/current_java_schema_coverage.json" \
    --md "$ROOT/docs/CURRENT_JAVA_SCHEMA_COVERAGE.md"
  cp "$ROOT/docs/CURRENT_JAVA_SCHEMA_COVERAGE.md" "$ROOT/docs/SCHEMA_COVERAGE.md"
else
  python3 "$ROOT/tools/compare_schema_inventory.py" \
    --original-json "$ROOT/docs/generated/original_demo_schema.json" \
    --migrations-dir "$ROOT/db/migrations" \
    --json "$ROOT/docs/generated/schema_coverage.json" \
    --md "$ROOT/docs/SCHEMA_COVERAGE.md"
fi

if [[ "${RUN_HTTP_COMPAT:-0}" == "1" ]]; then
  python3 "$ROOT/compat/test_http_compat.py"
fi

echo "Compatibility reports regenerated under docs/ and docs/generated/"
