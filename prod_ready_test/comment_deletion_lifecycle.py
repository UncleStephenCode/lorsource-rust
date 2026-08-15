#!/usr/bin/env python3
"""Idempotent browser/DB regression for Java-compatible comment deletion.

The HTTP mutation is performed through the public forms.  SQL is used only
for before/after assertions and narrowly-scoped restoration of the dedicated
prod_ready fixture, so reruns leave the comment, score, counters and events in
their original state.
"""

from __future__ import annotations

import argparse

from test_port import Client, db, require


COMMENT_ID = 9102004
CHILD_ID = 9102016
TOPIC_ID = 9101003
AUTHOR_ID = 9100009
MODERATOR_ID = 9100013
TOPIC_URL = "/forum/games/9101003"
REASON = "prod-ready comment lifecycle"


def scalar(sql: str) -> str:
    return db(sql).strip()


def sql_string(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:8181")
    args = parser.parse_args()

    snapshot = scalar(
        f"""SELECT c.deleted::text||'|'||u.score||'|'||u.unread_events||'|'||
                    t.stat1||'|'||t.stat3||'|'||extract(epoch FROM t.lastmod)||'|'||
                    COALESCE((SELECT max(id) FROM user_events),0)
               FROM comments c
               JOIN users u ON u.id=c.userid
               JOIN topics t ON t.id=c.topic
              WHERE c.id={COMMENT_ID}"""
    ).split("|")
    require(len(snapshot) == 7, "comment lifecycle fixture is absent")
    (
        initial_deleted,
        initial_score,
        initial_unread,
        initial_stat1,
        initial_stat3,
        initial_lastmod_epoch,
        initial_event_max,
    ) = snapshot
    require(initial_deleted == "false", "comment lifecycle fixture is already deleted")
    require(
        scalar(f"SELECT count(*) FROM del_info WHERE msgid={COMMENT_ID}") == "0",
        "comment lifecycle fixture already has del_info",
    )
    require(
        scalar(f"SELECT count(*) FROM user_events WHERE comment_id={COMMENT_ID}") == "0",
        "comment lifecycle fixture unexpectedly has events",
    )

    moderator = Client(args.base)
    created_event_ids: list[int] = []
    try:
        moderator.login("hawk_moderator")

        form = moderator.request(f"/delete_comment.jsp?msgid={COMMENT_ID}")
        require(form.status == 200, f"delete form returned {form.status}")
        form_html = form.text
        for fragment in [
            '<form method=POST action="delete_comment.jsp"',
            f'id="comment-{COMMENT_ID}"',
            f'id="comment-{CHILD_ID}"',
            'class="userpic"',
            f'/delete_comment.jsp?msgid={COMMENT_ID}',
            'class="reactions',
        ]:
            require(fragment in form_html, f"delete preview lacks {fragment!r}")
        require("Ответить" not in form_html, "commentsAllowed=false leaked Reply action")

        deleted = moderator.request(
            "/delete_comment.jsp",
            "POST",
            [
                ("msgid", str(COMMENT_ID)),
                ("reason", REASON),
                ("bonus", "7"),
                ("delete_replys", "false"),
            ],
        )
        require(deleted.status == 200, f"delete mutation returned {deleted.status}")
        require("Удалено успешно" in deleted.text, "delete success message is absent")
        require(
            f"Удаленные комментарии: {COMMENT_ID}" in deleted.text,
            "delete result IDs differ from Java",
        )
        require("Поиск по User-Agent" in deleted.text, "moderator navigation is absent")

        delete_state = scalar(
            f"""SELECT c.deleted::text||'|'||di.delby||'|'||di.reason||'|'||di.bonus||'|'||
                        u.score||'|'||u.unread_events||'|'||t.stat1||'|'||child.deleted::text
                   FROM comments c
                   JOIN del_info di ON di.msgid=c.id
                   JOIN users u ON u.id=c.userid
                   JOIN topics t ON t.id=c.topic
                   JOIN comments child ON child.id={CHILD_ID}
                  WHERE c.id={COMMENT_ID}"""
        ).split("|")
        require(
            delete_state
            == [
                "true",
                str(MODERATOR_ID),
                REASON,
                "-7",
                str(int(initial_score) - 7),
                str(int(initial_unread) + 1),
                str(int(initial_stat1) - 1),
                "false",
            ],
            f"unexpected delete DB delta: {delete_state!r}",
        )
        event_csv = scalar(
            f"""SELECT COALESCE(string_agg(id::text,',' ORDER BY id),'')
                   FROM user_events
                  WHERE id>{initial_event_max} AND userid={AUTHOR_ID}
                    AND type='DEL' AND private AND message_id={TOPIC_ID}
                    AND comment_id={COMMENT_ID} AND message={sql_string(REASON)}"""
        )
        created_event_ids = [int(value) for value in event_csv.split(",") if value]
        require(len(created_event_ids) == 1, "delete notification event was not inserted exactly once")

        undelete_form = moderator.request(f"/undelete_comment?msgid={COMMENT_ID}")
        require(undelete_form.status == 200, f"undelete form returned {undelete_form.status}")
        undelete_html = undelete_form.text
        require(
            f"Сообщение удалено hawk_moderator по причине: {REASON}"
            in undelete_html,
            "undelete preview lacks Java delete-info header",
        )
        require('class="userpic"' in undelete_html, "undelete preview lacks userpic slot")
        require('class="reply"' not in undelete_html, "showMenu=false leaked comment menu")
        require("Ответ на:" not in undelete_html, "prepareCommentOnly leaked reply context")

        restored = moderator.request(
            "/undelete_comment", "POST", [("msgid", str(COMMENT_ID))]
        )
        require(restored.status == 302, f"undelete mutation returned {restored.status}")
        require(
            restored.headers.get("Location") == f"{TOPIC_URL}?cid={COMMENT_ID}",
            f"unexpected undelete redirect: {restored.headers.get('Location')!r}",
        )
        require(
            scalar(
                f"""SELECT c.deleted::text||'|'||u.score||'|'||u.unread_events||'|'||
                            t.stat1||'|'||(SELECT count(*) FROM del_info WHERE msgid={COMMENT_ID})
                       FROM comments c JOIN users u ON u.id=c.userid
                       JOIN topics t ON t.id=c.topic WHERE c.id={COMMENT_ID}"""
            ).split("|")
            == [
                "false",
                initial_score,
                str(int(initial_unread) + 1),
                str(int(initial_stat1) - 1),
                "0",
            ],
            "undelete DB delta differs from Java",
        )
        print("PASS comment delete/undelete lifecycle")
        return 0
    finally:
        db(
            "BEGIN; "
            f"DELETE FROM user_events WHERE id>{initial_event_max} AND userid={AUTHOR_ID} "
            f"AND message_id={TOPIC_ID} AND comment_id={COMMENT_ID} AND type='DEL' "
            f"AND message={sql_string(REASON)}; "
            f"DELETE FROM del_info WHERE msgid={COMMENT_ID}; "
            f"UPDATE comments SET deleted={initial_deleted} WHERE id={COMMENT_ID}; "
            f"UPDATE users SET score={initial_score},unread_events={initial_unread} "
            f"WHERE id={AUTHOR_ID}; "
            f"UPDATE topics SET stat1={initial_stat1},stat3={initial_stat3},"
            f"lastmod=to_timestamp({initial_lastmod_epoch}) WHERE id={TOPIC_ID}; "
            "COMMIT"
        )


if __name__ == "__main__":
    raise SystemExit(main())
