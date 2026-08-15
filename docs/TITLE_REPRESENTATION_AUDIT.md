# Read-only title-representation audit

`tools/audit_title_representation.py` inventories the mixed title encodings
which can exist while moving a Java LOR database to the Rust application. It
reads only these physical values:

- `topics.title`;
- `comments.title`;
- non-null `edit_info.oldtitle` snapshots.

The tool has no migration mode and contains no data-changing SQL. Every
database session uses a `REPEATABLE READ READ ONLY` transaction in addition to
`default_transaction_read_only=on`. The report must not be presented as
evidence that a production clone was audited: it records
`production_clone_evidence: not_claimed` deliberately.

## Source contract and limitations

The current Java writer calls `StringUtil.escapeHtml`, which delegates to
Guava `HtmlEscapers.htmlEscaper()`. Its title-storage alphabet is exactly:

| Input | Stored marker |
|---|---|
| `&` | `&amp;` |
| `"` | `&quot;` |
| `'` | `&#39;` |
| `<` | `&lt;` |
| `>` | `&gt;` |

The edit form applies one Apache Commons Text HTML4-unescape layer before
escaping the submitted value again. This is not Python/HTML5 unescape:
`&apos;` and `&NewLine;` remain unchanged in the Java HTML4 model. The audit's
`decoded_once_sha256` follows that HTML4 boundary and never recursively
decodes a newly exposed entity.

The classifier is conservative and mutually exclusive. It retains the
secondary marker counters even when a higher-precedence class wins:

1. `raw_five_chars` — a literal `<`, `>`, `"`, `'`, or an ampersand not
   consumed by an entity-shaped token;
2. `double_encoded_or_ambiguous` — no raw marker, but an encoded ampersand
   followed by an entity tail, for example `&amp;lt;` or `&#38;quot;`;
3. `other_named_or_numeric_entities` — a direct named/decimal/hex entity not
   in the exact Guava set;
4. `canonical_entities` — only exact Guava markers;
5. `plain` — no observable raw/entity marker.

These classes describe bytes, not intent. In particular, `&amp;lt;` can be a
double-encoded `<`, but it is also the correct Guava storage for a user who
literally typed `&lt;`. Likewise, a canonical marker can have passed through an
older writer as literal entity text. No row is safe to rewrite based on the
class alone.

`edit_info.oldtitle` is reported as a separate
`history_title_snapshot_maketitle_or_raw` pipeline. Java snapshots the
in-memory `makeTitle` representation, while the current Rust path has stored a
raw database representation. Consequently its distribution must not be used
as if it were another `topics.title` encoder-era sample.

For current topic/comment titles, `written_at` is the latest matching
title-changing `edit_info.editdate`, falling back to `postdate`. It is never
`lastmod`, because unrelated comment or metadata activity changes `lastmod`.
For `edit_info.oldtitle`, `written_at` is the snapshot's own `editdate`.

## Fail-closed target contract

Run the audit only against an isolated PostgreSQL clone. It refuses to scan
until all of these checks pass, then repeats the identity check inside the
data-scan transaction:

- URL comes from a regular, owner-only, non-symlink file;
- explicitly supplied database, role, PostgreSQL system identifier and clone
  marker match the connected server;
- the database comment is exactly
  `lorsource-title-audit-clone:<clone-marker>`;
- the session reports `transaction_read_only=on`;
- the role is not superuser, role/database creator, replication role or RLS
  bypass role;
- the role has all required column `SELECT` privileges and none of
  `INSERT`, `UPDATE`, `DELETE`, `TRUNCATE`, or `TRIGGER` on the audited tables;
- all three relations are ordinary permanent tables, have neither normal nor
  forced row-level security, and the exact required column types/nullability
  match the current Java schema contract.

The clone marker must be an explicit 8–128 character identifier and cannot
contain a `live`, `prod`, or `production` component. The command-line
confirmation is intentionally fixed to `read-only-title-audit`.

### Clone preparation

These are one-time administrative actions on the isolated clone, not commands
issued by the audit tool. Adapt names and password handling to the operator's
secret-management policy. Never run the preparation block on the source
database.

```sql
CREATE ROLE lor_title_auditor LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE
  NOREPLICATION NOBYPASSRLS PASSWORD '<managed clone-only secret>';
GRANT CONNECT ON DATABASE lor_title_clone TO lor_title_auditor;
GRANT USAGE ON SCHEMA public TO lor_title_auditor;
GRANT SELECT (id, title, postdate) ON public.topics TO lor_title_auditor;
GRANT SELECT (id, title, postdate) ON public.comments TO lor_title_auditor;
GRANT SELECT (id, msgid, oldtitle, editdate, object_type)
  ON public.edit_info TO lor_title_auditor;
GRANT EXECUTE ON FUNCTION pg_control_system() TO lor_title_auditor;
COMMENT ON DATABASE lor_title_clone IS
  'lorsource-title-audit-clone:clone-20260815-a1b2c3d4';
```

Record the system identifier from the clone through a separately authenticated
administrative session:

```sql
SELECT system_identifier::text FROM pg_control_system();
```

