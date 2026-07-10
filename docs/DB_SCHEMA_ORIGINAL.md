# Original demo DB schema inventory

Tables in `sql/demo.db`: **39**

## `b_ips`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `ip` | `inet` | `False` | `` |
| `mod_id` | `integer` | `False` | `` |
| `date` | `timestamp with time zone` | `False` | `` |
| `reason` | `character varying(255)` | `True` | `` |
| `ban_date` | `timestamp without time zone` | `True` | `` |

## `ban_info`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `userid` | `integer` | `False` | `` |
| `bandate` | `timestamp without time zone` | `False` | `now()` |
| `reason` | `text` | `False` | `` |
| `ban_by` | `integer` | `False` | `` |

## `comments`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `integer` | `False` | `` |
| `topic` | `integer` | `False` | `` |
| `userid` | `integer` | `False` | `` |
| `title` | `character varying(255)` | `False` | `` |
| `postdate` | `timestamp with time zone` | `False` | `` |
| `replyto` | `integer` | `True` | `` |
| `deleted` | `boolean` | `False` | `false` |
| `postip` | `inet` | `True` | `` |
| `ua_id` | `integer` | `True` | `` |
| `topic_deleted` | `boolean` | `False` | `false` |

## `del_info`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `msgid` | `integer` | `False` | `` |
| `delby` | `integer` | `False` | `` |
| `reason` | `text` | `True` | `` |
| `deldate` | `timestamp without time zone` | `True` | `` |

## `edit_info`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `integer` | `False` | `` |
| `msgid` | `integer` | `False` | `` |
| `editor` | `integer` | `False` | `` |
| `oldmessage` | `text` | `True` | `` |
| `editdate` | `timestamp without time zone` | `False` | `now()` |
| `oldtitle` | `text` | `True` | `` |
| `oldtags` | `text` | `True` | `` |
| `oldlinktext` | `text` | `True` | `` |
| `oldurl` | `text` | `True` | `` |

## `groups`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `integer` | `False` | `` |
| `title` | `character varying(255)` | `False` | `` |
| `image` | `character varying(255)` | `True` | `` |
| `section` | `integer` | `False` | `` |
| `stat1` | `integer` | `False` | `0` |
| `stat2` | `integer` | `False` | `0` |
| `stat3` | `integer` | `False` | `0` |
| `stat4` | `integer` | `False` | `0` |
| `restrict_topics` | `integer` | `True` | `` |
| `info` | `text` | `True` | `` |
| `restrict_comments` | `integer` | `False` | `(-9999)` |
| `longinfo` | `text` | `True` | `` |
| `resolvable` | `boolean` | `False` | `false` |
| `urlname` | `text` | `False` | `` |

## `ignore_list`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `userid` | `integer` | `False` | `` |
| `ignored` | `integer` | `False` | `` |

## `users`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `integer` | `False` | `` |
| `name` | `character varying(255)` | `True` | `` |
| `nick` | `character varying(80)` | `False` | `` |
| `passwd` | `character varying(40)` | `True` | `` |
| `url` | `character varying(255)` | `True` | `` |
| `email` | `character varying(255)` | `True` | `` |
| `canmod` | `boolean` | `False` | `false` |
| `photo` | `character varying(100)` | `True` | `` |
| `town` | `character varying(100)` | `True` | `` |
| `candel` | `boolean` | `False` | `false` |
| `lostpwd` | `timestamp with time zone` | `False` | `'1970-01-01 03:00:00+03'::timestamp with time zone` |
| `blocked` | `boolean` | `True` | `` |
| `score` | `integer` | `True` | `` |
| `max_score` | `integer` | `True` | `` |
| `lastlogin` | `timestamp without time zone` | `True` | `` |
| `regdate` | `timestamp without time zone` | `True` | `` |
| `activated` | `boolean` | `False` | `false` |
| `corrector` | `boolean` | `False` | `false` |
| `userinfo` | `text` | `True` | `` |
| `unread_events` | `integer` | `False` | `0` |
| `new_email` | `character varying(255)` | `True` | `` |
| `style` | `character varying(15)` | `False` | `'tango'::character varying` |

## `jam_category`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `child_topic_id` | `integer` | `False` | `` |
| `category_name` | `character varying(200)` | `False` | `` |
| `sort_key` | `character varying(200)` | `True` | `` |

## `jam_configuration`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `config_key` | `character varying(50)` | `False` | `` |
| `config_value` | `character varying(500)` | `False` | `` |

## `jam_file`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `file_id` | `integer` | `False` | `` |
| `virtual_wiki_id` | `integer` | `False` | `` |
| `file_name` | `character varying(200)` | `False` | `` |
| `delete_date` | `timestamp without time zone` | `True` | `` |
| `file_read_only` | `integer` | `False` | `0` |
| `file_admin_only` | `integer` | `False` | `0` |
| `file_url` | `character varying(200)` | `False` | `` |
| `mime_type` | `character varying(100)` | `False` | `` |
| `topic_id` | `integer` | `False` | `` |
| `file_size` | `integer` | `False` | `` |

