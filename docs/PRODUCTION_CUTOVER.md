# Production cutover and rollback

This runbook is a release gate, not a claim that the port is already ready to
replace Java. Use an operator-created clone/snapshot of the real database for
every rehearsal. The Rust process never runs Liquibase and never modifies the
schema at startup.

## Required inputs

- an immutable Rust image identified by digest;
- a current Java PostgreSQL snapshot and a tested restore procedure;
- the existing gallery/userpic storage mounted read-write at `UPLOAD_DIR`;
- production OpenSearch, SMTP and CAPTCHA endpoints;
- validated `SMTP_HOST`, non-zero `SMTP_PORT` and RFC-shaped
  `SMTP_HELO_NAME`; these are now parsed by the central production
  configuration instead of being read lazily on first delivery;
- a valid crash-report recipient in `ADMIN_EMAIL`;
- independent `COOKIE_SECRET` and `SITE_SECRET` values of at least 32 bytes;
- `PUBLIC_URL=https://...`, matching `WS_URL=wss://.../`;
- the exact reverse-proxy CIDRs in `TRUSTED_PROXY_CIDRS`;
- `LOR_ENV=production` and `ENABLE_DEV_BYPASSES=false`;
- dashboards/alerts for HTTP 5xx, latency, PostgreSQL errors, OpenSearch
  failures, SMTP failures and container restarts.

Telegram errors are deliberately reduced to status/error classes because the
bot token is embedded in the request URL. Treat any appearance of `/bot<TOKEN>`
in retained logs as a credential incident and rotate the token before cutover.

The process fails closed on missing/insecure production configuration and on a
database that does not match the vendored Java/Liquibase contract.
Release images are built with `cargo build --release --locked`; a changed or
incomplete `Cargo.lock` therefore fails the image build instead of silently
changing the dependency graph.

Use `deploy/compose.production.yml` as the production-shape baseline. It
requires an image reference pinned by digest, runs the application process as
UID/GID 8181 with a read-only root filesystem and drops all Linux capabilities
except the four needed by the entrypoint to read mounted secrets and lower its
identity (`CHOWN`, `DAC_READ_SEARCH`, `SETGID`, `SETUID`). Secret files are
copied into `/tmp` tmpfs as mode `0400`, owned by 8181, before `gosu`
irreversibly starts the Rust process as that user. The manifest
binds the HTTP port to loopback for a local TLS proxy and mounts database,
cookie, site and CAPTCHA secrets as files. `DATABASE_URL`, `COOKIE_SECRET`,
`SITE_SECRET`, `CAPTCHA_PRIVATE_KEY` and `TELEGRAM_TOKEN` support mutually
exclusive `*_FILE` forms; secret contents are never printed. The baseline
manifest deliberately disables optional Telegram publishing. Enable it only
through an operator-owned override that mounts `TELEGRAM_TOKEN_FILE` and sets
the required fallback proxy. The dedicated media bind uses the private SELinux
`Z` relabel; confirm that this matches the host MAC policy before the first
rehearsal and never point it at a shared or broad filesystem root.

Before starting that manifest, run:

```bash
LORSOURCE_IMAGE=registry.example/lorsource@sha256:<64-hex-digest> \
UPLOAD_HOST_PATH=/srv/lorsource/uploads \
DATABASE_URL_SECRET_FILE=/secure/database-url \
COOKIE_SECRET_SOURCE=/secure/cookie-secret \
SITE_SECRET_SOURCE=/secure/site-secret \
CAPTCHA_PRIVATE_KEY_SOURCE=/secure/captcha-private-key \
PUBLIC_URL=https://www.linux.org.ru \
WS_URL=wss://www.linux.org.ru/ \
TRUSTED_PROXY_CIDRS=<proxy-cidrs> \
OPENSEARCH_URL=<search-url> \
CAPTCHA_PUBLIC_KEY=<site-key> \
SMTP_HOST=<mta-host> \
SMTP_HELO_NAME=www.linux.org.ru \
ADMIN_EMAIL=<operations-mailbox> \
ENABLE_BACKGROUND_JOBS=false \
scripts/check-production-runtime.sh
```

