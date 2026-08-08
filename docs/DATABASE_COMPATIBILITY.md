# PostgreSQL compatibility and operations

## Source of truth

The current Java/Liquibase schema is the only supported database contract.
The exact bootstrap inputs from Java commit
`2ddf930005adac28077cb6ad74d1481485f44096` are vendored under
`compat/java-db/sql/` with their original logical paths and SHA-256 manifest.

The historical Rust SQL files are offline reference material under
`compat/legacy-rust-db/offline-sql/`. They are not a migration chain and must
not be applied to either a fresh or an existing database.

## Runtime policy

The Rust process connects as the Java runtime role, normally `linuxweb`. At
startup it performs catalog-only validation of all 33 canonical tables and 214
columns, their PostgreSQL types/nullability, the five enums, 15 sequences, 12
database functions, two extensions and five enabled retained business triggers.
It performs no DDL or DML.

The validator does not read `databasechangelog`. This is deliberate: the Java
grants let `linuxweb` use application tables but not the Liquibase ledger.

Database classification is fail-closed:

| Classification | Evidence | Runtime/bootstrap result |
| --- | --- | --- |
| `java` | Liquibase ledger plus canonical Java markers, no SQLx/legacy markers | runtime structural validation; bootstrap becomes validate-only |
| `empty` / `missing` | no business tables / no target DB | runtime refuses; explicitly confirmed dev bootstrap may initialize |
| `legacy-rust` | `_sqlx_migrations` or known superseded Rust columns without Java ledger | refuse |
| `mixed` | Java ledger together with SQLx ledger or superseded columns | refuse |
| `unknown` | any other partial or unrelated schema | refuse |

This prevents a stale development migration from silently modifying a real
Java database and prevents an apparently successful start on an invented
parallel schema.

## Fresh disposable development database

Docker Compose runs the canonical bootstrap before the application:

```bash
docker compose up --build
```

For a local PostgreSQL process, set the administrative and migration URLs from
`.env.example`, then run:

```bash
LOR_DB_BOOTSTRAP_CONFIRM=bootstrap-empty-java-db \
  compat/java-db/manage.sh bootstrap
```

The confirmation authorizes creation of missing development roles/database,
ownership adjustment of an existing empty target, extension creation, demo
load and Liquibase update. The workflow never drops a database, table or
volume. It refuses to continue if any non-empty non-Java state is detected.

The historical demo dump is intentionally loaded with PostgreSQL's default
continue-on-error behavior because it contains obsolete `plpgsql` and OID
statements. Only the small, documented set of version-compatibility errors is
accepted; any other dump error aborts the workflow before Liquibase.

## Existing Java database and cutover clone

Use a clone/snapshot, never the only production copy. Supply operator-managed
credentials, then run:

```bash
compat/java-db/manage.sh validate
```

This uses the `maxcom`/migration-owner connection to run Liquibase validation
and requires the exact terminal changeset:

```text
id       2026080501
author   Maxim Valyanskiy
filename sql/updates/2026-08-05-userlog-userpic-idx.xml
md5sum   8:d52bfe13718eea6a248d7c3abc488f2d
```

It never runs `liquibase:update` on a detected Java database. Apply any missing
Java changesets using the established Java deployment procedure first, take a
new clone, then validate the clone again. After that, start Rust as `linuxweb`;
its independent structural validator catches missing/extra legacy columns even
though it cannot read the ledger.

## Schema-only diagnostics

A schema-only dump is useful for review but is not an executable bootstrap
source. Generating a snapshot after admin validation:

```bash
pg_dump "$JAVA_DATABASE_MIGRATION_URL" \
  --schema-only --no-owner --no-privileges \
  > /tmp/lorsource-java-schema.sql
sha256sum /tmp/lorsource-java-schema.sql
```

Do not commit this dump as a second schema authority. If the Java changelog is
updated, re-vendor it from a named Java commit, regenerate
`checksums.sha256`, rebuild `schema-contract.tsv` from an isolated migrated
database, and review the Rust SQL call sites against the changed contract.
