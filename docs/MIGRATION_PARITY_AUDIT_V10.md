# Migration parity audit v10

This audit compares the uploaded current Java/Scala project with the Rust port at the migration boundary that matters for a cutover:

1. HTTP path and method surface extracted from Spring controllers.
2. Axum route declarations and dynamic-segment compatibility.
3. Current Java/Liquibase PostgreSQL table and column names.
4. Rust SQL usage that must run against an already-migrated Java database.

## Result

| Area | Result |
|---|---:|
| Java/Scala endpoint entries | 184 |
| Rust route declarations covered | 184/184 |
| Missing route declarations | 0 |
| Method mismatches | 0 |
| Axum dynamic wildcard-name conflicts | 0 |
| Java tables missing in Rust migrations | 0 |
| Java columns missing in Rust migrations | 0 |
| Explicit `legacy::not_implemented` / 501 handlers | 0 |

## Fixes made in v10

### Migration safety for existing Java databases

Earlier migrations still assumed either a clean Rust database or the historical demo dump. That is unsafe for a real cutover from the Java project, where Liquibase has already renamed or dropped objects. The following cases are now guarded or corrected:

- `0004_current_java_schema_compat.sql` no longer blindly reads `votenames`, `votes`, or `users.style` when those objects are absent in an already-migrated Java database.
- `0005_verify_current_java_alignment.sql` no longer blindly reads legacy Rust/demo columns such as `topics.warning_counter`, `reactions_log.userid`, `reactions_log.msgid`, `user_invites.created_at`, `user_invites.used_by`, or old `message_warnings` columns.
- `0003_legacy_schema_compat.sql` no longer creates indexes on legacy-only columns if the database already contains the current Java table shape.

### Exact/current Java column names

Added `0006_java_runtime_migration_compat.sql` to cover current Java/Liquibase column names while keeping old Rust/demo aliases additive:

- `msgbase.markup` is now the runtime source; Rust no longer requires dropped Java column `msgbase.bbcode`.
- `user_tags.user_id` is used by Rust handlers instead of old `userid`.
- `user_remarks.user_id`, `ref_user_id`, `remark_text` are used by Rust handlers instead of old `userid`, `who`, `remark`.
- Added current Java columns for `adv_counts`, `b_ips`, `comments`, `del_info`, `edit_info`, `email_domains_block`, `images`, `sections`, `tags_synonyms`, `telegram_posts`, `topics`, `user_events`, and `users`.

### Axum runtime route correctness

Axum/matchit does not accept conflicting wildcard names at the same route-tree position. The route coverage was previously correct by shape, but router construction could fail. Dynamic names were normalized:

- `/forum/{group}/{id_or_year}/{page_or_month}` -> `/forum/{group}/{id}/{tail}`
- `/{section}/{group}/{id}/{page_marker}` and `/{section}/{group}/{id}/{commentid}/history` now share the same wildcard name at the conflicting position.

### Java DB runtime SQL fixes

- New topic/comment creation now writes `msgbase.markup='BBCODE_TEX'` instead of the dropped Java column `msgbase.bbcode`.
- Topic reads derive the old `bbcode` boolean from `msgbase.markup::text <> 'PLAIN'`.
- User filters and remarks now use current Java column names.
- Image deletion no longer requires `images.userid`; it falls back to owner of `images.topic`.
- Domain block lookup respects `email_domains_block.block_until`.
- `/usermod.jsp?action=freeze` now accepts Java-compatible `shift` labels and supports `Разморозить`, `frozen_by`, and `freezing_reason`.

## Remaining non-proven areas

This archive is statically aligned for routes and schema, but full cutover still requires runtime comparison because this sandbox has no `cargo`, `rustc`, PostgreSQL server, or Docker:

- execute `cargo build`, `cargo test`, `cargo clippy`;
- load a copy of a real Java database and run Rust migrations `0003..0006`;
- replay compatibility HTTP tests against both Java and Rust;
- compare HTML/status/redirect behavior for authenticated moderator/admin flows;
- verify SMTP, captcha, OpenSearch, image storage, realtime notifications, remember-me, and anti-flood behavior in the target environment.