This identifier proves which PostgreSQL cluster answered; it does not prove
that the data came from production. Preserve the backup ID, backup checksum,
restore log, clone creation time, operator and change ticket in the external
cutover evidence package. The audit report hashes its own tool, fixed SQL,
schema contract and deployment-window input so that those inputs can be tied
to that package.

## Deployment-window input

Use confirmed deployment timestamps, not Git commit dates. Windows are
half-open UTC intervals `[start, end)`, must not overlap, and may contain a gap.
Rows in gaps are grouped as `unassigned` for manual investigation.

```json
{
  "schema_version": 1,
  "windows": [
    {
      "name": "java_before_cutover",
      "start": null,
      "end": "2026-08-15T00:00:00Z"
    },
    {
      "name": "controlled_cutover",
      "start": "2026-08-15T00:00:00Z",
      "end": "2026-08-15T02:00:00Z"
    },
    {
      "name": "rust_after_cutover",
      "start": "2026-08-15T02:00:00Z",
      "end": null
    }
  ]
}
```

## Dry run

All executions are dry runs: there is no write or migration switch. Put the
clone URL in a private file rather than an argument so that its password does
not appear in the process list or shell history.

```bash
install -m 600 /dev/null /secure/operator/lor-title-clone.url
# Write exactly one postgresql:// URL into that file using the secret manager.

python3 tools/audit_title_representation.py \
  --database-url-file /secure/operator/lor-title-clone.url \
  --expected-database lor_title_clone \
  --expected-role lor_title_auditor \
  --expected-system-identifier 7612345678901234567 \
  --clone-marker clone-20260815-a1b2c3d4 \
  --deployment-windows /secure/operator/deployment-windows.json \
  --output-dir /secure/operator/title-audit-run-001 \
  --id-bucket-size 100000 \
  --statement-timeout-ms 900000 \
  --confirm read-only-title-audit
```

The output directory must be absent or empty and is changed to mode `0700`;
artifacts are created as `0600` and existing files are never accepted as audit
targets.

## Deterministic artifacts

Given the same verified target state, deployment-window bytes, bucket size and
tool version, output order and hashes are deterministic:

- `title-representation-rows.csv` — one row per physical value, ordered by
  source then row ID. It contains source IDs, topic/comment entity kind,
  pipeline, write date/window,
  marker counters, lengths, title hash, one-layer hash and row hash, but not the
  raw title;
- `title-representation-audit.json` — row count/dataset hash plus counts and
  ordered hashes grouped by source/entity kind, classification, ID bucket, UTC
  date and deployment window; it also records the verified target and contract
  hashes;
- `title-representation-summary.txt` — human-readable totals and SHA-256 values
  for the row set, CSV and JSON.

The row-set hash is SHA-256 over newline-delimited row hashes. Each group hash
uses the same stable row order. A changed count, row hash, group hash or input
contract hash invalidates comparison with an earlier run.

## Manual review

Select candidates from the CSV by source/ID and inspect only those IDs on the
same verified clone. Do not copy raw titles into the general audit bundle.

```sql
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT id, title FROM public.topics WHERE id IN (/* reviewed topic IDs */);
SELECT id, title FROM public.comments WHERE id IN (/* reviewed comment IDs */);
SELECT id, msgid, editdate, oldtitle
  FROM public.edit_info WHERE id IN (/* reviewed history row IDs */);
COMMIT;
```

For each candidate, record the source, row ID, current `title_sha256`, intended
semantic text, expected producer era, reviewer and disposition. Ambiguous,
plain and history rows require context; they are never automatic fix lists.

## Future migration and rollback strategy

This repository intentionally does not provide a title migration from the
audit output. If review later proves that a migration is required:

1. preserve an immutable database backup/PITR point, restore log and this audit
   bundle;
2. produce a separately reviewed manifest containing selected IDs, expected
   before-hashes, intended after-hashes and the reason for every row;
3. rehearse the independently implemented migration against a fresh clone,
   aborting on any before-hash mismatch;
4. freeze relevant writers for the production cutover, take another backup,
   use a small reviewed canary window, then re-run this read-only audit;
5. keep the previous application deployment and original-value manifest until
   browser/runtime verification and a post-cutover observation window pass;
6. on failure, stop writers and restore/PITR to the recorded pre-cutover point,
   or apply a separately reviewed reverse manifest when that has been proven
   safer than a full restore.

Rollback is an operational decision backed by restorable data, not an
unverified inverse encoder. Never normalize all rows of a category in place.

## Local validation

The classifier, one-layer boundaries, URL-file protections, fail-closed target
identity, deterministic artifact hashes and fixed read-only SQL are covered by:

```bash
PYTHONDONTWRITEBYTECODE=1 \
  python3 -m unittest tools.tests.test_audit_title_representation -v
PYTHONDONTWRITEBYTECODE=1 \
  python3 -m py_compile tools/audit_title_representation.py
```

A passing unit suite is not evidence that any real or production-derived clone
was scanned. Attach a separately authorized runtime transcript only after the
operator has supplied such a clone and its provenance package.
