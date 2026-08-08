#!/usr/bin/env bash
set -euo pipefail

sDir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$sDir"

sha256sum --check --strict --quiet checksums.sha256

sDemoHash="$(gzip --decompress --stdout sql/demo.db.gz | sha256sum | awk '{print $1}')"
test "$sDemoHash" = "9871b6148f59786da7cd00e8601931869129ec1968035cb8a6c2a1e3592a038e"

iUpdateFiles="$(find sql/updates -maxdepth 1 -type f -name '*.xml' | wc -l)"
test "$iUpdateFiles" -eq 116

iChangesets="$(grep --with-filename '<changeSet' sql/updates/*.xml | wc -l)"
test "$iChangesets" -eq 187

grep --fixed-strings --quiet '<includeAll path="sql/updates/" />' sql/main.xml
grep --fixed-strings --quiet '<changeSet id="2026080501" author="Maxim Valyanskiy"' \
  sql/updates/2026-08-05-userlog-userpic-idx.xml

sOriginalRoot="${LORSOURCE_JAVA_ROOT:-$sDir/../../../lorsource-java}"
if test -d "$sOriginalRoot/.git"; then
  test "$(git -C "$sOriginalRoot" rev-parse HEAD)" = \
    "2ddf930005adac28077cb6ad74d1481485f44096"
  cmp "$sOriginalRoot/sql/main.xml" sql/main.xml
  cmp "$sOriginalRoot/sql/demo.db" <(gzip --decompress --stdout sql/demo.db.gz)
  diff --recursive --brief "$sOriginalRoot/sql/updates" sql/updates
fi

echo "Canonical Java database vendor is intact ($iUpdateFiles XML files, $iChangesets changesets)."
