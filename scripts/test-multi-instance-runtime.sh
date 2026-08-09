#!/usr/bin/env bash
set -euo pipefail

if [[ "${MULTI_INSTANCE_ALLOW_MUTATION:-}" != "yes" ]]; then
  echo "MULTI_INSTANCE_ALLOW_MUTATION=yes is required for the login regression" >&2
  exit 1
fi
: "${MULTI_INSTANCE_TEST_NICK:?Set a disposable account nick}"
: "${MULTI_INSTANCE_TEST_PASSWORD:?Set the disposable account password}"

sRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sPrimaryUrl="${NEW_BASE_URL:-http://127.0.0.1:8181}"
iReplicaPort="${MULTI_INSTANCE_PORT:-8282}"
sReplicaUrl="http://127.0.0.1:${iReplicaPort}"
sCookieFile="$(mktemp /tmp/lorsource-multi-instance.XXXXXX)"
sReplicaId=""

cleanup() {
  if [[ -n "$sReplicaId" ]]; then
    docker stop "$sReplicaId" >/dev/null 2>&1 || true
  fi
  rm -f -- "$sCookieFile"
}
trap cleanup EXIT

sReplicaId="$(
  docker compose --project-directory "$sRoot" run --rm -d --no-deps \
    -p "127.0.0.1:${iReplicaPort}:8181" app | tail -n 1
)"

for _iAttempt in $(seq 1 60); do
  if curl --fail --silent "$sReplicaUrl/readyz" >/dev/null; then
    break
  fi
  if [[ "$_iAttempt" == 60 ]]; then
    echo "Second replica did not become ready" >&2
    exit 1
  fi
  sleep 1
done

curl --fail-with-body --silent --show-error --output /dev/null \
  --cookie-jar "$sCookieFile" "$sPrimaryUrl/login.jsp?from=/people/${MULTI_INSTANCE_TEST_NICK}/profile"
sCsrf="$(awk '$6 == "CSRF_TOKEN" { value=$7 } END { print value }' "$sCookieFile")"
if [[ -z "$sCsrf" ]]; then
  echo "Primary replica did not set CSRF_TOKEN" >&2
  exit 1
fi

iLoginStatus="$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' \
  --cookie "$sCookieFile" --cookie-jar "$sCookieFile" \
  --request POST "$sPrimaryUrl/login_process" \
  --data-urlencode "csrf=$sCsrf" \
  --data-urlencode "nick=$MULTI_INSTANCE_TEST_NICK" \
  --data-urlencode "passwd=$MULTI_INSTANCE_TEST_PASSWORD" \
  --data-urlencode "redirectUrl=/people/${MULTI_INSTANCE_TEST_NICK}/profile")"
if [[ "$iLoginStatus" != "302" ]]; then
  echo "Primary-replica login failed with HTTP $iLoginStatus" >&2
  exit 1
fi

sProfile="$(curl --fail-with-body --silent --show-error \
  --cookie "$sCookieFile" "$sReplicaUrl/people/${MULTI_INSTANCE_TEST_NICK}/profile")"
if [[ "$sProfile" != *"$MULTI_INSTANCE_TEST_NICK"* || "$sProfile" != *'data-style='* ]]; then
  echo "Second replica did not resolve the shared authenticated profile/theme" >&2
  exit 1
fi

echo "Multi-instance runtime regression passed: session/profile/theme crossed primary -> replica"
