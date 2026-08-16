# Cutover evidence contract

`scripts/run-cutover-gate.sh` accepts four JSON evidence documents plus a
strict JSON ActiveMQ probe or full-reindex reconciliation artifact. Merely
creating empty files is not sufficient: the validator binds every document to
the same release image digest, restored database snapshot, PostgreSQL WAL LSN
and rehearsal ID. Evidence is accepted for seven days by default; override
`CUTOVER_EVIDENCE_MAX_AGE_HOURS` only when the release process explicitly
defines another retention window.

Every document has this envelope:

```json
{
  "schema_version": 1,
  "kind": "configuration",
  "rehearsal_id": "prod-rehearsal-20260808-001",
  "captured_at": "2026-08-08T18:00:00Z",
  "image_digest": "sha256:<64 lowercase hex characters>",
  "database_snapshot_id": "prodclone-20260808-001",
  "database_wal_position": "16/B374D848",
  "status": "passed",
  "evidence": {}
}
```

The configuration document (`kind: configuration`) must set each of these
boolean checks to `true` inside `evidence`:

- `lor_env_production`;
- `public_https` and `websocket_wss_same_authority`;
- `runtime_database_role_least_privilege`;
- `java_site_secret_continuity_verified` and `secret_values_redacted`;
- `trusted_proxy_cidrs_configured`;
- `opensearch_configured`, `captcha_configured`, `smtp_configured` and
  `admin_email_configured`;
- `dev_bypasses_disabled`;
- `one_active_background_scheduler`;
- `scheduler_timezone_configured` and `legacy_jdbc_timezone_configured`;
- `telegram_proxy_configured_if_enabled`.

Do not store secret values, passwords, tokens or private CAPTCHA keys in this
document.

`scheduler_timezone_configured` means the same explicit IANA name was supplied
as both `SCHEDULER_TIMEZONE` and process `TZ`. Rust validates that equality at
startup; this binds Spring-style cron and Java `ZonedDateTime.plus(Period)`
moderation periods to the same system-default zone used by the original JVM.

`java_site_secret_continuity_verified` means that the mounted `SITE_SECRET`
was compared through a credential-free probe with the Java `Secret` used by
the rehearsed source deployment.  The value must remain the same during an
in-place migration: current Java uses that single secret both as the
remember-me signing key and as the base for activation/reset tokens.  Requiring
a separate cookie key would invalidate live Java cookies and would not be
behaviorally compatible.  Evidence records only the successful comparison,
never either secret value.

The media document (`kind: media`) records the dedicated absolute upload root,
runtime UID/GID `8181`, the exact directories `photos`, `gallery`, `images`, a
positive representative-file count and an explicit storage snapshot ID. It
must confirm successful read, write, atomic rename, cleanup, restart ownership
and backup/restore probes.

The external document (`kind: external-adapters`) contains exactly these
adapter keys: `opensearch`, `smtp`, `captcha`, `geoip`, `tor_exit_list`,
`disposable_email_domains`, and `telegram`. Each adapter records a fresh
`checked_at`, a credential-free endpoint, `status: passed`, and
`contract_verified: true`. Telegram alone may be `disabled`; in that case it
requires a concrete `disabled_reason` and `contract_verified: false`.

The operations document (`kind: operations`) is the fail-closed production
clone and lifecycle gate. Its `evidence` object contains exactly:

- `production_clone`: successful restore, Liquibase validation, Rust runtime
  schema-contract validation and Java↔Rust comparison against that clone;
- `scheduler`: the evidenced original JVM timezone, Rust scheduler timezone
  and `LEGACY_JDBC_TIMEZONE`. All three must be valid, identical IANA names;
  `active_scheduler_instances` must be exactly `1` and both verification flags
  must be true;
- `lifecycle`: successful SIGTERM drain, healthy restart, rollback switch and
  post-rollback smoke checks;
- `search_cutover`: one of the two strict alternatives below.

For an orderly queue drain use `mode: activemq-drained`. Capture the broker
probe only after Java writers have stopped. The document must name exactly
`lor.searchQueue`, record zero `ready_messages` and `inflight_messages`, prove
Java consumers/writers stopped, and record zero Rust `pending`/`processing`
spool files.

If zero legacy queue depth cannot be proved, use `mode: full-reindex`. Stop the
Java writers and consumers, take a recoverable OpenSearch snapshot, explicitly
record the legacy queue disposition, run a complete Rust reindex, and reconcile
a positive `expected_documents` count exactly with `indexed_documents`. At
least one representative query must be checked and the Rust pending/processing
spool counts must both be zero. Count equality is not sufficient: record
canonical expected/indexed SHA-256 digests for both the sorted document-ID set
(`expected_id_set_sha256`, `indexed_id_set_sha256`) and normalized document
content (`expected_content_sha256`, `indexed_content_sha256`). Each expected
digest must equal its indexed counterpart and must describe a non-empty set.

