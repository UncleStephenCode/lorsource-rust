#!/usr/bin/env bash
set -euo pipefail

sDir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
sMode="${1:-}"

PG_ADMIN_URL="${PG_ADMIN_URL:-postgres://postgres:postgres@localhost:5432/postgres}"
PG_TARGET_ADMIN_URL="${PG_TARGET_ADMIN_URL:-postgres://postgres:postgres@localhost:5432/lor}"
JAVA_DATABASE_NAME="${JAVA_DATABASE_NAME:-lor}"
JAVA_DATABASE_MIGRATION_USER="${JAVA_DATABASE_MIGRATION_USER:-maxcom}"
JAVA_DATABASE_MIGRATION_PASSWORD="${JAVA_DATABASE_MIGRATION_PASSWORD:-maxcom}"
JAVA_DATABASE_MIGRATION_URL="${JAVA_DATABASE_MIGRATION_URL:-postgres://maxcom:maxcom@localhost:5432/lor}"
JAVA_DATABASE_JDBC_URL="${JAVA_DATABASE_JDBC_URL:-jdbc:postgresql://localhost:5432/lor}"
JAVA_DATABASE_RUNTIME_USER="${JAVA_DATABASE_RUNTIME_USER:-linuxweb}"
JAVA_DATABASE_RUNTIME_PASSWORD="${JAVA_DATABASE_RUNTIME_PASSWORD:-linuxweb}"
JAVA_DATABASE_RUNTIME_URL="${JAVA_DATABASE_RUNTIME_URL:-}"
JAVA_DATABASE_JAMWIKI_USER="${JAVA_DATABASE_JAMWIKI_USER:-jamwiki}"

export JAVA_DATABASE_JDBC_URL
export JAVA_DATABASE_MIGRATION_USER
export JAVA_DATABASE_MIGRATION_PASSWORD

vUsage() {
  cat >&2 <<'EOF'
Usage: compat/java-db/manage.sh bootstrap|validate|classify

bootstrap  Create/load only a missing or empty database after explicit
           LOR_DB_BOOTSTRAP_CONFIRM=bootstrap-empty-java-db confirmation.
           A detected Java database is validated without being updated.
validate   Validate vendor checksums, the canonical Liquibase identity/checksum
           set, terminal changeset and sequence headroom without changing data.
classify   Print missing, empty, java, legacy-rust, mixed or unknown.
EOF
}

vDie() {
  echo "database workflow error: $*" >&2
  exit 1
}

vRequireTools() {
  local sTool
  for sTool in psql pg_isready gzip sha256sum mvn; do
    command -v "$sTool" >/dev/null || vDie "required command is missing: $sTool"
  done
}

sPsqlScalar() {
  local sUrl="$1"
  local sSql="$2"
  psql "$sUrl" --no-psqlrc --tuples-only --no-align --set ON_ERROR_STOP=1 \
    --command "$sSql"
}

bDatabaseExists() {
  local sExists
  sExists="$(
    psql "$PG_ADMIN_URL" --no-psqlrc --tuples-only --no-align \
      --set ON_ERROR_STOP=1 --set "target_database=$JAVA_DATABASE_NAME" \
      <<'SQL'
SELECT EXISTS (
  SELECT 1 FROM pg_catalog.pg_database WHERE datname = :'target_database'
);
SQL
  )"
  test "$sExists" = "t"
}

