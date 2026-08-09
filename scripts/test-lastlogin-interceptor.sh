#!/usr/bin/env bash
set -euo pipefail

if [[ "${LASTLOGIN_ALLOW_MUTATION:-}" != "yes" ]]; then
  echo "LASTLOGIN_ALLOW_MUTATION=yes is required for the last-login regression" >&2
  exit 1
fi
: "${LASTLOGIN_TEST_NICK:?Set LASTLOGIN_TEST_NICK to a disposable account}"
: "${LASTLOGIN_TEST_PASSWORD:?Set LASTLOGIN_TEST_PASSWORD for the disposable account}"

sRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sBaseUrl="${NEW_BASE_URL:-http://127.0.0.1:8181}"
sCookieFile="$(mktemp /tmp/lorsource-lastlogin.XXXXXX)"
trap 'rm -f -- "$sCookieFile"' EXIT

psql_value() {
  docker compose --project-directory "$sRoot" exec -T postgres \
    psql -U postgres -d lor -At --set ON_ERROR_STOP=1 -c "$1"
}

curl --fail-with-body --silent --show-error --output /dev/null \
  --cookie-jar "$sCookieFile" "$sBaseUrl/login.jsp?from=/about"
sCsrf="$(awk '$6 == "CSRF_TOKEN" { value=$7 } END { print value }' "$sCookieFile")"
if [[ -z "$sCsrf" ]]; then
  echo "Login form did not set CSRF_TOKEN" >&2
  exit 1
fi
iLoginStatus="$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' \
  --cookie "$sCookieFile" --cookie-jar "$sCookieFile" \
  --request POST "$sBaseUrl/login_process" \
  --data-urlencode "csrf=$sCsrf" \
  --data-urlencode "nick=$LASTLOGIN_TEST_NICK" \
  --data-urlencode "passwd=$LASTLOGIN_TEST_PASSWORD" \
  --data-urlencode 'redirectUrl=/about')"
if [[ "$iLoginStatus" != "302" && "$iLoginStatus" != "303" ]]; then
  echo "Disposable-account login failed with HTTP $iLoginStatus" >&2
  exit 1
fi

psql_value "UPDATE users SET lastlogin=CURRENT_TIMESTAMP-interval '2 hours' WHERE nick='${LASTLOGIN_TEST_NICK}'" >/dev/null
curl --fail-with-body --silent --show-error --output /dev/null \
  --cookie "$sCookieFile" "$sBaseUrl/about"
bRefreshed="$(psql_value "SELECT lastlogin>CURRENT_TIMESTAMP-interval '1 minute' FROM users WHERE nick='${LASTLOGIN_TEST_NICK}'")"
if [[ "$bRefreshed" != "t" ]]; then
  echo "Authenticated /about request did not refresh a stale lastlogin" >&2
  exit 1
fi

psql_value "UPDATE users SET lastlogin=date_trunc('second',CURRENT_TIMESTAMP-interval '30 minutes') WHERE nick='${LASTLOGIN_TEST_NICK}'" >/dev/null
iRecentBefore="$(psql_value "SELECT extract(epoch FROM lastlogin)::bigint FROM users WHERE nick='${LASTLOGIN_TEST_NICK}'")"
curl --fail-with-body --silent --show-error --output /dev/null \
  --cookie "$sCookieFile" "$sBaseUrl/about"
iRecentAfter="$(psql_value "SELECT extract(epoch FROM lastlogin)::bigint FROM users WHERE nick='${LASTLOGIN_TEST_NICK}'")"
if [[ "$iRecentAfter" != "$iRecentBefore" ]]; then
  echo "One-hour lastlogin throttle was not preserved; before=$iRecentBefore after=$iRecentAfter" >&2
  exit 1
fi

echo "LastLoginInterceptor regression passed: non-extracting route refreshed stale activity and preserved one-hour throttle"
