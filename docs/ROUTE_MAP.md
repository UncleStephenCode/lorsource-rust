# Route map

Generated files:

- `docs/generated/original_routes.csv` — extracted from original Java+Scala Spring controllers.
- `docs/generated/original_surface.json` — WebSocket, urlrewrite, servlet/resource mappings and static roots.
- `docs/generated/rust_routes.csv` — extracted from Rust/Axum route declarations.
- `docs/generated/route_coverage.csv` — original endpoint to Rust route coverage.
- `docs/ROUTE_COVERAGE.md` — human-readable coverage report.

Regenerate:

```bash
ORIGINAL_ROOT=/path/to/original/lorsource ./scripts/run-compatibility-suite.sh
```

The generated report is intentionally structural: it compares normalized paths
and declared methods and lists missing/partial declarations. It does not establish
semantic parity. Parameters, headers, content negotiation, security, responses,
database effects and UI must be checked with endpoint-specific tests.