Both modes include `artifact_sha256`. The validator computes SHA-256 over the
file supplied with `--search-artifact` and rejects a mismatch. That retained
file is not an arbitrary log: it must be UTF-8 JSON with the exact schema below.
The validator independently parses it and cross-checks its mode, release image,
rehearsal, database snapshot/WAL identity, timestamp, stopped-writer/consumer
flags, Rust spool counts and every mode-specific queue/reindex value against
the operations document. Empty, malformed, extra-field and contradictory
artifacts are rejected. JSON objects with duplicate keys and non-standard
numeric constants such as `NaN` or `Infinity` are also rejected.

The gate copies every supplied document and artifact into its evidence
directory first, then validates those retained bytes. A producer replacing a
source file during the gate therefore cannot make the audited GO directory
differ from the bytes that passed validation.

Minimal operations shape for the drained-queue alternative:

```json
{
  "production_clone": {
    "restore_verified": true,
    "liquibase_validate_passed": true,
    "runtime_schema_contract_passed": true,
    "java_rust_comparison_passed": true
  },
  "scheduler": {
    "original_java_timezone": "Europe/Moscow",
    "rust_scheduler_timezone": "Europe/Moscow",
    "legacy_jdbc_timezone": "Europe/Moscow",
    "timezone_match_verified": true,
    "active_scheduler_instances": 1,
    "single_scheduler_verified": true
  },
  "search_cutover": {
    "mode": "activemq-drained",
    "checked_at": "2026-08-16T12:00:00Z",
    "java_writers_stopped": true,
    "java_consumers_stopped": true,
    "rust_spool_pending": 0,
    "rust_spool_processing": 0,
    "artifact_sha256": "sha256:<64 lowercase hex characters>",
    "queue_name": "lor.searchQueue",
    "ready_messages": 0,
    "inflight_messages": 0
  },
  "lifecycle": {
    "sigterm_drain_passed": true,
    "restart_health_passed": true,
    "rollback_switch_passed": true,
    "post_rollback_smoke_passed": true
  }
}
```

These fields go inside the common `evidence` envelope with
`kind: operations`. The timezone above is illustrative only: use the value
proved from the original deployment, never copy it as a default.

The matching drained-queue artifact has no `status` or `evidence` wrapper and
contains exactly:

```json
{
  "schema_version": 1,
  "kind": "search-cutover",
  "rehearsal_id": "prod-rehearsal-20260816-001",
  "captured_at": "2026-08-16T12:00:00Z",
  "image_digest": "sha256:<64 lowercase hex characters>",
  "database_snapshot_id": "prodclone-20260816-001",
  "database_wal_position": "16/B374D848",
  "mode": "activemq-drained",
  "java_writers_stopped": true,
  "java_consumers_stopped": true,
  "rust_spool_pending": 0,
  "rust_spool_processing": 0,
  "queue_name": "lor.searchQueue",
  "ready_messages": 0,
  "inflight_messages": 0
}
```

Its `captured_at` must equal `search_cutover.checked_at`. A full-reindex
artifact replaces the three queue fields with every full-reindex field from
the operations document, including snapshot ID, exact counts, representative
query count and all four reconciliation digests. Generate this JSON directly
from the broker/reindex probe; do not hand-copy values from the operations
document. Compute `artifact_sha256` over the retained bytes after capture.

Validate documents independently before the full gate:

```bash
python3 tools/validate_cutover_evidence.py \
  --config /secure/evidence/config.json \
  --media /secure/evidence/media.json \
  --external /secure/evidence/external.json \
  --operations /secure/evidence/operations.json \
  --search-artifact /secure/evidence/search-cutover.json \
  --image-digest "$CUTOVER_IMAGE_DIGEST" \
  --snapshot-id "$CUTOVER_SNAPSHOT_ID" \
  --wal-position "$CUTOVER_WAL_POSITION"
```

The JSON reports prove that the named checks were performed; retain the
underlying deployment logs, MTA capture, ActiveMQ/OpenSearch output and storage
snapshot records alongside the gate directory for audit and rollback.

Run `scripts/check-production-runtime.sh` before capturing the configuration
and media documents. Its successful log is the executable evidence for pinned
image selection, non-root/read-only runtime shape, secret-file hygiene and the
media write/read/atomic-rename/cleanup checks. A local demo pass is useful for
testing the script but cannot be relabelled as production-clone evidence.
