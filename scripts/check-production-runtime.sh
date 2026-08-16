#!/usr/bin/env bash
set -euo pipefail

sRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sCompose="$sRoot/deploy/compose.production.yml"

: "${LORSOURCE_IMAGE:?Set the immutable release image reference}"
: "${UPLOAD_HOST_PATH:?Set the restored media root}"
: "${SCHEDULER_TIMEZONE:?Set the IANA timezone used by the original Java scheduler}"
: "${LEGACY_JDBC_TIMEZONE:?Set the verified IANA timezone used by the original Java JDBC runtime}"
: "${ENABLE_BACKGROUND_JOBS:?Explicitly select true on one scheduler replica or false on passive replicas}"

if [[ "$SCHEDULER_TIMEZONE" == /* || "$SCHEDULER_TIMEZONE" == *..* || \
      ! "$SCHEDULER_TIMEZONE" =~ ^[A-Za-z0-9._+-]+(/[A-Za-z0-9._+-]+)*$ ]]; then
  echo "SCHEDULER_TIMEZONE must be a safe relative IANA timezone name" >&2
  exit 1
fi

if [[ "$LEGACY_JDBC_TIMEZONE" == /* || "$LEGACY_JDBC_TIMEZONE" == *..* || \
      ! "$LEGACY_JDBC_TIMEZONE" =~ ^[A-Za-z0-9._+-]+(/[A-Za-z0-9._+-]+)*$ ]]; then
  echo "LEGACY_JDBC_TIMEZONE must be a safe relative IANA timezone name" >&2
  exit 1
fi

if [[ "$ENABLE_BACKGROUND_JOBS" != "true" && "$ENABLE_BACKGROUND_JOBS" != "false" ]]; then
  echo "ENABLE_BACKGROUND_JOBS must be the literal true or false" >&2
  exit 1
fi

if [[ ! "$LORSOURCE_IMAGE" =~ @sha256:[0-9a-f]{64}$ ]]; then
  echo "LORSOURCE_IMAGE must be pinned as registry/repository@sha256:<64 lowercase hex>" >&2
  exit 1
fi
sProbeImage="$LORSOURCE_IMAGE"
bLocalImage=false
if [[ -n "${LORSOURCE_PREFLIGHT_LOCAL_IMAGE:-}" ]]; then
  if [[ "${LORSOURCE_PREFLIGHT_ALLOW_LOCAL_IMAGE:-}" != "yes" ]]; then
    echo "LORSOURCE_PREFLIGHT_ALLOW_LOCAL_IMAGE=yes is required for a local image alias" >&2
    exit 1
  fi
  sExpectedDigest="${LORSOURCE_IMAGE##*@}"
  sActualDigest="$(docker image inspect "$LORSOURCE_PREFLIGHT_LOCAL_IMAGE" --format '{{.Id}}')"
  if [[ "$sActualDigest" != "$sExpectedDigest" ]]; then
    echo "Local image alias digest does not match LORSOURCE_IMAGE" >&2
    exit 1
  fi
  sProbeImage="$LORSOURCE_PREFLIGHT_LOCAL_IMAGE"
  bLocalImage=true
fi
if [[ "$UPLOAD_HOST_PATH" != /* || "$UPLOAD_HOST_PATH" == "/" ]]; then
  echo "UPLOAD_HOST_PATH must be a dedicated absolute directory" >&2
  exit 1
fi
if [[ ! -d "$UPLOAD_HOST_PATH" ]]; then
  echo "UPLOAD_HOST_PATH does not exist: $UPLOAD_HOST_PATH" >&2
  exit 1
fi

for sDirectory in photos gallery images; do
  if [[ ! -d "$UPLOAD_HOST_PATH/$sDirectory" ]]; then
    echo "Required media directory is missing: $UPLOAD_HOST_PATH/$sDirectory" >&2
    exit 1
  fi
done

sOwner="$(stat -c '%u:%g' "$UPLOAD_HOST_PATH")"
if [[ "$sOwner" != "8181:8181" ]]; then
  echo "UPLOAD_HOST_PATH must be owned by runtime UID/GID 8181:8181; got $sOwner" >&2
  exit 1
fi

for sVariable in \
  DATABASE_URL_SECRET_FILE \
  SITE_SECRET_SOURCE \
  CAPTCHA_PRIVATE_KEY_SOURCE; do
  sPath="${!sVariable:-}"
  if [[ -z "$sPath" || ! -f "$sPath" ]]; then
    echo "$sVariable must name an existing secret file" >&2
    exit 1
  fi
  sMode="$(stat -c '%a' "$sPath")"
  if (( (8#$sMode & 077) != 0 )); then
    echo "$sVariable must not be readable or writable by group/other (mode $sMode)" >&2
    exit 1
  fi
  if ! awk 'END { exit !(NR == 1 && length($0) > 0) }' "$sPath"; then
    echo "$sVariable must contain exactly one non-empty text line" >&2
    exit 1
  fi
done

docker run --rm \
  --user 8181:8181 \
  --read-only \
  --cap-drop ALL \
  --security-opt no-new-privileges:true \
  --env "TZ=$SCHEDULER_TIMEZONE" \
  --env "SCHEDULER_TIMEZONE=$SCHEDULER_TIMEZONE" \
  --env "LEGACY_JDBC_TIMEZONE=$LEGACY_JDBC_TIMEZONE" \
  --volume "$UPLOAD_HOST_PATH:/app/uploads:rw,Z" \
  --entrypoint /bin/sh \
  "$sProbeImage" \
  -c 'set -eu
      test -f "/usr/share/zoneinfo/$TZ"
      test "$SCHEDULER_TIMEZONE" = "$TZ"
      test -f "/usr/share/zoneinfo/$LEGACY_JDBC_TIMEZONE"
      sProbe=/app/uploads/.lorsource-preflight-$$
      trap '\''rm -f "$sProbe" "$sProbe.renamed"'\'' EXIT
      printf probe > "$sProbe"
      test "$(cat "$sProbe")" = probe
      mv "$sProbe" "$sProbe.renamed"
      rm "$sProbe.renamed"'

docker compose -f "$sCompose" config --quiet
if $bLocalImage; then
  echo "Local image alias was used; this pass is development evidence only." >&2
fi
echo "Production runtime manifest preflight passed for pinned image $LORSOURCE_IMAGE"
