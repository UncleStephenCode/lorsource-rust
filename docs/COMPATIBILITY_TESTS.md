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

The frozen-snapshot rerun on 2026-08-16 passed all 113 compatibility tests.
The route inventory contains 172 Rust declarations and 193 expanded Java
mappings: 174 are method-declared, 19 are structural partials and none are
missing or mismatched. The SQL audit examined 941 literals: 793 clean, 148
review-required, 0 invalid references/findings and 23 runtime probes.

The clean Docker `quality` target also passed repository-wide formatting,
all-target/all-feature checking, 774 passing tests plus 10 explicitly ignored
(784 total), no failures, and Clippy with warnings denied. Its manifest-list
digest is
`sha256:021c5e2ab962df45047f9f6eef87f4019cf78690c914aeccc0b847fad8ea4fb6`.
The rebuilt application manifest-list digest is
`sha256:2ba53e4f7b274ca227a51843248e8ad35db6d7d765ab427fbc60801ebba0323a`;
Compose reached healthy state and the production runtime-shape check passed.

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

Read-only exact probes of the current public instance on 2026-08-15 established
that both `/forum` and `/forum/` return 200, while `/news`, `/articles`,
`/gallery` and `/polls` return 404 without their canonical trailing slash. The
matrix preserves these distinctions instead of applying one blanket slash
redirect policy to section roots.

## HTTP checks against old and new apps

The repository includes a guarded local comparator launcher. It clones only
the fixed disposable `lor_java_parity` database, runs Java on port 8081 and
uses an independent ephemeral OpenSearch node so Java's canonical mapping is
not confused with an index created by the Rust runtime:

```bash
ORIGINAL_ROOT=/path/to/original/lorsource \
  ./scripts/run-java-parity-runtime.sh recreate
```

The normal Rust Compose stack must already be running. The clone command
refuses to target `lor`, and Java background jobs affect only the cloned
database. The launcher also creates the empty `messages` index using the exact
analysis, mapping and term-vector definition from Java's `MessageIndex`; this
keeps passive search requests valid without invoking a write-side reindex.
`recreate` waits for a successful Java HTTP response and prints the final
startup logs on failure. Once it returns, run:

```bash
OLD_BASE_URL=http://localhost:8081 \
NEW_BASE_URL=http://localhost:8181 \
python3 compat/test_http_compat.py --report /tmp/http-compat.json
```

Use `scripts/run-java-parity-runtime.sh stop` after the comparison. The Java
index is intentionally ephemeral; each fresh comparator creates its exact
mapping before search checks run.

The compatibility workflow performs the same comparison in CI. It checks out
`maxcom/lorsource` at the explicit `JAVA_BASELINE_SHA`, regenerates the static
route/schema inventory from that tree, starts the isolated Java comparator and
runs every case declared in `compat/endpoints.json` against both runtimes. The
resulting credential-free
`java-rust-http.json` is retained as the `java-rust-http-parity` artifact.
Updating the baseline SHA is therefore an explicit reviewed compatibility
change rather than an unobserved move of the upstream default branch.

The 2026-08-16 milestone run passed all 211 declared cases against pinned Java
SHA `2ddf930005adac28077cb6ad74d1481485f44096`:

- core 126/126: `/tmp/java-rust-endpoints-final-20260816.json`, generated
  `2026-08-16T09:14:16.772360+00:00`, SHA-256
  `9a186115c96f734b9b214c89ebef3f9506f091cb436bc768e9b5e29c1f52589a`;
- API/HTTP edge 71/71: `/tmp/java-rust-api-final-20260816.json`, generated
  `2026-08-16T09:14:12.142324+00:00`, SHA-256
  `6f4c7f0306373881666ce3e92c3c21103c7973feb4e13890ffb6aade3ba7d6d9`;
- conditioned UI 14/14: `/tmp/java-rust-ui-final-20260816.json`, generated
  `2026-08-16T09:14:12.706334+00:00`, SHA-256
  `5626a978c544139165f319501a8acfb10dae32453f180e6bf8a292e40d6ec9d8`.

The same rebuilt runtime passed all 30 disposable fixture groups, the 56-view
visual smoke, the 42-case seven-theme matrix, and the browser commenting
lifecycle. These remain local/disposable evidence and do not satisfy the
production cutover requirements described below.

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

