# Demo DB and migration comparison

The original `sql/demo.db` is a PostgreSQL dump, not SQLite. It is included as `sql/demo.db.gz`.

## Import original demo dump into a database

```bash
export DATABASE_URL=postgres://lor:lor@localhost:5432/lor
./scripts/import-original-demo.sh sql/demo.db.gz
```

## Static schema inventory

The repository includes a dependency-free parser:

```bash
./tools/extract_demo_schema.py sql/demo.db \
  --json docs/generated/original_demo_schema.json \
  --csv docs/generated/original_demo_schema.csv \
  --md docs/DB_SCHEMA_ORIGINAL.md
```

In this archive the inventory has already been generated from the uploaded source dump.

## Compare original inventory with Rust migrations

```bash
./tools/compare_schema_inventory.py \
  --original-json docs/generated/original_demo_schema.json \
  --migrations-dir db/migrations \
  --json docs/generated/schema_coverage.json \
  --md docs/SCHEMA_COVERAGE.md
```

`jam_*` tables are classified as `dropped-upstream`: they appear in the old demo dump, but the original Liquibase update history later removed JamWiki.

## Behaviour comparison on demo data

After both applications are running against equivalent demo data, use:

```bash
OLD_BASE_URL=http://localhost:8081 \
NEW_BASE_URL=http://localhost:8080 \
python3 compat/test_http_compat.py
```

The next step after smoke compatibility is to add endpoint-specific assertions for topic pagination, comment filtering, permissions, reactions, votes, moderation and notification workflows.
