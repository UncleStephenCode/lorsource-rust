# Schema coverage report

Tables covered: **20**
Missing original tables: **0**
Rust-only/current-update tables: **16**
Dropped upstream legacy tables: **19**

| Status | Table | Missing columns from Rust migration | Extra Rust columns |
|---|---|---|---|
| rust-only | `adv_counts` | `` | `adv, clicks, event_date, id, views` |
| covered | `b_ips` | `` | `` |
| covered | `ban_info` | `` | `` |
| covered | `comments` | `` | `edit_count, editdate, editor, reactions` |
| covered | `del_info` | `` | `` |
| covered | `edit_info` | `` | `minor, object_type, oldimage` |
| rust-only | `email_domains_block` | `` | `created_at, domain, reason` |
| covered | `groups` | `` | `` |
| covered | `ignore_list` | `` | `` |
| rust-only | `images` | `` | `deleted, height, id, medium, original, original_name, postdate, primary_image, thumbnail, topic, userid, width` |
| dropped-upstream | `jam_category` | `category_name, child_topic_id, sort_key` | `` |
| dropped-upstream | `jam_configuration` | `config_key, config_value` | `` |
| dropped-upstream | `jam_file` | `delete_date, file_admin_only, file_id, file_name, file_read_only, file_size, file_url, mime_type, topic_id, virtual_wiki_id` | `` |
| dropped-upstream | `jam_file_version` | `file_id, file_size, file_url, file_version_id, mime_type, upload_comment, upload_date, wiki_user_display, wiki_user_id` | `` |
| dropped-upstream | `jam_group` | `group_description, group_id, group_name` | `` |
| dropped-upstream | `jam_group_authorities` | `authority, group_id` | `` |
| dropped-upstream | `jam_group_members` | `group_id, id, username` | `` |
| dropped-upstream | `jam_interwiki` | `interwiki_pattern, interwiki_prefix, interwiki_type` | `` |
| dropped-upstream | `jam_log` | `display_name, log_comment, log_date, log_params, log_sub_type, log_type, topic_id, topic_version_id, virtual_wiki_id, wiki_user_id` | `` |
| dropped-upstream | `jam_namespace` | `main_namespace_id, namespace, namespace_id` | `` |
| dropped-upstream | `jam_namespace_translation` | `namespace, namespace_id, virtual_wiki_id` | `` |
| dropped-upstream | `jam_recent_change` | `change_comment, change_date, characters_changed, display_name, edit_type, log_params, log_sub_type, log_type, previous_topic_version_id, topic_id, topic_name, topic_version_id, virtual_wiki_id, virtual_wiki_name, wiki_user_id` | `` |
| dropped-upstream | `jam_role` | `role_description, role_name` | `` |
| dropped-upstream | `jam_topic` | `current_version_id, delete_date, namespace_id, page_name, page_name_lower, redirect_to, topic_admin_only, topic_id, topic_name, topic_read_only, topic_type, virtual_wiki_id` | `` |
| dropped-upstream | `jam_topic_links` | `link_topic_namespace_id, link_topic_page_name, topic_id` | `` |
| dropped-upstream | `jam_topic_version` | `characters_changed, edit_comment, edit_date, edit_type, previous_topic_version_id, topic_id, topic_version_id, version_content, version_params, wiki_user_display, wiki_user_id` | `` |
| dropped-upstream | `jam_user_block` | `block_date, block_end_date, block_reason, blocked_by_user_id, ip_address, unblock_date, unblock_reason, unblocked_by_user_id, user_block_id, wiki_user_id` | `` |
| dropped-upstream | `jam_virtual_wiki` | `create_date, default_topic_name, logo_image_url, meta_description, site_name, virtual_wiki_id, virtual_wiki_name` | `` |
| dropped-upstream | `jam_watchlist` | `topic_name, virtual_wiki_id, wiki_user_id` | `` |
| covered | `memories` | `` | `notify, watch` |
| rust-only | `message_warnings` | `` | `author, closed_by, closed_when, comment, comment_id, id, message, moderator, postdate, reason, resolved, resolved_at, topic, topic_id, userid, warning_type` |
| covered | `monthly_stats` | `` | `` |
| covered | `msgbase` | `` | `markup` |
| rust-only | `persistent_logins` | `` | `last_used, series, token, username` |
| rust-only | `polls` | `` | `id, multiselect, topic` |
| rust-only | `polls_variants` | `` | `id, label, vote, votes` |
| rust-only | `reactions_log` | `` | `action_date, comment_id, id, msgid, origin_user, reaction, set_date, set_value, topic_id, userid` |
| covered | `sections` | `` | `image_allowed, restrict_score, scroll_mode` |
| covered | `tags` | `` | `` |
| rust-only | `tags_synonyms` | `` | `id, synonym, tag_id` |
| covered | `tags_values` | `` | `` |
| rust-only | `telegram_posts` | `` | `postdate, telegram_id, topic` |
| rust-only | `topic_users_notified` | `` | `topic, userid` |
| covered | `topics` | `` | `draft, image, no_comments, open_warnings, reactions, score_loss, warning_counter` |
| covered | `user_agents` | `` | `` |
| covered | `user_events` | `` | `event_type, topic_id, warning_id` |
| rust-only | `user_invites` | `` | `created_at, email, id, invite_code, invited_user, issue_date, owner, used_at, used_by, valid_until` |
| rust-only | `user_log` | `` | `action, action_date, action_userid, id, info, userid` |
| rust-only | `user_remarks` | `` | `remark, userid, who` |
| rust-only | `user_settings` | `` | `id, settings` |
| rust-only | `user_tags` | `` | `is_favorite, tag_id, userid` |
| covered | `users` | `` | `force_unlogin, frozen_until, settings, userinfo_markup` |
| covered | `vote_users` | `` | `variant_id` |
| covered | `votenames` | `` | `` |
| covered | `votes` | `` | `` |
