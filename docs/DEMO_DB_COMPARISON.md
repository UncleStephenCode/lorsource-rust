# Canonical Java demo database

The original `sql/demo.db` is a historical PostgreSQL dump, not a complete
current schema. It is vendored byte-for-byte (gzip-compressed) at
`compat/java-db/sql/demo.db.gz`, then brought to the current schema by all 187
Java Liquibase changesets.

Do not import the dump by itself and do not compare it to the offline Rust SQL
files as though either represented the current schema. Use the guarded workflow:

```bash
LOR_DB_BOOTSTRAP_CONFIRM=bootstrap-empty-java-db \
  compat/java-db/manage.sh bootstrap
compat/java-db/manage.sh validate
```

The old inventories in `docs/generated/original_demo_schema.*` describe only
the pre-Liquibase dump and are retained as historical analysis. The current
runtime contract is `compat/java-db/schema-contract.tsv`; the source of truth
remains the vendored Java changelog.

See `docs/DATABASE_COMPATIBILITY.md` for fresh-database, existing-Java and
cutover-clone procedures.
