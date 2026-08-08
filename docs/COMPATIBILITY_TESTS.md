# Compatibility tests

The compatibility tests are intentionally split into two levels.

## Static inventory checks

Regenerate route/schema reports:

```bash
ORIGINAL_ROOT=/path/to/original/lorsource ./scripts/run-compatibility-suite.sh
```

This rebuilds:

- `docs/generated/original_routes.json`
- `docs/generated/original_surface.json` (WebSocket, urlrewrite, servlet/resource mappings and static roots)
- `docs/generated/rust_routes.json`
- `docs/generated/rust_sql_schema_audit.json`
- `docs/generated/rust_sql_schema_audit.csv`
- `docs/ROUTE_COVERAGE.md`
- `docs/SCHEMA_COVERAGE.md`
- `docs/RUST_SQL_SCHEMA_AUDIT.md`

The route report is a declaration-level comparison only. A matching count does
not verify parameter defaults, headers/content negotiation, authentication,
redirects, HTML, database writes or external side effects.

The Rust SQL report checks statically visible table, alias-qualified column,
unambiguous `INSERT`/`UPDATE` column and enum-literal references against
`compat/java-db/schema-contract.tsv` plus the enum definitions in the vendored
Java dump/Liquibase updates. Dynamic composition and bind values still require
runtime tests; a zero-finding static report is not database-behavior parity.

## HTTP smoke checks against the Rust port

Start the Rust port:

```bash
docker compose up --build
```

Then run:

```bash
NEW_BASE_URL=http://localhost:8181 python3 compat/test_http_compat.py
```

This verifies that known endpoints are not accidental 404s and that protected endpoints return expected auth/permission statuses. v4 no longer expects explicit 501 placeholder statuses.

## HTTP checks against old and new apps

Run the original Scala app on one port and the Rust port on another, then:

```bash
OLD_BASE_URL=http://localhost:8081 \
NEW_BASE_URL=http://localhost:8181 \
python3 compat/test_http_compat.py
```

The smoke comparator checks coarse compatibility only:

- same status class, for example 2xx vs 2xx;
- redirect path when both redirect;
- content-type family for successful responses.

It does not compare exact HTML because JSP and Askama markup are expected to differ during the port.
