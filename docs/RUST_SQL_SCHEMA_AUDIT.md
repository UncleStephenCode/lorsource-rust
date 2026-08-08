# Rust SQL vs canonical Java schema

This is a conservative static identifier audit. It is not a semantic-parity claim and does not execute the queries.

## Summary

- Canonical schema: **33 tables / 214 columns**.
- Rust SQL-bearing literals inspected: **423** (13 dynamic templates; 27 continuation fragments).
- Queries with confirmed identifier/type violations: **0**.
- Confirmed findings: **0**.
- Queries requiring static-review caution: **37**.
- Intentional negative schema probes: **23** (reported separately, not failures).

## Runtime-critical SQL surfaces

| Rank | Runtime surface | SQL-bearing literals | Invalid queries | Findings | Review |
|---|---|---:|---:|---:|---:|
| P0 | authentication/session | 17 | 0 | 0 | 0 |
| P0 | comment create/render/moderation | 47 | 0 | 0 | 0 |
| P0 | startup/schema compatibility | 7 | 0 | 0 | 0 |
| P0 | topic create/list/detail | 41 | 0 | 0 | 1 |
| P0 | topic persistence/list/detail | 21 | 0 | 0 | 0 |
| P1 | PostgreSQL repository | 50 | 0 | 0 | 5 |
| P1 | api routes | 45 | 0 | 0 | 10 |
| P1 | groups routes | 8 | 0 | 0 | 5 |
| P1 | legacy routes | 46 | 0 | 0 | 0 |
| P1 | search indexing | 7 | 0 | 0 | 0 |
| P1 | tags routes | 34 | 0 | 0 | 0 |
| P1 | topic browser flow | 25 | 0 | 0 | 5 |
| P1 | users routes | 42 | 0 | 0 | 11 |
| P2 | moderation/admin | 30 | 0 | 0 | 0 |
| P2 | supporting runtime | 3 | 0 | 0 | 0 |

## Confirmed findings, runtime-ranked

| Rank | Kind | Identifier | Runtime surface | Source |
|---|---|---|---|---|
| — | — | — | No confirmed static violations | — |

## Intentional legacy-absence probes

These identifiers occur as data in the startup schema fingerprint. PostgreSQL is asked whether they exist; they are not dereferenced as columns.

| Identifier | Source | Purpose |
|---|---|---|
| `adv_counts.id` | `src/infra/postgres/database.rs:223` | negative schema-fingerprint probe (not a column dereference) |
| `comments.editdate` | `src/infra/postgres/database.rs:213` | negative schema-fingerprint probe (not a column dereference) |
| `comments.editor` | `src/infra/postgres/database.rs:212` | negative schema-fingerprint probe (not a column dereference) |
| `comments.topic_deleted` | `src/infra/postgres/database.rs:214` | negative schema-fingerprint probe (not a column dereference) |
| `groups.stat1` | `src/infra/postgres/database.rs:218` | negative schema-fingerprint probe (not a column dereference) |
| `groups.stat2` | `src/infra/postgres/database.rs:219` | negative schema-fingerprint probe (not a column dereference) |
| `groups.stat4` | `src/infra/postgres/database.rs:220` | negative schema-fingerprint probe (not a column dereference) |
| `images.filename` | `src/infra/postgres/database.rs:222` | negative schema-fingerprint probe (not a column dereference) |
| `images.userid` | `src/infra/postgres/database.rs:221` | negative schema-fingerprint probe (not a column dereference) |
| `reactions_log.id` | `src/infra/postgres/database.rs:224` | negative schema-fingerprint probe (not a column dereference) |
| `reactions_log.msgid` | `src/infra/postgres/database.rs:225` | negative schema-fingerprint probe (not a column dereference) |
| `sections.add_info` | `src/infra/postgres/database.rs:216` | negative schema-fingerprint probe (not a column dereference) |
| `sections.image_allowed` | `src/infra/postgres/database.rs:217` | negative schema-fingerprint probe (not a column dereference) |
| `sections.preformat` | `src/infra/postgres/database.rs:215` | negative schema-fingerprint probe (not a column dereference) |
| `topics.image` | `src/infra/postgres/database.rs:209` | negative schema-fingerprint probe (not a column dereference) |
| `topics.no_comments` | `src/infra/postgres/database.rs:208` | negative schema-fingerprint probe (not a column dereference) |
| `topics.score_loss` | `src/infra/postgres/database.rs:211` | negative schema-fingerprint probe (not a column dereference) |
| `topics.stat2` | `src/infra/postgres/database.rs:206` | negative schema-fingerprint probe (not a column dereference) |
| `topics.stat4` | `src/infra/postgres/database.rs:207` | negative schema-fingerprint probe (not a column dereference) |
| `topics.warning_counter` | `src/infra/postgres/database.rs:210` | negative schema-fingerprint probe (not a column dereference) |
| `users.force_unlogin` | `src/infra/postgres/database.rs:205` | negative schema-fingerprint probe (not a column dereference) |
| `users.settings` | `src/infra/postgres/database.rs:204` | negative schema-fingerprint probe (not a column dereference) |
| `users.style` | `src/infra/postgres/database.rs:203` | negative schema-fingerprint probe (not a column dereference) |

## Static-analysis boundary

- Bind values and SQL assembled outside Rust string literals cannot be type-checked statically.
- Columns produced by CTEs, subqueries, table-valued functions and system catalogs are not guessed.
- Dynamic format fragments are inventoried and their statically visible identifiers are checked; runtime branches still require integration tests.
- A clean static result does not establish query behavior, transaction, authorization or migration parity.
