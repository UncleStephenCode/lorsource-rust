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

The Rust router now declares every original endpoint shape extracted by the current parser. Declaration coverage is not the same as functional parity: endpoints implemented through `legacy::not_implemented` still need their Scala business logic ported.