sClassifyDatabase() {
  if ! bDatabaseExists; then
    echo "missing"
    return
  fi

  local sFingerprint
  sFingerprint="$(sPsqlScalar "$PG_TARGET_ADMIN_URL" "
    SELECT
      to_regclass('public.users') IS NOT NULL,
      to_regclass('public.topics') IS NOT NULL,
      to_regclass('public.databasechangelog') IS NOT NULL,
      to_regclass('public._sqlx_migrations') IS NOT NULL,
      EXISTS (
        SELECT 1
          FROM pg_catalog.pg_attribute AS a
          JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid
          JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
           AND a.attnum > 0
           AND NOT a.attisdropped
           AND (c.relname, a.attname) IN (
             ('users', 'style'), ('users', 'settings'),
             ('users', 'force_unlogin'), ('topics', 'stat2'),
             ('topics', 'stat4'), ('topics', 'no_comments'),
             ('topics', 'image'), ('topics', 'warning_counter'),
             ('topics', 'score_loss'), ('comments', 'editor'),
             ('comments', 'editdate'), ('comments', 'topic_deleted'),
             ('sections', 'preformat'), ('sections', 'add_info'),
             ('sections', 'image_allowed'), ('groups', 'stat1'),
             ('groups', 'stat2'), ('groups', 'stat4'),
             ('images', 'userid'), ('images', 'filename'),
             ('adv_counts', 'id'), ('reactions_log', 'id'),
             ('reactions_log', 'msgid')
           )
      ),
      (
        SELECT count(*)
          FROM pg_catalog.pg_class AS c
          JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
           AND c.relkind IN ('r', 'p')
           AND c.relname NOT IN ('databasechangelog', 'databasechangeloglock')
      )
  ")"

  local bUsers bTopics bLiquibase bSqlx bLegacy iTables
  IFS='|' read -r bUsers bTopics bLiquibase bSqlx bLegacy iTables <<<"$sFingerprint"

  if test "$bLiquibase" = "t" && { test "$bSqlx" = "t" || test "$bLegacy" = "t"; }; then
    echo "mixed"
  elif test "$bLiquibase" = "t" && test "$bUsers" = "t" && test "$bTopics" = "t"; then
    echo "java"
  elif test "$bLiquibase" = "t"; then
    echo "unknown"
  elif test "$bSqlx" = "t" || test "$bLegacy" = "t"; then
    echo "legacy-rust"
  elif test "$iTables" = "0"; then
    echo "empty"
  else
    echo "unknown"
  fi
}

vWaitForPostgres() {
  local iAttempt
  for iAttempt in $(seq 1 30); do
    if pg_isready --dbname "$PG_ADMIN_URL" >/dev/null 2>&1; then
      return
    fi
    sleep 1
  done
  vDie "PostgreSQL did not become ready within 30 seconds"
}

vCreateFreshInfrastructure() {
  psql "$PG_ADMIN_URL" --no-psqlrc --set ON_ERROR_STOP=1 \
    --set "migration_role=$JAVA_DATABASE_MIGRATION_USER" \
    --set "migration_password=$JAVA_DATABASE_MIGRATION_PASSWORD" \
    --set "runtime_role=$JAVA_DATABASE_RUNTIME_USER" \
    --set "runtime_password=$JAVA_DATABASE_RUNTIME_PASSWORD" \
    --set "jamwiki_role=$JAVA_DATABASE_JAMWIKI_USER" <<'SQL'
SELECT format(
  'CREATE ROLE %I LOGIN PASSWORD %L NOSUPERUSER NOCREATEROLE CREATEDB',
  :'migration_role', :'migration_password'
) WHERE NOT EXISTS (
  SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = :'migration_role'
) \gexec
SELECT format(
  'CREATE ROLE %I LOGIN PASSWORD %L NOSUPERUSER NOCREATEROLE NOCREATEDB',
  :'runtime_role', :'runtime_password'
) WHERE NOT EXISTS (
  SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = :'runtime_role'
) \gexec
SELECT format(
  'CREATE ROLE %I LOGIN NOSUPERUSER NOCREATEROLE NOCREATEDB',
  :'jamwiki_role'
) WHERE NOT EXISTS (
  SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = :'jamwiki_role'
) \gexec
SQL

  psql "$PG_ADMIN_URL" --no-psqlrc --set ON_ERROR_STOP=1 \
    --set "target_database=$JAVA_DATABASE_NAME" \
    --set "migration_role=$JAVA_DATABASE_MIGRATION_USER" <<'SQL'
SELECT format('CREATE DATABASE %I OWNER %I', :'target_database', :'migration_role')
 WHERE NOT EXISTS (
   SELECT 1 FROM pg_catalog.pg_database WHERE datname = :'target_database'
 ) \gexec
SELECT format('ALTER DATABASE %I OWNER TO %I', :'target_database', :'migration_role')
 WHERE EXISTS (
   SELECT 1
     FROM pg_catalog.pg_database AS d
     JOIN pg_catalog.pg_roles AS r ON r.oid = d.datdba
    WHERE d.datname = :'target_database'
      AND r.rolname <> :'migration_role'
 ) \gexec
SQL

  psql "$PG_TARGET_ADMIN_URL" --no-psqlrc --set ON_ERROR_STOP=1 <<'SQL'
CREATE EXTENSION IF NOT EXISTS hstore;
CREATE EXTENSION IF NOT EXISTS fuzzystrmatch;
SQL
}

