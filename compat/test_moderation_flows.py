#!/usr/bin/env python3
"""Stateful user-moderation regression against the disposable Compose DB.

The test intentionally verifies both the HTTP contract and the canonical
Java/Liquibase tables. It refuses to run unless explicitly enabled.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import time

from stateful_database import psql_target
from test_write_flows import login, post, require, text, wait_for_topic_interval


last_topic_created_at = 0.0
last_comment_created_at = 0.0


def db(sql: str) -> str:
    command, child_env, _ = psql_target()
    result = subprocess.run(
        [*command, "-At", "-v", "ON_ERROR_STOP=1", "-c", sql],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=child_env,
    )
    return result.stdout.strip()


def verify_database_target() -> None:
    _, _, expected_database = psql_target()
    if expected_database is not None:
        require(
            db("SELECT current_database()") == expected_database,
            "connected PostgreSQL database differs from STATEFUL_EXPECTED_DATABASE",
        )


def action(client, target_id: int, name: str, *extra: tuple[str, str]):
    response = post(
        client,
        "/usermod.jsp",
        [("action", name), ("id", str(target_id)), *extra],
    )
    expected = 200 if name in {"reset-password", "block-n-delete-comments"} else 302
    require(
        response.status == expected,
        f"usermod {name} returned {response.status}: {text(response)[:500]}",
    )
    return response


def create_topic(client, group_id: str, author_id: int, suffix: str) -> int:
    global last_topic_created_at
    wait_for_topic_interval(last_topic_created_at)
    title = f"Rust moderation delete target {suffix}"
    response = post(
        client,
        "/add.jsp",
        [
            ("group", group_id),
            ("title", title),
            ("msg", f"Mass-delete topic body {suffix}"),
            ("tags", "linux"),
            ("allowAnonymous", "true"),
        ],
    )
    require(
        response.status == 303,
        f"mass-delete topic creation returned {response.status}: {text(response)[:500]}",
    )
    match = re.fullmatch(r"/forum/[^/]+/(\d+)", response.location_target or "")
    require(
        match is not None,
        f"mass-delete topic has unexpected redirect {response.location_target!r}",
    )
    topic_id = int(match.group(1))
    require(
        db(f"SELECT userid={author_id} AND NOT deleted FROM topics WHERE id={topic_id}") == "t",
        "mass-delete topic was not committed for its author",
    )
    last_topic_created_at = time.monotonic()
    return topic_id


def create_comment(
    client,
    topic_id: int,
    author_id: int,
    body: str,
    reply_to: int = 0,
    interval_seconds: float | None = None,
) -> int:
    global last_comment_created_at
    interval = (
        interval_seconds
        if interval_seconds is not None
        else float(os.environ.get("MODERATION_FLOW_COMMENT_INTERVAL_SECONDS", "3.2"))
    )
    remaining = interval - (time.monotonic() - last_comment_created_at)
    if remaining > 0:
        time.sleep(remaining)
    response = post(
        client,
        "/add_comment_ajax",
        [("topic", str(topic_id)), ("replyto", str(reply_to)), ("msg", body)],
    )
    require(
        response.status == 200,
        f"comment creation returned {response.status}: {text(response)[:500]}",
    )
    payload = json.loads(text(response))
    require(not payload.get("errors"), f"comment creation was rejected: {payload.get('errors')}")
    comment_id = db(
        f"SELECT c.id FROM comments c JOIN msgbase m ON m.id=c.id "
        f"WHERE c.topic={topic_id} AND c.userid={author_id} "
        f"AND m.message='{body}' ORDER BY c.id DESC LIMIT 1"
    )
    require(bool(comment_id), f"created comment is absent from canonical table: {body}")
    last_comment_created_at = time.monotonic()
    return int(comment_id)


def main() -> int:
    global last_comment_created_at, last_topic_created_at
    if sys.argv[1:] == ["--verify-database-only"]:
        verify_database_target()
        print("Stateful external database target verified")
        return 0
    if sys.argv[1:]:
        print("usage: test_moderation_flows.py [--verify-database-only]", file=sys.stderr)
        return 2
    if os.environ.get("MODERATION_FLOW_ALLOW_MUTATION") != "yes":
        print("MODERATION_FLOW_ALLOW_MUTATION=yes is required", file=sys.stderr)
        return 2
    verify_database_target()

    base = os.environ.get("NEW_BASE_URL", "http://127.0.0.1:8181")
    moderator_nick = os.environ["MODERATION_FLOW_MODERATOR_NICK"]
    moderator_password = os.environ["MODERATION_FLOW_MODERATOR_PASSWORD"]
    target_nick = os.environ["MODERATION_FLOW_TARGET_NICK"]
    low_nick = os.environ["MODERATION_FLOW_LOW_NICK"]
    low_password = os.environ["MODERATION_FLOW_LOW_PASSWORD"]
    delete_nick = os.environ["MODERATION_FLOW_DELETE_NICK"]
    delete_password = os.environ["MODERATION_FLOW_DELETE_PASSWORD"]
    corrector_nick = os.environ["MODERATION_FLOW_CORRECTOR_NICK"]
    corrector_password = os.environ["MODERATION_FLOW_CORRECTOR_PASSWORD"]
    group_id = os.environ.get("MODERATION_FLOW_GROUP_ID", "126")
    # The preceding write-flow in CI uses the same source IP. Start the
    # original per-IP interval here because the in-memory cache is not
    # observable from this separate process.
    last_topic_created_at = time.monotonic()
    last_comment_created_at = time.monotonic()

    moderator = login(base, moderator_nick, moderator_password)
    delete_user = login(base, delete_nick, delete_password)
    corrector = login(base, corrector_nick, corrector_password)
    target_id = int(db(f"SELECT id FROM users WHERE nick='{target_nick}'"))
    low_id = int(db(f"SELECT id FROM users WHERE nick='{low_nick}'"))
    delete_id = int(db(f"SELECT id FROM users WHERE nick='{delete_nick}'"))
    corrector_id = int(db(f"SELECT id FROM users WHERE nick='{corrector_nick}'"))
    moderator_id = int(db(f"SELECT id FROM users WHERE nick='{moderator_nick}'"))
    old_password = db(f"SELECT passwd FROM users WHERE id={target_id}")
    suffix = str(int(time.time() * 1000))
    host_topic_id = create_topic(moderator, group_id, moderator_id, f"host-{suffix}")
    warning_author = login(base, low_nick, low_password)
    low_score_warning_form = warning_author.request(
        f"/post-warning?topic={host_topic_id}", "GET"
    )
    require(
        low_score_warning_form.status == 200
        and "Вы не можете отправить уведомление" in text(low_score_warning_form),
        "score<50 warning form differs from Java validation response",
    )

    removed_userpic = post(moderator, "/remove-userpic.jsp", [("id", str(target_id))])
    require(removed_userpic.status == 302, f"remove-userpic returned {removed_userpic.status}")
    action(moderator, target_id, "remove_userinfo")
    action(moderator, target_id, "remove_town")
    action(moderator, target_id, "remove_url")
    action(moderator, target_id, "toggle_corrector")
    require(db(f"SELECT corrector FROM users WHERE id={target_id}") == "t", "corrector was not set")
    action(moderator, target_id, "toggle_corrector")
    action(moderator, target_id, "freeze", ("reason", "compat freeze"), ("shift", "час"))
    require(
        db(f"SELECT frozen_until > CURRENT_TIMESTAMP FROM users WHERE id={target_id}") == "t",
        "freeze deadline was not stored",
    )
    action(
        moderator,
        target_id,
        "freeze",
        ("reason", "compat defrost"),
        ("shift", "Разморозить"),
    )
    action(moderator, target_id, "reset-password")
    action(moderator, target_id, "block", ("reason", "compat block"))
    require(
        db(f"SELECT blocked FROM users WHERE id={target_id}") == "t",
        "block flag was not stored",
    )
    action(moderator, target_id, "unblock")
    action(moderator, low_id, "score50")

    delete_topic_id = create_topic(delete_user, group_id, delete_id, suffix)
    leaf_parent_id = create_comment(
        delete_user, host_topic_id, delete_id, f"delete leaf parent {suffix}"
    )
    leaf_child_id = create_comment(
        delete_user,
        host_topic_id,
        delete_id,
        f"delete leaf child {suffix}",
        leaf_parent_id,
    )
    skipped_id = create_comment(
        delete_user, host_topic_id, delete_id, f"keep replied parent {suffix}"
    )
    surviving_reply_id = create_comment(
        moderator,
        host_topic_id,
        moderator_id,
        f"surviving moderator reply {suffix}",
        skipped_id,
    )
    db(
        f"INSERT INTO user_events(userid,type,private,message_id) "
        f"VALUES({moderator_id},'REF',false,{delete_topic_id});"
        f"INSERT INTO user_events(userid,type,private,message_id,comment_id) "
        f"VALUES({moderator_id},'REPLY',false,{host_topic_id},{leaf_child_id});"
        f"UPDATE users SET unread_events=(SELECT count(*) FROM user_events "
        f"WHERE userid={moderator_id} AND unread) WHERE id={moderator_id}"
    )
    mass_delete = action(
        moderator,
        delete_id,
        "block-n-delete-comments",
        ("reason", "compat mass delete"),
    )
    mass_delete_html = text(mass_delete)
    require(
        "Удалено тем: 1; удалено комментариев: 2" in mass_delete_html,
        "mass-delete result has unexpected counters",
    )
    require(
        f"msgid={skipped_id}" in mass_delete_html,
        "mass-delete result omits the skipped comment",
    )

    target_state = db(
        f"SELECT userinfo IS NULL,town IS NULL,url IS NULL,score,corrector,blocked,"
        f"frozen_until <= CURRENT_TIMESTAMP + interval '5 seconds',passwd<>'{old_password}',"
        f"lostpwd='epoch'::timestamptz FROM users WHERE id={target_id}"
    )
    require(
        target_state == "t|t|t|220|f|f|t|t|t",
        f"unexpected final moderated user state: {target_state}",
    )
    require(
        db(f"SELECT score||'|'||max_score FROM users WHERE id={low_id}") == "50|50",
        "score50 did not update score and max_score",
    )
    require(
        db(f"SELECT count(*) FROM ban_info WHERE userid={target_id}") == "0",
        "unblock kept ban_info",
    )
    require(
        db(f"SELECT blocked FROM users WHERE id={delete_id}") == "t",
        "mass-delete target was not blocked",
    )
    require(
        db(f"SELECT reason FROM ban_info WHERE userid={delete_id}") == "compat mass delete",
        "mass-delete ban reason differs from Java",
    )
    require(
        db(
            f"SELECT info->'reason' FROM user_log WHERE userid={delete_id} "
            f"AND action_userid={moderator_id} AND action='block_user' "
            f"ORDER BY id DESC LIMIT 1"
        )
        == "compat mass delete",
        "mass-delete block audit payload differs from Java",
    )
    deleted_state = db(
        f"SELECT (SELECT deleted FROM topics WHERE id={delete_topic_id}),"
        f"(SELECT deleted FROM comments WHERE id={leaf_parent_id}),"
        f"(SELECT deleted FROM comments WHERE id={leaf_child_id}),"
        f"(SELECT deleted FROM comments WHERE id={skipped_id}),"
        f"(SELECT deleted FROM comments WHERE id={surviving_reply_id})"
    )
    require(
        deleted_state == "t|t|t|f|f",
        f"unexpected mass-delete graph state: {deleted_state}",
    )
    deleted_chain_view = moderator.request(f"/view-deleted?id={leaf_child_id}", "GET")
    deleted_chain_html = text(deleted_chain_view)
    require(
        deleted_chain_view.status == 200
        and f"delete leaf parent {suffix}" in deleted_chain_html
        and f"delete leaf child {suffix}" in deleted_chain_html
        and "Ответ на:" in deleted_chain_html,
        "moderator deleted-comment view does not render the original parent chain",
    )
    require(
        db(
            f"SELECT stat1=(SELECT count(*) FROM comments WHERE topic={host_topic_id} "
            f"AND NOT deleted) FROM topics WHERE id={host_topic_id}"
        )
        == "t",
        "mass-delete topic comment counter is inconsistent",
    )
    require(
        db(
            f"SELECT count(*) FROM del_info WHERE msgid IN "
            f"({delete_topic_id},{leaf_parent_id},{leaf_child_id}) "
            f"AND reason='\u0411\u043b\u043e\u043a\u0438\u0440\u043e\u0432\u043a\u0430 \u043f\u043e\u043b\u044c\u0437\u043e\u0432\u0430\u0442\u0435\u043b\u044f \u0441 \u0443\u0434\u0430\u043b\u0435\u043d\u0438\u0435\u043c \u0441\u043e\u043e\u0431\u0449\u0435\u043d\u0438\u0439' AND delby={moderator_id}"
        )
        == "3",
        "mass-delete del_info rows differ from Java",
    )
    require(
        db(
            f"SELECT count(*) FROM del_info WHERE msgid IN "
            f"({skipped_id},{surviving_reply_id})"
        )
        == "0",
        "mass-delete wrote del_info for surviving comments",
    )
    require(
        db(
            f"SELECT count(*) FROM user_events WHERE userid={moderator_id} AND "
            f"(message_id={delete_topic_id} OR comment_id={leaf_child_id})"
        )
        == "0",
        "mass-delete kept events for deleted content",
    )
    require(
        db(
            f"SELECT unread_events=(SELECT count(*) FROM user_events "
            f"WHERE userid={moderator_id} AND unread) FROM users WHERE id={moderator_id}"
        )
        == "t",
        "mass-delete did not recalculate unread event counter",
    )

    warning_form = warning_author.request(f"/post-warning?topic={host_topic_id}", "GET")
    warning_form_html = text(warning_form)
    require(warning_form.status == 200, f"warning form returned {warning_form.status}")
    require(
        'name="warningType"' in warning_form_html
        and 'name="ruleType"' in warning_form_html
        and 'name="text"' in warning_form_html,
        "warning form does not expose the Java bean field names",
    )
    topic_lastmod_before = db(f"SELECT lastmod FROM topics WHERE id={host_topic_id}")
    topic_warning = post(
        warning_author,
        "/post-warning",
        [
            ("topic", str(host_topic_id)),
            ("comment", "0"),
            ("warningType", "rule"),
            ("ruleType", "4.1 Офтопик"),
            ("text", f"compat topic warning {suffix}"),
        ],
    )
    require(
        topic_warning.status == 200 and "Уведомление отправлено" in text(topic_warning),
        f"topic warning returned an unexpected response: {topic_warning.status}",
    )
    topic_warning_id = int(
        db(
            f"SELECT id FROM message_warnings WHERE topic={host_topic_id} "
            f"AND comment IS NULL AND author={low_id} ORDER BY id DESC LIMIT 1"
        )
    )
    require(
        db(
            f"SELECT warning_type||'|'||message FROM message_warnings "
            f"WHERE id={topic_warning_id}"
        )
        == f"rule|[4.1 Офтопик] compat topic warning {suffix}",
        "topic warning payload differs from Java",
    )
    active_moderators = db(
        "SELECT count(*) FROM users WHERE canmod "
        "AND lastlogin>CURRENT_TIMESTAMP-interval '30 days'"
    )
    require(
        db(f"SELECT count(*) FROM user_events WHERE warning_id={topic_warning_id}")
        == active_moderators,
        "rule warning recipients differ from active Java moderators",
    )
    require(
        db(
            f"SELECT bool_and(private AND type='WARNING' AND origin_user={low_id} "
            f"AND message='[Нарушение правил] [4.1 Офтопик] "
            f"compat topic warning {suffix}') FROM user_events WHERE warning_id={topic_warning_id}"
        )
        == "t",
        "rule warning event payload differs from Java",
    )
    require(
        db(
            f"SELECT open_warnings=1 AND lastmod>'{topic_lastmod_before}' "
            f"FROM topics WHERE id={host_topic_id}"
        )
        == "t",
        "topic warning did not update lastmod/open_warnings",
    )
    require(
        db(
            f"SELECT count(*) FROM users u WHERE u.id IN "
            f"(SELECT userid FROM user_events WHERE warning_id={topic_warning_id}) "
            f"AND u.unread_events<>(SELECT count(*) FROM user_events e "
            f"WHERE e.userid=u.id AND e.unread)"
        )
        == "0",
        "warning events left an inconsistent unread counter",
    )
    moderator_warning_page = moderator.request("/notifications?filter=warning", "GET")
    moderator_warning_html = text(moderator_warning_page)
    expected_topic_warning_event = (
        f"[Нарушение правил] [4.1 Офтопик] compat topic warning {suffix}"
    )
    require(
        moderator_warning_page.status == 200,
        f"warning notification page returned {moderator_warning_page.status}",
    )
    require(
        expected_topic_warning_event in moderator_warning_html,
        "warning notification presentation omits the Java event message",
    )
    require(
        "(Форум)" in moderator_warning_html,
        "warning notification presentation omits the Java section name",
    )
    # Both show-replies JSPs call <lor:user> without link=true. user.tag
    # therefore renders a plain (escaped) nick here, not a profile anchor.
    expected_warning_author = (
        f'<div class="notifications-who-when"><p>{low_nick}, <time '
    )
    require(
        expected_warning_author in moderator_warning_html,
        "warning notification presentation omits the Java plain author data",
    )
    cleared_topic_warning = post(
        corrector, "/clear-warning", [("id", str(topic_warning_id))]
    )
    require(
        cleared_topic_warning.status == 302
        and cleared_topic_warning.location_target == f"/forum/general/{host_topic_id}",
        "corrector topic-warning clear redirect differs from Java",
    )
    require(
        db(
            f"SELECT closed_by={corrector_id} AND closed_when IS NOT NULL "
            f"FROM message_warnings WHERE id={topic_warning_id}"
        )
        == "t"
        and db(f"SELECT open_warnings FROM topics WHERE id={host_topic_id}") == "0",
        "topic warning clear did not persist closure/counter",
    )
    closed_warning_html = text(moderator.request("/notifications?filter=warning", "GET"))
    require(
        f"<s>{expected_topic_warning_event}</s>" in closed_warning_html,
        "closed warning notification is not struck through like the original JSP",
    )

    comment_lastmod_before = db(f"SELECT lastmod FROM topics WHERE id={host_topic_id}")
    comment_warning = post(
        warning_author,
        "/post-warning",
        [
            ("topic", str(host_topic_id)),
            ("comment", str(surviving_reply_id)),
            ("text", f"compat comment warning {suffix}"),
        ],
    )
    require(comment_warning.status == 200, f"comment warning returned {comment_warning.status}")
    require(
        f"/forum/general/{host_topic_id}#comment-{surviving_reply_id}" in text(comment_warning),
        "comment warning action link differs from Java",
    )
    comment_warning_id = int(
        db(
            f"SELECT id FROM message_warnings WHERE comment={surviving_reply_id} "
            f"AND author={low_id} ORDER BY id DESC LIMIT 1"
        )
    )
    require(
        db(
            f"SELECT warning_type||'|'||message FROM message_warnings "
            f"WHERE id={comment_warning_id}"
        )
        == f"rule|compat comment warning {suffix}",
        "comment warning did not apply Java's implicit single warning type",
    )
    require(
        db(f"SELECT lastmod='{comment_lastmod_before}' FROM topics WHERE id={host_topic_id}")
        == "t",
        "comment warning unexpectedly changed topic lastmod",
    )
    cleared_comment_warning = post(
        corrector, "/clear-warning", [("id", str(comment_warning_id))]
    )
    require(
        cleared_comment_warning.status == 302
        and cleared_comment_warning.location_target == f"/forum/general/{host_topic_id}"
        and cleared_comment_warning.location_fragment == f"comment-{surviving_reply_id}",
        "comment-warning clear redirect differs from Java",
    )
    require(
        db(f"SELECT lastmod='{comment_lastmod_before}' FROM topics WHERE id={host_topic_id}")
        == "t",
        "comment warning clear unexpectedly changed topic lastmod",
    )

    last_tag_warning_id = 0
    for index in range(3):
        tag_warning = post(
            warning_author,
            "/post-warning",
            [
                ("topic", str(host_topic_id)),
                ("warningType", "tag"),
                ("text", f"compat tag warning {suffix}-{index}"),
            ],
        )
        require(tag_warning.status == 200, f"tag warning {index} returned {tag_warning.status}")
        last_tag_warning_id = int(
            db(
                f"SELECT id FROM message_warnings WHERE topic={host_topic_id} "
                f"AND author={low_id} ORDER BY id DESC LIMIT 1"
            )
        )
    require(
        db(
            f"SELECT count(*) FROM user_events WHERE warning_id={last_tag_warning_id} "
            f"AND userid={corrector_id}"
        )
        == "1",
        "tag warning did not notify an active corrector",
    )
    limited_warning = post(
        warning_author,
        "/post-warning",
        [
            ("topic", str(host_topic_id)),
            ("warningType", "tag"),
            ("text", f"compat limited warning {suffix}"),
        ],
    )
    require(
        limited_warning.status == 200
        and "Вы не можете отправить более 5 уведомлений в час" in text(limited_warning)
        and db(
            f"SELECT count(*) FROM message_warnings WHERE author={low_id} "
            f"AND postdate>CURRENT_TIMESTAMP-interval '1 hour'"
        )
        == "5",
        "sixth warning was not rejected with the Java form error",
    )

    deleted_view_comment_id = create_comment(
        warning_author,
        host_topic_id,
        low_id,
        f"deleted view payload {suffix}",
        interval_seconds=30.5,
    )
    deleted_reason = f"compat deleted notification {suffix}"
    deleted_comment = post(
        moderator,
        "/delete_comment.jsp",
        [
            ("msgid", str(deleted_view_comment_id)),
            ("reason", deleted_reason),
            ("bonus", "2"),
        ],
    )
    require(
        deleted_comment.status == 200,
        f"moderator comment deletion returned {deleted_comment.status}",
    )
    deleted_event_id = int(
        db(
            f"SELECT id FROM user_events WHERE userid={low_id} AND type='DEL' "
            f"AND comment_id={deleted_view_comment_id} ORDER BY id DESC LIMIT 1"
        )
    )
    deleted_notifications = warning_author.request("/notifications?filter=deleted", "GET")
    deleted_notifications_html = text(deleted_notifications)
    require(
        deleted_notifications.status == 200
        and deleted_reason in deleted_notifications_html
        and "(-2)" in deleted_notifications_html
        and f'name="firstId" value="{deleted_event_id}"' in deleted_notifications_html,
        "deleted notification omits reason/bonus or is not addressable",
    )
    deleted_click = post(
        warning_author,
        "/notifications-click",
        [("firstId", str(deleted_event_id)), ("lastId", str(deleted_event_id))],
    )
    require(
        deleted_click.status == 302
        and deleted_click.location_target
        == f"/view-deleted?id={deleted_view_comment_id}"
        and deleted_click.location_fragment == f"comment-{deleted_view_comment_id}",
        "DEL notification click does not use the original /view-deleted target",
    )
    deleted_view = warning_author.request(
        f"/view-deleted?id={deleted_view_comment_id}", "GET"
    )
    require(
        deleted_view.status == 200
        and "Просмотр удаленного комментария" in text(deleted_view)
        and f"deleted view payload {suffix}" in text(deleted_view)
        and deleted_reason in text(deleted_view),
        "recent non-frozen author cannot view the original deleted-comment page",
    )

    expected_actions = {
        "block_user",
        "defrosted",
        "frozen",
        "reset_info",
        "reset_password",
        "reset_town",
        "reset_url",
        "reset_userpic",
        "set_corrector",
        "unset_corrector",
        "unblock_user",
    }
    actual_actions = set(
        db(
            f"SELECT action::text FROM user_log WHERE userid={target_id} "
            f"AND action_userid={moderator_id} ORDER BY id"
        ).splitlines()
    )
    require(
        expected_actions <= actual_actions,
        f"missing user_log actions: {expected_actions - actual_actions}",
    )
    require(
        db(
            f"SELECT info->'old_userpic',info->'bonus' FROM user_log "
            f"WHERE userid={target_id} AND action='reset_userpic' ORDER BY id DESC LIMIT 1"
        )
        == "compat-userpic.png|-10",
        "reset_userpic audit payload differs from Java",
    )
    require(
        db(
            f"SELECT info->'old_info',info->'bonus' FROM user_log "
            f"WHERE userid={target_id} AND action='reset_info' ORDER BY id DESC LIMIT 1"
        )
        == "bad profile|-10",
        "reset_info audit payload differs from Java",
    )
    require(
        db(
            f"SELECT info->'reason',info ? 'until' FROM user_log "
            f"WHERE userid={target_id} AND action='frozen' ORDER BY id DESC LIMIT 1"
        )
        == "compat freeze|t",
        "freeze audit payload differs from Java",
    )
    require(
        db(
            f"SELECT info->'reason',info ? 'until' FROM user_log "
            f"WHERE userid={target_id} AND action='defrosted' ORDER BY id DESC LIMIT 1"
        )
        == "compat defrost|f",
        "defrost audit payload differs from Java",
    )

    # EditProfileChecker: for 24 hours after a moderator reset the browser
    # form is readonly and POST may update only password/email, never restore
    # the moderated profile fields.
    db(f"UPDATE users SET userinfo='temporary moderated info' WHERE id={low_id}")
    action(moderator, low_id, "remove_userinfo")
    restricted_edit = warning_author.request(
        f"/people/{low_nick}/edit", "GET"
    )
    restricted_html = text(restricted_edit)
    require(
        restricted_edit.status == 200
        and "Сейчас доступна только смена пароля и email" in restricted_html
        and re.search(r'<textarea id="info" name="info"\s+readonly', restricted_html)
        is not None,
        "recent moderator reset did not make profile-info fields readonly",
    )
    low_email = db(f"SELECT email FROM users WHERE id={low_id}")
    self_set_info_before = int(
        db(
            f"SELECT count(*) FROM user_log WHERE userid={low_id} "
            "AND userid=action_userid AND action='set_info'"
        )
    )
    restricted_post = post(
        warning_author,
        f"/people/{low_nick}/edit",
        [
            ("email", low_email),
            ("info", "must not be restored"),
            ("infoMarkup", "markdown"),
            ("oldpass", low_password),
        ],
    )
    require(restricted_post.status == 302, "restricted profile update did not complete")
    require(
        db(f"SELECT COALESCE(userinfo,'') FROM users WHERE id={low_id}") == ""
        and int(
            db(
                f"SELECT count(*) FROM user_log WHERE userid={low_id} "
                "AND userid=action_userid AND action='set_info'"
            )
        )
        == self_set_info_before,
        "restricted profile POST changed info or wrote a false set_info audit",
    )

    tracker = moderator.request("/tracker/", "GET")
    tracker_body = text(tracker)
    require(tracker.status == 200, f"moderator tracker returned {tracker.status}")
    for marker in (
        "<h2>Пользователи</h2>",
        "Заблокированные пользователи за последние 3 дня:",
        "Разблокированные пользователи за последние 3 дня:",
        "Размороженные пользователи за последние 3 дня:",
    ):
        require(marker in tracker_body, f"moderator tracker is missing {marker!r}")

    print(
        f"moderation flow passed: moderator={moderator_id} target={target_id} low={low_id} "
        f"deleted_topic={delete_topic_id} deleted_comments={leaf_parent_id},{leaf_child_id} "
        f"skipped={skipped_id} warnings={topic_warning_id},{comment_warning_id}"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as error:
        detail = error.stderr.strip() if error.stderr else str(error)
        print(f"moderation flow failed: {detail}", file=sys.stderr)
        raise SystemExit(1)
    except AssertionError as error:
        print(f"moderation flow failed: {error}", file=sys.stderr)
        raise SystemExit(1)
