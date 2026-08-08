#!/usr/bin/env bash
set -euo pipefail

sRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

"$sRoot/compat/java-db/check-vendor.sh"
bash -n \
  "$sRoot/compat/java-db/check-vendor.sh" \
  "$sRoot/compat/java-db/manage.sh" \
  "$sRoot/.devcontainer/init-db.sh" \
  "$sRoot/scripts/import-original-demo.sh"

test ! -d "$sRoot/db/migrations"
test -d "$sRoot/compat/legacy-rust-db/offline-sql"
test "$(find "$sRoot/compat/legacy-rust-db/offline-sql" -type f -name '*.sql' | wc -l)" -eq 19

if rg --fixed-strings 'sqlx::migrate!' "$sRoot/src" >/dev/null; then
  echo "compile-time SQLx migration entrypoint is forbidden" >&2
  exit 1
fi
if rg --fixed-strings 'RUN_MIGRATIONS' \
  "$sRoot/src" "$sRoot/Dockerfile" "$sRoot/docker-compose.yml" \
  "$sRoot/docker-compose.dev.yml" "$sRoot/.devcontainer" >/dev/null; then
  echo "runtime migration switch is forbidden" >&2
  exit 1
fi

echo "Database workflow static checks passed."
