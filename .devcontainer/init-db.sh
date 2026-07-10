#!/usr/bin/env bash
set -euo pipefail

PGHOST="${PGHOST:-postgres}"
PGUSER="${PGUSER:-lor}"
PGPASSWORD="${PGPASSWORD:-lor}"
DATABASE_URL="${DATABASE_URL:-postgres://lor:lor@postgres:5432/lor}"
export PGHOST PGUSER PGPASSWORD DATABASE_URL

printf 'Waiting for PostgreSQL at %s...\n' "$PGHOST"
until pg_isready -h "$PGHOST" -U "$PGUSER" -d lor >/dev/null 2>&1; do
  sleep 1
done

printf 'Ensuring PostgreSQL extensions...\n'
psql "$DATABASE_URL" -v ON_ERROR_STOP=1 <<'SQL'
CREATE EXTENSION IF NOT EXISTS hstore;
CREATE EXTENSION IF NOT EXISTS fuzzystrmatch;
SQL

if command -v sqlx >/dev/null 2>&1; then
  printf 'Running sqlx migrations...\n'
  sqlx migrate run --source /workspace/db/migrations
else
  printf 'sqlx-cli not found, running migrations with psql fallback...\n'
  for f in /workspace/db/migrations/*.sql; do
    printf 'Applying %s\n' "$f"
    psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f "$f"
  done
fi

mkdir -p /workspace/uploads/photos /workspace/uploads/images /workspace/uploads/gallery/preview
printf 'Rust port devcontainer database initialization complete.\n'
