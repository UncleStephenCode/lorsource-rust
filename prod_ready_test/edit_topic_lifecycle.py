#!/usr/bin/env python3
"""Idempotent HTTP/DB regression for the Java-compatible topic edit flow.

All user-visible mutations go through ``/edit.jsp`` (and the commit form is
opened through ``/commit.jsp``).  SQL is deliberately limited to installing,
observing, and removing four reserved fixtures.  Cleanup also reverses the
canonical ``topins_t`` group counter/memory effects, the commit score bonus,
and ``new_event_t`` unread-counter effects, so an interrupted run can be
repaired safely by the next invocation.
"""

from __future__ import annotations

import argparse
import json
import re

from test_port import Client, db, require, require_html


EDIT_TOPIC = 2_130_100_101
POLL_TOPIC = 2_130_100_102
DRAFT_TOPIC = 2_130_100_103
COMMIT_TOPIC = 2_130_100_104
TOPIC_IDS = (EDIT_TOPIC, POLL_TOPIC, DRAFT_TOPIC, COMMIT_TOPIC)

POLL_ID = 2_130_100_201
POLL_VARIANT_A = 2_130_100_211
POLL_VARIANT_B = 2_130_100_212

EDIT_AUTHOR = 9_100_008  # raven1000
POLL_AUTHOR = 9_100_009  # crane2000
DRAFT_AUTHOR = 9_100_011  # tern_corrector (publish-limit exempt)
COMMIT_AUTHOR = 9_100_001  # swift45
MENTION_RECIPIENT = 9_100_002  # finch50
MODERATOR = 9_100_013  # hawk_moderator
COMMIT_BONUS = 7

EDIT_GROUP = 10_161
POLL_GROUP = 19_387
NEWS_SOURCE_GROUP = 19_399
NEWS_TARGET_GROUP = 6
GROUP_IDS = (NEWS_TARGET_GROUP, EDIT_GROUP, POLL_GROUP, NEWS_SOURCE_GROUP)

OLD_TAG = "edit-http-old"
NEW_TAG = "edit-http-new"
EXTRA_TAG = "edit-http-extra"
POLL_TAG = "edit-http-poll"
DRAFT_TAG = "edit-http-draft"
COMMIT_TAG = "edit-http-commit"
TAG_VALUES = (OLD_TAG, NEW_TAG, EXTRA_TAG, POLL_TAG, DRAFT_TAG, COMMIT_TAG)

# A leading ``[tag]`` is intentionally rejected by the original
# AddTopicRequestValidator/EditTopicRequestValidator.  Keep the reserved
# fixture marker valid as a user-visible title so successful edit scenarios
# test the mutation path rather than the validation-error render path.
TITLE_PREFIX = "edit-http-lifecycle"
BODY_PREFIX = "edit-http-lifecycle:"


def scalar(sql: str) -> str:
    return db(sql).strip()


