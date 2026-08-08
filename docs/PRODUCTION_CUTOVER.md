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

## Rehearsal on a production clone

1. Restore a fresh snapshot into an isolated PostgreSQL instance.
2. Run `compat/java-db/manage.sh validate` with migration-owner credentials.
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
   CUTOVER_IMAGE_DIGEST=sha256:<64-hex-digest> \
   CUTOVER_SNAPSHOT_ID=<snapshot-id> \
   CUTOVER_WAL_POSITION=<wal-position> \
   CUTOVER_CONFIG_MANIFEST=/path/to/redacted-config.txt \
   CUTOVER_MEDIA_EVIDENCE=/path/to/media-rehearsal.txt \
   CUTOVER_EXTERNAL_EVIDENCE=/path/to/external-adapters.txt \
   EVIDENCE_DIR=/path/to/rehearsal-evidence \
   ./scripts/run-cutover-gate.sh
   ```

   The full gate fails closed unless the immutable image digest, database
   snapshot/WAL identity, redacted configuration, media rehearsal and external
   adapter evidence are supplied. For a deliberately read-only local dry run,
   `CUTOVER_REQUIRE_RELEASE_EVIDENCE=0` is allowed, but the script will not emit
   a cutover go/no-go pass.
6. Compare Java and Rust responses for login, profile/settings, topic and
   comment writes, moderation, tracker, search, reactions, gallery uploads and
   password recovery. Compare database rows and external side effects, not
   only status codes.
7. Exercise SIGTERM and verify that the process drains requests, exits, and
   restarts healthy as UID/GID 8181. Verify upload-volume ownership survives an
   upgrade from the older root-running image.
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
5. Rebuild/reindex OpenSearch if cross-version documents are incompatible.
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
