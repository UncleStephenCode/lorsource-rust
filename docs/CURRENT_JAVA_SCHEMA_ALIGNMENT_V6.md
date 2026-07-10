# Current Java schema alignment v6

This document records schema differences discovered by comparing the Rust migrations with the current Java/Scala DAO code and Liquibase updates, not only with the old `sql/demo.db` dump.

## Corrected current tables

| Area | Current Java/Liquibase shape | Rust v6 status |
|---|---|---|
| Polls | `polls`, `polls_variants`, `vote_users.variant_id` | aligned in `0004_current_java_schema_compat.sql` |
| User settings | `user_settings(id, settings hstore)` after `users.style` migration | handlers now read/write `user_settings.settings` |
| User audit | `user_log` with `user_log_action` enum and hstore `info` | basic logging kept in `src/audit.rs` |
| Reactions | `reactions_log(origin_user, topic_id, comment_id, set_date, reaction)` plus JSONB on topics/comments | aligned/backfilled in `0005_verify_current_java_alignment.sql`; handlers use Java column names |
| Warnings | `message_warnings(topic, comment, author, message, warning_type, closed_by, closed_when)` | aligned/backfilled in `0005_verify_current_java_alignment.sql`; handlers use Java column names |
| Warning counters | `topics.open_warnings` | added/backfilled and recalculated on warning post/clear |
| Warning events | `user_events.warning_id` | added in v6 migration |
| Invites | `user_invites(invite_code text, owner, issue_date, invited_user, email, valid_until)` | added/backfilled in v6 migration |

## Compatibility columns intentionally kept

The earlier Rust migrations created some draft compatibility columns (`message_warnings.reason`, `message_warnings.topic_id`, `reactions_log.msgid`, `user_invites.created_at`, etc.). v6 does not drop them because existing dev databases may already contain data. New Rust handlers use the current Java column names where the Java code does.