## `jam_file_version`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `file_version_id` | `integer` | `False` | `` |
| `file_id` | `integer` | `False` | `` |
| `upload_comment` | `character varying(200)` | `True` | `` |
| `file_url` | `character varying(200)` | `False` | `` |
| `wiki_user_id` | `integer` | `True` | `` |
| `upload_date` | `timestamp without time zone` | `False` | `now()` |
| `mime_type` | `character varying(100)` | `False` | `` |
| `file_size` | `integer` | `False` | `` |
| `wiki_user_display` | `character varying(100)` | `True` | `` |

## `jam_group_members`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `integer` | `False` | `` |
| `username` | `character varying(100)` | `False` | `` |
| `group_id` | `integer` | `False` | `` |

## `jam_group`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `group_id` | `integer` | `False` | `` |
| `group_name` | `character varying(30)` | `False` | `` |
| `group_description` | `character varying(200)` | `True` | `` |

## `jam_group_authorities`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `group_id` | `integer` | `False` | `` |
| `authority` | `character varying(30)` | `False` | `` |

## `jam_interwiki`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `interwiki_prefix` | `character varying(30)` | `False` | `` |
| `interwiki_pattern` | `character varying(200)` | `False` | `` |
| `interwiki_type` | `integer` | `False` | `` |

## `jam_log`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `log_date` | `timestamp without time zone` | `False` | `now()` |
| `virtual_wiki_id` | `integer` | `False` | `` |
| `wiki_user_id` | `integer` | `True` | `` |
| `display_name` | `character varying(200)` | `False` | `` |
| `topic_id` | `integer` | `True` | `` |
| `topic_version_id` | `integer` | `True` | `` |
| `log_type` | `integer` | `False` | `` |
| `log_comment` | `character varying(200)` | `True` | `` |
| `log_params` | `character varying(500)` | `True` | `` |
| `log_sub_type` | `integer` | `True` | `` |

## `jam_namespace`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `namespace_id` | `integer` | `False` | `` |
| `namespace` | `character varying(200)` | `False` | `` |
| `main_namespace_id` | `integer` | `True` | `` |

## `jam_namespace_translation`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `namespace_id` | `integer` | `False` | `` |
| `virtual_wiki_id` | `integer` | `False` | `` |
| `namespace` | `character varying(200)` | `False` | `` |

## `jam_recent_change`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `topic_version_id` | `integer` | `True` | `` |
| `previous_topic_version_id` | `integer` | `True` | `` |
| `topic_id` | `integer` | `True` | `` |
| `topic_name` | `character varying(200)` | `True` | `` |
| `change_date` | `timestamp without time zone` | `False` | `now()` |
| `change_comment` | `character varying(200)` | `True` | `` |
| `wiki_user_id` | `integer` | `True` | `` |
| `display_name` | `character varying(200)` | `False` | `` |
| `edit_type` | `integer` | `True` | `` |
| `virtual_wiki_id` | `integer` | `True` | `` |
| `virtual_wiki_name` | `character varying(100)` | `True` | `` |
| `characters_changed` | `integer` | `True` | `` |
| `log_type` | `integer` | `True` | `` |
| `log_params` | `character varying(500)` | `True` | `` |
| `log_sub_type` | `integer` | `True` | `` |

## `jam_role`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `role_name` | `character varying(30)` | `False` | `` |
| `role_description` | `character varying(200)` | `True` | `` |

## `jam_topic`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `topic_id` | `integer` | `False` | `` |
| `virtual_wiki_id` | `integer` | `False` | `` |
| `topic_name` | `character varying(200)` | `False` | `` |
| `delete_date` | `timestamp without time zone` | `True` | `` |
| `topic_read_only` | `integer` | `False` | `0` |
| `topic_admin_only` | `integer` | `False` | `0` |
| `current_version_id` | `integer` | `True` | `` |
| `topic_type` | `integer` | `False` | `` |
| `redirect_to` | `character varying(200)` | `True` | `` |
| `namespace_id` | `integer` | `False` | `0` |
| `page_name` | `character varying(200)` | `True` | `` |
| `page_name_lower` | `character varying(200)` | `True` | `` |

## `jam_topic_links`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `topic_id` | `integer` | `False` | `` |
| `link_topic_namespace_id` | `integer` | `False` | `0` |
| `link_topic_page_name` | `character varying(200)` | `False` | `` |

## `jam_topic_version`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `topic_version_id` | `integer` | `False` | `` |
| `topic_id` | `integer` | `False` | `` |
| `edit_comment` | `character varying(200)` | `True` | `` |
| `version_content` | `text` | `True` | `` |
| `wiki_user_id` | `integer` | `True` | `` |
| `edit_date` | `timestamp without time zone` | `False` | `now()` |
| `edit_type` | `integer` | `False` | `` |
| `previous_topic_version_id` | `integer` | `True` | `` |
| `characters_changed` | `integer` | `True` | `` |
| `version_params` | `character varying(500)` | `True` | `` |
| `wiki_user_display` | `character varying(100)` | `True` | `` |