The preflight rejects mutable image tags, insecure secret-file permissions,
shared cookie/site secrets, missing media directories and wrong media
ownership. It also performs a write/read/atomic-rename/cleanup probe as UID
8181 and validates the fully interpolated Compose model without displaying it.
For local development only, a non-pushable BuildKit image ID can be bound to a
local tag with `LORSOURCE_PREFLIGHT_LOCAL_IMAGE` plus
`LORSOURCE_PREFLIGHT_ALLOW_LOCAL_IMAGE=yes`; the script verifies that the IDs
match and marks the result as development evidence, never production evidence.
`/healthz` remains a process liveness endpoint. `/readyz` checks PostgreSQL and
the Java-compatible OpenSearch index; the image healthcheck uses `/readyz` so
an instance is not considered ready while either required dependency is down.

## Rehearsal on a production clone

1. Restore a fresh snapshot into an isolated PostgreSQL instance.
2. Run `compat/java-db/manage.sh validate` with migration-owner credentials.
   When Maven is absent on the operator host, the cutover gate invokes the
   same validator through the repository's `db-bootstrap` Compose service.
3. Start Rust against the clone as the Java runtime role (`linuxweb`). Do not
   provide migration-owner credentials to the application.
4. Point SMTP, CAPTCHA and OpenSearch to isolated test services. Copy a
   representative read-only snapshot of uploaded media.
   The Rust suite covers isolated GeoIP and TOR/disposable-feed HTTP contracts;
   retain successful production-network egress probes as additional evidence.
5. Run the static suite, the HTTP matrix, the authenticated stateful write
   flow and the user-moderation transaction/audit flow documented in
   `docs/COMPATIBILITY_TESTS.md`. Pre-create the explicitly named disposable
   moderator, profile/score targets, mass-delete target and corrector on the
   isolated clone; the script never creates or selects a real user implicitly.
   The complete local gate can be invoked as:

   ```bash
   ORIGINAL_ROOT=/path/to/lorsource-java \
   OLD_BASE_URL=http://127.0.0.1:8081 \
   NEW_BASE_URL=http://127.0.0.1:8181 \
   WRITE_FLOW_ALLOW_MUTATION=yes \
   MODERATION_FLOW_ALLOW_MUTATION=yes \
   MODERATION_FLOW_MODERATOR_NICK=<disposable-moderator> \
   MODERATION_FLOW_MODERATOR_PASSWORD=<test-password> \
   MODERATION_FLOW_TARGET_NICK=<disposable-target> \
   MODERATION_FLOW_LOW_NICK=<disposable-score50-target> \
   MODERATION_FLOW_LOW_PASSWORD=<test-password> \
   MODERATION_FLOW_DELETE_NICK=<disposable-mass-delete-target> \
   MODERATION_FLOW_DELETE_PASSWORD=<test-password> \
   MODERATION_FLOW_CORRECTOR_NICK=<disposable-corrector> \
   MODERATION_FLOW_CORRECTOR_PASSWORD=<test-password> \
   STATEFUL_DATABASE_URL_FILE=/run/secrets/rehearsal-database-url \
   STATEFUL_DATABASE_IS_DISPOSABLE=yes \
   STATEFUL_EXPECTED_DATABASE=lorsource_rehearsal \
   CUTOVER_IMAGE_DIGEST=sha256:<64-hex-digest> \
   CUTOVER_SNAPSHOT_ID=<snapshot-id> \
   CUTOVER_WAL_POSITION=<wal-position> \
   CUTOVER_CONFIG_MANIFEST=/path/to/redacted-config.json \
   CUTOVER_MEDIA_EVIDENCE=/path/to/media-rehearsal.json \
   CUTOVER_EXTERNAL_EVIDENCE=/path/to/external-adapters.json \
   EVIDENCE_DIR=/path/to/rehearsal-evidence \
   ./scripts/run-cutover-gate.sh
   ```

   `STATEFUL_DATABASE_URL_FILE` lets the moderation verifier inspect the same
   isolated clone used by the Rust runtime instead of assuming the development
   Compose database. It must be a private, one-line PostgreSQL URL file. Remote
   mode refuses to run without the explicit disposable marker and an exact
   database-name match; credentials are passed to `psql` through libpq
   environment variables rather than command arguments. Omit these three
   variables only for the repository's disposable Compose stack.

   The full gate fails closed unless the immutable image digest, database
   snapshot/WAL identity, redacted configuration, media rehearsal and external
   adapter evidence are supplied. These three files use the strict JSON
   contract documented in `docs/CUTOVER_EVIDENCE.md`: they must share a fresh
   rehearsal ID and match the supplied digest, snapshot and PostgreSQL LSN.
   Empty files, stale reports and placeholder identifiers are rejected. For a
   deliberately read-only local dry run,
   `CUTOVER_REQUIRE_RELEASE_EVIDENCE=0` is allowed, but the script will not emit
   a cutover go/no-go pass.
