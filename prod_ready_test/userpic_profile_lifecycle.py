#!/usr/bin/env python3
"""Rerun-safe HTTP/DB/filesystem lifecycle for profile userpics and settings.

Every user-visible mutation goes through the public Rust routes.  SQL and
``docker compose exec`` are used only to snapshot, assert, and restore the
dedicated ``bird49`` fixture.  A small checkpoint under ``/tmp`` lets the next
run repair an interrupted previous run before starting a new one.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import html
import io
import json
import os
import re
import secrets
import stat
import subprocess
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

from test_port import Client, MANIFEST, ROOT, Response, db, require, require_html


USER_ID = 9_100_049
NICK = "bird49"
CHECKPOINT = Path("/tmp/lorsource-userpic-profile-lifecycle-state.json")
PHOTO_DIR = "/app/uploads/photos"
NEW_PASSWORD = "Birds-ProdReady-2026-Profile-Lifecycle"
PHOTO_NAME = re.compile(rf"^{USER_ID}:-?\d+\.(?:gif|jpg|png)$")
SAFE_NAME = re.compile(r"^[A-Za-z0-9_.:-]+$")
DEFAULT_BASE = "http://localhost:8181"
ALLOWED_BASES = frozenset(
    {DEFAULT_BASE, "http://127.0.0.1:8181", "http://[::1]:8181"}
)


def local_base(value: str) -> str:
    if value not in ALLOWED_BASES:
        raise argparse.ArgumentTypeError(
            "--base must be exactly http://localhost:8181, "
            "http://127.0.0.1:8181, or http://[::1]:8181"
        )
    return value


def sql_string(value: str | None) -> str:
    if value is None:
        return "NULL"
    return "'" + value.replace("'", "''") + "'"


def query_json(sql: str) -> Any:
    value = db(sql)
    require(bool(value), "database JSON query returned no row")
    return json.loads(value)


def compose(
    *arguments: str,
    input_bytes: bytes | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["docker", "compose", *arguments],
        cwd=ROOT,
        input=input_bytes,
        check=check,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def verify_target() -> None:
    services = compose("ps", "--status", "running", "--services")
    running = set(services.stdout.decode("utf-8").split())
    require({"postgres", "app"} <= running, "Compose postgres/app are not running")
    identity = db(
        "SELECT current_database()||'|'||current_user||'|'||"
        "(to_regclass('public.databasechangelog') IS NOT NULL)::text"
    )
    require(identity == "lor|postgres|true", f"unexpected database target: {identity!r}")


def validate_photo_name(name: str) -> str:
    require(
        name not in {".", ".."} and SAFE_NAME.fullmatch(name) is not None,
        f"unsafe photo filename in fixture state: {name!r}",
    )
    return name


def app_file_exists(name: str) -> bool:
    name = validate_photo_name(name)
    result = compose("exec", "-T", "app", "test", "-f", f"{PHOTO_DIR}/{name}", check=False)
    require(result.returncode in {0, 1}, result.stderr.decode("utf-8", errors="replace"))
    return result.returncode == 0


def list_prefixed_files() -> set[str]:
    result = compose(
        "exec",
        "-T",
        "app",
        "find",
        PHOTO_DIR,
        "-maxdepth",
        "1",
        "-type",
        "f",
        "-name",
        f"{USER_ID}*",
        "-printf",
        "%f\n",
    )
    return {
        validate_photo_name(value)
        for value in result.stdout.decode("utf-8").splitlines()
        if value
    }


def read_app_file(name: str) -> dict[str, str]:
    name = validate_photo_name(name)
    require(app_file_exists(name), f"active fixture userpic is absent: {name}")
    content = compose("exec", "-T", "app", "cat", f"{PHOTO_DIR}/{name}").stdout
    mode = (
        compose("exec", "-T", "app", "stat", "-c", "%a", f"{PHOTO_DIR}/{name}")
        .stdout.decode("ascii")
        .strip()
    )
    mtime = (
        compose("exec", "-T", "app", "stat", "-c", "%y", f"{PHOTO_DIR}/{name}")
        .stdout.decode("utf-8")
        .strip()
    )
    require(re.fullmatch(r"[0-7]{3,4}", mode) is not None, f"invalid mode for {name}: {mode}")
    require(bool(mtime), f"invalid mtime for {name}")
    return {
        "base64": base64.b64encode(content).decode("ascii"),
        "sha256": hashlib.sha256(content).hexdigest(),
        "mode": mode,
        "mtime": mtime,
    }


def write_app_file(name: str, snapshot: dict[str, str]) -> None:
    name = validate_photo_name(name)
    content = base64.b64decode(snapshot["base64"], validate=True)
    require(
        hashlib.sha256(content).hexdigest() == snapshot["sha256"],
        f"checkpoint checksum mismatch for {name}",
    )
    compose(
        "exec",
        "-T",
        "app",
        "dd",
        f"of={PHOTO_DIR}/{name}",
        "status=none",
        input_bytes=content,
    )
    compose("exec", "-T", "app", "chmod", snapshot["mode"], f"{PHOTO_DIR}/{name}")
    compose(
        "exec",
        "-T",
        "app",
        "touch",
        "-d",
        snapshot["mtime"],
        f"{PHOTO_DIR}/{name}",
    )


def remove_app_file(name: str) -> None:
    name = validate_photo_name(name)
    compose("exec", "-T", "app", "rm", "-f", f"{PHOTO_DIR}/{name}")


def snapshot_user() -> dict[str, Any]:
    return query_json(
        f"""SELECT row_to_json(s)::text FROM (
          SELECT id,nick,name,passwd,url,email,new_email,photo,town,score,max_score,
                 unread_events,token_generation,userinfo,userinfo_markup::text AS userinfo_markup,
                 lostpwd::text AS lostpwd,lastlogin::text AS lastlogin,
                 regdate::text AS regdate,activated,blocked,canmod,candel,corrector,
                 frozen_until::text AS frozen_until
            FROM users WHERE id={USER_ID}
        ) s"""
    )


def snapshot_settings() -> dict[str, Any]:
    exists = db(f"SELECT count(*) FROM user_settings WHERE id={USER_ID}") == "1"
    if not exists:
        return {"exists": False, "text": None, "values": {}}
    return {
        "exists": True,
        "text": db(f"SELECT settings::text FROM user_settings WHERE id={USER_ID}"),
        "values": query_json(
            f"SELECT hstore_to_json(settings)::text FROM user_settings WHERE id={USER_ID}"
        ),
    }


def snapshot_logs() -> list[dict[str, Any]]:
    return query_json(
        f"""SELECT COALESCE(json_agg(row_to_json(s) ORDER BY s.id),'[]'::json)::text
              FROM (
                SELECT id,userid,action_userid,action_date::text AS action_date,
                       action::text AS action,info::text AS info_text,
                       hstore_to_json(info) AS info
                  FROM user_log
                 WHERE userid={USER_ID} OR action_userid={USER_ID}
              ) s"""
    )


def snapshot_state() -> dict[str, Any]:
    user = snapshot_user()
    require(user["id"] == USER_ID and user["nick"] == NICK, "bird49 fixture is absent")
    require(user["activated"] and not user["blocked"], "bird49 must be active and unblocked")
    require(not user["canmod"] and not user["candel"], "bird49 must remain an ordinary user")
    require(user["score"] >= 45 and user["passwd"], "bird49 cannot upload a userpic")
    require(user["photo"] is not None, "bird49 must begin with a local userpic")
    settings = snapshot_settings()
    require(settings["exists"], "bird49 settings fixture is absent")
    recent_uploads = int(
        db(
            f"""SELECT count(*) FROM user_log
                  WHERE userid={USER_ID} AND action='set_userpic'::user_log_action
                    AND action_date>CURRENT_TIMESTAMP-interval '1 hour'"""
        )
    )
    require(recent_uploads <= 1, "bird49 has too many recent uploads for a two-upload lifecycle")

    names = list_prefixed_files()
    names.add(validate_photo_name(user["photo"]))
    files = {name: read_app_file(name) for name in sorted(names)}
    return {
        "version": 2,
        "user_id": USER_ID,
        "nick": NICK,
        "user": user,
        "settings": settings,
        "logs": snapshot_logs(),
        "files": files,
        "generated_files": [],
    }


def save_checkpoint(state: dict[str, Any]) -> None:
    temporary = CHECKPOINT.with_suffix(".tmp")
    before = None
    try:
        before = os.lstat(temporary)
        require(stat.S_ISREG(before.st_mode), "checkpoint temp path is not a regular file")
    except FileNotFoundError:
        pass
    flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC | getattr(os, "O_NOFOLLOW", 0)
    if before is None:
        flags |= os.O_EXCL
    descriptor = os.open(temporary, flags, 0o600)
    try:
        opened = os.fstat(descriptor)
        require(stat.S_ISREG(opened.st_mode), "checkpoint temp descriptor is not regular")
        if before is not None:
            require(
                (before.st_dev, before.st_ino) == (opened.st_dev, opened.st_ino),
                "checkpoint temp path changed while opening",
            )
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as checkpoint:
            descriptor = -1
            json.dump(state, checkpoint, ensure_ascii=False, sort_keys=True)
            checkpoint.flush()
            os.fsync(checkpoint.fileno())
    finally:
        if descriptor >= 0:
            os.close(descriptor)
    temporary.replace(CHECKPOINT)


def read_checkpoint() -> dict[str, Any]:
    before = os.lstat(CHECKPOINT)
    require(stat.S_ISREG(before.st_mode), "checkpoint path is not a regular file")
    descriptor = os.open(CHECKPOINT, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    try:
        opened = os.fstat(descriptor)
        require(stat.S_ISREG(opened.st_mode), "checkpoint descriptor is not regular")
        require(
            (before.st_dev, before.st_ino) == (opened.st_dev, opened.st_ino),
            "checkpoint path changed while opening",
        )
        with os.fdopen(descriptor, "r", encoding="utf-8") as checkpoint:
            descriptor = -1
            state = json.load(checkpoint)
    finally:
        if descriptor >= 0:
            os.close(descriptor)
    require(isinstance(state, dict), "checkpoint root is not an object")
    return state


def restore_user_sql(user: dict[str, Any]) -> str:
    return f"""UPDATE users SET
      name={sql_string(user['name'])},passwd={sql_string(user['passwd'])},
      url={sql_string(user['url'])},email={sql_string(user['email'])},
      new_email={sql_string(user['new_email'])},photo={sql_string(user['photo'])},
      town={sql_string(user['town'])},score={int(user['score'])},
      max_score={int(user['max_score'])},unread_events={int(user['unread_events'])},
      token_generation={int(user['token_generation'])},userinfo={sql_string(user['userinfo'])},
      userinfo_markup={sql_string(user['userinfo_markup'])}::markup_type,
      lostpwd={sql_string(user['lostpwd'])}::timestamptz,
      lastlogin={sql_string(user['lastlogin'])}::timestamptz
    WHERE id={USER_ID};"""


def restore_settings_sql(settings: dict[str, Any]) -> str:
    statement = f"DELETE FROM user_settings WHERE id={USER_ID};"
    if settings["exists"]:
        statement += (
            f"INSERT INTO user_settings(id,settings) VALUES"
            f"({USER_ID},{sql_string(settings['text'])}::hstore);"
        )
    return statement


def restore_logs_sql(logs: list[dict[str, Any]]) -> str:
    statement = (
        f"DELETE FROM user_log WHERE userid={USER_ID} OR action_userid={USER_ID};"
    )
    for row in logs:
        statement += f"""INSERT INTO user_log(
          id,userid,action_userid,action_date,action,info
        ) VALUES(
          {int(row['id'])},{int(row['userid'])},{int(row['action_userid'])},
          {sql_string(row['action_date'])}::timestamptz,
          {sql_string(row['action'])}::user_log_action,
          {sql_string(row['info_text'])}::hstore
        );"""
    return statement


def verify_restored(state: dict[str, Any]) -> None:
    require(snapshot_user() == state["user"], "user row was not restored exactly")
    require(snapshot_settings() == state["settings"], "user settings were not restored exactly")
    require(snapshot_logs() == state["logs"], "user_log rows were not restored exactly")
    current_names = list_prefixed_files()
    expected_prefixed = {
        name for name in state["files"] if name.startswith(str(USER_ID))
    }
    require(
        current_names == expected_prefixed,
        f"stale or missing fixture userpic files: {sorted(current_names ^ expected_prefixed)}",
    )
    for name, file_snapshot in state["files"].items():
        require(read_app_file(name) == file_snapshot, f"file was not restored exactly: {name}")


def validate_log_collisions(state: dict[str, Any]) -> None:
    initial = {int(row["id"]): row for row in state["logs"]}
    current = {int(row["id"]): row for row in current_logs()}
    for row_id, row in initial.items():
        require(current.get(row_id) == row, f"pre-existing user_log row {row_id} changed")
    allowed_actions = {"set_userpic", "reset_userpic", "set_password", "set_info"}
    for row_id, row in current.items():
        if row_id in initial:
            continue
        require(
            row["userid"] == USER_ID
            and row["action_userid"] == USER_ID
            and row["action"] in allowed_actions,
            f"unrelated user_log collision during lifecycle: {row!r}",
        )
        for key in ("old_userpic", "new_userpic"):
            if value := row["info"].get(key):
                require(
                    value in state["files"] or PHOTO_NAME.fullmatch(value) is not None,
                    f"unexpected {key} in lifecycle audit row {row_id}: {value!r}",
                )


def restore_state(state: dict[str, Any], *, reseeded: bool = False) -> None:
    require(state.get("version") == 2, "unsupported lifecycle checkpoint")
    require(state.get("user_id") == USER_ID and state.get("nick") == NICK, "wrong checkpoint fixture")
    snapshot_names = set(state["files"])
    current_prefixed = list_prefixed_files()
    extra = current_prefixed - {name for name in snapshot_names if name.startswith(str(USER_ID))}
    for name in extra:
        require(PHOTO_NAME.fullmatch(name) is not None, f"refusing to delete colliding file {name!r}")

    if reseeded:
        # SQL seeding replaces the user row but intentionally keeps the upload
        # volume.  Only remove lifecycle's colon-named residue in this branch;
        # restoring the old timestamped SQL snapshot would undo the fresh seed.
        for name in sorted(extra):
            remove_app_file(name)
        CHECKPOINT.unlink(missing_ok=True)
        return

    validate_log_collisions(state)
    # Make the original media bytes available before pointing users.photo back
    # at them.  The exact SQL snapshot is restored in one transaction.
    for name, file_snapshot in state["files"].items():
        write_app_file(name, file_snapshot)
    db(
        "BEGIN; SET LOCAL lock_timeout='10s'; SET LOCAL statement_timeout='60s';"
        + restore_user_sql(state["user"])
        + restore_settings_sql(state["settings"])
        + restore_logs_sql(state["logs"])
        + "COMMIT"
    )
    for name in sorted(extra):
        remove_app_file(name)
    verify_restored(state)
    CHECKPOINT.unlink(missing_ok=True)


def recover_checkpoint() -> None:
    temporary = CHECKPOINT.with_suffix(".tmp")
    if not os.path.lexists(CHECKPOINT):
        temporary.unlink(missing_ok=True)
        return
    state = read_checkpoint()
    current = snapshot_user()
    reseeded = current["regdate"] != state["user"]["regdate"]
    print(
        "Recovering interrupted userpic lifecycle checkpoint"
        + (" after a fixture reseed" if reseeded else "")
    )
    restore_state(state, reseeded=reseeded)
    temporary.unlink(missing_ok=True)


def cookie_value(client: Client, name: str) -> str | None:
    return next((cookie.value for cookie in client.cookies if cookie.name == name), None)


def raw_request(
    client: Client,
    path: str,
    method: str,
    body: bytes,
    headers: dict[str, str],
) -> Response:
    request = urllib.request.Request(
        urllib.parse.urljoin(client.base, path.lstrip("/")),
        data=body,
        method=method,
        headers={"User-Agent": "lorsource-userpic-profile-lifecycle/1", **headers},
    )
    try:
        with client.opener.open(request, timeout=20) as response:
            return Response(response.status, response.headers, response.read(4_000_000))
    except urllib.error.HTTPError as error:
        return Response(error.code, error.headers, error.read(4_000_000))


def upload(client: Client, filename: str, content_type: str, payload: bytes) -> Response:
    token = cookie_value(client, "CSRF_TOKEN")
    require(token is not None, "CSRF cookie is absent before multipart upload")
    boundary = "lor-userpic-" + secrets.token_hex(12)
    body = bytearray()

    def field(name: str, value: bytes, *, file_name: str | None = None, mime: str | None = None) -> None:
        body.extend(f"--{boundary}\r\n".encode("ascii"))
        disposition = f'Content-Disposition: form-data; name="{name}"'
        if file_name is not None:
            disposition += f'; filename="{file_name}"'
        body.extend((disposition + "\r\n").encode("utf-8"))
        if mime is not None:
            body.extend(f"Content-Type: {mime}\r\n".encode("ascii"))
        body.extend(b"\r\n")
        body.extend(value)
        body.extend(b"\r\n")

    field("csrf", token.encode("ascii"))
    field("file", payload, file_name=filename, mime=content_type)
    body.extend(f"--{boundary}--\r\n".encode("ascii"))
    return raw_request(
        client,
        "/addphoto.jsp",
        "POST",
        bytes(body),
        {"Content-Type": f"multipart/form-data; boundary={boundary}"},
    )


def image_bytes(image_format: str, size: tuple[int, int], *, animated: bool = False) -> bytes:
    try:
        from PIL import Image
    except ImportError as error:
        raise RuntimeError("Pillow is required for the userpic lifecycle") from error

    first = Image.new("RGB", size, (35, 90, 155))
    output = io.BytesIO()
    if animated:
        second = Image.new("RGB", size, (175, 65, 45))
        first.save(
            output,
            format=image_format,
            save_all=True,
            append_images=[second],
            duration=80,
            loop=0,
        )
    else:
        arguments = {"quality": 88} if image_format in {"JPEG", "WEBP"} else {}
        first.save(output, format=image_format, **arguments)
    return output.getvalue()


def current_photo() -> str | None:
    value = db(f"SELECT COALESCE(photo,'') FROM users WHERE id={USER_ID}")
    return value or None


def current_logs() -> list[dict[str, Any]]:
    return snapshot_logs()


def added_logs(state: dict[str, Any]) -> list[dict[str, Any]]:
    initial = {int(row["id"]) for row in state["logs"]}
    return [row for row in current_logs() if int(row["id"]) not in initial]


def remember_generated(state: dict[str, Any], name: str) -> None:
    require(PHOTO_NAME.fullmatch(name) is not None, f"upload filename differs from Java: {name}")
    if name not in state["generated_files"]:
        state["generated_files"].append(name)
        save_checkpoint(state)


def require_nocache_redirect(response: Response, context: str) -> None:
    require(response.status == 302, f"{context}: expected 302, got {response.status}")
    location = response.headers.get("Location", "")
    require(
        re.fullmatch(rf"/people/{NICK}/profile\?nocache=-?\d+", location) is not None,
        f"{context}: wrong redirect {location!r}",
    )


def require_addphoto_form(
    response: Response, context: str, *, expected_status: int = 200
) -> str:
    require(
        response.status == expected_status,
        f"{context}: expected {expected_status}, got {response.status}",
    )
    require(
        response.headers.get("Content-Type", "").startswith("text/html"),
        f"{context}: response is not HTML",
    )
    page = response.text
    require('<main id="bd">' in page, f"{context}: #bd is absent")
    for fragment in (
        'action="addphoto.jsp"',
        'method="POST"',
        'enctype="multipart/form-data"',
        'name="file"',
        'data-style="tango-auto"',
    ):
        require(fragment in page, f"{context}: missing {fragment!r}")
    return page


def settings_values(snapshot: dict[str, Any], avatar: str) -> list[tuple[str, str]]:
    current = snapshot["settings"]["values"]
    values = [
        ("style", current["style"]),
        ("format_mode", current["format.mode"]),
        ("topics", current["topics"]),
        ("messages", current["messages"]),
        ("avatar", avatar),
        ("trackerMode", current["trackerMode"]),
    ]
    # ``photos`` is deliberately omitted to persist false.  Preserve every
    # other checkbox exactly as it was in the fixture snapshot.
    for key in (
        "hideAdsense",
        "mainGallery",
        "oldTracker",
        "oldNotifications",
        "reactionNotification",
    ):
        if current.get(key) == "true":
            values.append((key, "on"))
    return values


def profile_form_values(snapshot: dict[str, Any]) -> list[tuple[str, str]]:
    user = snapshot["user"]
    markup = {
        "MARKDOWN": "markdown",
        "BBCODE_TEX": "lorcode",
        "BBCODE_ULB": "ntobr",
    }.get(user["userinfo_markup"], "lorcode")
    return [
        ("name", user["name"] or ""),
        ("password", NEW_PASSWORD),
        ("password2", NEW_PASSWORD),
        ("url", user["url"] or ""),
        ("email", user["email"] or ""),
        ("town", user["town"] or ""),
        ("info", user["userinfo"] or ""),
        ("infoMarkup", markup),
        ("oldpass", MANIFEST["password"]),
    ]


def login_with_password(client: Client, password: str) -> Response:
    return client.request(
        "/login_process",
        "POST",
        [("nick", NICK), ("passwd", password), ("redirectUrl", f"/people/{NICK}/profile")],
    )


def run_lifecycle(base: str, state: dict[str, Any]) -> None:
    client = Client(base)
    client.login(NICK)

    addphoto = require_addphoto_form(client.request("/addphoto.jsp"), "themed addphoto form")
    require("Технические требования к изображению" in addphoto, "addphoto requirements are absent")

    baseline_photo = current_photo()
    baseline_logs = current_logs()
    baseline_files = list_prefixed_files()

    empty = upload(client, "empty.png", "image/png", b"")
    require(empty.status == 200, f"empty upload: expected Java 200, got {empty.status}")
    require_addphoto_form(empty, "empty upload response")
    require("Ошибка! изображение не задано" in empty.text, "empty upload error differs")

    webp = upload(client, "unsupported.webp", "image/webp", image_bytes("WEBP", (80, 80)))
    require(webp.status == 400, f"WebP upload: expected 400, got {webp.status}")
    require("Does unsupported format WebP" in webp.text, "WebP error differs")
    require_addphoto_form(webp, "WebP rejection", expected_status=400)

    animated = upload(
        client,
        "animated.gif",
        "image/gif",
        image_bytes("GIF", (80, 80), animated=True),
    )
    require(animated.status == 400, f"animated GIF: expected 400, got {animated.status}")
    require("анимация не допустима" in animated.text, "animated GIF error differs")
    require_addphoto_form(animated, "animated GIF rejection", expected_status=400)
    require(current_photo() == baseline_photo, "rejected upload changed users.photo")
    require(current_logs() == baseline_logs, "rejected upload wrote user_log")
    require(list_prefixed_files() == baseline_files, "rejected upload left a file")

    static_gif_bytes = image_bytes("GIF", (80, 120))
    static_gif = upload(client, "static.gif", "image/gif", static_gif_bytes)
    require_nocache_redirect(static_gif, "static GIF upload")
    gif_name = current_photo()
    require(gif_name is not None and gif_name.endswith(".gif"), "static GIF was not persisted")
    remember_generated(state, gif_name)
    logs = added_logs(state)
    require(len(logs) == 1 and logs[0]["action"] == "set_userpic", "GIF audit count/action differs")
    require(
        logs[0]["info"] == {"old_userpic": baseline_photo, "new_userpic": gif_name},
        f"GIF old/new audit differs: {logs[0]['info']!r}",
    )
    media = client.request(f"/photos/{gif_name}")
    require(media.status == 200 and media.body == static_gif_bytes, "GIF public media differs")
    require(media.headers.get("Content-Type", "").startswith("image/gif"), "GIF media type differs")
    profile = require_html(client.request(f"/people/{NICK}/profile"), "GIF profile")
    require(
        f'<div class="userpic"><img class="photo" src="/photos/{gif_name}" alt="" width=80 height=120 ></div>'
        in profile,
        "small portrait GIF profile dimensions/DOM differ from Java",
    )

    png_bytes = image_bytes("PNG", (240, 120))
    png = upload(client, "landscape.png", "image/png", png_bytes)
    require_nocache_redirect(png, "PNG upload")
    png_name = current_photo()
    require(png_name is not None and png_name.endswith(".png"), "PNG was not persisted")
    remember_generated(state, png_name)
    logs = added_logs(state)
    require(
        len(logs) == 2 and [row["action"] for row in logs] == ["set_userpic", "set_userpic"],
        "PNG audit count/action differs",
    )
    require(
        logs[1]["info"] == {"old_userpic": gif_name, "new_userpic": png_name},
        f"PNG old/new audit differs: {logs[1]['info']!r}",
    )
    media = client.request(f"/photos/{png_name}")
    require(media.status == 200 and media.body == png_bytes, "PNG public media differs")
    require(media.headers.get("Content-Type", "").startswith("image/png"), "PNG media type differs")
    profile = require_html(client.request(f"/people/{NICK}/profile"), "PNG profile")
    require(
        f'<div class="userpic"><img class="photo" src="/photos/{png_name}" alt="" width=150 height=75 ></div>'
        in profile,
        "landscape PNG profile dimensions/DOM differ from Java",
    )

    removed = client.request(
        "/remove-userpic.jsp", "POST", [("id", str(USER_ID))]
    )
    require_nocache_redirect(removed, "remove userpic")
    require(current_photo() is None, "remove-userpic did not clear users.photo")
    logs = added_logs(state)
    require(
        len(logs) == 3 and logs[2]["action"] == "reset_userpic",
        "remove-userpic audit count/action differs",
    )
    require(
        logs[2]["info"] == {"old_userpic": png_name},
        f"remove-userpic audit differs: {logs[2]['info']!r}",
    )
    require(app_file_exists(png_name), "remove-userpic eagerly deleted historical media")

    avatar = "monsterid" if state["settings"]["values"].get("avatar") != "monsterid" else "retro"
    saved = client.request(
        f"/people/{NICK}/settings",
        "POST",
        settings_values(state, avatar),
    )
    require(saved.status == 302, f"settings save: expected 302, got {saved.status}")
    require(
        saved.headers.get("Location") == f"/people/{NICK}/profile",
        f"settings redirect differs: {saved.headers.get('Location')!r}",
    )
    persisted = query_json(
        f"SELECT hstore_to_json(settings)::text FROM user_settings WHERE id={USER_ID}"
    )
    require(persisted["photos"] == "false", "photos=false was not persisted")
    require(persisted["avatar"] == avatar, "avatar fallback was not persisted")
    settings_page = require_html(client.request(f"/people/{NICK}/settings"), "saved settings")
    photos_tag = re.search(r'<input[^>]*id="photos"[^>]*>', settings_page)
    require(photos_tag is not None and "checked" not in photos_tag.group(0), "photos form state differs")
    avatar_tag = re.search(
        rf'<input[^>]*name="avatar"[^>]*value="{re.escape(avatar)}"[^>]*>', settings_page
    )
    require(avatar_tag is not None and "checked" in avatar_tag.group(0), "avatar form state differs")
    fallback_profile = require_html(client.request(f"/people/{NICK}/profile"), "fallback profile")
    require(f"d={avatar}" in html.unescape(fallback_profile), "saved avatar fallback is not rendered")

    edit = require_html(client.request(f"/people/{NICK}/edit"), "profile edit form")
    require('id="editRegForm"' in edit and 'name="oldpass"' in edit, "profile edit form differs")
    previous_cookie = cookie_value(client, "remember_me")
    changed = client.request(
        f"/people/{NICK}/edit",
        "POST",
        profile_form_values(state),
    )
    require(changed.status == 302, f"password change: expected 302, got {changed.status}")
    require(
        changed.headers.get("Location") == f"/people/{NICK}/profile",
        f"password redirect differs: {changed.headers.get('Location')!r}",
    )
    refreshed_cookie = cookie_value(client, "remember_me")
    require(
        previous_cookie and refreshed_cookie and refreshed_cookie != previous_cookie,
        "password change did not refresh remember_me",
    )
    require(
        snapshot_user()["passwd"] != state["user"]["passwd"],
        "password hash did not change",
    )
    logs = added_logs(state)
    require(
        len(logs) == 4 and logs[3]["action"] == "set_password",
        f"password audit differs: {[(row['id'], row['action']) for row in logs]!r}",
    )
    authenticated = require_html(
        client.request(f"/people/{NICK}/profile"), "profile with refreshed remember_me"
    )
    require('action="logout"' in authenticated, "refreshed cookie does not authenticate")
    fresh_client = Client(base)
    fresh_login = login_with_password(fresh_client, NEW_PASSWORD)
    require(fresh_login.status == 302, "new password cannot create a fresh session")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default=DEFAULT_BASE, type=local_base)
    args = parser.parse_args()

    verify_target()
    recover_checkpoint()
    state = snapshot_state()
    save_checkpoint(state)
    try:
        run_lifecycle(args.base, state)
    finally:
        restore_state(state)
    print("PASS userpic/profile/settings/password lifecycle and exact cleanup")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
