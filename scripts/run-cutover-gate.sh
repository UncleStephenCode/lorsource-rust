#!/usr/bin/env bash
set -euo pipefail

sRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${ORIGINAL_ROOT:?Set ORIGINAL_ROOT to the checked-out Java source tree}"
: "${OLD_BASE_URL:?Set OLD_BASE_URL to the isolated Java runtime}"
sNewBaseUrl="${NEW_BASE_URL:-http://127.0.0.1:8181}"
sEvidenceDir="${EVIDENCE_DIR:-$(mktemp -d /tmp/lorsource-cutover-evidence.XXXXXX)}"
mkdir -p "$sEvidenceDir"
bFullGate=true

exec > >(tee "$sEvidenceDir/gate.log") 2>&1

echo "Cutover rehearsal evidence: $sEvidenceDir"
echo "Java runtime: $OLD_BASE_URL"
echo "Rust runtime: $sNewBaseUrl"
echo "Original source: $ORIGINAL_ROOT"

if [[ "${CUTOVER_REQUIRE_RELEASE_EVIDENCE:-1}" == "1" ]]; then
  : "${CUTOVER_IMAGE_DIGEST:?Set CUTOVER_IMAGE_DIGEST to the immutable sha256 image digest}"
  : "${CUTOVER_SNAPSHOT_ID:?Set CUTOVER_SNAPSHOT_ID to the restored production-clone snapshot identifier}"
  : "${CUTOVER_WAL_POSITION:?Set CUTOVER_WAL_POSITION to the source snapshot/WAL position}"
  : "${CUTOVER_CONFIG_MANIFEST:?Set CUTOVER_CONFIG_MANIFEST to a redacted production-shape configuration manifest}"
  : "${CUTOVER_MEDIA_EVIDENCE:?Set CUTOVER_MEDIA_EVIDENCE to the media-mount rehearsal evidence file}"
  : "${CUTOVER_EXTERNAL_EVIDENCE:?Set CUTOVER_EXTERNAL_EVIDENCE to the external-adapter rehearsal evidence file}"

  for sEvidenceFile in \
    "$CUTOVER_CONFIG_MANIFEST" \
    "$CUTOVER_MEDIA_EVIDENCE" \
    "$CUTOVER_EXTERNAL_EVIDENCE"; do
    if [[ ! -f "$sEvidenceFile" ]]; then
      echo "Required cutover evidence file does not exist: $sEvidenceFile" >&2
      exit 1
    fi
  done

  python3 "$sRoot/tools/validate_cutover_evidence.py" \
    --config "$CUTOVER_CONFIG_MANIFEST" \
    --media "$CUTOVER_MEDIA_EVIDENCE" \
    --external "$CUTOVER_EXTERNAL_EVIDENCE" \
    --image-digest "$CUTOVER_IMAGE_DIGEST" \
    --snapshot-id "$CUTOVER_SNAPSHOT_ID" \
    --wal-position "$CUTOVER_WAL_POSITION" \
    --max-age-hours "${CUTOVER_EVIDENCE_MAX_AGE_HOURS:-168}"

  printf '%s\n' \
    "image_digest=$CUTOVER_IMAGE_DIGEST" \
    "snapshot_id=$CUTOVER_SNAPSHOT_ID" \
    "wal_position=$CUTOVER_WAL_POSITION" \
    > "$sEvidenceDir/release-provenance.txt"
  cp "$CUTOVER_CONFIG_MANIFEST" "$sEvidenceDir/config-manifest.redacted.json"
  cp "$CUTOVER_MEDIA_EVIDENCE" "$sEvidenceDir/media-rehearsal.json"
  cp "$CUTOVER_EXTERNAL_EVIDENCE" "$sEvidenceDir/external-adapters.json"
else
  echo "Release provenance/evidence checks skipped because CUTOVER_REQUIRE_RELEASE_EVIDENCE=${CUTOVER_REQUIRE_RELEASE_EVIDENCE:-0}"
  bFullGate=false
fi

ORIGINAL_ROOT="$ORIGINAL_ROOT" "$sRoot/scripts/run-compatibility-suite.sh"

OLD_BASE_URL="$OLD_BASE_URL" NEW_BASE_URL="$sNewBaseUrl" \
  PYTHONDONTWRITEBYTECODE=1 python3 "$sRoot/compat/test_http_compat.py" \
  --report "$sEvidenceDir/http-compat.json"

for sPath in / /forum /forum/ /news/ /articles/ /gallery/ /polls/ \
  /tracker /tracker/ /tracker.jsp /login.jsp /section-rss.jsp?section=1; do
  curl --fail-with-body --silent --show-error --output /dev/null \
    --write-out "$sPath %{http_code} %{content_type}\n" "$sNewBaseUrl$sPath"
