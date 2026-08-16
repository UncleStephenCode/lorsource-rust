#!/usr/bin/env bash
set -euo pipefail

readonly sRepoRoot="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly sJavaContainer="lorsource-java-parity"
readonly sSearchContainer="java-parity-opensearch"
readonly sParityDatabase="lor_java_parity"
readonly sSourceDatabase="lor"
readonly sNetwork="lorsource-rust_default"
readonly sMavenCache="/tmp/lorsource-java-m2"

vUsage() {
  cat <<'EOF'
Usage: ORIGINAL_ROOT=/path/to/lorsource-java scripts/run-java-parity-runtime.sh COMMAND

Commands:
  refresh-db  Replace only the guarded lor_java_parity database from lor.
  start       Start isolated OpenSearch and the passive Java runtime on 8081.
  recreate    Stop, refresh-db, and start.
  wait        Wait until the Java HTTP endpoint is ready or startup fails.
  stop        Stop the two exact disposable comparator containers.
  status      Show comparator containers and database availability.

The script never changes the source `lor` database. Java runs against the
disposable `lor_java_parity` clone and its own ephemeral OpenSearch node.
EOF
}

bContainerExists() {
  docker container inspect "$1" >/dev/null 2>&1
}

bContainerRunning() {
  [[ "$(docker container inspect -f '{{.State.Running}}' "$1" 2>/dev/null || true)" == true ]]
}

vRequireOriginal() {
  : "${ORIGINAL_ROOT:?Set ORIGINAL_ROOT to the original lorsource Java checkout}"
  test -f "$ORIGINAL_ROOT/pom.xml" || {
    echo "Original checkout has no pom.xml: $ORIGINAL_ROOT" >&2
    exit 2
  }
}

vRequireComposeStack() {
  cd "$sRepoRoot"
  docker compose exec -T postgres pg_isready -U postgres -d postgres >/dev/null
  docker network inspect "$sNetwork" >/dev/null
}

vStop() {
  if bContainerExists "$sJavaContainer"; then
    docker rm -f "$sJavaContainer" >/dev/null
  fi
  if bContainerExists "$sSearchContainer"; then
    docker rm -f "$sSearchContainer" >/dev/null
  fi
}

vRefreshDatabase() {
  vRequireComposeStack
  if [[ "$sParityDatabase" == "$sSourceDatabase" || "$sParityDatabase" != lor_*_parity ]]; then
    echo "Refusing unsafe parity database target: $sParityDatabase" >&2
    exit 2
  fi
  if bContainerExists "$sJavaContainer"; then
    echo "Stop $sJavaContainer before refreshing its database" >&2
    exit 2
  fi

  docker compose exec -T postgres dropdb \
    -U postgres --if-exists --force "$sParityDatabase"
  docker compose exec -T postgres createdb \
    -U postgres -O maxcom "$sParityDatabase"
  docker compose exec -T postgres pg_dump \
    -U postgres --format=custom "$sSourceDatabase" |
    docker compose exec -T postgres pg_restore \
      -U postgres --dbname "$sParityDatabase" --no-owner --role=maxcom

  docker compose exec -T postgres psql \
    -U postgres -d "$sParityDatabase" -v ON_ERROR_STOP=1 \
    -c 'GRANT CONNECT ON DATABASE lor_java_parity TO linuxweb'
  echo "Refreshed disposable database $sParityDatabase from $sSourceDatabase"
}

vWaitForSearch() {
  local iAttempt
  for iAttempt in $(seq 1 60); do
    if docker exec "$sSearchContainer" \
      curl -sf http://127.0.0.1:9200/_cluster/health >/dev/null 2>&1; then
      return
    fi
    sleep 2
  done
  echo "Comparator OpenSearch did not become healthy" >&2
  exit 1
}

