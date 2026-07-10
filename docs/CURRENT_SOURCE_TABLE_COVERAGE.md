# Current Java source table coverage

This is a focused source-level check for tables referenced by the current Java/Scala code or Liquibase updates but not necessarily present in the old `sql/demo.db` snapshot.

| Table | Java source/update usage | Rust migration status |
|---|---|---|
| `adv_counts` | advertising counters | covered |
| `email_domains_block` | registration/email domain policy | covered |
| `images` | gallery and image upload/delete flows | covered |
| `message_warnings` | moderator warnings | covered |
| `persistent_logins` | Spring remember-me sessions | covered |
| `polls` | current poll metadata table | covered in `0004` |
| `polls_variants` | current poll answer table | covered in `0004` |
| `reactions_log` | reaction audit | covered |
| `telegram_posts` | Telegram posting integration | covered |
| `topic_users_notified` | notification de-duplication | covered |
| `user_invites` | invite registration flow | covered |
| `user_log` | moderation/account audit | covered in `0004` |
| `user_remarks` | user remarks | covered |
| `user_settings` | current UI settings storage | covered in `0004` |
| `user_tags` | favorite/ignored tag filters | covered |

The old demo dump still contains the pre-Liquibase `votenames/votes` poll tables. The Rust migration keeps those names importable and copies them into current `polls/polls_variants` so both the dump and current source model can coexist during the port.
