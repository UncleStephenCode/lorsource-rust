#!/usr/bin/env bash
set -euo pipefail

readonly sRepoRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${OPENSEARCH_URL:?Set OPENSEARCH_URL to the cluster base URL}"
: "${SEARCH_REBUILD_SNAPSHOT_ID:?Record and provide the recoverable OpenSearch snapshot ID}"

if [[ "${SEARCH_REBUILD_CONFIRM:-}" != "rebuild-derived-messages-index" ]]; then
  echo "Refusing to delete the derived messages index." >&2
  echo "Set SEARCH_REBUILD_CONFIRM=rebuild-derived-messages-index after verifying the snapshot." >&2
  exit 2
fi

readonly sBaseUrl="${OPENSEARCH_URL%/}"
if [[ ! "$sBaseUrl" =~ ^https?://[^/?#]+(:[0-9]+)?$ ]]; then
  echo "OPENSEARCH_URL must be an HTTP(S) cluster base URL without a path, query or fragment" >&2
  exit 2
fi

vecCurl=(curl --silent --show-error --fail-with-body)
if [[ -n "${OPENSEARCH_CURL_CONFIG:-}" ]]; then
  test -f "$OPENSEARCH_CURL_CONFIG" || {
    echo "OPENSEARCH_CURL_CONFIG is not a regular file" >&2
    exit 2
  }
  vecCurl+=(--config "$OPENSEARCH_CURL_CONFIG")
fi

echo "Snapshot: $SEARCH_REBUILD_SNAPSHOT_ID"
echo "Current derived-document count:"
"${vecCurl[@]}" "$sBaseUrl/messages/_count"
echo

# The exact target is intentionally fixed: this operation must never accept a
# caller-supplied index name or wildcard. PostgreSQL remains the source of
# truth; the recorded snapshot provides recovery until full reindex finishes.
"${vecCurl[@]}" -X DELETE "$sBaseUrl/messages"
echo
"${vecCurl[@]}" -X PUT \
  -H 'Content-Type: application/json' \
  --data-binary "@$sRepoRoot/compat/java-runtime/messages-index.json" \
  "$sBaseUrl/messages"
echo
"${vecCurl[@]}" "$sBaseUrl/messages/_mapping"
echo
echo "The Java-compatible empty index is ready. Start Rust, invoke /admin/search-reindex with action=all, and retain the snapshot until counts are reconciled."
