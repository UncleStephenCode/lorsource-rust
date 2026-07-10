#!/usr/bin/env bash
set -euo pipefail

# Optional helper. It imports the old PostgreSQL dump from the Scala project
# into the database used by the Rust port. Accepts plain .sql/.db or .gz.
: "${DATABASE_URL:=postgres://lor:lor@localhost:5432/lor}"
DUMP=${1:-sql/demo.db.gz}

if ! command -v psql >/dev/null; then
  echo "psql is required" >&2
  exit 1
fi

case "$DUMP" in
  *.gz) gzip -dc "$DUMP" | psql "$DATABASE_URL" -v ON_ERROR_STOP=1 ;;
  *)    psql "$DATABASE_URL" -v ON_ERROR_STOP=1 < "$DUMP" ;;
esac
