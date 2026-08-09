# Cutover evidence contract

`scripts/run-cutover-gate.sh` accepts three JSON evidence documents. Merely
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
- `cookie_and_site_secrets_distinct` and `secret_values_redacted`;
- `trusted_proxy_cidrs_configured`;
- `opensearch_configured`, `captcha_configured`, `smtp_configured` and
  `admin_email_configured`;
- `dev_bypasses_disabled`;
- `one_active_background_scheduler`;
- `telegram_proxy_configured_if_enabled`.

Do not store secret values, passwords, tokens or private CAPTCHA keys in this
document.

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

Validate documents independently before the full gate:

```bash
python3 tools/validate_cutover_evidence.py \
  --config /secure/evidence/config.json \
  --media /secure/evidence/media.json \
  --external /secure/evidence/external.json \
  --image-digest "$CUTOVER_IMAGE_DIGEST" \
  --snapshot-id "$CUTOVER_SNAPSHOT_ID" \
  --wal-position "$CUTOVER_WAL_POSITION"
```

The JSON reports prove that the named checks were performed; retain the
underlying deployment logs, MTA capture, OpenSearch probe output and storage
snapshot records alongside the gate directory for audit and rollback.

Run `scripts/check-production-runtime.sh` before capturing the configuration
and media documents. Its successful log is the executable evidence for pinned
image selection, non-root/read-only runtime shape, secret-file hygiene and the
media write/read/atomic-rename/cleanup checks. A local demo pass is useful for
testing the script but cannot be relabelled as production-clone evidence.
