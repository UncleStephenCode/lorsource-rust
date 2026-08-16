#!/usr/bin/env bash
set -euo pipefail

sDir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
sDatabaseUrl="${JAVA_DATABASE_RUNTIME_URL:-postgres://linuxweb:linuxweb@localhost:5432/lor}"
sActual="$(mktemp)"
trap 'rm -f "$sActual"' EXIT

# This command is deliberately read-only. It is an exact reproducibility check
# for the vendored canonical bootstrap, not the application's compatibility
# policy: startup permits additional operator indexes/grants/ACL/owners and
# reports them as drift, while additional canonical-table constraints and
# enabled triggers are blocking.
psql "$sDatabaseUrl" \
  --no-psqlrc \
  --set ON_ERROR_STOP=1 \
  --tuples-only \
  --no-align \
  --field-separator=$'\t' \
  --file "$sDir/export-schema-objects.sql" >"$sActual"

if ! cmp --silent "$sDir/schema-objects-contract.tsv" "$sActual"; then
  diff --unified \
    --label expected/schema-objects-contract.tsv \
    --label actual/pg_catalog \
    "$sDir/schema-objects-contract.tsv" \
    "$sActual" || true
  echo "schema-object contract mismatch" >&2
  exit 1
fi

echo "Canonical schema-object contract matches pg_catalog (728 objects)."