6. Compare Java and Rust responses for login, profile/settings, topic and
   comment writes, moderation, tracker, search, reactions, gallery uploads and
   password recovery. Compare database rows and external side effects, not
   only status codes. For local passive HTTP comparison, use the guarded
   `scripts/run-java-parity-runtime.sh` workflow documented in
   `docs/COMPATIBILITY_TESTS.md`; its Java database and OpenSearch node are
   isolated from the Rust runtime.
7. Exercise SIGTERM and verify that the process drains requests, exits, and
   restarts healthy as UID/GID 8181. Verify upload-volume ownership survives an
   upgrade from the older root-running image. The hardened manifest does not
   chown media on startup: pre-existing storage must pass the UID/GID 8181
   preflight before the non-root container starts.
8. Restore the snapshot again and repeat until the run is reproducible.

Do not proceed while a release-blocking difference remains or while a required
external adapter has not passed the isolated rehearsal. During dual-runtime
comparison set `ENABLE_BACKGROUND_JOBS=false` on the passive Rust replica so
Java and Rust do not both run ratings, cleanup or external-publishing jobs.
At cutover, set it to `true` on exactly the active Rust scheduler deployment;
PostgreSQL advisory locks protect accidental overlap between Rust replicas.

## Cutover

1. Announce a write freeze and drain/stop Java writers. Never run Java and Rust
   as concurrent writers during the first migration.
2. Record the Java image/version, database WAL position, OpenSearch index
   aliases and upload-storage snapshot. Take a final database snapshot.
3. Re-run `compat/java-db/manage.sh validate`; it is validate-only for an
   existing Java database.
4. Start the pinned Rust image with production configuration. Require the
   container healthcheck and `/healthz` to pass before adding it upstream.
5. Run read-only critical-path smoke checks, then one controlled authenticated
   topic/comment/reaction workflow in a designated operational test group.
6. Switch the reverse proxy gradually. Monitor errors, latency, PostgreSQL
   locks/connections, OpenSearch indexing, WebSocket sessions, SMTP and upload
   writes.
7. Keep the stopped Java deployment and final snapshot immediately available
   for the entire rollback window.

## Rollback

1. Remove Rust from the proxy and stop new Rust writes.
2. Stop Rust gracefully and record its last logs and database WAL position.
3. Start the recorded Java release against the same database. Rust uses the
   Java schema and performs no DDL, so ordinary compatible writes do not
   require a database downgrade.
4. Verify login, topic/comment creation, media access and moderation before
   restoring traffic.
5. Rebuild/reindex OpenSearch if the fixed `messages` mapping is incompatible.
   Rust validates the Java mapping at production startup and refuses traffic
   when analyzer, `message.raw`, term-vector or field-type requirements differ;
   after taking a recoverable OpenSearch snapshot, run the guarded
   `scripts/rebuild-search-index.sh`, start Rust and invoke
   `/admin/search-reindex` with `action=all`. Retain the snapshot until indexed
   counts are reconciled, then retry cutover.
6. Restore the final snapshot only for confirmed data corruption; doing so
   discards every post-cutover write and therefore requires an explicit data
   recovery decision.

## Evidence to retain

- image digest and configuration manifest with secrets redacted;
- database validation output and snapshot/WAL identifiers;
- static, Rust, Docker, HTTP and stateful regression results;
- canonical `users`, `ban_info` and `user_log` assertions from the moderation
  regression;
- old/new endpoint comparison report;
- cutover and rollback timestamps;
- observed errors and the final go/no-go decision.
