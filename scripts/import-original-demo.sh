#!/usr/bin/env bash
set -euo pipefail

sRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if test "$#" -gt 0 && test "$1" != "$sRoot/compat/java-db/sql/demo.db.gz" \
  && test "$1" != "compat/java-db/sql/demo.db.gz"; then
  echo "arbitrary dump imports are disabled; use the checksummed canonical Java bootstrap" >&2
  exit 2
fi

echo "This compatibility wrapper initializes only a missing/empty database and requires explicit confirmation." >&2
exec "$sRoot/compat/java-db/manage.sh" bootstrap
