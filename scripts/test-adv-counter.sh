#!/usr/bin/env bash
set -euo pipefail

if [[ "${ADV_COUNTER_ALLOW_MUTATION:-}" != "yes" ]]; then
  echo "ADV_COUNTER_ALLOW_MUTATION=yes is required for the advertisement counter regression" >&2
  exit 1
fi

sRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sBaseUrl="${NEW_BASE_URL:-http://127.0.0.1:8181}"
sGoodPath="/adv/bare-metal-new-h2-320x100.png"
sMissingPath="/adv/not-found-counter-regression.png"
bStopped=false

cleanup() {
  if $bStopped; then
    docker compose --project-directory "$sRoot" up -d app >/dev/null
  fi
}
trap cleanup EXIT

wait_ready() {
  for _iAttempt in $(seq 1 60); do
    if curl --fail --silent "$sBaseUrl/readyz" >/dev/null; then
      return
    fi
    sleep 1
  done
  echo "Application did not become ready during advertisement counter regression" >&2
  exit 1
}

graceful_restart() {
  docker compose --project-directory "$sRoot" stop app >/dev/null
  bStopped=true
  docker compose --project-directory "$sRoot" up -d app >/dev/null
  bStopped=false
  wait_ready
}

counter() {
  local sPath="$1"
  docker compose --project-directory "$sRoot" exec -T postgres \
    psql -U postgres -d lor -At --set ON_ERROR_STOP=1 \
    -c "SELECT COALESCE(sum(counter),0) FROM adv_counts WHERE path='${sPath}' AND day=CURRENT_DATE"
}

# Flush any advertisement requests made by earlier compatibility steps before
# capturing the baseline. This keeps the exact +3 assertion deterministic.
graceful_restart
iGoodBefore="$(counter "$sGoodPath")"
iMissingBefore="$(counter "$sMissingPath")"

for _iRequest in 1 2 3; do
  curl --fail-with-body --silent --show-error --output /dev/null "$sBaseUrl$sGoodPath"
done
iMissingStatus="$(curl --silent --show-error --output /dev/null --write-out '%{http_code}' "$sBaseUrl$sMissingPath")"
if [[ "$iMissingStatus" != "404" ]]; then
  echo "Expected $sMissingPath to return 404, got $iMissingStatus" >&2
  exit 1
fi

docker compose --project-directory "$sRoot" stop app >/dev/null
bStopped=true

iGoodAfter="$(counter "$sGoodPath")"
iMissingAfter="$(counter "$sMissingPath")"
if (( iGoodAfter - iGoodBefore != 3 )); then
  echo "Expected exactly three successful advertisement hits; before=$iGoodBefore after=$iGoodAfter" >&2
  exit 1
fi
if (( iMissingAfter != iMissingBefore )); then
  echo "A 404 advertisement request changed the counter; before=$iMissingBefore after=$iMissingAfter" >&2
  exit 1
fi

docker compose --project-directory "$sRoot" up -d app >/dev/null
bStopped=false
wait_ready
echo "Advertisement counter regression passed: +3 successful hits, 404 ignored, graceful flush persisted"