def sql_string(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def sql_ints(values: tuple[int, ...]) -> str:
    return ",".join(str(value) for value in values)


def sql_text_array(values: tuple[str, ...]) -> str:
    return "ARRAY[" + ",".join(sql_string(value) for value in values) + "]::text[]"


TOPIC_CSV = sql_ints(TOPIC_IDS)
TAG_ARRAY = sql_text_array(TAG_VALUES)


def cleanup() -> None:
    """Remove a previous run and reverse its deterministic trigger deltas."""

    scalar(
        f"""
BEGIN;
SET LOCAL lock_timeout='10s';
SET LOCAL statement_timeout='60s';

DO $$
BEGIN
  IF current_database()<>'lor' THEN
    RAISE EXCEPTION 'edit topic lifecycle only supports the disposable lor database';
  END IF;
  IF EXISTS(
    SELECT 1 FROM topics t
     WHERE t.id IN ({TOPIC_CSV})
       AND (t.title NOT LIKE {sql_string(TITLE_PREFIX + '%')}
            OR t.userid<>CASE t.id
                 WHEN {EDIT_TOPIC} THEN {EDIT_AUTHOR}
                 WHEN {POLL_TOPIC} THEN {POLL_AUTHOR}
                 WHEN {DRAFT_TOPIC} THEN {DRAFT_AUTHOR}
                 WHEN {COMMIT_TOPIC} THEN {COMMIT_AUTHOR}
               END)
  ) THEN
    RAISE EXCEPTION 'reserved edit lifecycle topic id belongs to unrelated data';
  END IF;
  IF EXISTS(
    SELECT 1 FROM msgbase m LEFT JOIN topics t ON t.id=m.id
     WHERE m.id IN ({TOPIC_CSV}) AND t.id IS NULL
       AND m.message NOT LIKE {sql_string(BODY_PREFIX + '%')}
  ) THEN
    RAISE EXCEPTION 'reserved edit lifecycle msgbase id belongs to unrelated data';
  END IF;
  IF EXISTS(SELECT 1 FROM polls WHERE id={POLL_ID} AND topic<>{POLL_TOPIC}) THEN
    RAISE EXCEPTION 'reserved edit lifecycle poll id belongs to unrelated data';
  END IF;
  IF EXISTS(
    SELECT 1 FROM polls_variants
     WHERE id IN ({POLL_VARIANT_A},{POLL_VARIANT_B}) AND vote<>{POLL_ID}
  ) THEN
    RAISE EXCEPTION 'reserved edit lifecycle poll variant id belongs to unrelated data';
  END IF;
  IF EXISTS(
    SELECT 1 FROM tags_values tv JOIN tags t ON t.tagid=tv.id
     WHERE tv.value=ANY({TAG_ARRAY}) AND t.msgid NOT IN ({TOPIC_CSV})
  ) THEN
    RAISE EXCEPTION 'reserved edit lifecycle tag belongs to unrelated data';
  END IF;
END
$$;

CREATE TEMP TABLE edit_http_stale_topics ON COMMIT DROP AS
SELECT t.id,t.userid,
       CASE t.id
         WHEN {EDIT_TOPIC} THEN {EDIT_GROUP}
         WHEN {POLL_TOPIC} THEN {POLL_GROUP}
         WHEN {DRAFT_TOPIC} THEN {NEWS_SOURCE_GROUP}
         WHEN {COMMIT_TOPIC} THEN {NEWS_SOURCE_GROUP}
       END AS inserted_group,
       CASE WHEN t.id={COMMIT_TOPIC} AND t.moderate AND t.commitby={MODERATOR}
            THEN {COMMIT_BONUS} ELSE 0 END AS committed_bonus
  FROM topics t WHERE t.id IN ({TOPIC_CSV});

CREATE TEMP TABLE edit_http_stale_comments ON COMMIT DROP AS
SELECT id FROM comments WHERE topic IN ({TOPIC_CSV});

WITH deleted AS (
  DELETE FROM user_events
   WHERE message_id IN ({TOPIC_CSV})
      OR comment_id IN (SELECT id FROM edit_http_stale_comments)
  RETURNING userid,unread
), removed AS (
  SELECT userid,count(*) FILTER(WHERE unread)::integer AS amount
    FROM deleted GROUP BY userid
)
UPDATE users u SET unread_events=u.unread_events-r.amount
  FROM removed r WHERE u.id=r.userid;

DELETE FROM reactions_log
 WHERE topic_id IN ({TOPIC_CSV})
    OR comment_id IN (SELECT id FROM edit_http_stale_comments);
DELETE FROM message_warnings
 WHERE topic IN ({TOPIC_CSV})
    OR comment IN (SELECT id FROM edit_http_stale_comments);
DELETE FROM edit_info
 WHERE msgid IN ({TOPIC_CSV})
    OR msgid IN (SELECT id FROM edit_http_stale_comments);
DELETE FROM del_info
 WHERE msgid IN ({TOPIC_CSV})
    OR msgid IN (SELECT id FROM edit_http_stale_comments);
DELETE FROM comments WHERE id IN (SELECT id FROM edit_http_stale_comments);
DELETE FROM msgbase WHERE id IN (SELECT id FROM edit_http_stale_comments);
DELETE FROM images WHERE topic IN ({TOPIC_CSV});
DELETE FROM telegram_posts WHERE topic_id IN ({TOPIC_CSV});
DELETE FROM topic_users_notified WHERE topic IN ({TOPIC_CSV});
DELETE FROM vote_users
 WHERE vote={POLL_ID}
    OR vote IN (SELECT id FROM polls WHERE topic IN ({TOPIC_CSV}))
    OR variant_id IN (
      SELECT pv.id FROM polls_variants pv
       WHERE pv.vote={POLL_ID}
          OR pv.vote IN (SELECT id FROM polls WHERE topic IN ({TOPIC_CSV}))
    );
DELETE FROM polls_variants
 WHERE vote={POLL_ID}
    OR vote IN (SELECT id FROM polls WHERE topic IN ({TOPIC_CSV}));
DELETE FROM polls WHERE id={POLL_ID} OR topic IN ({TOPIC_CSV});
DELETE FROM memories WHERE topic IN ({TOPIC_CSV});
DELETE FROM tags WHERE msgid IN ({TOPIC_CSV});

WITH bonus AS (
  SELECT userid,sum(committed_bonus)::integer AS amount
    FROM edit_http_stale_topics
   WHERE committed_bonus<>0 GROUP BY userid
)
UPDATE users u SET score=u.score-b.amount FROM bonus b WHERE u.id=b.userid;

WITH deleted AS (
  DELETE FROM topics WHERE id IN ({TOPIC_CSV}) RETURNING id
), removed AS (
  SELECT s.inserted_group,count(*)::integer AS amount
    FROM edit_http_stale_topics s JOIN deleted d USING(id)
   GROUP BY s.inserted_group
)
UPDATE groups g SET stat3=g.stat3-r.amount FROM removed r WHERE g.id=r.inserted_group;

DELETE FROM msgbase WHERE id IN ({TOPIC_CSV});
DELETE FROM tags_values tv
 WHERE tv.value=ANY({TAG_ARRAY})
   AND NOT EXISTS(SELECT 1 FROM tags t WHERE t.tagid=tv.id);
COMMIT
"""
    )


def setup() -> None:
    scalar(
        f"""
BEGIN;
SET LOCAL lock_timeout='10s';
SET LOCAL statement_timeout='60s';
DO $$
BEGIN
  IF (SELECT count(*) FROM users WHERE id IN
      ({EDIT_AUTHOR},{POLL_AUTHOR},{DRAFT_AUTHOR},{COMMIT_AUTHOR},{MENTION_RECIPIENT},{MODERATOR}))<>6 THEN
    RAISE EXCEPTION 'seed.py account fixture is missing';
  END IF;
  IF (SELECT count(*) FROM groups WHERE id IN ({sql_ints(GROUP_IDS)}))<>4 THEN
    RAISE EXCEPTION 'required Java group catalog is missing';
  END IF;
END
$$;

INSERT INTO msgbase(id,message,markup) VALUES
  ({EDIT_TOPIC},{sql_string(BODY_PREFIX + ' normal original body')},'MARKDOWN'),
  ({POLL_TOPIC},{sql_string(BODY_PREFIX + ' poll original body')},'MARKDOWN'),
  ({DRAFT_TOPIC},{sql_string(BODY_PREFIX + ' draft original body')},'MARKDOWN'),
  ({COMMIT_TOPIC},{sql_string(BODY_PREFIX + ' commit original body')},'MARKDOWN');

INSERT INTO topics(
  id,groupid,userid,title,url,moderate,postdate,linktext,deleted,
  stat1,stat3,lastmod,commitby,notop,commitdate,postscore,postip,
  sticky,resolved,minor,draft,allow_anonymous,reactions,open_warnings
) VALUES
  ({EDIT_TOPIC},{EDIT_GROUP},{EDIT_AUTHOR},
   {sql_string(TITLE_PREFIX + ' normal original')},NULL,false,
   CURRENT_TIMESTAMP-interval '70 minutes',NULL,false,0,0,
   CURRENT_TIMESTAMP-interval '69 minutes',NULL,false,NULL,-9999,'192.0.2.201',
   false,false,false,false,true,'{{}}',0),
  ({POLL_TOPIC},{POLL_GROUP},{POLL_AUTHOR},
   {sql_string(TITLE_PREFIX + ' poll original')},NULL,true,
   CURRENT_TIMESTAMP-interval '65 minutes',NULL,false,0,0,
   CURRENT_TIMESTAMP-interval '64 minutes',{MODERATOR},false,
   CURRENT_TIMESTAMP-interval '63 minutes',-9999,'192.0.2.202',
   false,false,false,false,true,'{{}}',0),
  ({DRAFT_TOPIC},{NEWS_SOURCE_GROUP},{DRAFT_AUTHOR},
   {sql_string(TITLE_PREFIX + ' draft original')},'https://example.test/edit-draft',false,
   CURRENT_TIMESTAMP-interval '60 minutes','Draft source',false,0,0,
   CURRENT_TIMESTAMP-interval '59 minutes',NULL,false,NULL,-9999,'192.0.2.203',
   false,false,false,true,true,'{{}}',0),
  ({COMMIT_TOPIC},{NEWS_SOURCE_GROUP},{COMMIT_AUTHOR},
   {sql_string(TITLE_PREFIX + ' commit original')},'https://example.test/edit-commit',false,
   CURRENT_TIMESTAMP-interval '55 minutes','Commit source',false,0,0,
   CURRENT_TIMESTAMP-interval '54 minutes',NULL,false,NULL,-9999,'192.0.2.204',
   false,false,false,false,true,'{{}}',0);

-- topins_t intentionally owns group.stat3/memories and overwrites lastmod.
-- Reset only the fixture timestamps after observing that canonical trigger.
UPDATE topics SET lastmod=date_trunc('milliseconds',CASE id
  WHEN {EDIT_TOPIC} THEN CURRENT_TIMESTAMP-interval '69 minutes'
  WHEN {POLL_TOPIC} THEN CURRENT_TIMESTAMP-interval '64 minutes'
  WHEN {DRAFT_TOPIC} THEN CURRENT_TIMESTAMP-interval '59 minutes'
  WHEN {COMMIT_TOPIC} THEN CURRENT_TIMESTAMP-interval '54 minutes'
END) WHERE id IN ({TOPIC_CSV});

INSERT INTO tags_values(value,counter)
SELECT value,0 FROM unnest({TAG_ARRAY}) AS value
ON CONFLICT(value) DO NOTHING;
INSERT INTO tags(msgid,tagid)
SELECT fixture.msgid,tv.id
  FROM (VALUES
    ({EDIT_TOPIC},{sql_string(OLD_TAG)}),
    ({POLL_TOPIC},{sql_string(POLL_TAG)}),
    ({DRAFT_TOPIC},{sql_string(DRAFT_TAG)}),
    ({COMMIT_TOPIC},{sql_string(COMMIT_TAG)})
  ) AS fixture(msgid,value)
  JOIN tags_values tv USING(value);
UPDATE tags_values tv
   SET counter=(SELECT count(*)::integer FROM tags t WHERE t.tagid=tv.id)
 WHERE tv.value=ANY({TAG_ARRAY});

INSERT INTO polls(id,topic,multiselect) VALUES({POLL_ID},{POLL_TOPIC},false);
INSERT INTO polls_variants(id,vote,label,votes) VALUES
  ({POLL_VARIANT_A},{POLL_ID},'poll alpha original',3),
  ({POLL_VARIANT_B},{POLL_ID},'poll beta original',1);
COMMIT
"""
    )


def fixture_state() -> str:
    """Stable serialization used to prove preview/error requests are read-only."""

    return scalar(
        f"""
SELECT jsonb_build_object(
  'topics',COALESCE((
    SELECT jsonb_agg(jsonb_build_object(
      'id',t.id,'groupid',t.groupid,'title',t.title,'moderate',t.moderate,
      'draft',t.draft,'commitby',t.commitby,'commitdate',t.commitdate,
      'postdate',t.postdate,'lastmod',t.lastmod,'minor',t.minor
    ) ORDER BY t.id) FROM topics t WHERE t.id IN ({TOPIC_CSV})
  ),'[]'::jsonb),
  'messages',COALESCE((
    SELECT jsonb_agg(jsonb_build_object('id',m.id,'message',m.message) ORDER BY m.id)
      FROM msgbase m WHERE m.id IN ({TOPIC_CSV})
  ),'[]'::jsonb),
  'tags',COALESCE((
    SELECT jsonb_agg(jsonb_build_object('msgid',t.msgid,'value',tv.value,'counter',tv.counter)
                     ORDER BY t.msgid,tv.value)
      FROM tags t JOIN tags_values tv ON tv.id=t.tagid
     WHERE t.msgid IN ({TOPIC_CSV})
  ),'[]'::jsonb),
  'polls',COALESCE((
    SELECT jsonb_agg(jsonb_build_object('id',p.id,'topic',p.topic,'multiselect',p.multiselect)
                     ORDER BY p.id)
      FROM polls p WHERE p.topic IN ({TOPIC_CSV})
  ),'[]'::jsonb),
  'variants',COALESCE((
    SELECT jsonb_agg(jsonb_build_object('id',pv.id,'vote',pv.vote,'label',pv.label,'votes',pv.votes)
                     ORDER BY pv.id)
      FROM polls_variants pv JOIN polls p ON p.id=pv.vote
     WHERE p.topic IN ({TOPIC_CSV})
  ),'[]'::jsonb),
  'history',COALESCE((
    SELECT jsonb_agg(to_jsonb(e) ORDER BY e.id) FROM edit_info e
     WHERE e.msgid IN ({TOPIC_CSV})
  ),'[]'::jsonb),
  'events',COALESCE((
    SELECT jsonb_agg(to_jsonb(e) ORDER BY e.id) FROM user_events e
     WHERE e.message_id IN ({TOPIC_CSV})
  ),'[]'::jsonb),
  'notified',COALESCE((
    SELECT jsonb_agg(to_jsonb(n) ORDER BY n.topic,n.userid) FROM topic_users_notified n
     WHERE n.topic IN ({TOPIC_CSV})
  ),'[]'::jsonb),
  'users',COALESCE((
    SELECT jsonb_agg(jsonb_build_object('id',u.id,'score',u.score,'unread',u.unread_events)
                     ORDER BY u.id)
      FROM users u WHERE u.id IN ({COMMIT_AUTHOR},{MENTION_RECIPIENT})
  ),'[]'::jsonb),
  'groups',COALESCE((
    SELECT jsonb_agg(jsonb_build_object('id',g.id,'stat3',g.stat3) ORDER BY g.id)
      FROM groups g WHERE g.id IN ({sql_ints(GROUP_IDS)})
  ),'[]'::jsonb)
)::text
"""
    )


def user_state() -> str:
    return scalar(
        f"SELECT jsonb_agg(jsonb_build_object('id',id,'score',score,'unread',unread_events) "
        f"ORDER BY id)::text FROM users WHERE id IN ({COMMIT_AUTHOR},{MENTION_RECIPIENT})"
    )


def group_state() -> str:
    return scalar(
        f"SELECT jsonb_agg(jsonb_build_object('id',id,'stat3',stat3) ORDER BY id)::text "
        f"FROM groups WHERE id IN ({sql_ints(GROUP_IDS)})"
    )


def lastmod_millis(topic_id: int) -> int:
    return int(
        scalar(
            f"SELECT (extract(epoch FROM lastmod)*1000)::bigint FROM topics WHERE id={topic_id}"
        )
    )


def topic_article(html: str) -> str:
    match = re.search(r'<article class="msg".*?</article>', html, re.S)
    require(match is not None, "edit preview lacks the source-compatible topic card")
    return match.group(0)


def expect_redirect(response, expected: str, context: str) -> None:
    require(response.status == 302, f"{context}: expected 302, got {response.status}")
    require(
        response.headers.get("Location") == expected,
        f"{context}: unexpected Location {response.headers.get('Location')!r}",
    )


def post_values(
    topic_id: int,
    title: str,
    message: str,
    tags: str,
    *,
    url: str | None = None,
    linktext: str | None = None,
) -> list[tuple[str, str]]:
    values = [
        ("msgid", str(topic_id)),
        ("title", title),
        ("msg", message),
        ("tags", tags),
        ("minor", "false"),
    ]
    if url is not None:
        values.append(("url", url))
    if linktext is not None:
        values.append(("linktext", linktext))
    return values


def cleanup_proof() -> str:
    return scalar(
        f"""
SELECT jsonb_build_object(
  'topics',(SELECT count(*) FROM topics WHERE id IN ({TOPIC_CSV})),
  'msgbase',(SELECT count(*) FROM msgbase WHERE id IN ({TOPIC_CSV})),
  'polls',(SELECT count(*) FROM polls WHERE topic IN ({TOPIC_CSV})),
  'variants',(SELECT count(*) FROM polls_variants WHERE vote={POLL_ID}),
  'tags',(SELECT count(*) FROM tags WHERE msgid IN ({TOPIC_CSV})),
  'tag_values',(SELECT count(*) FROM tags_values WHERE value=ANY({TAG_ARRAY})),
  'history',(SELECT count(*) FROM edit_info WHERE msgid IN ({TOPIC_CSV})),
  'events',(SELECT count(*) FROM user_events WHERE message_id IN ({TOPIC_CSV})),
  'notified',(SELECT count(*) FROM topic_users_notified WHERE topic IN ({TOPIC_CSV})),
  'memories',(SELECT count(*) FROM memories WHERE topic IN ({TOPIC_CSV}))
)::text
"""
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:8181")
    args = parser.parse_args()

    # Repair a prior interrupted invocation before taking the true baseline.
    cleanup()
    baseline_users = user_state()
    baseline_groups = group_state()

    try:
        setup()
        fixture_groups_after_topins = group_state()

        editor = Client(args.base)
        poll_author = Client(args.base)
        draft_author = Client(args.base)
        moderator = Client(args.base)
        editor.login("raven1000")
        # A pending poll deliberately renders the disabled form even with
        # results=true.  Use a freshly committed poll and a moderator editor
        # here so the lifecycle reaches Java's PreparedPoll result branch and
        # can verify the percentages without weakening author permissions.
        poll_author.login("hawk_moderator")
        draft_author.login("tern_corrector")
        moderator.login("hawk_moderator")

        # Spring exposes explicit legacy method metadata.  In particular a
        # bodyless POST to the GET-only commit form reaches method routing and
        # returns 405; it must not be intercepted as a generic CSRF 403.
        for path, expected_allow in (
            ("/edit.jsp", "POST,GET,HEAD,OPTIONS"),
            ("/commit.jsp", "GET,HEAD,OPTIONS"),
        ):
            options = editor.request(path, "OPTIONS")
            require(options.status == 200, f"OPTIONS {path}: expected 200")
            require(
                options.headers.get("Allow") == expected_allow,
                f"OPTIONS {path}: unexpected Allow {options.headers.get('Allow')!r}",
            )
            require(options.body == b"", f"OPTIONS {path}: response body is not empty")
        unsupported_commit = editor.request("/commit.jsp", "POST")
        require(
            unsupported_commit.status == 405,
            f"POST /commit.jsp: expected 405, got {unsupported_commit.status}",
        )
        require(
            unsupported_commit.headers.get("Allow") == "GET",
            "POST /commit.jsp: unexpected Allow header",
        )

        # Preview is a full PreparedTopic-shaped card, but is strictly read-only.
        edit_form = require_html(
            editor.request(f"/edit.jsp?msgid={EDIT_TOPIC}"), "edit topic form"
        )
        require('action="edit.jsp"' in edit_form, "edit form action differs from Java JSP")
        require('id="messageForm"' in edit_form, "edit form id differs from Java JSP")
        before_preview = fixture_state()
        preview = editor.request(
            "/edit.jsp",
            "POST",
            post_values(
                EDIT_TOPIC,
                TITLE_PREFIX + " normal preview",
                BODY_PREFIX + " normal **preview** body",
                f"{NEW_TAG}, {EXTRA_TAG}",
            )
            + [("preview", "")],
        )
        preview_html = require_html(preview, "topic edit preview")
        preview_card = topic_article(preview_html)
        for fragment in [
            TITLE_PREFIX + " normal preview",
            "<strong>preview</strong>",
            NEW_TAG,
            EXTRA_TAG,
            'class="msg-container"',
            "raven1000",
            f'id="topic-{EDIT_TOPIC}"',
            'class="userpic"',
            'class="reactions zero-reactions"',
        ]:
            require(fragment in preview_card, f"edit preview lacks {fragment!r}")
        for canonical_only in [
            'id="topicMenu"',
            'class="fav-buttons"',
            "Последнее исправление:",
            'class="clear-warning-form"',
        ]:
            require(
                canonical_only not in preview_card,
                f"edit preview leaked canonical-only markup {canonical_only!r}",
            )
        require(fixture_state() == before_preview, "preview mutated PostgreSQL state")

        # EditTopicController throws BadInputException before mutation for both
        # an absent and an all-whitespace title when content is editable.
        missing_title = editor.request(
            "/edit.jsp",
            "POST",
            [
                ("msgid", str(EDIT_TOPIC)),
                ("msg", BODY_PREFIX + " crafted missing-title body"),
                ("tags", OLD_TAG),
                ("minor", "false"),
            ],
        )
        blank_title = editor.request(
            "/edit.jsp",
            "POST",
            [
                ("msgid", str(EDIT_TOPIC)),
                ("title", "   "),
                ("msg", BODY_PREFIX + " crafted blank-title body"),
                ("tags", OLD_TAG),
                ("minor", "false"),
            ],
        )
        for response, name in ((missing_title, "missing"), (blank_title, "blank")):
            require(response.status == 500, f"{name} title: expected Java 500")
            require(
                "ru.org.linux.site.BadInputException" in response.text
                and "заголовок сообщения не может быть пустым" in response.text,
                f"{name} title: wrong legacy error page",
            )
        require(fixture_state() == before_preview, "bad-title request mutated PostgreSQL state")

        # A normal edit changes content/tags atomically, records old bytes,
        # advances lastmod by exactly one second, and lets new_event_t own the
        # mentioned user's unread counter increment.
        edit_old_lastmod = lastmod_millis(EDIT_TOPIC)
        recipient_unread = int(
            scalar(f"SELECT unread_events FROM users WHERE id={MENTION_RECIPIENT}")
        )
        normal_title = TITLE_PREFIX + " normal edited"
        normal_body = BODY_PREFIX + " normal edited body mentioning @finch50"
        applied = editor.request(
            "/edit.jsp",
            "POST",
            post_values(
                EDIT_TOPIC,
                normal_title,
                normal_body,
                f"{NEW_TAG}, {EXTRA_TAG}",
            ),
        )
        expect_redirect(
            applied,
            f"/forum/games/{EDIT_TOPIC}?lastmod={edit_old_lastmod}",
            "normal edit",
        )
        normal_row = scalar(
            f"""
SELECT t.title||'|'||m.message||'|'||
       (extract(epoch FROM t.lastmod)*1000)::bigint||'|'||
       COALESCE((SELECT string_agg(tv.value,',' ORDER BY tv.value)
                   FROM tags tg JOIN tags_values tv ON tv.id=tg.tagid
                  WHERE tg.msgid=t.id),'')
  FROM topics t JOIN msgbase m ON m.id=t.id WHERE t.id={EDIT_TOPIC}
"""
        ).split("|")
        require(
            normal_row
            == [
                normal_title,
                normal_body,
                str(edit_old_lastmod + 1000),
                ",".join(sorted((NEW_TAG, EXTRA_TAG))),
            ],
            f"normal edit DB delta differs: {normal_row!r}",
        )
        require(
            scalar(
                f"SELECT string_agg(value||'='||counter,',' ORDER BY value) "
                f"FROM tags_values WHERE value=ANY({sql_text_array((OLD_TAG, NEW_TAG, EXTRA_TAG))})"
            )
            == f"{EXTRA_TAG}=1,{NEW_TAG}=1,{OLD_TAG}=1",
            "tag counters do not preserve Java's add/inert-delete semantics",
        )
        history = json.loads(
            scalar(
                f"""
SELECT jsonb_build_object(
  'count',count(*),
  'editor',max(editor),
  'oldmessage',max(oldmessage),
  'oldtitle',max(oldtitle),
  'oldtags',max(oldtags),
  'oldminor',max(oldminor::text)
)::text FROM edit_info
 WHERE msgid={EDIT_TOPIC} AND object_type='TOPIC'::edit_event_type
"""
            )
        )
        require(
            history
            == {
                "count": 1,
                "editor": EDIT_AUTHOR,
                "oldmessage": BODY_PREFIX + " normal original body",
                "oldtitle": TITLE_PREFIX + " normal original",
                "oldtags": OLD_TAG,
                "oldminor": None,
            },
            f"normal edit history differs: {history!r}",
        )
        require(
            scalar(
                f"SELECT count(*) FROM user_events WHERE userid={MENTION_RECIPIENT} "
                f"AND message_id={EDIT_TOPIC} AND type='REF' AND NOT private AND unread"
            )
            == "1",
            "normal edit did not create exactly one REF event",
        )
        require(
            int(scalar(f"SELECT unread_events FROM users WHERE id={MENTION_RECIPIENT}"))
            == recipient_unread + 1,
            "new_event_t did not increment unread_events exactly once",
        )
        require(
            scalar(
                f"SELECT count(*) FROM topic_users_notified WHERE topic={EDIT_TOPIC} "
                f"AND userid={MENTION_RECIPIENT}"
            )
            == "1",
            "mention deduplication row is absent",
        )

        # Binding newPoll without any poll[...] key leaves form.poll=null in
        # Spring. The old poll must remain visible in preview and unchanged in DB.
        poll_form = require_html(
            poll_author.request(f"/edit.jsp?msgid={POLL_TOPIC}"), "poll edit form"
        )
        require(
            f'name="poll[{POLL_VARIANT_A}]"' in poll_form
            and f'name="poll[{POLL_VARIANT_B}]"' in poll_form,
            "poll edit form lacks original variant keys",
        )
        before_poll_preview = fixture_state()
        crafted_label = "crafted newPoll must be ignored"
        poll_preview = poll_author.request(
            "/edit.jsp",
            "POST",
            post_values(
                POLL_TOPIC,
                TITLE_PREFIX + " poll original",
                BODY_PREFIX + " poll original body",
                POLL_TAG,
            )
            + [("newPoll[0]", crafted_label), ("preview", "")],
        )
        poll_preview_html = require_html(poll_preview, "poll-map absence preview")
        poll_card = topic_article(poll_preview_html)
        require("poll alpha original" in poll_card, "old poll alpha vanished from preview")
        require("poll beta original" in poll_card, "old poll beta vanished from preview")
        require(crafted_label not in poll_card, "newPoll leaked into null poll-map preview")
        require(
            fixture_state() == before_poll_preview,
            "crafted null poll-map preview mutated PostgreSQL state",
        )

        # A present poll map is adapted without persistence: existing labels
        # are replaced in original id order, new id=0 rows are appended, and
        # preview percentages use only the variants still present in the form.
        submitted_poll_preview = poll_author.request(
            "/edit.jsp?results=true",
            "POST",
            post_values(
                POLL_TOPIC,
                TITLE_PREFIX + " poll preview",
                BODY_PREFIX + " poll preview body",
                POLL_TAG,
            )
            + [
                (f"poll[{POLL_VARIANT_A}]", "poll alpha preview"),
                (f"poll[{POLL_VARIANT_B}]", "poll beta preview"),
                ("poll[999999]", "unknown variant must be ignored"),
                ("newPoll[0]", "poll gamma preview"),
                ("preview", ""),
            ],
        )
        submitted_poll_card = topic_article(
            require_html(submitted_poll_preview, "submitted poll preview")
        )
        for fragment in [
            "poll alpha preview",
            "3 (75%)",
            "poll beta preview",
            "1 (25%)",
            "poll gamma preview",
            "0 (0%)",
            "Всего голосов: 4",
        ]:
            require(fragment in submitted_poll_card, f"poll preview lacks {fragment!r}")
        require(
            "unknown variant must be ignored" not in submitted_poll_card,
            "unknown poll variant leaked into preview",
        )
        require(
            fixture_state() == before_poll_preview,
            "submitted poll preview mutated PostgreSQL state",
        )

        poll_old_lastmod = lastmod_millis(POLL_TOPIC)
        poll_edit = poll_author.request(
            "/edit.jsp",
            "POST",
            post_values(
                POLL_TOPIC,
                TITLE_PREFIX + " poll original",
                BODY_PREFIX + " poll original body",
                POLL_TAG,
            )
            + [
                (f"poll[{POLL_VARIANT_A}]", "poll alpha edited"),
                (f"poll[{POLL_VARIANT_B}]", ""),
                ("newPoll[0]", "poll gamma new"),
                ("multiselect", "true"),
            ],
        )
        expect_redirect(
            poll_edit,
            f"/polls/polls/{POLL_TOPIC}?lastmod={poll_old_lastmod}",
            "poll edit",
        )
        poll_state = json.loads(
            scalar(
                f"""
SELECT jsonb_build_object(
  'multiselect',p.multiselect,
  'labels',(SELECT jsonb_agg(label ORDER BY label) FROM polls_variants WHERE vote=p.id),
  'lastmod',(extract(epoch FROM t.lastmod)*1000)::bigint,
  'history',(SELECT count(*) FROM edit_info WHERE msgid=t.id),
  'oldpoll',(SELECT oldpoll FROM edit_info WHERE msgid=t.id ORDER BY id DESC LIMIT 1)
)::text FROM polls p JOIN topics t ON t.id=p.topic WHERE p.topic={POLL_TOPIC}
"""
            )
        )
        require(poll_state["multiselect"] is True, "poll multiselect was not updated")
        require(
            poll_state["labels"] == ["poll alpha edited", "poll gamma new"],
            f"poll variant delta differs: {poll_state['labels']!r}",
        )
        require(
            poll_state["lastmod"] == poll_old_lastmod + 1000,
            "poll edit lastmod delta is not exactly one second",
        )
        require(poll_state["history"] == 1, "poll edit history row is absent")
        old_poll = poll_state["oldpoll"]
        require(old_poll["multiSelect"] is False, "history lost old multiselect")
        require(
            [variant["label"] for variant in old_poll["variants"]]
            == ["poll alpha original", "poll beta original"],
            "history lost old poll variants",
        )

        # Publishing a draft in a premoderated section returns the moderated
        # confirmation page rather than a redirect, and does not commit it.
        draft_form = require_html(
            draft_author.request(f"/edit.jsp?msgid={DRAFT_TOPIC}"), "draft edit form"
        )
        require('name="publish"' in draft_form, "draft publish control is absent")
        draft_old_lastmod = lastmod_millis(DRAFT_TOPIC)
        draft_old_postdate = int(
            scalar(
                f"SELECT (extract(epoch FROM postdate)*1000)::bigint FROM topics WHERE id={DRAFT_TOPIC}"
            )
        )
        published = draft_author.request(
            "/edit.jsp",
            "POST",
            post_values(
                DRAFT_TOPIC,
                TITLE_PREFIX + " draft original",
                BODY_PREFIX + " draft original body",
                DRAFT_TAG,
                url="https://example.test/edit-draft",
                linktext="Draft source",
            )
            + [("publish", "")],
        )
        published_html = require_html(published, "premoderated draft confirmation")
        expected_draft_url = f"/news/android/{DRAFT_TOPIC}?lastmod={draft_old_lastmod}"
        require(
            "Вы поместили сообщение в защищённый раздел" in published_html
            and f'href="{expected_draft_url}"' in published_html,
            "draft publish did not render the Java moderated confirmation",
        )
        draft_state = scalar(
            f"""
SELECT draft::text||'|'||moderate::text||'|'||COALESCE(commitby::text,'')||'|'||
       ((extract(epoch FROM postdate)*1000)::bigint>{draft_old_postdate})::text||'|'||
       (postdate=lastmod)::text||'|'||
       (SELECT count(*) FROM edit_info WHERE msgid=topics.id)
  FROM topics WHERE id={DRAFT_TOPIC}
"""
        ).split("|")
        require(
            draft_state == ["false", "false", "", "true", "true", "0"],
            f"draft publish DB delta differs: {draft_state!r}",
        )

        # /commit.jsp is only the GET form. The exact JSP posts to edit.jsp;
        # commit can move within a section and applies additive score bonuses.
        commit_form = require_html(
            moderator.request(f"/commit.jsp?msgid={COMMIT_TOPIC}"), "commit form"
        )
        require('action="edit.jsp"' in commit_form, "commit form action is not edit.jsp")
        require('name="commit"' in commit_form, "commit submit control is absent")
        require(
            f'<option value="{NEWS_TARGET_GROUP}">' in commit_form,
            "commit form lacks same-section target group",
        )
        commit_old_lastmod = lastmod_millis(COMMIT_TOPIC)
        commit_author_score = int(
            scalar(f"SELECT score FROM users WHERE id={COMMIT_AUTHOR}")
        )
        committed = moderator.request(
            "/edit.jsp",
            "POST",
            post_values(
                COMMIT_TOPIC,
                TITLE_PREFIX + " commit original",
                BODY_PREFIX + " commit original body",
                COMMIT_TAG,
                url="https://example.test/edit-commit",
                linktext="Commit source",
            )
            + [
                ("chgrp", str(NEWS_TARGET_GROUP)),
                ("bonus", str(COMMIT_BONUS)),
                ("commit", ""),
            ],
        )
        # TopicLinkBuilder keeps the pre-move Topic instance for the response.
        expect_redirect(
            committed,
            f"/news/android/{COMMIT_TOPIC}?lastmod={commit_old_lastmod}",
            "commit",
        )
        commit_state = scalar(
            f"""
SELECT groupid||'|'||moderate::text||'|'||commitby||'|'||
       (commitdate IS NOT NULL)::text||'|'||(lastmod=commitdate)::text||'|'||
       (SELECT score FROM users WHERE id={COMMIT_AUTHOR})||'|'||
       (SELECT count(*) FROM edit_info WHERE msgid=topics.id)
  FROM topics WHERE id={COMMIT_TOPIC}
"""
        ).split("|")
        require(
            commit_state
            == [
                str(NEWS_TARGET_GROUP),
                "true",
                str(MODERATOR),
                "true",
                "true",
                str(commit_author_score + COMMIT_BONUS),
                "0",
            ],
            f"commit/chgrp/bonus DB delta differs: {commit_state!r}",
        )
        require(
            group_state() == fixture_groups_after_topins,
            "topic group move unexpectedly repeated or reversed topins_t counters",
        )
    finally:
        cleanup()
        proof = json.loads(cleanup_proof())
        require(all(value == 0 for value in proof.values()), f"cleanup is incomplete: {proof!r}")
        require(user_state() == baseline_users, "cleanup did not restore score/unread state")
        require(group_state() == baseline_groups, "cleanup did not restore topins_t group counters")

    print("PASS topic edit/preview/poll/publish/commit lifecycle")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
