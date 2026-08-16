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
It also validates the 728-record schema-object contract: required PK/FK/UNIQUE
and CHECK constraints, defaults, index definitions, function and trigger
definitions, sequence parameters/`OWNED BY` links, minimum effective
`linuxweb` table/sequence grants, effective canonical function `EXECUTE`
grants, relation RLS/forced-RLS flags, schema/enum semantics and effective
runtime schema/enum `USAGE`. Direct relation/function ACL text is retained as
advisory provenance, so equivalent inherited-role privileges do not block
startup. Replica-identity and clustered-index flags are operator metadata and
are not blocking application-schema requirements. Historical `CREATE` on
`public` is not required. It performs no DDL or DML.

The object query is bounded to named application relations/functions; it does
not scan application data, Liquibase rows or extension-owned functions.
Missing or changed canonical semantic objects stop startup. Additional
constraints and enabled triggers on canonical tables also stop startup because
they can reject, rewrite or add effects to Java-compatible writes. Additional
operator indexes/grants, direct-ACL provenance and owner-role-name differences
produce a bounded drift warning instead. This prevents an observability index
or deployment-specific role layout from making an otherwise compatible clone
unstartable while preserving evidence of divergence. Missing required
`linuxweb` grants remain blocking.

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

This uses the `maxcom`/migration-owner connection to run Liquibase validation,
then compares the complete set of 187 ledger rows against
`liquibase-changesets.tsv`. ID, author, logical path and checksum must match,
and the execution state must be successful (`EXECUTED`, `MARK_RAN` or
`RERAN`); a missing middle row is rejected even when the terminal row is still
present. Relative execution order and the exact successful-state profile are
advisory because long-lived Java databases can legitimately differ from a
fresh bootstrap after conditional execution or rollback/reapply. It also
performs a read-only headroom check for all 13 sequences that
allocate canonical application primary keys (nine `OWNED BY` mappings and the
four unowned mappings proved by current Java DAOs). The check also rejects a
changed increment/cycle contract and a next value outside configured sequence
bounds, including exhaustion on an empty target table. Finally, it retains the
explicit exact terminal gate:

```text
id       2026080501
author   Maxim Valyanskiy
filename sql/updates/2026-08-05-userlog-userpic-idx.xml
md5sum   8:d52bfe13718eea6a248d7c3abc488f2d
```

The headroom query uses the migration-owner connection. This is required by
the canonical Java ACLs: `linuxweb` cannot read every sequence state, and a
direct runtime-role run fails closed at `images_id_seq`. Rust startup remains
catalog-only and does not scan application ID values; the operator validation
must be run separately on the named cutover clone.

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
database, regenerate `schema-objects-contract.tsv` through
`export-schema-objects.sql`, and review the Rust SQL call sites against the
changed contracts. Validate exact reproducibility on that disposable database
with:

```bash
JAVA_DATABASE_RUNTIME_URL=postgres://linuxweb:linuxweb@localhost:5432/lor \
  bash compat/java-db/check-schema-object-contract.sh
```

The same catalog path can be exercised through SQLx without any mutation:

```bash
LOR_SCHEMA_INTEGRATION_CONFIRM=read-only-canonical-contract \
LOR_SCHEMA_INTEGRATION_DATABASE_URL=postgres://linuxweb:linuxweb@localhost:5432/lor \
  cargo test canonicalSchemaObjectContractMatchesRuntimeCatalog -- --ignored
```

The local 2026-08-15 evidence used PostgreSQL 16.14. It is not evidence that a
production clone, production privileges, or a different PostgreSQL major has
passed the comparison; those remain release rehearsal requirements.

## Mixed historical title representation

Java and older Rust write paths can leave distinguishable and ambiguous title
encodings in the same database. Never normalize them by a blanket startup
migration or by matching entity text alone. The fail-closed, strictly
read-only classifier, clone-role requirements, hashed artifacts and manual
review/rollback procedure are documented in
[`TITLE_REPRESENTATION_AUDIT.md`](TITLE_REPRESENTATION_AUDIT.md). A report from
the local demo database is not a substitute for running that procedure on a
named, provenance-backed production clone.
