# Current Java/Scala compatibility notes

> Historical report only. References to `db/migrations` describe the retired
> Rust development schema, now stored offline in
> `compat/legacy-rust-db/offline-sql/`. Do not execute it; see
> `docs/DATABASE_COMPATIBILITY.md` for the active Java/Liquibase workflow.

The HTTP inventory section below is regenerated from the current adjacent
`lorsource-java` source tree and the current Rust worktree. Older implementation
notes later in this document remain historical and must not override the source.

## Automated checks

| Check | Result |
|---|---:|
| Spring handler methods | 179 |
| Expanded Spring mapping variants | 193 |
| Unique normalized Spring MVC paths | 131 |
| Axum route declarations | 158 |
| Path + all declared methods present | 111 |
| Partial method coverage (`ANY` or explicit method subset) | 80 |
| Missing route declarations | 2 (`/gallery.boxlet`, `/tagcloud.boxlet`) |
| Explicit `legacy::not_implemented` handlers | 0 |

The structural comparison is kept in `docs/ROUTE_COVERAGE.md`. It does not prove
parameter, authorization, response, UI, database or side-effect compatibility;
even a future equal count would not prove semantic parity.

## Bugs and mismatches fixed in this iteration

### `/vote.jsp` and poll schema

The Java code in `PollDao.scala` and `VoteController.scala` uses the post-Liquibase table names:

- `polls`;
- `polls_variants`;
- `vote_users.vote` = poll id;
- `vote_users.variant_id` = selected answer id;
- POST form shape: `voteid=<poll id>&vote=<variant id>[&vote=<variant id>...]`.

The Rust port previously kept the old demo-dump names `votenames/votes` and handled POST `/vote.jsp` as if `vote` were a single variant id. That was not compatible with the current Java source.

Fixed now:

- added migration `db/migrations/0004_current_java_schema_compat.sql`;
- added `polls` and `polls_variants`;
- migrated old `votenames/votes` rows into the current table names when present;
- added `vote_users.variant_id`;
- changed POST `/vote.jsp` to require `voteid`, accept repeated `vote`, validate multiselect and update `polls_variants.votes`.

### `user_settings`

The Java branch moved UI settings out of `users.style` into `user_settings.settings` backed by PostgreSQL `hstore`.

Fixed now:

- enabled `hstore` extension;
- added `user_settings(id, settings)`;
- mirrored existing `users.style` into `user_settings` during migration.

### `user_log`

The Java branch records moderator/account actions through `UserLogDao` into `user_log` with enum `user_log_action`.

Fixed now:

- added `user_log_action` enum with the action values found in the Java Liquibase updates;
- added `user_log` table;
- added `src/audit.rs` helper;
- wired audit events for user activation, email activation, userpic set/reset, deregistration, password changes and basic moderator actions.

### Compatibility scripts

`run-compatibility-suite.sh` previously assumed the Python tools were executable. Archives may lose executable bits, so the script failed with `Permission denied`.

Fixed now:

- scripts call tools through `python3 ...`;
- executable bits were restored for scripts/tools in the generated archive.

### Minor handler issue

The group moderation form had a duplicate hidden `id` field. It has been removed.

## Still not a mathematically exact port

The current Rust tree is substantially closer to the Java source, but it is still not safe to call it a production-identical rewrite. The remaining parity gaps are service-level, not URL-level:

- exact Spring Security roles and remember-me token generation;
- captcha, flood protection, IP block policy and slow-mode rules;
- SMTP templates and full password-reset/activation mail flow;
- exact image resizing/storage/CDN pipeline;
- OpenSearch indexing/ranking and reindex workers;
- full tracker/notification/realtime event generation;
- JSP-level presentation details and all model attributes;
- exact moderator audit side effects and score penalties for every action.

The next phase should run both apps against the same migrated DB and convert the smoke tests into endpoint-specific assertions for these subsystems.
