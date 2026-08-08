#!/usr/bin/env bash
set -euo pipefail

export PG_ADMIN_URL="${PG_ADMIN_URL:-postgres://postgres:postgres@postgres:5432/postgres}"
export PG_TARGET_ADMIN_URL="${PG_TARGET_ADMIN_URL:-postgres://postgres:postgres@postgres:5432/lor}"
export JAVA_DATABASE_MIGRATION_URL="${JAVA_DATABASE_MIGRATION_URL:-postgres://maxcom:maxcom@postgres:5432/lor}"
export JAVA_DATABASE_JDBC_URL="${JAVA_DATABASE_JDBC_URL:-jdbc:postgresql://postgres:5432/lor}"
export JAVA_DATABASE_RUNTIME_URL="${JAVA_DATABASE_RUNTIME_URL:-postgres://linuxweb:linuxweb@postgres:5432/lor}"
export LOR_DB_BOOTSTRAP_CONFIRM=bootstrap-empty-java-db

/workspace/compat/java-db/manage.sh bootstrap

mkdir -p \
  /workspace/uploads/photos \
  /workspace/uploads/images \
  /workspace/uploads/gallery/preview
printf 'Canonical Java-compatible devcontainer database is ready.\n'
