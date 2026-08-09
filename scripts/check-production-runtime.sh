#!/usr/bin/env bash
set -euo pipefail

sRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sCompose="$sRoot/deploy/compose.production.yml"

: "${LORSOURCE_IMAGE:?Set the immutable release image reference}"
: "${UPLOAD_HOST_PATH:?Set the restored media root}"

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
  COOKIE_SECRET_SOURCE \
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

if [[ "$COOKIE_SECRET_SOURCE" -ef "$SITE_SECRET_SOURCE" ]]; then
  echo "COOKIE_SECRET_SOURCE and SITE_SECRET_SOURCE must be different files" >&2
  exit 1
fi
if cmp --silent "$COOKIE_SECRET_SOURCE" "$SITE_SECRET_SOURCE"; then
  echo "Cookie and site secrets must have different values" >&2
  exit 1
fi

docker run --rm \
  --user 8181:8181 \
  --read-only \
  --cap-drop ALL \
  --security-opt no-new-privileges:true \
  --volume "$UPLOAD_HOST_PATH:/app/uploads:rw,Z" \
  --entrypoint /bin/sh \
  "$sProbeImage" \
  -c 'set -eu
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