vLoadDemo() {
  local sErrorsFile sErrorLines sUnexpectedErrors iPsqlStatus
  sErrorsFile="$(mktemp)"

  set +e
  LC_ALL=C gzip --decompress --stdout "$sDir/sql/demo.db.gz" \
    | psql "$JAVA_DATABASE_MIGRATION_URL" --no-psqlrc > /dev/null 2>"$sErrorsFile"
  iPsqlStatus="${PIPESTATUS[1]}"
  set -e

  if test "$iPsqlStatus" -ne 0; then
    sed -n '1,120p' "$sErrorsFile" >&2
    rm -f "$sErrorsFile"
    vDie "psql failed while loading the canonical Java demo dump"
  fi

  sErrorLines="$(grep --extended-regexp '(ERROR|FATAL|PANIC):' "$sErrorsFile" || true)"
  sUnexpectedErrors="$(
    printf '%s\n' "$sErrorLines" \
      | grep --extended-regexp --invert-match \
        'extension "plpgsql" already exists|language "plpgsql" already exists|must be owner of (extension|language) plpgsql|unrecognized configuration parameter "default_with_oids"|tables declared WITH OIDS are not supported' \
      || true
  )"
  if test -n "$sUnexpectedErrors"; then
    printf '%s\n' "$sUnexpectedErrors" >&2
    rm -f "$sErrorsFile"
    vDie "the historical demo dump produced an unexpected PostgreSQL error"
  fi
  rm -f "$sErrorsFile"

  test "$(sPsqlScalar "$JAVA_DATABASE_MIGRATION_URL" "SELECT to_regclass('public.users') IS NOT NULL AND to_regclass('public.topics') IS NOT NULL")" = "t" \
    || vDie "the canonical Java demo dump did not create its baseline tables"
}

vRunLiquibase() {
  local sGoal="$1"
  mvn --batch-mode --no-transfer-progress --file "$sDir/pom.xml" "liquibase:$sGoal"
}

vValidateLiquibaseHistory() {
  local sActualRaw sActualRequired sExpectedRequired sActualProfile sInvalidExecutions
  sActualRaw="$(mktemp)"
  sActualRequired="$(mktemp)"
  sExpectedRequired="$(mktemp)"
  sActualProfile="$(mktemp)"

  cut --fields=2-5 "$sDir/liquibase-changesets.tsv" \
    | LC_ALL=C sort >"$sExpectedRequired"

  if ! LC_ALL=C psql "$JAVA_DATABASE_MIGRATION_URL" \
    --no-psqlrc --tuples-only --no-align --quiet --set ON_ERROR_STOP=1 \
    --command "COPY (
      SELECT id, author, filename, md5sum
        FROM databasechangelog
    ) TO STDOUT" >"$sActualRaw"; then
    rm -f "$sActualRaw" "$sActualRequired" "$sExpectedRequired" "$sActualProfile"
    vDie "failed to read the Liquibase history"
  fi
  LC_ALL=C sort "$sActualRaw" >"$sActualRequired"

  if ! cmp --silent "$sExpectedRequired" "$sActualRequired"; then
    echo "Liquibase identity/checksum set differs from the canonical 187-changeSet ledger:" >&2
    diff --unified=3 "$sExpectedRequired" "$sActualRequired" \
      | sed -n '1,120p' >&2 || true
    rm -f "$sActualRaw" "$sActualRequired" "$sExpectedRequired" "$sActualProfile"
    vDie "Liquibase history has a missing, additional or non-canonical identity/path/checksum row"
  fi

  if ! sInvalidExecutions="$(sPsqlScalar "$JAVA_DATABASE_MIGRATION_URL" "
      SELECT COALESCE(
        string_agg(
          format('%s/%s/%s=%s', filename, id, author, exectype),
          E'\\n' ORDER BY filename, id, author
        ),
        ''
      )
        FROM databasechangelog
       WHERE exectype IS NULL
          OR exectype NOT IN ('EXECUTED', 'MARK_RAN', 'RERAN')
    ")"; then
    rm -f "$sActualRaw" "$sActualRequired" "$sExpectedRequired" "$sActualProfile"
    vDie "failed to inspect Liquibase execution states"
  fi
  if test -n "$sInvalidExecutions"; then
    printf '%s\n' "$sInvalidExecutions" >&2
    rm -f "$sActualRaw" "$sActualRequired" "$sExpectedRequired" "$sActualProfile"
    vDie "Liquibase history contains a non-successful execution state"
  fi

  if ! LC_ALL=C psql "$JAVA_DATABASE_MIGRATION_URL" \
    --no-psqlrc --tuples-only --no-align --quiet --set ON_ERROR_STOP=1 \
    --command "COPY (
      SELECT
        row_number() OVER (ORDER BY orderexecuted, id, author, filename),
        id,
        author,
        filename,
        md5sum,
        exectype
        FROM databasechangelog
       ORDER BY orderexecuted, id, author, filename
    ) TO STDOUT" >"$sActualProfile"; then
    rm -f "$sActualRaw" "$sActualRequired" "$sExpectedRequired" "$sActualProfile"
    vDie "failed to read the Liquibase execution profile"
  fi

  if ! cmp --silent "$sDir/liquibase-changesets.tsv" "$sActualProfile"; then
    echo "database workflow warning: Liquibase relative order/execution profile differs from the fresh canonical bootstrap; identity/path/checksum set and successful states are compatible" >&2
    diff --unified=3 "$sDir/liquibase-changesets.tsv" "$sActualProfile" \
      | sed -n '1,80p' >&2 || true
  fi

  rm -f "$sActualRaw" "$sActualRequired" "$sExpectedRequired" "$sActualProfile"
}

