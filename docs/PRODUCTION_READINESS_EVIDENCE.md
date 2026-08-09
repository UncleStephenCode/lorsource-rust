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

| Requirement | Executable evidence | Current status |
|---|---|---|
| Pinned Java parity baseline | CI checks out `maxcom/lorsource` at `2ddf930005adac28077cb6ad74d1481485f44096`; `scripts/run-java-parity-runtime.sh` creates an isolated DB/OpenSearch runtime | **proved locally**; changing upstream baseline is an explicit reviewed CI edit |
| Static HTTP/schema inventory | `scripts/run-compatibility-suite.sh`, route extractor/coverage, CSRF audit, canonical 116-file/187-changeset vendor and SQL identifier audit | **proved locally**; inventory alone is not semantic parity |
| Differential browser contract | `compat/endpoints.json` plus `compat/test_http_compat.py --old ... --new ...`; CI retains `java-rust-http.json` | **proved locally**, 80/80 pinned Java/Rust cases |
| Rust quality and warnings | Docker `quality` target: format, all-target/all-feature check, tests and `clippy -D warnings`; release build uses `--locked` | **proved locally**, 193 Rust tests |
| Authentication/security | Spring cookie fixtures, legacy Jasypt/noop/bcrypt verification, global session hydration, CSRF audit and negative HTTP cases, safe redirects, proxy/origin/CSP tests | **proved locally** for covered contracts; real hash distribution and proxy topology require clone/config evidence |
| Posting and transactions | `compat/test_write_flows.py` creates a topic, split tags, comment and galleries and validates canonical redirects/content | **proved locally** on a fresh disposable Java-schema DB |
| Authorization/moderation | `compat/test_moderation_flows.py` verifies freeze/block/score/corrector/warnings/mass-delete plus canonical table, counter, audit and event state | **proved locally** on disposable fixtures |
| Markup modes and sanitization | `markup::tests`, preview HTTP cases, stateful topic/comment rendering and OpenSearch document tests | **proved locally** for HTML/LORCODE/MARKDOWN/TEXT and stored/reflected sanitization paths |
| Reactions and visibility | Widget unit tests plus stateful add/list/remove, zero-reaction reveal/hide and author notification checks | **proved locally** |
| Media and gallery modes | Image processing unit tests plus authenticated preview, single/multi-image DOM, derivatives and deleted-topic direct-URL transition | **proved locally**; restored production media ownership/CDN/backup evidence is a **release blocker** |
| Search | Mapping contract, query unit tests, durable spool tests and stateful create→index→search flow against OpenSearch | **proved locally**; production index capacity/rebuild rehearsal is operator evidence |
| Events/tracker/realtime | Event grouping/click/feed tests, stateful reaction/warning/delete events, unread counters, tracker last-comment contract and WebSocket protocol tests | **proved locally**; production load/timing evidence remains operator evidence |
| Background jobs | Scheduler/SQL/advisory-lock unit coverage, isolated SMTP/GeoIP/list/Telegram adapters and disabled-by-default production runtime | **proved locally** for deterministic contracts; one active scheduler and live egress are **release blockers** |
| Advertisement and last-login interceptors | `scripts/test-adv-counter.sh` and `scripts/test-lastlogin-interceptor.sh` | **proved locally**, including graceful flush and one-hour throttle |
| Java database compatibility | startup schema classifier, canonical Liquibase validator and current terminal changeset check | **proved locally** on the demo schema; restore/validate against a named production snapshot/WAL is a **release blocker** |
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
