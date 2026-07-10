# Current Java schema compatibility report

Java tables without Rust migration table: **0**
Java tables with missing Rust columns: **0**

| Status | Table | Missing Java columns in Rust | Extra Rust compatibility columns |
|---|---|---|---|
| covered | `adv_counts` | `` | `adv, clicks, event_date, id, views` |
| covered | `b_ips` | `` | `` |
| covered | `ban_info` | `` | `` |
| covered | `comments` | `` | `editdate, editor` |
| covered | `del_info` | `` | `` |
| covered | `edit_info` | `` | `minor` |
| covered | `email_domains_block` | `` | `created_at, reason` |
| covered | `groups` | `` | `` |
| covered | `ignore_list` | `` | `` |
| covered | `images` | `` | `height, medium, original_name, postdate, thumbnail, userid, width` |
| covered | `memories` | `` | `notify` |
| covered | `message_warnings` | `` | `comment_id, moderator, reason, resolved, resolved_at, topic_id, userid` |
| covered | `monthly_stats` | `` | `` |
| covered | `msgbase` | `` | `bbcode` |
| covered | `persistent_logins` | `` | `` |
| covered | `polls` | `` | `` |
| covered | `polls_variants` | `` | `` |
| covered | `reactions_log` | `` | `action_date, id, msgid, set_value, userid` |
| covered | `sections` | `` | `image_allowed, restrict_score` |
| covered | `tags` | `` | `` |
| covered | `tags_synonyms` | `` | `id, synonym, tag_id` |
| covered | `tags_values` | `` | `` |
| covered | `telegram_posts` | `` | `topic` |
| covered | `topic_users_notified` | `` | `` |
| covered | `topics` | `` | `image, no_comments, score_loss, warning_counter` |
| covered | `user_agents` | `` | `` |
| covered | `user_events` | `` | `event_type, topic_id` |
| covered | `user_invites` | `` | `created_at, id, used_at, used_by` |
| covered | `user_log` | `` | `` |
| covered | `user_remarks` | `` | `remark, userid, who` |
| covered | `user_settings` | `` | `` |
| covered | `user_tags` | `` | `userid` |
| covered | `users` | `` | `force_unlogin, settings, style` |
| covered | `vote_users` | `` | `` |
| rust-only | `votenames` | `` | `id, multiselect, topic` |
| rust-only | `votes` | `` | `id, label, vote, votes` |