vValidateSequenceHeadroom() {
  local sProblems
  sProblems="$(
    LC_ALL=C psql "$JAVA_DATABASE_MIGRATION_URL" \
      --no-psqlrc --tuples-only --no-align --quiet --set ON_ERROR_STOP=1 \
      --file "$sDir/check-sequence-headroom.sql"
  )" || vDie "failed to inspect canonical sequence headroom"

  if test -n "$sProblems"; then
    printf '%s\n' "$sProblems" >&2
    vDie "a canonical primary-key sequence is missing its mapping or can issue an existing ID"
  fi
}

vValidateCurrentJavaDatabase() {
  local sKind sTerminal
  sKind="$(sClassifyDatabase)"
  test "$sKind" = "java" \
    || vDie "expected a clean Java database, detected: $sKind"

  vRunLiquibase validate
  vValidateLiquibaseHistory

  sTerminal="$(sPsqlScalar "$JAVA_DATABASE_MIGRATION_URL" "
    SELECT count(*) = 1
      FROM databasechangelog
     WHERE id = '2026080501'
       AND author = 'Maxim Valyanskiy'
       AND filename = 'sql/updates/2026-08-05-userlog-userpic-idx.xml'
       AND md5sum = '8:d52bfe13718eea6a248d7c3abc488f2d'
       AND exectype = 'EXECUTED'
  ")"
  test "$sTerminal" = "t" \
    || vDie "terminal Liquibase changeset 2026080501 is absent or has an unexpected identity/checksum"

  vValidateSequenceHeadroom

  if test -n "$JAVA_DATABASE_RUNTIME_URL"; then
    test "$(sPsqlScalar "$JAVA_DATABASE_RUNTIME_URL" "
      SELECT to_regclass('public.users') IS NOT NULL
         AND to_regclass('public.topics') IS NOT NULL
         AND EXISTS (SELECT 1 FROM users LIMIT 1)
    ")" = "t" || vDie "runtime role cannot read the Java application schema"
  fi

  echo "Java database validation passed for all 187 canonical changesets and primary-key sequence headroom."
}

vBootstrap() {
  local sKind
  sKind="$(sClassifyDatabase)"

  case "$sKind" in
    java)
      echo "Existing Java database detected; bootstrap is validate-only."
      vValidateCurrentJavaDatabase
      return
      ;;
    missing|empty)
      test "${LOR_DB_BOOTSTRAP_CONFIRM:-}" = "bootstrap-empty-java-db" \
        || vDie "refusing to initialize $sKind database without LOR_DB_BOOTSTRAP_CONFIRM=bootstrap-empty-java-db"
      ;;
    legacy-rust|mixed|unknown)
      vDie "refusing to bootstrap database classified as $sKind; no automatic conversion or repair is safe"
      ;;
    *)
      vDie "internal error: unknown classification $sKind"
      ;;
  esac

  vCreateFreshInfrastructure
  test "$(sClassifyDatabase)" = "empty" \
    || vDie "target ceased to be empty before demo import"
  vLoadDemo
  vRunLiquibase update
  vValidateCurrentJavaDatabase
}

case "$sMode" in
  bootstrap|validate|classify) ;;
  *)
    vUsage
    exit 2
    ;;
esac

vRequireTools
vWaitForPostgres
"$sDir/check-vendor.sh"

case "$sMode" in
  bootstrap) vBootstrap ;;
  validate) vValidateCurrentJavaDatabase ;;
  classify) sClassifyDatabase ;;
esac
