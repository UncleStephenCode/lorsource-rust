# Production readiness evidence matrix

This document is the completion audit for the migration objective. A green
local check is not promoted to production proof when its scope is narrower.

Status meanings:

- **proved locally** — executable evidence exists in this repository and has
  passed against the canonical demo Java schema/runtime;
- **operator evidence required** — implementation and a fail-closed verifier
  exist, but the required production clone, storage or external dependency is
  not present in the repository;
- **release blocker** — cutover must not proceed while any required operator
  evidence is absent or failed.

## Current milestone rerun (2026-08-15)

This section is the authoritative status for the current edit, moderation,
comment-message and topic-deletion changes. A green result retained from an
older revision does not replace a rerun against this worktree.

| Gate | Current result |
|---|---|
| Static compatibility suite | **passed**: canonical Java vendor validation, generated route inventory, CSRF audit and Rust SQL identifier/schema audit completed; [`ROUTE_COVERAGE.md`](ROUTE_COVERAGE.md) and the generated artifacts are the source of truth for changing totals |
| Support-tool checks | **passed**: Python compilation and tool tests, JavaScript tests, shell syntax checks and `git diff --check` completed on the current worktree |
| Docker `quality` aggregate | **passed** in a clean image: locked release build, repository-wide formatting, all-target/all-feature check, 457 passing tests with 2 explicitly ignored tests, and `clippy -D warnings` |
| Rebuilt application | **passed**: the application image was rebuilt, the Compose service started, and `/healthz` returned healthy before the stateful checks |
| Targeted content lifecycles | **passed**: `edit_topic_lifecycle.py`, `comment_deletion_lifecycle.py` and `topic_deletion_lifecycle.py` completed against the rebuilt application and performed their scoped cleanup |
| Full disposable fixture suite | **passed**, 26/26 groups in `prod_ready_test/test_port.py` |
| Browser-created content and resume | **passed** from a clean checkpoint and again on immediate resume; the database tuple remained exactly `10 topics | 5 comments | 2 reactions | 9 votes`, with at most 2 authenticated browser contexts; result: `/tmp/prod_ready_browser_seed_result.json` |
| Seven-day local diagnostic | **passed**; persistence, poll percentages, browser-operation timings and a small concurrent public-read sample are recorded below and in `/tmp/prod_ready_7d_benchmark_final.json`; registration was explicitly excluded |
| Java↔Rust differential runtime | **passed**, 126/126 declared cases against pinned Java SHA `2ddf930005adac28077cb6ad74d1481485f44096`; report generated `2026-08-15T05:41:34Z`, SHA-256 `add67c20ebdf56d12145b13668f1adc8a55e7191d12b8707e27dfeb0589547b1` |
| Public production trailing-slash contract | **recorded by read-only exact probes**: `/forum` and `/forum/` return 200, while `/news`, `/articles`, `/gallery` and `/polls` return 404 without their canonical trailing slash; these cases are locked into the differential matrix |
| Visual browser smoke | **passed**: HTTP preflight and 52 desktop/mobile Chromium captures completed; this is structural smoke evidence, not a Java↔Rust perceptual-equivalence claim |

The rows below summarize retained repository-wide evidence. All milestone
results above remain local/disposable evidence and do not satisfy the external
completion conditions or release blockers below.

One incompatibility is deliberate: correctors in tags-only edit mode cannot
use a crafted request to modify existing URL/link or poll fields. Rust rejects
those protected-field deltas instead of reproducing the Java authorization
gap; see [`MODERATION_WORKFLOW_PARITY.md`](MODERATION_WORKFLOW_PARITY.md).

### Browser-seed and seven-day diagnostic details

Both browser-seed passes used the scoped disposable Compose fixture. The clean
run and the immediately resumed checkpoint run completed without duplicate
content: database counts before and after resume were identically 10 topics,
5 comments, 2 reactions and 9 vote rows. Peak live authenticated browser
contexts were 2. The browser result is
`/tmp/prod_ready_browser_seed_result.json`.

The 168-hour verifier passed with `registration_tested=false` and wrote
`/tmp/prod_ready_7d_benchmark_final.json`:

| Diagnostic | Recorded result |
|---|---|
| Topic day buckets | day 0..6: `13, 18, 2, 16, 16, 2, 17` |
| Topics by section ID | `1: 30`, `2: 41`, `3: 6`, `5: 4`, `6: 3` |
| Single-choice poll | 4 voters; percentages `[50, 25, 25]` |
| Multi-choice poll | 3 voters; percentages `[33, 100, 33]` |
| Browser activities | 56 operations; p50 43.98 ms, p95 2896.99 ms, max 3137.98 ms |
| Public read sample | 57 requests, concurrency 8; 889.15 requests/s; p50 3.88 ms, p95 33.33 ms, max 39.62 ms |

These timings are local disposable diagnostics only. The read sample is small,
loopback-based and excludes the production proxy/TLS, database, media,
OpenSearch and network topology. It is neither production capacity evidence
nor an SLO/load-limit claim; the production-clone, load and cutover conditions
below remain open.

