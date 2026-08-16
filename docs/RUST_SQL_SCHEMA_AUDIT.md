# Rust SQL vs canonical Java schema

This is a conservative static identifier audit. It is not a semantic-parity claim and does not execute the queries.

## Summary

- Canonical schema: **33 tables / 214 columns**.
- Rust SQL-bearing literals inspected: **941** (21 dynamic templates; 128 continuation fragments).
- SQL literals in Rust test scope: **211** (local CREATE TABLE fixture columns apply only there).
- Queries with confirmed identifier/type violations: **0**.
- Confirmed findings: **0**.
- Queries requiring static-review caution: **148**.
- Intentional negative schema probes: **23** (reported separately, not failures).

## Runtime-critical SQL surfaces

| Rank | Runtime surface | SQL-bearing literals | Invalid queries | Findings | Review |
|---|---|---:|---:|---:|---:|
| P0 | authentication/session | 20 | 0 | 0 | 1 |
| P0 | comment create/render/moderation | 43 | 0 | 0 | 8 |
| P0 | startup/schema compatibility | 14 | 0 | 0 | 0 |
| P0 | topic create/list/detail | 53 | 0 | 0 | 2 |
| P0 | topic persistence/list/detail | 46 | 0 | 0 | 19 |
| P1 | PostgreSQL repository | 379 | 0 | 0 | 56 |
| P1 | api routes | 71 | 0 | 0 | 12 |
| P1 | groups routes | 21 | 0 | 0 | 12 |
| P1 | legacy routes | 43 | 0 | 0 | 1 |
| P1 | search indexing | 10 | 0 | 0 | 0 |
| P1 | tags routes | 52 | 0 | 0 | 8 |
| P1 | topic browser flow | 29 | 0 | 0 | 7 |
| P1 | users routes | 61 | 0 | 0 | 16 |
| P2 | media routes | 2 | 0 | 0 | 0 |
| P2 | moderation/admin | 16 | 0 | 0 | 3 |
| P2 | search routes | 4 | 0 | 0 | 0 |
| P2 | supporting runtime | 77 | 0 | 0 | 3 |

## Confirmed findings, runtime-ranked

| Rank | Kind | Identifier | Runtime surface | Source |
|---|---|---|---|---|
| — | — | — | No confirmed static violations | — |

## Intentional legacy-absence probes

These identifiers occur as data in the startup schema fingerprint. PostgreSQL is asked whether they exist; they are not dereferenced as columns.

| Identifier | Source | Purpose |
|---|---|---|
| `adv_counts.id` | `src/infra/postgres/database.rs:264` | negative schema-fingerprint probe (not a column dereference) |
| `comments.editdate` | `src/infra/postgres/database.rs:254` | negative schema-fingerprint probe (not a column dereference) |
| `comments.editor` | `src/infra/postgres/database.rs:253` | negative schema-fingerprint probe (not a column dereference) |
| `comments.topic_deleted` | `src/infra/postgres/database.rs:255` | negative schema-fingerprint probe (not a column dereference) |
| `groups.stat1` | `src/infra/postgres/database.rs:259` | negative schema-fingerprint probe (not a column dereference) |
| `groups.stat2` | `src/infra/postgres/database.rs:260` | negative schema-fingerprint probe (not a column dereference) |
| `groups.stat4` | `src/infra/postgres/database.rs:261` | negative schema-fingerprint probe (not a column dereference) |
| `images.filename` | `src/infra/postgres/database.rs:263` | negative schema-fingerprint probe (not a column dereference) |
| `images.userid` | `src/infra/postgres/database.rs:262` | negative schema-fingerprint probe (not a column dereference) |
| `reactions_log.id` | `src/infra/postgres/database.rs:265` | negative schema-fingerprint probe (not a column dereference) |
| `reactions_log.msgid` | `src/infra/postgres/database.rs:266` | negative schema-fingerprint probe (not a column dereference) |
| `sections.add_info` | `src/infra/postgres/database.rs:257` | negative schema-fingerprint probe (not a column dereference) |
| `sections.image_allowed` | `src/infra/postgres/database.rs:258` | negative schema-fingerprint probe (not a column dereference) |
| `sections.preformat` | `src/infra/postgres/database.rs:256` | negative schema-fingerprint probe (not a column dereference) |
| `topics.image` | `src/infra/postgres/database.rs:250` | negative schema-fingerprint probe (not a column dereference) |
| `topics.no_comments` | `src/infra/postgres/database.rs:249` | negative schema-fingerprint probe (not a column dereference) |
| `topics.score_loss` | `src/infra/postgres/database.rs:252` | negative schema-fingerprint probe (not a column dereference) |
| `topics.stat2` | `src/infra/postgres/database.rs:247` | negative schema-fingerprint probe (not a column dereference) |
| `topics.stat4` | `src/infra/postgres/database.rs:248` | negative schema-fingerprint probe (not a column dereference) |
| `topics.warning_counter` | `src/infra/postgres/database.rs:251` | negative schema-fingerprint probe (not a column dereference) |
| `users.force_unlogin` | `src/infra/postgres/database.rs:246` | negative schema-fingerprint probe (not a column dereference) |
| `users.settings` | `src/infra/postgres/database.rs:245` | negative schema-fingerprint probe (not a column dereference) |
| `users.style` | `src/infra/postgres/database.rs:244` | negative schema-fingerprint probe (not a column dereference) |

## Static-analysis boundary

- Bind values and SQL assembled outside Rust string literals cannot be type-checked statically.
- Columns produced by CTEs, subqueries, table-valued functions and system catalogs are not guessed.
- Dynamic format fragments are inventoried and their statically visible identifiers are checked; runtime branches still require integration tests.
- SQL in Rust test functions is inventoried against explicitly declared local CREATE TABLE fixture columns as well as the canonical schema; production literals never receive that fixture overlay.
- A clean static result does not establish query behavior, transaction, authorization or migration parity.
