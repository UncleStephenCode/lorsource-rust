# Java-compatible PostgreSQL workflow

The Java demo dump plus the complete Liquibase chain in this directory are the
only active schema bootstrap source. The Rust application never creates,
upgrades, backfills or repairs schema objects at startup.

For a missing or empty disposable development database:

```bash
LOR_DB_BOOTSTRAP_CONFIRM=bootstrap-empty-java-db \
  compat/java-db/manage.sh bootstrap
```

For an existing Java database (read-only validation of schema history and
primary-key sequence headroom):

```bash
compat/java-db/manage.sh validate
```

Both commands fail closed for legacy Rust, mixed Liquibase/SQLx and unknown
schemas. `bootstrap` also becomes validate-only when it detects a Java database;
it does not run `liquibase:update` there. Updating a real Java database remains
an explicit Java/Liquibase operator operation outside application startup.

The application additionally runs a bounded, read-only `pg_catalog` validator
over the canonical 33 tables, 15 sequences and 12 retained functions. The
checked-in `schema-objects-contract.tsv` verifies required constraints,
defaults, index/function/trigger definitions, sequence parameters and
ownership links, and the minimum effective `linuxweb` grants. Direct ACL text
and different migration-owner role names are advisory provenance, so an
equivalent inherited-role grant remains compatible. Additional indexes and
grants are advisory; additional constraints and enabled triggers on canonical
tables are blocking because they can change writes. The contract also requires
canonical row-security flags and effective `linuxweb` function `EXECUTE`
grants. Schema presence, enum
kind/category, advisory schema/type ownership, and effective `linuxweb`
schema/enum `USAGE` are covered as well. Historical `CREATE` on `public` is
not a runtime requirement and is deliberately not blocking.

Operator validation compares the complete set of 187 ledger identities with
`liquibase-changesets.tsv`: ID, author, logical path and Liquibase 4.17.2
checksum must match, and each row must have a successful execution state
(`EXECUTED`, `MARK_RAN` or `RERAN`). Relative execution order and the exact
successful-state profile from the fresh bootstrap are reported as advisory
drift, because a legitimate long-lived Java ledger may differ after
conditional execution or rollback/reapply. This closes the gap where
Liquibase's own `validate` accepts a missing middle row when a later terminal
row is still present. The separate
`check-sequence-headroom.sql` performs reads only and rejects a next sequence
value that can collide with an existing primary key, falls outside its bounds,
or uses a non-canonical increment/cycle contract. It covers nine `OWNED
BY` sequences plus the four unowned generators mapped explicitly by current
Java DAOs (`s_uid`, `s_msgid`, `vote_id`, `votes_id`).

The query intentionally runs through the migration-owner connection: the
canonical `linuxweb` grants do not permit reading every sequence's current
state (for example `images_id_seq`). Runtime startup therefore remains a
bounded catalog-only check and does not scan application IDs; operators run
the data headroom gate before cutover.

For an *exact* regeneration check on a fresh disposable canonical bootstrap:

```bash
JAVA_DATABASE_RUNTIME_URL=postgres://linuxweb:linuxweb@localhost:5432/lor \
  bash compat/java-db/check-schema-object-contract.sh
```

That command performs catalog reads only. Exact equality is CI/provenance
evidence, not the more permissive production startup policy.

The bootstrap confirmation permits creating missing roles and the `lor`
database, changing ownership of an existing *empty* target database, loading
the demo fixture, and applying the vendored chain. It never drops a database,
table or Docker volume and never changes passwords of pre-existing roles.

Environment variables and their development defaults:

| Variable | Default | Purpose |
| --- | --- | --- |
| `PG_ADMIN_URL` | `postgres://postgres:postgres@localhost:5432/postgres` | cluster administrator connection |
| `PG_TARGET_ADMIN_URL` | `postgres://postgres:postgres@localhost:5432/lor` | administrator connection to target DB |
| `JAVA_DATABASE_MIGRATION_URL` | `postgres://maxcom:maxcom@localhost:5432/lor` | `psql` connection as schema owner |
| `JAVA_DATABASE_JDBC_URL` | `jdbc:postgresql://localhost:5432/lor` | Liquibase JDBC URL |
| `JAVA_DATABASE_RUNTIME_URL` | `postgres://linuxweb:linuxweb@localhost:5432/lor` | optional runtime grant smoke check |
| `JAVA_DATABASE_NAME` | `lor` | target database name |

Passwords and URLs above are development-only. Production operators must
provide secret-managed values. The runtime role is `linuxweb`; it is expected
not to have `SELECT` on `databasechangelog`.