done

if [[ "${CUTOVER_VALIDATE_DB:-1}" == "1" ]]; then
  if command -v mvn >/dev/null 2>&1; then
    "$sRoot/compat/java-db/manage.sh" validate
  elif command -v docker >/dev/null 2>&1; then
    echo "Host Maven is unavailable; using the repository DB validator image."
    (cd "$sRoot" && docker compose run --rm db-bootstrap validate)
  else
    echo "Database validation requires either host Maven or Docker Compose" >&2
    exit 1
  fi
else
  echo "Database validation skipped because CUTOVER_VALIDATE_DB=$CUTOVER_VALIDATE_DB"
  bFullGate=false
fi

# Resolve and connect to an external assertion database before either HTTP
# flow can mutate the clone. Compose-mode CI keeps its existing local target.
if [[ "${CUTOVER_MODERATION_FLOW:-1}" == "1" && -n "${STATEFUL_DATABASE_URL_FILE:-}" ]]; then
  : "${STATEFUL_DATABASE_IS_DISPOSABLE:?Set STATEFUL_DATABASE_IS_DISPOSABLE=yes for the isolated clone}"
  : "${STATEFUL_EXPECTED_DATABASE:?Set the exact isolated-clone database name}"
  if [[ "$STATEFUL_DATABASE_IS_DISPOSABLE" != "yes" ]]; then
    echo "STATEFUL_DATABASE_IS_DISPOSABLE=yes is required for an external stateful database" >&2
    exit 1
  fi
  PYTHONDONTWRITEBYTECODE=1 \
    python3 "$sRoot/compat/test_moderation_flows.py" --verify-database-only
  printf '%s\n' "expected_database=$STATEFUL_EXPECTED_DATABASE" \
    > "$sEvidenceDir/stateful-database-target.txt"
fi

if [[ "${CUTOVER_WRITE_FLOW:-1}" == "1" ]]; then
  if [[ "${WRITE_FLOW_ALLOW_MUTATION:-}" != "yes" ]]; then
    echo "WRITE_FLOW_ALLOW_MUTATION=yes is required for the rehearsal write flow" >&2
    exit 1
  fi
  NEW_BASE_URL="$sNewBaseUrl" PYTHONDONTWRITEBYTECODE=1 \
    python3 "$sRoot/compat/test_write_flows.py"
else
  echo "Stateful write flow skipped because CUTOVER_WRITE_FLOW=$CUTOVER_WRITE_FLOW"
  bFullGate=false
fi

if [[ "${CUTOVER_MODERATION_FLOW:-1}" == "1" ]]; then
  if [[ "${MODERATION_FLOW_ALLOW_MUTATION:-}" != "yes" ]]; then
    echo "MODERATION_FLOW_ALLOW_MUTATION=yes is required for the rehearsal moderation flow" >&2
    exit 1
  fi
  : "${MODERATION_FLOW_MODERATOR_NICK:?Set the disposable-clone moderator nick}"
  : "${MODERATION_FLOW_MODERATOR_PASSWORD:?Set the disposable-clone moderator password}"
  : "${MODERATION_FLOW_TARGET_NICK:?Set the disposable moderation target nick}"
  : "${MODERATION_FLOW_LOW_NICK:?Set the disposable score50 target nick}"
  : "${MODERATION_FLOW_LOW_PASSWORD:?Set the disposable score50 target password}"
  : "${MODERATION_FLOW_DELETE_NICK:?Set the disposable mass-delete target nick}"
  : "${MODERATION_FLOW_DELETE_PASSWORD:?Set the disposable mass-delete target password}"
  : "${MODERATION_FLOW_CORRECTOR_NICK:?Set the disposable corrector nick}"
  : "${MODERATION_FLOW_CORRECTOR_PASSWORD:?Set the disposable corrector password}"
  NEW_BASE_URL="$sNewBaseUrl" PYTHONDONTWRITEBYTECODE=1 \
    python3 "$sRoot/compat/test_moderation_flows.py"
else
  echo "Stateful moderation flow skipped because CUTOVER_MODERATION_FLOW=$CUTOVER_MODERATION_FLOW"
  bFullGate=false
fi

if $bFullGate; then
  echo "Cutover rehearsal gate passed. Retain $sEvidenceDir with the image digest and snapshot/WAL identifiers."
else
  echo "Read-only rehearsal checks passed, but skipped checks prevent a cutover go/no-go decision."
fi
