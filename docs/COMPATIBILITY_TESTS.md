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

This verifies that known endpoints are not accidental 404s, browser routes such
as all three tracker URLs return HTML, trailing-slash contracts are preserved,
and protected endpoints return expected auth/permission statuses. The GitHub
Actions compatibility workflow now starts the complete Java-schema PostgreSQL +
OpenSearch + Rust stack and runs this matrix after the release-image build.

## HTTP checks against old and new apps

Run the original Scala app on one port and the Rust port on another, then:

```bash
OLD_BASE_URL=http://localhost:8081 \
NEW_BASE_URL=http://localhost:8181 \
python3 compat/test_http_compat.py --report /tmp/http-compat.json
```

The comparator keeps an independent cookie jar for each application and adds
the double-submit `CSRF_TOKEN` value to POST form data by default. A case can
set `"csrf_mode": "omit"` or `"invalid"` for a negative security contract.

The default old/new comparison remains coarse for legacy inventory cases:

- same status class, for example 2xx vs 2xx;
- redirect path when both redirect;
- content-type family for successful responses.

Cases marked `"compare_exact": true` compare exact status, media type and the
redirect path including its query string. Declarative per-side assertions are
also supported:

- `new_expected_status` / `old_expected_status`;
- `new_expected_content_type`;
- `new_expected_location`;
- `new_body_contains` and `new_body_not_contains`;
- `new_expected_cookie_names`;
- the corresponding `old_*` fields.

HTML is checked through stable protocol/DOM fragments rather than byte equality.
The optional JSON report records statuses, media types, redirect targets and
cookie names, but deliberately excludes bodies and cookie values so it is safe
to retain as rehearsal evidence.

For a production-clone rehearsal, `scripts/run-cutover-gate.sh` combines the
static inventory, dual-runtime matrix, critical browser probes, Java/Liquibase
database validation and the guarded posting/reaction and moderation stateful
flows. It requires explicit Java/Rust URLs, the original source tree,
`WRITE_FLOW_ALLOW_MUTATION=yes`, `MODERATION_FLOW_ALLOW_MUTATION=yes` and
explicit disposable moderation accounts; use only an operator-created clone.
A full pass additionally requires an immutable image digest, snapshot/WAL
identifiers, a redacted configuration manifest and media/external-adapter
evidence files. Skipping either stateful flow or release evidence produces a
dry-run result, never a cutover go/no-go pass.

## Stateful write-flow regression

`compat/test_write_flows.py` exercises the migration-critical browser path on
a disposable database: two logins, topic creation, canonical redirect, comma-
separated tag persistence, comment creation, reaction add/list/remove, the
collapsed/expanded reaction DOM, real multipart gallery upload, and both the
single-image and multi-image slider DOM modes. Its CI author is a disposable
moderator, so new-tag creation is covered through the previously fragile
moderator permission path. It refuses to mutate unless
`WRITE_FLOW_ALLOW_MUTATION=yes` is explicitly set. CI seeds two throw-away
accounts in the Compose volume and runs this check before deleting the volume.
The flow deliberately waits out the original per-IP 30-second topic flood
interval between its forum and gallery writes; a faster search-index refresh
must not make the regression nondeterministic.
Unit regressions additionally cover current-state filtering and Java-style
grouping of reaction notifications, the legacy `oldNotifications` switch and
the final reaction-history pagination boundary.

`compat/test_moderation_flows.py` is a separately guarded Compose regression
for `/usermod.jsp` and `/remove-userpic.jsp`. It exercises profile/userpic
cleanup, score penalties, corrector toggling, freeze/defrost, password reset,
block/unblock, score50 and block-with-mass-delete. The destructive graph covers
a deleted topic, a leaf-to-root comment chain, a skipped comment with a live
reply, event cleanup, unread-counter recalculation and `del_info`. It then
asserts canonical `users`, `ban_info` and `user_log` state including the Java
hstore audit payloads. The same flow proves that a score-50 non-moderator can
post topic/comment warnings with Java bean field names, localized events and
active moderator/corrector recipients; corrector clearing, open-warning and
lastmod semantics, and the five-per-hour limit are checked in the canonical
tables. CI seeds isolated targets and runs this test after the posting/reaction
workflow.
It likewise observes the original per-IP topic and high-score comment flood
intervals. A successful HTTP 200 JSON preview containing `errors` is treated
as a rejected comment, not as a completed write.
The flow also verifies the rendered warning message/section/author, strikeout
after clearing, DEL reason and score bonus, notification click-through to
`/view-deleted?id=...`, and the original 14-day non-frozen-author access path
to the deleted comment body.
