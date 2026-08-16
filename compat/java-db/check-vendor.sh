#!/usr/bin/env bash
set -euo pipefail

sDir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$sDir"
export LC_ALL=C

sha256sum --check --strict --quiet checksums.sha256

sDemoHash="$(gzip --decompress --stdout sql/demo.db.gz | sha256sum | awk '{print $1}')"
test "$sDemoHash" = "9871b6148f59786da7cd00e8601931869129ec1968035cb8a6c2a1e3592a038e"

sSchemaObjectQueryHash="$(sha256sum export-schema-objects.sql | awk '{print $1}')"
test "$sSchemaObjectQueryHash" = "930539ad7662d66ff8037a979f0e2a89f560bb055573b81b40065a53d85ff3d7"
sSchemaObjectContractHash="$(sha256sum schema-objects-contract.tsv | awk '{print $1}')"
test "$sSchemaObjectContractHash" = "eaed5aacda3724e56f4508a98ebc98e45a48fec6acba3f9e35a342d72d9e84f0"
test "$(wc -l < schema-objects-contract.tsv)" -eq 728

sLiquibaseHistoryHash="$(sha256sum liquibase-changesets.tsv | awk '{print $1}')"
test "$sLiquibaseHistoryHash" = "af0e920c0fe922f87d41957583ace19ff7db669a75a7f7d1f1d292c7cee1a644"
test "$(wc -l < liquibase-changesets.tsv)" -eq 187
awk -F '\t' '
  NF != 6 || $1 != NR || $5 !~ /^8:[0-9a-f]{32}$/ ||
    ($6 != "EXECUTED" && $6 != "MARK_RAN") { exit 1 }
' liquibase-changesets.tsv

sHeadroomQueryHash="$(sha256sum check-sequence-headroom.sql | awk '{print $1}')"
test "$sHeadroomQueryHash" = "c156537595af3e703e975fec83ae6494fa8200bacf56e8c90628c66756967c31"

iUpdateFiles="$(find sql/updates -maxdepth 1 -type f -name '*.xml' | wc -l)"
test "$iUpdateFiles" -eq 116

iChangesets="$(grep --with-filename '<changeSet' sql/updates/*.xml | wc -l)"
test "$iChangesets" -eq 187

sVendoredIdentities="$(mktemp)"
sContractIdentities="$(mktemp)"
trap 'rm -f "$sVendoredIdentities" "$sContractIdentities"' EXIT
iOrder=0
for sUpdate in sql/updates/*.xml; do
  while IFS= read -r sLine; do
    iOrder=$((iOrder + 1))
    sId="$(sed -n 's/.*<changeSet id="\([^"]*\)" author="[^"]*".*/\1/p' <<<"$sLine")"
    sAuthor="$(sed -n 's/.*<changeSet id="[^"]*" author="\([^"]*\)".*/\1/p' <<<"$sLine")"
    test -n "$sId"
    test -n "$sAuthor"
    printf '%s\t%s\t%s\t%s\n' "$iOrder" "$sId" "$sAuthor" "$sUpdate" \
      >>"$sVendoredIdentities"
  done < <(grep '<changeSet' "$sUpdate")
done
cut --fields=1-4 liquibase-changesets.tsv >"$sContractIdentities"
cmp "$sVendoredIdentities" "$sContractIdentities"
rm -f "$sVendoredIdentities" "$sContractIdentities"
trap - EXIT

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
