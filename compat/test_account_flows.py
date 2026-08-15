#!/usr/bin/env python3
"""Guarded stateful regression for self-service account deregistration."""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys

from stateful_database import psql_target
from test_http_compat import HttpClient
from test_write_flows import login, post, require, text


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


def main() -> int:
    if os.environ.get("ACCOUNT_FLOW_ALLOW_MUTATION") != "yes":
        print("ACCOUNT_FLOW_ALLOW_MUTATION=yes is required", file=sys.stderr)
        return 2
    verify_database_target()

    base = os.environ.get("NEW_BASE_URL", "http://127.0.0.1:8181")
    nick = os.environ["ACCOUNT_FLOW_DEREGISTER_NICK"]
    password = os.environ["ACCOUNT_FLOW_DEREGISTER_PASSWORD"]
    require(
        re.fullmatch(r"[a-z][a-z0-9_-]{0,31}", nick) is not None,
        "ACCOUNT_FLOW_DEREGISTER_NICK has an unsafe shape",
    )
    user_id = int(db(f"SELECT id FROM users WHERE nick='{nick}'"))
    require(
        db(f"SELECT NOT blocked FROM users WHERE id={user_id}") == "t",
        "deregistration fixture is already blocked",
    )

    client = login(base, nick, password)
    remember_me = client.cookie("remember_me")
    form = client.request("/deregister.jsp", "GET")
    form_html = text(form)
    require(form.status == 200, f"deregister form returned {form.status}")
    for fragment in (
        'id="registerForm"',
        'name="password"',
        'name="acceptBlock"',
        'name="acceptOneway"',
        'class="h-captcha"',
    ):
        require(fragment in form_html, f"deregister form is missing {fragment}")

    rejected = post(client, "/deregister.jsp", [("password", "wrong password")])
    rejected_html = text(rejected)
    require(rejected.status == 200, f"invalid deregistration returned {rejected.status}")
    for message in (
        "Вы не согласились с блокировкой аккаунта",
        "Вы не согласились с невозможностью восстановления аккаунта",
        "Неверный пароль",
        "Код проверки защиты от роботов не указан",
    ):
        require(message in rejected_html, f"deregister form lost validation error: {message}")
    require(
        db(f"SELECT NOT blocked FROM users WHERE id={user_id}") == "t",
        "failed deregistration changed the account",
    )
    require(
        db(f"SELECT count(*) FROM ban_info WHERE userid={user_id}") == "0",
        "failed deregistration created ban_info",
    )

    password_hash_before = db(f"SELECT passwd FROM users WHERE id={user_id}")
    last_login_before = db(
        f"SELECT extract(epoch FROM lastlogin)::text FROM users WHERE id={user_id}"
    )
    validation_only = post(
        client,
        "/deregister.jsp",
        [("password", password), ("acceptBlock", "true"), ("acceptOneway", "true")],
    )
    require(validation_only.status == 200, "captcha-only rejection did not re-render form")
    require(
        "Код проверки защиты от роботов не указан" in text(validation_only),
        "captcha-only rejection lost its validation error",
    )
    require(
        db(f"SELECT passwd FROM users WHERE id={user_id}") == password_hash_before,
        "password validation unexpectedly rewrote the stored hash",
    )
    require(
        db(f"SELECT extract(epoch FROM lastlogin)::text FROM users WHERE id={user_id}")
        == last_login_before,
        "password validation unexpectedly changed lastlogin",
    )

    completed = post(
        client,
        "/deregister.jsp",
        [
            ("password", password),
            ("acceptBlock", "true"),
            ("acceptOneway", "true"),
            ("h-captcha-response", "dev-captcha"),
        ],
    )
    require(completed.status == 200, f"deregistration returned {completed.status}")
    require(
        "Удаление пользователя прошло успешно." in text(completed),
        "deregistration confirmation is missing",
    )
    require(
        client.cookie("remember_me") == remember_me,
        "deregistration unexpectedly changed the stateless Java remember-me cookie",
    )

    user_state = json.loads(
        db(
            "SELECT json_build_object("
            "'blocked',blocked,'name',name,'url',url,'town',town,"
            "'userinfo',userinfo,'markup',userinfo_markup::text,'photo',photo)::text "
            f"FROM users WHERE id={user_id}"
        )
    )
    require(
        user_state
        == {
            "blocked": True,
            "name": "",
            "url": "",
            "town": "",
            "userinfo": "",
            "markup": "MARKDOWN",
            "photo": None,
        },
        f"unexpected deregistered profile state: {user_state!r}",
    )
    reason = "самостоятельная блокировка аккаунта"
    require(
        db(
            "SELECT reason || '|' || (ban_by=userid)::text FROM ban_info "
            f"WHERE userid={user_id}"
        )
        == f"{reason}|true",
        "ban_info does not match Java self-block semantics",
    )
    require(
        db(
            "SELECT action::text || '|' || (action_userid=userid)::text || '|' || "
            "COALESCE(info->'reason','') FROM user_log "
            f"WHERE userid={user_id} ORDER BY id DESC LIMIT 1"
        )
        == f"block_user|true|{reason}",
        "user_log does not match Java self-block audit semantics",
    )

    blocked_client = HttpClient(base)
    login_attempt = post(
        blocked_client,
        "/login_process",
        [("nick", nick), ("passwd", password), ("redirectUrl", "/forum/")],
    )
    require(login_attempt.status == 302, "blocked-account login did not return Java redirect")
    require(
        login_attempt.location_target == f"/people/{nick}/profile",
        f"blocked-account login redirected to {login_attempt.location_target!r}",
    )
    require(
        blocked_client.cookie("remember_me") is None,
        "blocked-account login unexpectedly created a remember-me cookie",
    )

    print("Stateful account deregistration flow passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