vEnsureJavaSearchIndex() {
  if docker exec "$sSearchContainer" \
    curl -sfI http://127.0.0.1:9200/messages >/dev/null; then
    return
  fi
  docker exec -i "$sSearchContainer" \
    curl -fsS -X PUT \
      -H 'Content-Type: application/json' \
      --data-binary @- http://127.0.0.1:9200/messages \
    <"$sRepoRoot/compat/java-runtime/messages-index.json" >/dev/null
}

vWaitForJava() {
  local iAttempt
  for iAttempt in $(seq 1 180); do
    if curl -sf http://127.0.0.1:8081/ >/dev/null 2>&1; then
      echo "Java comparator is ready on http://127.0.0.1:8081/"
      return
    fi
    if ! bContainerRunning "$sJavaContainer"; then
      echo "Java comparator exited during startup" >&2
      docker logs --tail 200 "$sJavaContainer" >&2
      exit 1
    fi
    sleep 2
  done
  echo "Java comparator did not become ready within six minutes" >&2
  docker logs --tail 200 "$sJavaContainer" >&2
  exit 1
}

vStart() {
  vRequireOriginal
  vRequireComposeStack
  docker compose exec -T postgres pg_isready \
    -U linuxweb -d "$sParityDatabase" >/dev/null

  if ! bContainerRunning "$sSearchContainer"; then
    if bContainerExists "$sSearchContainer"; then
      docker rm "$sSearchContainer" >/dev/null
    fi
    docker run -d \
      --name "$sSearchContainer" \
      --network "$sNetwork" \
      -e discovery.type=single-node \
      -e DISABLE_SECURITY_PLUGIN=true \
      -e DISABLE_INSTALL_DEMO_CONFIG=true \
      -e 'OPENSEARCH_JAVA_OPTS=-Xms512m -Xmx512m' \
      opensearchproject/opensearch:3.6.0 >/dev/null
  fi
  vWaitForSearch
  vEnsureJavaSearchIndex

  if bContainerRunning "$sJavaContainer"; then
    echo "$sJavaContainer is already running"
    return
  fi
  if bContainerExists "$sJavaContainer"; then
    echo "Removing previous exited comparator; its logs were available via docker logs $sJavaContainer"
    docker rm "$sJavaContainer" >/dev/null
  fi

  mkdir -p "$sMavenCache"
  # The official Maven entrypoint unsets MAVEN_CONFIG before Maven starts.
  # GitHub's runner UID is absent from the image passwd file, so Java otherwise
  # resolves user.home to `/` and tries to create `/.m2/repository`.
  docker run -d \
    --name "$sJavaContainer" \
    --network "$sNetwork" \
    -p 127.0.0.1:8081:8080 \
    -u "$(id -u):$(id -g)" \
    -e MAVEN_CONFIG=/tmp/m2 \
    -v "$ORIGINAL_ROOT:/workspace:Z" \
    -v "$sMavenCache:/tmp/m2:Z" \
    -v "$sRepoRoot/compat/java-runtime/config.properties:/workspace/src/main/webapp/WEB-INF/config.properties:ro,Z" \
    -v lorsource-rust_lor_uploads:/uploads \
    -w /workspace \
    maven:3.9.14-eclipse-temurin-25 \
    mvn --batch-mode --no-transfer-progress \
      -Dmaven.repo.local=/tmp/m2/repository \
      -DskipTests package jetty:run-war >/dev/null

  echo "Java comparator is building on http://127.0.0.1:8081/"
  vWaitForJava
}

vStatus() {
  cd "$sRepoRoot"
  docker ps -a \
    --filter "name=^/${sJavaContainer}$" \
    --filter "name=^/${sSearchContainer}$" \
    --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
  docker compose exec -T postgres pg_isready \
    -U linuxweb -d "$sParityDatabase"
}

case "${1:-}" in
  refresh-db)
    vRefreshDatabase
    ;;
  start)
    vStart
    ;;
  recreate)
    vStop
    vRefreshDatabase
    vStart
    ;;
  wait)
    vWaitForJava
    ;;
  stop)
    vStop
    ;;
  status)
    vStatus
    ;;
  *)
    vUsage
    exit 2
    ;;
esac