Static-asset cases additionally compare exact `Cache-Control` values for theme
CSS, query cachebusters, OpenSans, webjars, ordinary images, advertisement
images and the original queried-PNG regex edge case. Fresh-session cases also
assert the exact cookie-name set: Spring Security-excluded CSS/JS/image/font
responses must not create `CSRF_TOKEN`, while `manifest.json`, `robots.txt` and
`qrerror` resources remain inside the security chain. The Java-generated
runtime bundles and error-page assets can be refreshed reproducibly with
`ORIGINAL_ROOT=... make static-sync` after building the original webapp.
The seed matrix also locks the intentionally different access contracts for
topic and comment edit-history pages and verifies both diff scripts and their
DOM loader hooks.

For a production-clone rehearsal, `scripts/run-cutover-gate.sh` combines the
static inventory, dual-runtime matrix, critical browser probes, Java/Liquibase
database validation and the guarded posting/reaction and moderation stateful
flows. It requires explicit Java/Rust URLs, the original source tree,
`WRITE_FLOW_ALLOW_MUTATION=yes`, `MODERATION_FLOW_ALLOW_MUTATION=yes` and
explicit disposable moderation accounts; use only an operator-created clone.
A full pass additionally requires an immutable image digest, snapshot/WAL
identifiers, a redacted configuration manifest and media/external-adapter
evidence files, plus operational evidence for the production clone, verified
JVM/JDBC timezone, exactly one scheduler, ActiveMQ drain or full search reindex,
SIGTERM and rollback. The retained search probe/reconciliation artifact is
SHA-256-bound to that evidence. Skipping either stateful flow or release
evidence produces a dry-run result, never a cutover go/no-go pass.

The moderation verifier normally reads assertion rows through the local
Compose PostgreSQL service. When the Rust runtime is pointed at an external
isolated clone, set `STATEFUL_DATABASE_URL_FILE`,
`STATEFUL_DATABASE_IS_DISPOSABLE=yes` and `STATEFUL_EXPECTED_DATABASE`. The URL
file must be private and contain one PostgreSQL URL line. The verifier checks
the URL database name and the connected `current_database()` before issuing
the HTTP mutations; credentials are not placed in the `psql` argument list.

## Stateful write-flow regression

`compat/test_write_flows.py` exercises the migration-critical browser path on
a disposable database: two logins, all seven profile themes and their
stylesheet/header/footer DOM, Java profile-edit fields and Markdown rendering,
private remarks, ignored users, favorite comma-separated tags, topic creation,
canonical redirect, exact Java HTML-escaped title bytes plus single-decoded DOM
text for all five HTML-significant characters, comma-separated topic-tag
persistence, comment creation,
reaction add/list/remove, the collapsed/expanded reaction DOM, real multipart
gallery upload, both the single-image and slider DOM modes, authenticated-only
preview access, and the direct-image visibility transition after topic
deletion. The test asserts that private filter state is non-cacheable HTML and
that an anonymous direct image URL is rejected while the author retains
history access. Its CI author is a disposable
moderator, so new-tag creation is covered through the previously fragile
moderator permission path. It refuses to mutate unless
`WRITE_FLOW_ALLOW_MUTATION=yes` is explicitly set. CI seeds two throw-away
accounts in the Compose volume and runs this check before deleting the volume.
The reaction part also verifies that the owner notification RSS contains the
original reaction note and rendered target-message body. The comment is posted
by the second account so the same flow proves that tracker activity shows the
last-comment author, links with the matching `lastmod=<cid>`, and that an
anonymous `/tracker.jsp` request preserves Java's default `filter=all` through
an exact 302 redirect.
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

`compat/test_account_flows.py` is the guarded self-service deregistration
regression. It verifies forced hCaptcha and Spring-style in-form validation,
then checks that profile cleanup, `ban_info` and `user_log` commit with the
original self-block reason. It also locks the stateless Java remember-me cookie
behavior and proves that the blocked account cannot authenticate again.

`scripts/test-multi-instance-runtime.sh` starts a temporary second Compose app
replica with the same PostgreSQL, secrets and media volume. It logs in through
the primary instance and requires the resulting authenticated profile and
saved theme to resolve through the second instance. The script is guarded by
`MULTI_INSTANCE_ALLOW_MUTATION=yes` because login updates `lastlogin`, and it
always removes its cookie fixture and temporary replica.

`scripts/test-adv-counter.sh` sends three successful advertisement requests
and one 404, performs a graceful app stop, verifies the exact `adv_counts`
delta, and starts the application again. It requires
`ADV_COUNTER_ALLOW_MUTATION=yes` because the canonical counter table changes.

`scripts/test-lastlogin-interceptor.sh` authenticates a disposable user and
requests `/about`, whose handler does not extract `CurrentUser`. It proves that
the global interceptor refreshes a two-hour-old `lastlogin` while leaving a
30-minute-old value byte-for-byte unchanged, matching Java's one-hour gate.
