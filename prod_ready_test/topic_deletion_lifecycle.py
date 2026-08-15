#!/usr/bin/env python3
"""Idempotent browser/DB regression for Java-compatible topic deletion.

The mutations go through the public legacy forms.  SQL is used only for
before/after assertions and narrowly-scoped restoration of the dedicated
prod_ready fixture.  The test deliberately uses two different moderators so
the score penalty and DEL notification paths are exercised.
"""

from __future__ import annotations

import argparse
import json
from html import unescape

from test_port import Client, db, require


TOPIC_ID = 9101013
AUTHOR_ID = 9100013
MODERATOR_ID = 9100014
GROUP_ID = 4068
TOPIC_URL = "/forum/linux-org-ru/9101013"
REASON = "prod-ready topic lifecycle"
PENALTY = 7


def scalar(sql: str) -> str:
    return db(sql).strip()


def sql_string(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def topic_state() -> list[str]:
    value = scalar(
        f"""SELECT t.deleted::text||'|'||t.sticky::text||'|'||t.lastmod::text||'|'||
                    t.groupid||'|'||u.score||'|'||u.unread_events||'|'||g.stat3
               FROM topics t
               JOIN users u ON u.id=t.userid
               JOIN groups g ON g.id=t.groupid
              WHERE t.id={TOPIC_ID} AND t.userid={AUTHOR_ID}"""
    )
    return value.split("|", 6) if value else []


def topic_memory_snapshot() -> list[dict[str, object]]:
    value = scalar(
        f"""SELECT COALESCE(json_agg(row_to_json(fixture_memory)
                                      ORDER BY fixture_memory.id)::text,'[]')
               FROM (
                 SELECT id,userid,topic,add_date,watch
                   FROM memories
                  WHERE topic={TOPIC_ID}
               ) fixture_memory"""
    )
    parsed = json.loads(value)
    require(isinstance(parsed, list), "topic memory snapshot is not a JSON list")
    return parsed


def restore_memories(snapshot: list[dict[str, object]]) -> str:
    statements = [f"DELETE FROM memories WHERE topic={TOPIC_ID}"]
    for row in snapshot:
        statements.append(
            "INSERT INTO memories(id,userid,topic,add_date,watch) VALUES("
            f"{int(row['id'])},{int(row['userid'])},{int(row['topic'])},"
            f"{sql_string(str(row['add_date']))}::timestamptz,"
            f"{'true' if bool(row['watch']) else 'false'})"
        )
    return "; ".join(statements)


def assert_topins_state(initial_group_stat3: str, initial_memories: list[dict[str, object]]) -> None:
    require(
        scalar(f"SELECT stat3::text FROM groups WHERE id={GROUP_ID}")
        == initial_group_stat3,
        "topic delete/undelete changed the topins_t-owned group counter",
    )
    require(
        topic_memory_snapshot() == initial_memories,
        "topic delete/undelete changed the topins_t-owned memories rows",
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:8181")
    args = parser.parse_args()

    initial_state = topic_state()
    require(len(initial_state) == 7, "topic lifecycle fixture is absent")
    (
        initial_deleted,
        initial_sticky,
        initial_lastmod,
        initial_group_id,
        initial_score,
        initial_unread,
        initial_group_stat3,
    ) = initial_state
    require(initial_deleted == "false", "topic lifecycle fixture is already deleted")
    require(initial_sticky == "true", "topic lifecycle fixture must start sticky")
    require(initial_group_id == str(GROUP_ID), "topic lifecycle fixture moved groups")
    require(
        scalar(f"SELECT count(*) FROM del_info WHERE msgid={TOPIC_ID}") == "0",
        "topic lifecycle fixture already has del_info",
    )
    require(
        scalar(f"SELECT count(*) FROM user_events WHERE message_id={TOPIC_ID}") == "0",
        "topic lifecycle fixture unexpectedly has events",
    )
    initial_memories = topic_memory_snapshot()
    require(
        any(
            int(row["userid"]) == AUTHOR_ID and int(row["topic"]) == TOPIC_ID
            for row in initial_memories
        ),
        "canonical topins_t author memory is absent",
    )
    initial_event_max = int(
        scalar("SELECT COALESCE(max(id),0)::text FROM user_events")
    )

    anonymous = Client(args.base)
    moderator = Client(args.base)
    try:
        # Spring binds required scalar parameters before AuthorizedOnly.
        require(
            anonymous.request("/delete.jsp").status == 400,
            "missing GET msgid does not use Spring's 400 binding response",
        )
        require(
            anonymous.request(f"/delete.jsp?msgid={TOPIC_ID}").status == 403,
            "anonymous user can open the topic delete form",
        )

        moderator.login("eagle_moderator")
        delete_form = moderator.request(f"/delete.jsp?msgid={TOPIC_ID}")
        require(delete_form.status == 200, f"delete form returned {delete_form.status}")
        delete_html = delete_form.text
        for fragment in (
            "<h1>Удаление сообщения</h1>",
            '<form method=POST action="delete.jsp" class="form-horizontal">',
            'name="csrf"',
            'name=reason_select',
            'id="reason-input" type=text name=reason',
            'id="bonus-input" type=number name=bonus value="7" min="0" max="20"',
            f"score автора: {initial_score}",
            f"name=msgid value=\"{TOPIC_ID}\"",
            'class="btn btn-danger">Удалить</button>',
        ):
            require(fragment in delete_html, f"delete form lacks {fragment!r}")

        missing_reason = moderator.request(
            "/delete.jsp",
            "POST",
            [("msgid", str(TOPIC_ID)), ("bonus", str(PENALTY))],
        )
        require(
            missing_reason.status == 400,
            "missing required reason does not use Spring's 400 binding response",
        )
        malformed_bonus = moderator.request(
            "/delete.jsp",
            "POST",
            [("msgid", str(TOPIC_ID)), ("reason", REASON), ("bonus", "x")],
        )
        require(
            malformed_bonus.status == 400,
            "malformed bonus does not use Spring's 400 binding response",
        )
        invalid_range = moderator.request(
            "/delete.jsp",
            "POST",
            [("msgid", str(TOPIC_ID)), ("reason", REASON), ("bonus", "-1")],
        )
        require(
            invalid_range.status == 500,
            "out-of-range bonus does not use the Java BadParameterException response",
        )
        require(
            "Неправильный формат параметра ``неправильный размер штрафа''"
            in unescape(invalid_range.text),
            "out-of-range bonus hides or changes the Java exception message",
        )
        require(
            "Скрипту, генерирующему страничку"
            in invalid_range.text,
            "BadParameterException lost the Java SCRIPT_ERROR explanation",
        )
        require(topic_state() == initial_state, "invalid requests mutated the topic")
        require(
            scalar(f"SELECT count(*) FROM del_info WHERE msgid={TOPIC_ID}") == "0",
            "invalid requests inserted del_info",
        )

        deleted = moderator.request(
            "/delete.jsp",
            "POST",
            [
                ("msgid", str(TOPIC_ID)),
                ("reason", REASON),
                ("bonus", str(PENALTY)),
            ],
        )
        require(deleted.status == 200, f"delete mutation returned {deleted.status}")
        require("Сообщение удалено" in deleted.text, "delete success text is absent")
        require(">Продолжить</a>" not in deleted.text, "delete added a link")

        deleted_state = topic_state()
        require(len(deleted_state) == 7, "deleted topic state is absent")
        require(deleted_state[0] == "true", "topic was not soft-deleted")
        require(deleted_state[1] == "false", "topic sticky flag was not cleared")
        require(
            float(scalar(f"SELECT extract(epoch FROM lastmod) FROM topics WHERE id={TOPIC_ID}"))
            > float(
                scalar(
                    f"SELECT extract(epoch FROM {sql_string(initial_lastmod)}::timestamptz)"
                )
            ),
            "msgdel_t did not update topic lastmod",
        )
        require(deleted_state[4] == str(int(initial_score) - PENALTY), "score penalty differs")
        require(
            deleted_state[5] == str(int(initial_unread) + 1),
            "new_event_t did not increment unread_events",
        )
        require(
            scalar(
                f"""SELECT delby::text||'|'||reason||'|'||bonus
                       FROM del_info WHERE msgid={TOPIC_ID}"""
            )
            == f"{MODERATOR_ID}|{REASON}|-{PENALTY}",
            "plain del_info insert differs from Java",
        )
        event_ids = [
            int(value)
            for value in scalar(
                f"""SELECT COALESCE(string_agg(id::text,',' ORDER BY id),'')
                       FROM user_events
                      WHERE id>{initial_event_max} AND userid={AUTHOR_ID}
                        AND type='DEL' AND private AND message_id={TOPIC_ID}
                        AND comment_id IS NULL AND message={sql_string(REASON)}"""
            ).split(",")
            if value
        ]
        require(len(event_ids) == 1, "topic DEL notification was not inserted exactly once")
        assert_topins_state(initial_group_stat3, initial_memories)

        already_deleted = moderator.request(
            "/delete.jsp",
            "POST",
            [
                ("msgid", str(TOPIC_ID)),
                ("reason", "must not replace del_info"),
                ("bonus", "20"),
            ],
        )
        require(
            already_deleted.status == 500,
            "already-deleted topic does not use the Java UserErrorException response",
        )
        require(
            "Сообщение уже удалено" in already_deleted.text,
            "already-deleted topic hides the Java UserErrorException message",
        )
        require(topic_state() == deleted_state, "repeat delete changed topic or score state")
        require(
            scalar(
                f"""SELECT delby::text||'|'||reason||'|'||bonus
                       FROM del_info WHERE msgid={TOPIC_ID}"""
            )
            == f"{MODERATOR_ID}|{REASON}|-{PENALTY}",
            "repeat delete replaced the original del_info row",
        )

        undelete_form = moderator.request(f"/undelete?msgid={TOPIC_ID}")
        require(
            undelete_form.status == 200,
            f"undelete form returned {undelete_form.status}",
        )
        undelete_html = undelete_form.text
        for fragment in (
            "<h1>Восстановление сообщения</h1>",
            "Вы можете восстановить удалённое сообщение.",
            f'<article class="msg" id="topic-{TOPIC_ID}">',
            'class="msg-container"',
            'class="msg-text"',
            "Модераторская проверка интерфейса темы",
            '<form method=POST action="undelete">',
            'name="csrf"',
            f"name=msgid value=\"{TOPIC_ID}\"",
            'name=undel class="btn btn-primary">Восстановить</button>',
        ):
            require(fragment in undelete_html, f"undelete form lacks {fragment!r}")
        require('id="topicMenu"' not in undelete_html, "showMenu=false leaked topic menu")
        require('class="reply"' not in undelete_html, "showMenu=false leaked reply controls")

        restored = moderator.request(
            "/undelete", "POST", [("msgid", str(TOPIC_ID)), ("undel", "")]
        )
        require(restored.status == 200, f"undelete mutation returned {restored.status}")
        require("Сообщение восстановлено" in restored.text, "undelete text is absent")
        require(
            f'href="{TOPIC_URL}">Продолжить</a>' in restored.text,
            "undelete canonical Continue link differs",
        )

        restored_state = topic_state()
        require(restored_state[0] == "false", "topic was not restored")
        require(restored_state[1] == "false", "undelete incorrectly restored sticky")
        require(restored_state[4] == initial_score, "undelete did not reverse the penalty")
        require(
            restored_state[5] == str(int(initial_unread) + 1),
            "undelete unexpectedly removed the DEL notification unread count",
        )
        require(
            scalar(f"SELECT count(*) FROM del_info WHERE msgid={TOPIC_ID}") == "0",
            "undelete left del_info behind",
        )
        require(
            scalar(
                f"""SELECT count(*) FROM user_events
                      WHERE id>{initial_event_max} AND userid={AUTHOR_ID}
                        AND type='DEL' AND private AND message_id={TOPIC_ID}
                        AND comment_id IS NULL AND message={sql_string(REASON)}"""
            )
            == "1",
            "undelete unexpectedly removed the DEL notification",
        )
        assert_topins_state(initial_group_stat3, initial_memories)
        print("PASS topic delete/undelete lifecycle")
        return 0
    finally:
        # Scope cleanup to the dedicated fixture and to the exact event this
        # run could have created.  Delete del_info before restoring lastmod,
        # because canonical msgundel_t updates lastmod on that DELETE.
        memory_restore_sql = restore_memories(initial_memories)
        db(
            "BEGIN; "
            f"DELETE FROM user_events WHERE id>{initial_event_max} AND userid={AUTHOR_ID} "
            f"AND message_id={TOPIC_ID} AND comment_id IS NULL AND type='DEL' "
            f"AND message={sql_string(REASON)}; "
            f"DELETE FROM del_info WHERE msgid={TOPIC_ID}; "
            f"UPDATE topics SET deleted={initial_deleted},sticky={initial_sticky},"
            f"lastmod={sql_string(initial_lastmod)}::timestamptz WHERE id={TOPIC_ID}; "
            f"UPDATE users SET score={initial_score},unread_events={initial_unread} "
            f"WHERE id={AUTHOR_ID}; "
            f"UPDATE groups SET stat3={initial_group_stat3} WHERE id={GROUP_ID}; "
            f"{memory_restore_sql}; "
            "COMMIT"
        )
        require(topic_state() == initial_state, "topic lifecycle cleanup did not restore state")
        require(topic_memory_snapshot() == initial_memories, "memory cleanup did not restore state")


if __name__ == "__main__":
    raise SystemExit(main())
