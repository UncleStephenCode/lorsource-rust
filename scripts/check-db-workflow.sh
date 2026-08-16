#!/usr/bin/env bash
set -euo pipefail

sRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

"$sRoot/compat/java-db/check-vendor.sh"
bash -n \
  "$sRoot/compat/java-db/check-vendor.sh" \
  "$sRoot/compat/java-db/check-schema-object-contract.sh" \
  "$sRoot/compat/java-db/manage.sh" \
  "$sRoot/.devcontainer/init-db.sh" \
  "$sRoot/scripts/import-original-demo.sh"

test ! -d "$sRoot/db/migrations"
test -d "$sRoot/compat/legacy-rust-db/offline-sql"
test "$(find "$sRoot/compat/legacy-rust-db/offline-sql" -type f -name '*.sql' | wc -l)" -eq 19

if grep --recursive --fixed-strings 'sqlx::migrate!' "$sRoot/src" >/dev/null; then
  echo "compile-time SQLx migration entrypoint is forbidden" >&2
  exit 1
fi
if grep --recursive --fixed-strings 'RUN_MIGRATIONS' \
  "$sRoot/src" "$sRoot/Dockerfile" "$sRoot/docker-compose.yml" \
  "$sRoot/docker-compose.dev.yml" "$sRoot/.devcontainer" >/dev/null; then
  echo "runtime migration switch is forbidden" >&2
  exit 1
fi

grep --fixed-strings --quiet 'relrowsecurity' \
  "$sRoot/compat/java-db/export-schema-objects.sql"
grep --fixed-strings --quiet 'relforcerowsecurity' \
  "$sRoot/compat/java-db/export-schema-objects.sql"
grep --fixed-strings --quiet "pg_catalog.has_table_privilege('linuxweb'" \
  "$sRoot/compat/java-db/export-schema-objects.sql"
grep --fixed-strings --quiet "pg_catalog.has_sequence_privilege('linuxweb'" \
  "$sRoot/compat/java-db/export-schema-objects.sql"
grep --fixed-strings --quiet "pg_catalog.has_function_privilege('linuxweb'" \
  "$sRoot/compat/java-db/export-schema-objects.sql"
grep --fixed-strings --quiet 'pg_catalog.has_schema_privilege' \
  "$sRoot/compat/java-db/export-schema-objects.sql"
grep --fixed-strings --quiet 'pg_catalog.has_type_privilege' \
  "$sRoot/compat/java-db/export-schema-objects.sql"

if grep --extended-regexp --ignore-case \
  '(^|[^[:alnum:]_])(insert|update|delete|alter|drop|truncate|setval|nextval)([^[:alnum:]_]|$)' \
  "$sRoot/compat/java-db/check-sequence-headroom.sql" >/dev/null; then
  echo "sequence headroom validator must remain read-only" >&2
  exit 1
fi

echo "Database workflow static checks passed."
