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

  if [[ ! "$CUTOVER_IMAGE_DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]]; then
    echo "CUTOVER_IMAGE_DIGEST must be sha256:<64 lowercase hex characters>" >&2
    exit 1
  fi
  for sEvidenceFile in \
    "$CUTOVER_CONFIG_MANIFEST" \
    "$CUTOVER_MEDIA_EVIDENCE" \
    "$CUTOVER_EXTERNAL_EVIDENCE"; do
    if [[ ! -f "$sEvidenceFile" ]]; then
      echo "Required cutover evidence file does not exist: $sEvidenceFile" >&2
      exit 1
    fi
  done

  printf '%s\n' \
    "image_digest=$CUTOVER_IMAGE_DIGEST" \
    "snapshot_id=$CUTOVER_SNAPSHOT_ID" \
    "wal_position=$CUTOVER_WAL_POSITION" \
    > "$sEvidenceDir/release-provenance.txt"
  cp "$CUTOVER_CONFIG_MANIFEST" "$sEvidenceDir/config-manifest.redacted"
  cp "$CUTOVER_MEDIA_EVIDENCE" "$sEvidenceDir/media-rehearsal.txt"
  cp "$CUTOVER_EXTERNAL_EVIDENCE" "$sEvidenceDir/external-adapters.txt"
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
  "$sRoot/compat/java-db/manage.sh" validate
else
  echo "Database validation skipped because CUTOVER_VALIDATE_DB=$CUTOVER_VALIDATE_DB"
  bFullGate=false
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