## `jam_user_block`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `user_block_id` | `integer` | `False` | `` |
| `wiki_user_id` | `integer` | `True` | `` |
| `ip_address` | `character varying(39)` | `True` | `` |
| `block_date` | `timestamp without time zone` | `False` | `now()` |
| `block_end_date` | `timestamp without time zone` | `True` | `` |
| `block_reason` | `character varying(200)` | `True` | `` |
| `blocked_by_user_id` | `integer` | `False` | `` |
| `unblock_date` | `timestamp without time zone` | `True` | `` |
| `unblock_reason` | `character varying(200)` | `True` | `` |
| `unblocked_by_user_id` | `integer` | `True` | `` |

## `jam_virtual_wiki`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `virtual_wiki_id` | `integer` | `False` | `` |
| `virtual_wiki_name` | `character varying(100)` | `False` | `` |
| `default_topic_name` | `character varying(200)` | `True` | `` |
| `create_date` | `timestamp without time zone` | `False` | `now()` |
| `logo_image_url` | `character varying(200)` | `True` | `` |
| `site_name` | `character varying(200)` | `True` | `` |
| `meta_description` | `character varying(500)` | `True` | `` |

## `jam_watchlist`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `wiki_user_id` | `integer` | `False` | `` |
| `topic_name` | `character varying(200)` | `False` | `` |
| `virtual_wiki_id` | `integer` | `False` | `` |

## `memories`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `integer` | `False` | `` |
| `userid` | `integer` | `False` | `` |
| `topic` | `integer` | `False` | `` |
| `add_date` | `timestamp without time zone` | `False` | `now()` |

## `monthly_stats`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `section` | `integer` | `True` | `` |
| `year` | `integer` | `False` | `` |
| `month` | `integer` | `False` | `` |
| `c` | `integer` | `False` | `` |
| `groupid` | `integer` | `True` | `` |

## `msgbase`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `bigint` | `False` | `` |
| `message` | `text` | `False` | `` |
| `bbcode` | `boolean` | `True` | `` |

## `sections`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `integer` | `False` | `` |
| `name` | `character varying(255)` | `False` | `` |
| `moderate` | `boolean` | `False` | `` |
| `imagepost` | `boolean` | `False` | `` |
| `preformat` | `boolean` | `False` | `` |
| `linktext` | `character varying(255)` | `True` | `` |
| `havelink` | `boolean` | `False` | `` |
| `expire` | `interval` | `False` | `` |
| `vote` | `boolean` | `True` | `false` |
| `add_info` | `text` | `True` | `` |

## `tags`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `msgid` | `integer` | `True` | `` |
| `tagid` | `integer` | `True` | `` |

## `tags_values`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `integer` | `False` | `` |
| `counter` | `integer` | `True` | `0` |
| `value` | `character varying(255)` | `False` | `` |

## `topics`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `integer` | `False` | `` |
| `groupid` | `integer` | `False` | `` |
| `userid` | `integer` | `False` | `` |
| `title` | `character varying(255)` | `False` | `` |
| `url` | `character varying(255)` | `True` | `` |
| `moderate` | `boolean` | `False` | `false` |
| `postdate` | `timestamp with time zone` | `False` | `` |
| `linktext` | `character varying(255)` | `True` | `` |
| `deleted` | `boolean` | `False` | `false` |
| `stat1` | `integer` | `False` | `0` |
| `stat2` | `integer` | `False` | `0` |
| `stat3` | `integer` | `False` | `0` |
| `stat4` | `integer` | `False` | `0` |
| `lastmod` | `timestamp with time zone` | `True` | `` |
| `commitby` | `integer` | `True` | `` |
| `notop` | `boolean` | `True` | `` |
| `commitdate` | `timestamp without time zone` | `True` | `` |
| `postscore` | `integer` | `True` | `` |
| `postip` | `inet` | `True` | `` |
| `sticky` | `boolean` | `False` | `false` |
| `ua_id` | `integer` | `True` | `` |
| `resolved` | `boolean` | `True` | `` |
| `minor` | `boolean` | `False` | `false` |

## `user_agents`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `integer` | `False` | `` |
| `name` | `character varying(512)` | `True` | `''::character varying` |

## `user_events`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `userid` | `integer` | `False` | `` |
| `type` | `event_type` | `False` | `` |
| `private` | `boolean` | `False` | `` |
| `event_date` | `timestamp without time zone` | `False` | `now()` |
| `message_id` | `integer` | `True` | `` |
| `comment_id` | `integer` | `True` | `` |
| `message` | `text` | `True` | `` |
| `unread` | `boolean` | `False` | `true` |
| `id` | `integer` | `False` | `` |

## `vote_users`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `vote` | `integer` | `False` | `` |
| `userid` | `integer` | `False` | `` |

## `votenames`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `integer` | `False` | `` |
| `topic` | `integer` | `False` | `0` |
| `multiselect` | `boolean` | `False` | `false` |

## `votes`

| Column | Type | Nullable | Default |
|---|---|---:|---|
| `id` | `integer` | `False` | `` |
| `vote` | `integer` | `False` | `` |
| `label` | `text` | `False` | `` |
| `votes` | `integer` | `False` | `0` |