| Requirement | Executable evidence | Current status |
|---|---|---|
| Pinned Java parity baseline | CI checks out `maxcom/lorsource` at `2ddf930005adac28077cb6ad74d1481485f44096`; `scripts/run-java-parity-runtime.sh` creates an isolated DB/OpenSearch runtime | **proved locally**; changing upstream baseline is an explicit reviewed CI edit |
| Static HTTP/schema inventory | `scripts/run-compatibility-suite.sh`, route extractor/coverage, CSRF audit, canonical 116-file/187-changeset vendor and SQL identifier audit | **proved locally**; inventory alone is not semantic parity |
| Differential browser contract | `compat/endpoints.json` plus `compat/test_http_compat.py --old ... --new ...`; CI retains `java-rust-http.json` | **proved locally** for every case declared by the current matrix; the artifact records the derived case total |
| Rust quality and warnings | Docker `quality` target: format, all-target/all-feature check, tests and `clippy -D warnings`; release build uses `--locked` | **proved locally** by the current CI run; the test output is the source of truth for its changing test total |
| Authentication/security | Spring cookie fixtures, legacy Jasypt/noop/bcrypt verification, global session hydration, CSRF audit and negative HTTP cases, safe redirects, proxy/origin/CSP tests | **proved locally** for covered contracts; real hash distribution and proxy topology require clone/config evidence |
| Profile/privacy/filter contracts | Java-source audit plus `compat/test_write_flows.py` covers edit fields/markup, remarks add/delete, ignore users, favorite comma-separated tags and private no-store HTML; the HTTP matrix covers anonymous denials | **proved locally** on disposable accounts |
| Posting and transactions | `compat/test_write_flows.py` creates a topic, split tags, comment and both galleries and validates canonical redirects/content | **proved locally** on a fresh disposable Java-schema DB |
| Authorization/moderation | `compat/test_moderation_flows.py` verifies freeze/block/score/corrector/warnings/mass-delete plus canonical table, counter, audit and event state | **proved locally** on disposable fixtures |
| Markup modes and sanitization | AST-based LORCODE/Markdown tests, database-aware user-reference corpus, preview HTTP cases, stateful topic/comment rendering, RSS and OpenSearch document tests | **proved locally** for the implemented source-derived corpus, cut modes, nofollow and hostile HTML/image cases; an exhaustive generated Java↔Rust parser corpus is still required before claiming byte-level parser parity |
| Reactions and visibility | Widget unit tests plus stateful add/list/remove, zero-reaction reveal/hide and author notification checks | **proved locally** |
| Themes and structural DOM | Theme mapping/bundle unit tests plus authenticated stateful switching through all seven Java theme IDs, stylesheet, header, `#bd` and `#ft` assertions | **proved locally**; exhaustive pixel comparison for every page remains non-gating diagnostic work |
| Media and gallery modes | Image processing unit tests plus authenticated preview, single/multi-image DOM, derivatives and deleted-topic direct-URL transition | **proved locally**; restored production media ownership/CDN/backup evidence is a **release blocker** |
| Search | Mapping contract, query unit tests, durable spool tests and stateful create→index→search flow against OpenSearch | **proved locally**; production index capacity/rebuild rehearsal is operator evidence |
| Events/tracker/realtime | Event grouping/click/feed tests, stateful reaction/warning/delete events, unread counters, tracker last-comment contract and WebSocket protocol tests | **proved locally**; production load/timing evidence remains operator evidence |
| Background jobs and replicas | Scheduler/SQL/advisory-lock unit coverage, `scripts/test-multi-instance-runtime.sh`, isolated SMTP/GeoIP/list/Telegram adapters and disabled-by-default production runtime | **proved locally** for advisory-lock contracts and cross-replica session/profile/theme reads; one active production scheduler and live egress are **release blockers** |
| Advertisement and last-login interceptors | `scripts/test-adv-counter.sh` and `scripts/test-lastlogin-interceptor.sh` | **proved locally**, including graceful flush and one-hour throttle |
| Java database compatibility | startup schema classifier, canonical Liquibase validator, terminal changeset check and exact 605-record PK/FK/UNIQUE/CHECK/default/index/sequence/owner/ACL catalog contract | **proved locally** on the demo schema through the runtime role; restore/validate against a named production snapshot/WAL is a **release blocker** |
| Mixed title representation | canonical Java write codec, one-layer display/read contracts, stateful exact DB-byte assertions and `tools/audit_title_representation.py` | future writes and covered reads are **proved locally**; a read-only report from a named production clone and operator classification of ambiguous historical rows are **release blockers** before any normalization |
| Production runtime hardening | `deploy/compose.production.yml`, `scripts/check-production-runtime.sh`, `scripts/test-production-runtime-shape.sh` | **proved locally** for UID/GID 8181, read-only rootfs, secret staging and media mount shape; immutable registry digest/operator host evidence required |
| Cutover/rollback | `scripts/run-cutover-gate.sh`, strict JSON evidence validator and `docs/PRODUCTION_CUTOVER.md` | read-only rehearsal passed; full go/no-go remains fail-closed without image, snapshot/WAL, media, external-adapter and rollback evidence |

## External completion conditions

The migration goal is complete only after one retained rehearsal directory
proves all of the following against the same release and rehearsal ID:

1. immutable registry image digest;
2. restored production-clone snapshot ID and PostgreSQL WAL position;
3. Rust and Java runtime comparison against that clone;
4. stateful write/moderation checks using explicitly disposable accounts;
5. representative restored media read/write/rename/cleanup and backup restore;
6. isolated OpenSearch, SMTP, CAPTCHA, GeoIP, TOR/domain-list and optional
   Telegram adapter checks from the production network;
7. exactly one enabled background scheduler deployment;
8. successful SIGTERM drain, rollback switch and post-rollback smoke checks.

`scripts/run-cutover-gate.sh` must be the final arbiter: disabling release
evidence, write flow, moderation flow or DB validation can produce useful
diagnostics but can never produce a cutover approval.
