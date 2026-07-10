# Compatibility tests

The compatibility tests are intentionally split into two levels.

## Static inventory checks

Regenerate route/schema reports:

```bash
ORIGINAL_ROOT=/path/to/original/lorsource ./scripts/run-compatibility-suite.sh
```

This rebuilds:

- `docs/generated/original_routes.json`
- `docs/generated/rust_routes.json`
- `docs/ROUTE_COVERAGE.md`
- `docs/SCHEMA_COVERAGE.md`

## HTTP smoke checks against the Rust port

Start the Rust port:

```bash
docker compose up --build
```

Then run:

```bash
NEW_BASE_URL=http://localhost:8080 python3 compat/test_http_compat.py
```

This verifies that known endpoints are not accidental 404s and that intentionally unported endpoints return their expected placeholder status.

## HTTP checks against old and new apps

Run the original Scala app on one port and the Rust port on another, then:

```bash
OLD_BASE_URL=http://localhost:8081 \
NEW_BASE_URL=http://localhost:8080 \
python3 compat/test_http_compat.py
```

The smoke comparator checks coarse compatibility only:

- same status class, for example 2xx vs 2xx;
- redirect path when both redirect;
- content-type family for successful responses.

It does not compare exact HTML because JSP and Askama markup are expected to differ during the port.
