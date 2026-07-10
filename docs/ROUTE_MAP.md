# Route map

Generated files:

- `docs/generated/original_routes.csv` — extracted from original Scala/Spring controllers.
- `docs/generated/rust_routes.csv` — extracted from Rust/Axum route declarations.
- `docs/generated/route_coverage.csv` — original endpoint to Rust route coverage.
- `docs/ROUTE_COVERAGE.md` — human-readable coverage report.

Regenerate:

```bash
ORIGINAL_ROOT=/path/to/original/lorsource ./scripts/run-compatibility-suite.sh
```

The Rust router now declares every original endpoint shape extracted by the current parser. In v4 there are no routes mapped to `legacy::not_implemented`; declaration coverage still is not the same as production functional parity, so service-level behavior must continue to be compared with endpoint-specific tests.
