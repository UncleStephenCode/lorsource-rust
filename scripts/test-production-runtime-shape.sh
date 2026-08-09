#!/usr/bin/env bash
set -euo pipefail

sRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sCompose="$sRoot/deploy/compose.production.yml"
sFixtureRoot="$(mktemp -d /tmp/lorsource-runtime-shape.XXXXXX)"
sProject="lorsource-runtime-shape-$$"

cleanup() {
  docker compose --project-name "$sProject" -f "$sCompose" down --remove-orphans \
    >/dev/null 2>&1 || true
  rm -rf -- "$sFixtureRoot"
}
trap cleanup EXIT

mkdir -p \
  "$sFixtureRoot/uploads/photos" \
  "$sFixtureRoot/uploads/gallery" \
  "$sFixtureRoot/uploads/images" \
  "$sFixtureRoot/secrets"
printf '%s\n' 'postgres://runtime:fixture-password@database/lor' \
  > "$sFixtureRoot/secrets/database_url"
printf '%s\n' 'fixture-site-secret-fedcba9876543210' \
  > "$sFixtureRoot/secrets/site_secret"
printf '%s\n' 'fixture-captcha-private-key' \
  > "$sFixtureRoot/secrets/captcha_private_key"
chmod 0600 "$sFixtureRoot"/secrets/*

LORSOURCE_IMAGE="${LORSOURCE_RUNTIME_TEST_IMAGE:-lorsource-rust-app:latest}" \
UPLOAD_HOST_PATH="$sFixtureRoot/uploads" \
DATABASE_URL_SECRET_FILE="$sFixtureRoot/secrets/database_url" \
SITE_SECRET_SOURCE="$sFixtureRoot/secrets/site_secret" \
CAPTCHA_PRIVATE_KEY_SOURCE="$sFixtureRoot/secrets/captcha_private_key" \
PUBLIC_URL=https://www.linux.org.ru \
WS_URL=wss://www.linux.org.ru/ \
TRUSTED_PROXY_CIDRS=127.0.0.1/32 \
OPENSEARCH_URL=https://search.example.invalid \
CAPTCHA_PUBLIC_KEY=fixture-public-key \
SMTP_HOST=smtp.example.invalid \
SMTP_HELO_NAME=www.linux.org.ru \
ADMIN_EMAIL=operations@example.invalid \
ENABLE_BACKGROUND_JOBS=false \
docker compose --project-name "$sProject" -f "$sCompose" run --rm --no-deps app \
  /bin/sh -c '
    set -eu
    test "$(id -u)" = 8181
    test "$(id -g)" = 8181
    for sPath in /tmp/lorsource-secrets/*; do
      test "$(stat -c %u:%g "$sPath")" = 8181:8181
      test "$(stat -c %a "$sPath")" = 400
      test -r "$sPath"
    done
    touch /tmp/runtime-shape-write
    rm /tmp/runtime-shape-write
    if touch /app/runtime-shape-write 2>/dev/null; then
      echo "read-only root filesystem probe unexpectedly succeeded" >&2
      exit 1
    fi
  '

echo "Production runtime shape test passed"
