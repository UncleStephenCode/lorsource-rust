#!/usr/bin/env python3
"""State/HTTP/DOM regression suite for the prod_ready_test fixture."""

from __future__ import annotations

import argparse
import html
import http.cookiejar
import json
import re
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MANIFEST = json.loads(Path(__file__).with_name("manifest.json").read_text("utf-8"))


@dataclass
class Response:
    status: int
    headers: object
    body: bytes

    @property
    def text(self) -> str:
        return self.body.decode("utf-8", errors="replace")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        return None


class Client:
    def __init__(self, base: str):
        self.base = base.rstrip("/") + "/"
        self.cookies = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.cookies), NoRedirect()
        )

    def request(
        self,
        path: str,
        method: str = "GET",
        values: list[tuple[str, str]] | None = None,
    ) -> Response:
        payload = None
        headers = {"User-Agent": "lorsource-prod-ready-test/1"}
        if values is not None:
            if method.upper() == "POST" and not any(key == "csrf" for key, _ in values):
                token = next(
                    (cookie.value.strip('"') for cookie in self.cookies if cookie.name == "CSRF_TOKEN"),
                    None,
                )
                if token is None:
                    bootstrap = self.request("/")
                    require(bootstrap.status == 200, "CSRF bootstrap failed")
                    token = next(
                        (cookie.value.strip('"') for cookie in self.cookies if cookie.name == "CSRF_TOKEN"),
                        None,
                    )
                require(token is not None, "CSRF cookie is absent")
                values = [*values, ("csrf", token)]
            payload = urllib.parse.urlencode(values).encode()
            headers["Content-Type"] = "application/x-www-form-urlencoded"
        request = urllib.request.Request(
            urllib.parse.urljoin(self.base, path.lstrip("/")),
            data=payload,
            method=method,
            headers=headers,
        )
        try:
            with self.opener.open(request, timeout=20) as response:
                return Response(response.status, response.headers, response.read(4_000_000))
        except urllib.error.HTTPError as error:
            return Response(error.code, error.headers, error.read(4_000_000))

    def login(self, nick: str) -> None:
        response = self.request(
            "/login_process",
            "POST",
            [
                ("nick", nick),
                ("passwd", MANIFEST["password"]),
                ("redirectUrl", "/forum/"),
            ],
        )
        require(response.status == 302, f"login {nick}: expected 302, got {response.status}")
        require(
            any(cookie.name == "remember_me" for cookie in self.cookies),
            f"login {nick}: remember_me cookie is absent",
        )


def db(sql: str) -> str:
    command = [
        "docker",
        "compose",
        "exec",
        "-T",
        "postgres",
        "psql",
        "-X",
        "-U",
        "postgres",
        "-d",
        "lor",
        "-At",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        sql,
    ]
    result = subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return result.stdout.strip()


TESTS: list[tuple[str, Callable[[], None]]] = []


def test(name: str):
    def register(function):
        TESTS.append((name, function))
        return function

    return register


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def require_html(response: Response, context: str) -> str:
    require(response.status == 200, f"{context}: expected 200, got {response.status}")
    content_type = response.headers.get("Content-Type", "")
    require(content_type.startswith("text/html"), f"{context}: not HTML: {content_type}")
    require('<main id="bd">' in response.text, f"{context}: #bd is absent")
    return response.text


_LOOPBACK_HOST_ALIASES = frozenset(("localhost", "127.0.0.1", "::1"))


def local_form_action_path(source: str, action: str) -> str | None:
    """Resolve a form action while treating supported loopback hosts as one origin.

    The application intentionally renders some absolute actions from PUBLIC_URL.
    Docker's default PUBLIC_URL uses ``localhost`` while this suite defaults to
    ``127.0.0.1``; those names address the same test deployment when their
    scheme and effective port match.
    """
    if action.startswith("//") or any(
        marker in action for marker in ("{", "}", "None", "undefined")
    ):
        return None
    try:
        st_base = urllib.parse.urlsplit(BASE)
        source_url = urllib.parse.urljoin(BASE, source.lstrip("/"))
        st_target = urllib.parse.urlsplit(urllib.parse.urljoin(source_url, action))
        base_port = st_base.port or (443 if st_base.scheme == "https" else 80)
        target_port = st_target.port or (443 if st_target.scheme == "https" else 80)
    except ValueError:
        return None

    if (
        st_target.scheme not in ("http", "https")
        or st_target.scheme != st_base.scheme
        or st_target.username is not None
        or st_target.password is not None
        or target_port != base_port
    ):
        return None
    base_host = (st_base.hostname or "").lower()
    target_host = (st_target.hostname or "").lower()
    if target_host != base_host and not {
        base_host,
        target_host,
    }.issubset(_LOOPBACK_HOST_ALIASES):
        return None
    return st_target.path or "/"


def topic(path: str, client: Client | None = None) -> str:
    return require_html((client or ANON).request(path), path)


BASE = ""
ANON: Client


@test("database user and role matrix")
def database_users() -> None:
    require(
        db("SELECT string_agg(score::text,',' ORDER BY score) FROM users WHERE id BETWEEN 9100001 AND 9100010")
        == "45,50,70,201,300,500,750,1000,2000,3000",
        "regular score matrix differs from manifest",
    )
    require(
        db("SELECT count(*) FROM users WHERE id BETWEEN 9100001 AND 9100014 AND corrector AND NOT canmod")
        == "2",
        "expected exactly two correctors",
    )
    require(
        db("SELECT count(*) FROM users WHERE id BETWEEN 9100001 AND 9100014 AND canmod")
        == "2",
        "expected exactly two moderators",
    )
    require(
        db("SELECT count(DISTINCT regdate) FROM users WHERE id BETWEEN 9100001 AND 9100014")
        == "14",
        "registration dates are not all distinct",
    )
    require(
        db("SELECT count(DISTINCT userinfo_markup) FROM users WHERE id BETWEEN 9100001 AND 9100014")
        == "4",
        "profile markup does not cover all four stored modes",
    )


@test("database content matrix and authorship")
def database_content() -> None:
    require(db("SELECT count(*) FROM topics WHERE id BETWEEN 9101001 AND 9101099") == "18", "topic count")
    require(db("SELECT count(*) FROM comments WHERE id BETWEEN 9102001 AND 9102099") == "18", "comment count")
    require(db("SELECT count(*) FROM images WHERE id BETWEEN 9104001 AND 9104099") == "6", "image count")
    require(db("SELECT count(*) FROM polls WHERE id BETWEEN 9103001 AND 9103099") == "2", "poll count")
    require(
        db(
            "SELECT count(*) FROM users u WHERE u.id BETWEEN 9100001 AND 9100014 "
            "AND (EXISTS(SELECT 1 FROM topics t WHERE t.userid=u.id AND t.id BETWEEN 9101001 AND 9101099) "
            "OR EXISTS(SELECT 1 FROM comments c WHERE c.userid=u.id AND c.id BETWEEN 9102001 AND 9102099))"
        )
        == "14",
        "every fixture user must author a topic or comment",
    )


@test("month-scale fixture contract")
def month_scale_fixture() -> None:
    require(
        db("SELECT count(*) FROM users WHERE id BETWEEN 9100001 AND 9100050") == "50",
        "monthly fixture must contain exactly 50 users",
    )
    require(
        db("SELECT count(*) FROM topics WHERE userid BETWEEN 9100001 AND 9100050") == "1000",
        "monthly fixture must contain exactly 1000 topics",
    )
    require(
        int(db("SELECT last_value FROM images_id_seq")) >= 9_140_060,
        "image sequence overlaps the deterministic monthly media directory range",
    )
    require(
        db("SELECT count(*) FROM comments WHERE userid BETWEEN 9100001 AND 9100050") == "5000",
        "monthly fixture must contain exactly 5000 comments",
    )
    require(
        db(
            "SELECT count(*) FROM groups g JOIN sections s ON s.id=g.section "
            "WHERE s.id IN (1,2,3,5,6) AND NOT EXISTS (SELECT 1 FROM topics t "
            "WHERE t.groupid=g.id AND t.userid BETWEEN 9100001 AND 9100050)"
        )
        == "0",
        "at least one section/group has no fixture topic",
    )
    require(
        db(
            "SELECT (max(postdate)<=CURRENT_TIMESTAMP AND "
            "min(postdate)>=CURRENT_TIMESTAMP-interval '31 days')::text "
            "FROM topics WHERE userid BETWEEN 9100001 AND 9100050"
        )
        == "true",
        "monthly fixture is stale or contains future topics; rerun seed.py",
    )
    require(
        db(
            "SELECT (max(postdate)-min(postdate)>=interval '29 days')::text "
            "FROM topics WHERE userid BETWEEN 9100001 AND 9100050"
        )
        == "true",
        "topic dates do not span the rolling month",
    )


@test("database poll and reaction consistency")
def database_interactions() -> None:
    require(
        db(
            "SELECT count(*) FROM polls_variants pv WHERE pv.vote BETWEEN 9103001 AND 9103099 "
            "AND pv.votes<>(SELECT count(*) FROM vote_users vu WHERE vu.variant_id=pv.id)"
        )
        == "0",
        "poll aggregate differs from vote_users",
    )
    require(
        db(
            "WITH stored AS ("
            "SELECT t.id topic_id,NULL::integer comment_id,(e.key)::integer origin_user,e.value reaction "
            "FROM topics t CROSS JOIN LATERAL jsonb_each_text(t.reactions) e WHERE t.id BETWEEN 9101001 AND 9101099 "
            "UNION ALL "
            "SELECT c.topic,c.id,(e.key)::integer,e.value FROM comments c "
            "CROSS JOIN LATERAL jsonb_each_text(c.reactions) e WHERE c.id BETWEEN 9102001 AND 9102099) "
            "SELECT count(*) FROM stored s LEFT JOIN reactions_log rl ON rl.topic_id=s.topic_id "
            "AND rl.comment_id IS NOT DISTINCT FROM s.comment_id AND rl.origin_user=s.origin_user "
            "AND rl.reaction=s.reaction WHERE rl.origin_user IS NULL"
        )
        == "0",
        "a stored reaction has no reactions_log row",
    )


@test("public profiles and profile sanitizer")
def profiles() -> None:
    for user in MANIFEST["users"]:
        if user.get("blocked", False):
            blocked = ANON.request(f"/people/{user['nick']}/profile")
            require(
                blocked.status == 403,
                f"anonymous blocked profile must be forbidden: {user['nick']}",
            )
            require(
                blocked.headers.get("Content-Type", "").startswith("text/html")
                and '<main id="bd">' in blocked.text
                and f"Пользователь {user['nick']} забанен." in blocked.text
                and "Проверка страницы заблокированного профиля" in blocked.text
                and "начиная с" in blocked.text,
                f"blocked profile does not use the original user-banned model: {user['nick']}",
            )
            continue
        page = require_html(ANON.request(f"/people/{user['nick']}/profile"), user["nick"])
        require(user["nick"] in page, f"profile nick missing: {user['nick']}")
    legacy = require_html(ANON.request("/people/albatross3000/profile"), "legacy profile")
    require("<script>alert(1)</script>" not in legacy, "profile stored XSS was not sanitized")

    retired = ANON.request("/people/swift45/?output=rss")
    require(retired.status == 410, f"retired user RSS returned {retired.status}")
    require(
        retired.headers.get("Content-Type", "").startswith("text/html")
        and '<main id="bd">' in retired.text
        and "RSS-фид для этой страницы удалён." in retired.text
        and "The RSS feed for this page has been retired." in retired.text,
        "retired user RSS does not render the original themed 410 page",
    )

    anonymous_feed = ANON.request("/people/anonymous/")
    require(
        anonymous_feed.status == 500
        and "Лента для пользователя anonymous не доступна" in anonymous_feed.text,
        "anonymous user feed does not preserve UserErrorException 500",
    )


@test("local userpics for all monthly fixture users")
def fixture_userpics() -> None:
    disabled = ANON.request("/img/p.gif")
    require(disabled.status == 200, "DisabledUserpic asset is not served")
    require(
        disabled.headers.get("Content-Type", "").startswith("image/gif")
        and disabled.body[:6] in {b"GIF87a", b"GIF89a"}
        and disabled.body[6:10] == b"\x01\x00\x01\x00",
        "DisabledUserpic is not the Java-compatible 1x1 GIF",
    )
    require(
        db(
            "SELECT count(*) FROM users WHERE id BETWEEN 9100001 AND 9100050 "
            "AND photo=id::text||'.png'"
        )
        == "50",
        "not every fixture user has a local avatar filename",
    )
    for user_id in range(9_100_001, 9_100_051):
        response = ANON.request(f"/photos/{user_id}.png")
        require(response.status == 200, f"avatar {user_id}: HTTP {response.status}")
        require(
            response.headers.get("Content-Type", "").startswith("image/png"),
            f"avatar {user_id}: wrong content type",
        )
        require(response.body.startswith(b"\x89PNG\r\n\x1a\n"), f"avatar {user_id}: invalid PNG")
        if user_id == 9_100_001:
            require(
                response.headers.get("Cache-Control") == "max-age=31556926",
                "active userpic does not use Java media cache period",
            )
            csp = response.headers.get("Content-Security-Policy", "")
            require(
                "img-src 'self' data:" in csp
                and "https://secure.gravatar.com" in csp,
                "userpic response CSP blocks local or Gravatar fallbacks",
            )
        if user_id in {9_100_001, 9_100_002, 9_100_003}:
            actual_size = (
                int.from_bytes(response.body[16:20], "big"),
                int.from_bytes(response.body[20:24], "big"),
            )
            expected_size = {
                9_100_001: (300, 150),
                9_100_002: (150, 300),
                9_100_003: (120, 100),
            }[user_id]
            require(
                actual_size == expected_size,
                f"avatar {user_id}: dimensions {actual_size}, expected {expected_size}",
            )

    profile = require_html(ANON.request("/people/crane2000/profile"), "crane avatar profile")
    require('src="/photos/9100009.png"' in profile, "profile does not use crane local avatar")

    historical = ANON.request("/photos/9100009:-123456.png")
    require(historical.status == 302, "anonymous historical userpic is not redirected")
    require(
        historical.headers.get("Location") == "/photos/9100009.png",
        "historical userpic redirect does not target the active photo",
    )
    require(
        ANON.request("/photos/9999999.png").status == 404,
        "unknown userpic owner is not rejected",
    )
    require(
        db("SELECT blocked::text FROM users WHERE id=9100050") == "true",
        "blocked userpic fixture lost its account state",
    )
    require(
        ANON.request("/photos/9100050.png").status == 200,
        "active photo of a blocked user is not public like Java",
    )

    # Java gates topic/comment userpics with the *viewer's* `photos`
    # setting.  Keep the primary browser account useful for visual tests and
    # retain lark70 as an explicit negative fixture.
    crane = Client(BASE)
    crane.login("crane2000")
    visible = require_html(
        crane.request("/forum/games/9101003"), "crane userpic-enabled topic"
    )
    require(
        'class="userpic"><img class="photo"' in visible
        and "width=150 height=150" in visible,
        "crane viewer does not see Java-compatible userpics",
    )
    require(
        re.search(
            r'<img class="photo" src="/photos/9100001\.png" alt="" '
            r'width=150 height=75 >',
            visible,
        )
        is not None,
        "landscape userpic is not scaled with Java ImageInfo proportions",
    )

    hidden_viewer = Client(BASE)
    hidden_viewer.login("lark70")
    hidden = require_html(
        hidden_viewer.request("/forum/games/9101003"), "photo-disabled topic"
    )
    require('class="userpic"' not in hidden, "photos=false still renders a userpic")
    require(
        "message-w-userpic" not in hidden,
        "photos=false still reserves the userpic column",
    )
    hidden_profile = require_html(
        hidden_viewer.request("/people/lark70/profile"),
        "photo-disabled viewer profile",
    )
    require(
        re.search(
            r'<img class="photo" src="/photos/9100003\.png" alt="" '
            r'width=120 height=100 >',
            hidden_profile,
        )
        is not None,
        "profile incorrectly applies the topic/comment photos=false gate",
    )

    moderator = Client(BASE)
    moderator.login("hawk_moderator")
    tracker = require_html(moderator.request("/tracker"), "moderator recent userpics")
    for user_id, width, height in (
        (9_100_001, 150, 75),
        (9_100_002, 75, 150),
        (9_100_003, 120, 100),
    ):
        require(
            re.search(
                rf'<div class="userpic"><img class="photo" '
                rf'src="/photos/{user_id}\.png" alt="" '
                rf'width={width} height={height} ></div>',
                tracker,
            )
            is not None,
            f"moderator tracker userpic {user_id} has wrong Java tag DOM",
        )


@test("notifications and private user activity pages")
def private_activity_pages() -> None:
    for path in (
        "/notifications",
        "/people/crane2000/tracked",
        "/people/crane2000/deleted-topics",
        "/people/crane2000/reactions",
    ):
        require(ANON.request(path).status == 403, f"anonymous private access allowed: {path}")

    require(
        ANON.request("/people/does-not-exist/drafts").status == 403,
        "anonymous draft lookup leaks whether the target user exists",
    )

    favs = require_html(ANON.request("/people/finch50/favs"), "public favorites")
    require(
        "<title>Избранные сообщения finch50</title>" in favs,
        "favorites page title differs from UserTopicListController",
    )
    require(
        '<h1>Избранные сообщения <a href="/people/finch50/profile">finch50</a></h1>'
        in favs,
        "favorites heading/profile link differs",
    )
    require('id="bd"' in favs and 'id="topic-9101003"' in favs, "favorites are not full news cards")
    require(
        re.search(r"</h1>\s*<nav>\s*</nav>", favs) is not None,
        "favorites do not preserve the empty user-topics.jsp nav element",
    )
    require(
        "Поиск в темах пользователя" not in favs,
        "favorites unexpectedly expose the regular user-topic search block",
    )
    committed = require_html(
        ANON.request("/people/robin201/?section=1"),
        "committed premoderated author topic",
    )
    require(
        re.search(r'id="topic-9101002".*?itemprop="datePublished"', committed, re.S)
        is not None,
        "committed premoderated card does not expose commitdate/datePublished",
    )

    crane = Client(BASE)
    crane.login("crane2000")
    notifications = require_html(crane.request("/notifications"), "notifications")
    require("<h1>Уведомления</h1>" in notifications, "notifications title differs")
    for label in ("ответы", "отслеживаемое", "удаленное", "упоминания", "теги", "реакции", "предупреждения"):
        require(label in notifications, f"notification filter is absent: {label}")
    require("notifications-item" in notifications, "notification cards are absent")
    require("RSS подписка на новые уведомления" in notifications, "notification RSS link is absent")
    first_id = re.search(r'name="firstId" value="(\d+)"', notifications)
    last_id = re.search(r'name="lastId" value="(\d+)"', notifications)
    require(first_id is not None and last_id is not None, "notification click range is absent")
    clicked = crane.request(
        "/notifications-click",
        "POST",
        [("firstId", first_id.group(1)), ("lastId", last_id.group(1))],
    )
    require(clicked.status == 302, f"notification click: expected 302, got {clicked.status}")
    require(
        clicked.headers.get("Location", "").startswith(
            ("/news/", "/forum/", "/gallery/", "/polls/", "/articles/", "/view-deleted")
        ),
        "notification click does not redirect to its content",
    )

    tracked = require_html(crane.request("/people/crane2000/tracked"), "tracked topics")
    require(
        '<h1>Отслеживаемые сообщения <a href="/people/crane2000/profile">crane2000</a></h1>'
        in tracked,
        "tracked heading/profile link differs",
    )
    require('class="news"' in tracked, "tracked topics are not complete news cards")
    require("следующие →" in tracked, "tracked first page has no paginator")
    tracked_second = require_html(
        crane.request("/people/crane2000/tracked?offset=20"), "tracked second page"
    )
    require("← предыдущие" in tracked_second, "tracked previous-page link is absent")

    deleted = require_html(
        crane.request("/people/crane2000/deleted-topics"), "deleted topics"
    )
    for fragment in ("Причина удаления", "Штраф", "написано", "удалено", "Тестовая причина удаления"):
        require(fragment in deleted, f"deleted-topic table misses: {fragment}")

    own_reactions = require_html(
        crane.request("/people/crane2000/reactions"), "reactions made by crane"
    )
    received_reactions = require_html(
        crane.request("/people/crane2000/reactions/to"), "reactions received by crane"
    )
    require("мои реакции" in own_reactions and "reactions-view-item" in own_reactions, "own reactions are absent")
    require("на мои сообщения" in received_reactions and "reactions-view-item" in received_reactions, "received reactions are absent")

    other = Client(BASE)
    other.login("raven1000")
    require(
        other.request("/people/crane2000/reactions").status == 403,
        "ordinary user can inspect another user's reactions",
    )
    moderator = Client(BASE)
    moderator.login("hawk_moderator")
    require_html(
        moderator.request("/people/crane2000/deleted-topics"),
        "moderator views another user's deleted topics",
    )

    swift = Client(BASE)
    swift.login("swift45")
    drafts = require_html(swift.request("/people/swift45/drafts"), "owner drafts")
    require(
        '<h1>Черновики <a href="/people/swift45/profile">swift45</a></h1>' in drafts,
        "draft heading/profile link differs",
    )
    for fragment in (
        'id="topic-9101016"',
        "Черновик нового пользователя",
        'href="delete.jsp?msgid=9101016"',
        'href="edit.jsp?msgid=9101016"',
    ):
        require(fragment in drafts, f"draft full-card/menu fragment is absent: {fragment}")
    require(
        'comment-message.jsp?topic=9101016' not in drafts,
        "draft card incorrectly offers comment creation",
    )
    require(
        re.search(r'id="topic-9101016".*?itemprop="dateCreated"', drafts, re.S)
        is not None,
        "draft signature does not expose the original postdate/dateCreated contract",
    )
    require(
        other.request("/people/swift45/drafts").status == 403,
        "ordinary user can inspect another user's drafts",
    )
    moderator_drafts = require_html(
        moderator.request("/people/swift45/drafts"),
        "moderator views another user's drafts",
    )
    require('id="topic-9101016"' in moderator_drafts, "moderator draft override is absent")
    require(
        "Новый пользователь: проверить ограничения score=45" in moderator_drafts,
        "viewer-owned private remark is absent from prepared draft card",
    )
    require(
        "Новый пользователь: проверить ограничения score=45" not in drafts,
        "another viewer's private remark leaked to the draft owner",
    )


@test("user topic history uses complete cards and includes pending content")
def user_topic_history() -> None:
    pending = require_html(ANON.request("/people/swift45/"), "pending author history")
    require('id="topic-9101001"' in pending, "pending author topic is hidden from user history")
    require("(не подтверждено)" in pending, "pending marker is absent from user history")
    require("9101016" not in pending, "draft leaked into public user history")

    news = require_html(ANON.request("/people/robin201/?section=1"), "news author history")
    require('id="topic-9101002"' in news, "section-filtered user topic is absent")
    require('href="/tag/linux%20foundation"' in news, "user history tag URL is not encoded")
    require("(linux.org.ru)" in news, "external source host is absent from user history")

    gallery = require_html(ANON.request("/people/raven1000/"), "gallery author history")
    require('class="slider-parent"' in gallery, "multi-image gallery is not rendered in user history")
    poll = require_html(ANON.request("/people/crane2000/"), "poll author history")
    require('name="vote"' in poll and "Прошёл большую часть" in poll, "poll is not rendered in user history")


@test("tag index cloud and role-aware letter lists")
def tag_index() -> None:
    root = require_html(ANON.request("/tags"), "tag index")
    require(re.search(r"<title>\s*Список меток", root) is not None, "original tag title is absent")
    require('class="tags-first-letters"' in root, "tag first-letter index is absent")
    require(re.search(r'class="cloud\d+"[^>]*>prod-ready</a>', root) is not None, "tag cloud is absent")
    require(
        re.search(r'class="tags-first-letters".*?</div>\s*<ul>', root, re.S) is None,
        "tag root incorrectly renders the per-letter list",
    )

    legacy = ANON.request("/tags.jsp")
    require(legacy.status == 302, f"legacy tags redirect: expected 302, got {legacy.status}")
    require(legacy.headers.get("Location") == "/tags", "legacy tags redirect target differs")

    anonymous = require_html(ANON.request("/tags/l"), "anonymous tag letter")
    require(re.search(r'<span>l</span>', anonymous) is not None, "current tag letter is not selected")
    require(">lor</a>" in anonymous, "popular l tag is absent")
    require("lineageos" not in anonymous, "anonymous user sees a one-topic tag")
    require("action-buttons" not in anonymous, "anonymous user sees tag moderation actions")

    corrector = Client(BASE)
    corrector.login("tern_corrector")
    corrector_page = require_html(corrector.request("/tags/l"), "corrector tag letter")
    require("lineageos" in corrector_page, "corrector does not see a one-topic tag")
    require("action-buttons" not in corrector_page, "corrector sees moderator tag actions")

    moderator = Client(BASE)
    moderator.login("hawk_moderator")
    moderator_page = require_html(moderator.request("/tags/l"), "moderator tag letter")
    require("lineageos" in moderator_page, "moderator does not see a one-topic tag")
    require("Изменить" in moderator_page and "Удалить" in moderator_page, "tag moderation actions are absent")
    require(ANON.request("/tags/definitely-missing-letter").status == 404, "empty tag letter is not 404")


@test("tag aggregate, section feeds, offsets and viewer controls")
def tag_page_and_section_contracts() -> None:
    aggregate = require_html(ANON.request("/tag/prod-ready"), "aggregate tag page")
    require(
        '<h1><i class="icon-tag"></i> Метка: Prod-ready</h1>' in aggregate,
        "aggregate tag heading differs from TagPageController",
    )
    # TagPageController promotes only the newest recent news item to a full
    # card; older news and forum topics remain in the brief section lists.
    require(
        'id="topic-9101012"' in aggregate
        and "/news/russia/9101002" in aggregate
        and "/forum/games/9101003" in aggregate
        and "Проходите ли вы игры, которые покупаете?" in aggregate,
        "aggregate tag page does not combine its news and forum fixtures",
    )
    require(
        'id="tagFavNoth" href="#"' in aggregate
        and 'id="tagIgnNoth" href="#"' in aggregate,
        "anonymous aggregate tag controls differ from the JSP placeholders",
    )
    require(
        "/user-filter/favorite-tag" not in aggregate
        and "/user-filter/ignore-tag" not in aggregate,
        "aggregate tag page exposes direct mutation forms instead of /user-filter links",
    )

    aggregate_with_offset = require_html(
        ANON.request("/tag/prod-ready?offset=-1"),
        "aggregate tag page with an ignored offset",
    )
    require(
        'id="topic-9101012"' in aggregate_with_offset
        and "/news/russia/9101002" in aggregate_with_offset
        and "/forum/games/9101003" in aggregate_with_offset,
        "offset without section incorrectly selects the section-feed controller",
    )

    news = require_html(
        ANON.request("/tag/prod-ready?section=1"),
        "news tag section",
    )
    require(
        '<h1><i class="icon-tag"></i> <a href="/tag/prod-ready">Prod-ready</a></h1>'
        in news,
        "news tag feed does not link its heading to the aggregate page",
    )
    require(
        'href="/tag/prod-ready?section=1" class="btn btn-selected"' in news,
        "news tag section is not selected in section navigation",
    )
    require(
        'id="topic-9101002"' in news and 'id="topic-9101003"' not in news,
        "news tag feed is not isolated from forum topics",
    )

    forum = require_html(
        ANON.request("/tag/prod-ready?section=2"),
        "forum tag section",
    )
    require('class="tracker"' in forum, "forum tag feed does not use tracker DOM")
    require(
        'href="/tag/prod-ready?section=2" class="btn btn-selected"' in forum,
        "forum tag section is not selected in section navigation",
    )
    require(
        "/forum/games/9101003" in forum
        and "Проходите ли вы игры, которые покупаете?" in forum
        and 'id="topic-9101002"' not in forum,
        "forum tag feed is not isolated from news topics",
    )

    news_at_zero = require_html(
        ANON.request("/tag/prod-ready?section=1&offset=0"),
        "news tag section at zero offset",
    )
    news_at_negative = require_html(
        ANON.request("/tag/prod-ready?section=1&offset=-1"),
        "news tag section at negative offset",
    )
    require(
        'id="topic-9101002"' in news_at_zero
        and 'id="topic-9101002"' in news_at_negative,
        "TopicListService.fixOffset compatibility does not clamp a negative offset to zero",
    )

    for path, label in (
        ("/tag/prod-ready?section=", "empty section"),
        ("/tag/prod-ready?section=0", "zero section"),
    ):
        require(
            ANON.request(path).status == 404,
            f"{label} does not use the source not-found 404 contract",
        )

    for path, label in (
        ("/tag/prod-ready?section=invalid", "malformed section"),
        ("/tag/prod-ready?section=1&offset=invalid", "malformed offset"),
    ):
        require(
            ANON.request(path).status == 400,
            f"{label} does not use the live Spring binding 400 contract",
        )

    registered = Client(BASE)
    registered.login("lark70")
    registered_aggregate = require_html(
        registered.request("/tag/prod-ready"),
        "registered aggregate tag page",
    )
    for fragment in (
        'id="tagFavAdd" href="/user-filter?newFavoriteTagName=prod-ready"',
        'id="tagIgnore" href="/user-filter?newIgnoreTagName=prod-ready"',
    ):
        require(fragment in registered_aggregate, f"registered aggregate control is absent: {fragment}")
    require(
        'id="tagFavNoth"' not in registered_aggregate
        and 'id="tagIgnNoth"' not in registered_aggregate,
        "registered aggregate tag page still renders anonymous placeholders",
    )

    registered_news = require_html(
        registered.request("/tag/prod-ready?section=1"),
        "registered news tag section",
    )
    require(
        'id="tagFavAdd" href="/user-filter?newFavoriteTagName=prod-ready"'
        in registered_news
        and 'id="tagIgnore" href="/user-filter?newIgnoreTagName=prod-ready"'
        in registered_news,
        "registered section tag controls differ from the aggregate/JSP contract",
    )

    stored = Client(BASE)
    stored.login("robin201")
    favorite = require_html(
        stored.request("/tag/linux%20foundation"),
        "stored favorite tag state",
    )
    ignored = require_html(
        stored.request("/tag/%D0%B8%D0%B3%D1%80%D1%8B"),
        "stored ignored tag state",
    )
    require(
        'id="tagFavAdd" href="/user-filter" class="selected"' in favorite,
        "stored favorite does not render the selected removal control",
    )
    require(
        'id="tagIgnore" href="/user-filter" class="selected"' in ignored,
        "stored ignored tag does not render the selected removal control",
    )


@test("canonical content routes")
def canonical_routes() -> None:
    home_response = ANON.request("/")
    require_html(home_response, "canonical home page")
    require(
        home_response.headers.get("X-Frame-Options") == "DENY",
        "global X-Frame-Options does not match the pinned Spring Security response",
    )
    require(
        home_response.headers.get("X-XSS-Protection") == "0",
        "global Spring Security X-XSS-Protection compatibility header is absent",
    )
    require(
        home_response.headers.get("Expires")
        == "Thu, 01 Jan 1970 00:00:00 GMT",
        "dynamic response does not preserve the Java expiry header",
    )
    require(
        home_response.headers.get("X-Content-Type-Options") == "nosniff",
        "global X-Content-Type-Options is absent",
    )
    require(
        "frame-ancestors 'self'"
        in home_response.headers.get("Content-Security-Policy", ""),
        "global CSP frame-ancestors policy is absent",
    )
    for fixture in MANIFEST["topics"]:
        page = topic(fixture["path"])
        require(str(fixture["id"]) in page, f"topic id missing: {fixture['path']}")
    expanded_cut = topic("/news/russia/9101002")
    require('id="cut"' in expanded_cut and "Эта часть должна быть скрыта" in expanded_cut, "topic cut is not expanded")

    # The 1,000-topic month fixture deliberately fills the section-level first
    # page.  Exercise the same Java TopicListController preview path through
    # the fixture topic's canonical group feed, where the anchor topic is
    # guaranteed to be part of the response.
    news_feed = require_html(ANON.request("/news/russia"), "news cut preview")
    require("читать дальше..." in news_feed, "news feed has no collapsed cut link")
    require("Эта часть должна быть скрыта" not in news_feed, "news feed exposes content below cut")
    article_group = require_html(
        ANON.request("/articles/development"), "article group with shared urlname"
    )
    require(
        "Проверка длинной статьи" in article_group,
        "article group is confused with the forum group of the same urlname",
    )
    forum_group = require_html(
        ANON.request("/forum/linux-org-ru"), "forum group with shared urlname"
    )
    require(
        "Вайбкодю реакции" in forum_group,
        "forum group is confused with the news group of the same urlname",
    )


@test("main page follows Java commit-date ordering")
def main_page_ordering() -> None:
    rows = db(
        "SELECT t.id::text||'|'||t.minor::text FROM topics t "
        "JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section "
        "WHERE NOT t.deleted AND NOT t.draft AND t.open_warnings<=2 "
        "AND s.moderate AND t.commitdate IS NOT NULL "
        "AND s.id IN (1,3,5,6) "
        "AND t.commitdate>=CURRENT_TIMESTAMP-interval '3 months' "
        "ORDER BY t.commitdate DESC LIMIT 30"
    ).splitlines()
    expected: list[int] = []
    regular_cards = 0
    for row in rows:
        topic_id, minor = row.split("|", 1)
        if regular_cards >= 10:
            break
        expected.append(int(topic_id))
        if minor == "false":
            regular_cards += 1

    home = require_html(ANON.request("/"), "anonymous main ordering")
    main_feed = home.split('<aside id="boxlets">', 1)[0]
    actual = [int(value) for value in re.findall(r'id="topic-(\d+)"', main_feed)]
    require(actual == expected, f"main cards differ from Java commitdate order: {actual} != {expected}")


@test("single and multi-image gallery DOM")
def galleries() -> None:
    single = topic("/gallery/screenshots/9101005")
    require(single.count("medium-image-container") == 1, "single gallery has no responsive image")
    require("slider-parent" not in single, "single gallery unexpectedly uses slider")
    slider = topic("/gallery/screenshots/9101006")
    require("slider-parent" in slider and "swiffy-slider" in slider, "multi-image slider is absent")
    require(slider.count("/images/910400") >= 12, "slider does not expose all image derivatives")
    gallery_feed = require_html(ANON.request("/gallery/"), "gallery feed")
    single_card = re.search(r'<article class="news" id="topic-9101005">(.*?)</article>', gallery_feed, re.S)
    slider_card = re.search(r'<article class="news" id="topic-9101006">(.*?)</article>', gallery_feed, re.S)
    require(single_card is not None and "medium-image-container" in single_card.group(1), "single image is absent from gallery feed")
    require(single_card is not None and "swiffy-slider" not in single_card.group(1), "single feed image incorrectly uses slider")
    require(slider_card is not None and "swiffy-slider" in slider_card.group(1), "multi-image slider is absent from gallery feed")
    require(slider_card is not None and slider_card.group(1).count('class="slider-nav') == 2, "gallery feed slider controls are malformed")
    for image_id in range(9104001, 9104007):
        response = ANON.request(f"/images/{image_id}/1000px.jpg")
        require(response.status == 200, f"image {image_id} is unavailable")
        require(response.headers.get("Content-Type") == "image/jpeg", f"image {image_id} content type")


@test("comment actions follow viewer security restrictions")
def comment_action_visibility() -> None:
    unrestricted_feed_path = db(
        "SELECT CASE s.id WHEN 1 THEN '/news/' WHEN 2 THEN '/forum/' "
        "WHEN 3 THEN '/gallery/' WHEN 5 THEN '/polls/' WHEN 6 THEN '/articles/' END||g.urlname "
        "FROM topics t JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section "
        "WHERE t.id=9101014"
    )
    unrestricted_feed = require_html(
        ANON.request(unrestricted_feed_path), "anonymous unrestricted feed"
    )
    unrestricted_card = re.search(
        r'<article class="news" id="topic-9101014">(.*?)</article>',
        unrestricted_feed,
        re.S,
    )
    require(
        unrestricted_card is not None
        and "5 комментариев" in unrestricted_card.group(1)
        and "comment-message.jsp?topic=9101014" not in unrestricted_card.group(1),
        "feed card does not follow the Java existing-comments action contract",
    )
    anonymous_topic = topic("/forum/games/9101003")
    require(
        re.search(r">\s*Ответить\s*</a>", anonymous_topic) is not None,
        "anonymous user does not see a reply action on an unrestricted topic",
    )
    require(
        'id="commentForm"' in anonymous_topic
        and 'name="nick"' in anonymous_topic
        and 'name="password"' in anonymous_topic
        and re.search(
            r'<input[^>]*name="password"[^>]*></div><div class="help-block">.*?</div>',
            anonymous_topic,
            re.S,
        )
        is not None,
        "anonymous unrestricted topic omits the inline identity form",
    )
    require(
        "/js/add-form.js" in anonymous_topic,
        "anonymous unrestricted topic does not load the reply-form script",
    )

    restricted_topic = topic("/news/opensource/9101012")
    require(
        'id="commentForm"' not in restricted_topic
        and re.search(r">\s*Ответить\s*</a>", restricted_topic) is None,
        "registered-only topic exposes anonymous reply controls",
    )
    require(
        "Для того чтобы оставить комментарий" in restricted_topic
        and "register.jsp" in restricted_topic,
        "registered-only topic omits the Java login/register invite",
    )
    restricted_feed = require_html(ANON.request("/news/opensource"), "anonymous restricted feed")
    restricted_card = re.search(
        r'<article class="news" id="topic-9101012">(.*?)</article>', restricted_feed, re.S
    )
    require(
        restricted_card is not None
        and "comment-message.jsp?topic=9101012" not in restricted_card.group(1),
        "registered-only feed card exposes an anonymous comment action",
    )

    registered = Client(BASE)
    registered.login("lark70")
    require_html(registered.request("/"), "registered home page")
    registered_news = topic("/news/opensource/9101012", registered)
    require(
        "comment-message.jsp?topic=9101012" in registered_news,
        "eligible registered user does not see the news reply action",
    )
    registered_topic = topic("/forum/games/9101003", registered)
    require(
        "comment-message.jsp?topic=9101003" in registered_topic,
        "eligible registered user does not see the topic reply action",
    )
    require(
        "add_comment.jsp?topic=9101003&replyto=9102004" in registered_topic,
        "eligible registered user does not see the comment reply action",
    )
    require(
        'id="commentForm"' in registered_topic and "/js/add-form.js" in registered_topic,
        "eligible registered user does not receive the inline comment form",
    )
    require(
        "/js/lor/forms-and-reactions.js" not in registered_topic,
        "topic loads the conflicting non-original comment handler",
    )
    require(
        re.search(
            r'href="add_comment\.jsp\?topic=9101003&replyto=9102004"\s+'
            r'data-author-readonly="(?:true|false)"',
            registered_topic,
        )
        is not None,
        "comment reply link differs from the original add-form.js DOM contract",
    )
    require(
        '/post-warning?topic=9101003&amp;comment=9102004' in registered_topic,
        "eligible user does not see the comment moderator-warning action",
    )

    moderator = Client(BASE)
    moderator.login("hawk_moderator")
    moderator_topic = topic("/forum/games/9101003", moderator)
    require(
        '/delete_comment.jsp?msgid=9102004' in moderator_topic,
        "moderator does not see the comment delete action",
    )
    require(
        'itemprop="creator"' in moderator_topic
        and '<br class="visible-phone"> <span class="hideon-phone">(</span>' in moderator_topic,
        "comment sign differs from the Java mobile/user DOM",
    )
    require(
        'sameip.jsp?ip=198.51.100.4' in moderator_topic
        and 'prod-ready-browser/1.0' in moderator_topic
        and 'sameip.jsp?ua=' in moderator_topic,
        "moderator comment IP/User-Agent metadata is absent",
    )
    require(
        "Новый пользователь: проверить ограничения score=45" in moderator_topic,
        "viewer-owned comment author remark is absent",
    )
    require(
        "[Нарушение правил] Проверка DOM предупреждения" in moderator_topic
        and 'class="clear-warning-form"' in moderator_topic,
        "prepared comment warning DOM is absent",
    )

    edit_fixture = db(
        "SELECT '/'||CASE s.id WHEN 1 THEN 'news' WHEN 2 THEN 'forum' "
        "WHEN 3 THEN 'gallery' WHEN 5 THEN 'polls' WHEN 6 THEN 'articles' END||"
        "'/'||g.urlname||'/'||t.id "
        "FROM comments c JOIN topics t ON t.id=c.topic JOIN groups g ON g.id=t.groupid "
        "JOIN sections s ON s.id=g.section WHERE c.id=9102014"
    )
    edited_topic = topic(edit_fixture, moderator)
    require(
        "Последнее исправление:" in edited_topic
        and f'{edit_fixture}/9102014/history' in edited_topic
        and "исправлений: 1" in edited_topic,
        "comment edit summary/history link is absent",
    )


@test("comment forms, preview and reply target validation")
def comment_form_contracts() -> None:
    anonymous_edit = ANON.request(
        "/edit_comment?topic=9101003&original=9102004"
    )
    require(
        anonymous_edit.status == 403 and anonymous_edit.headers.get("Location") is None,
        "anonymous edit-comment must use the Java AuthorizedOnly 403 contract",
    )
    restricted_form = require_html(
        ANON.request("/add_comment.jsp?topic=9101012"),
        "restricted anonymous comment form",
    )
    require(
        "Это сообщение нельзя комментировать" in restricted_form,
        "GET comment validation escaped as HTTP error instead of BindingResult form",
    )

    client = Client(BASE)
    client.login("crane2000")

    topic_form = require_html(
        client.request("/comment-message.jsp?topic=9101003"),
        "top-level comment form",
    )
    require('id="topic-9101003"' in topic_form, "top-level form omits topic context")
    require(
        'name="replyto"' not in topic_form,
        "dedicated comment-message form adds a reply target absent in the original JSP",
    )
    inline_topic_form = topic("/forum/games/9101003", client)
    require(
        'name="replyto" value="0"' in inline_topic_form,
        "inline top-level form does not preserve the original zero reply target",
    )

    reply_form = require_html(
        client.request("/add_comment.jsp?topic=9101003&replyto=9102004"),
        "reply comment form",
    )
    require('id="comment-9102004"' in reply_form, "reply form omits parent comment context")
    require(
        'name="replyto" value="9102004"' in reply_form,
        "reply form loses its parent comment id",
    )

    stale_edit = client.request(
        "/edit_comment?topic=9101003&original=9102004"
    )
    require(
        stale_edit.status == 302
        and stale_edit.headers.get("Location")
        == "/forum/games/9101003?cid=9102004",
        "non-editable GET comment did not use the Java topic redirect",
    )
    stale_edit_post = require_html(
        client.request(
            "/edit_comment",
            "POST",
            [
                ("topic", "9101003"),
                ("original", "9102004"),
                ("msg", "unchanged old comment"),
            ],
        ),
        "non-editable POST comment form",
    )
    require(
        "Истек срок редактирования" in stale_edit_post,
        "non-editable POST comment escaped as HTTP error instead of form validation",
    )

    preview = client.request(
        "/add_comment_ajax",
        "POST",
        [
            ("topic", "9101003"),
            ("replyto", "0"),
            ("msg", "**comment preview**"),
            ("preview", "Предпросмотр"),
        ],
    )
    require(preview.status == 200, f"comment preview returned {preview.status}")
    preview_json = json.loads(preview.text)
    require(preview_json.get("errors") == [], "valid comment preview contains errors")
    require(
        "<strong>comment preview</strong>" in str(preview_json.get("preview")),
        "comment preview does not use the user's Markdown mode",
    )

    cross_topic = client.request(
        "/add_comment_ajax",
        "POST",
        [
            ("topic", "9101003"),
            ("replyto", "9102001"),
            ("msg", "cross-topic reply must be rejected"),
        ],
    )
    require(cross_topic.status == 200, f"cross-topic validation returned {cross_topic.status}")
    cross_topic_json = json.loads(cross_topic.text)
    require(
        "некорректная тема" in cross_topic_json.get("errors", []),
        "reply to a comment from another topic was accepted",
    )


@test("LORCODE MemberTag and Markdown LorUser resolve existing, blocked and missing users")
def lorcode_member_tag_contract() -> None:
    require(
        db("SELECT COALESCE(blocked,false)::text FROM users WHERE nick='bird50'") == "true",
        "blocked MemberTag fixture user is absent",
    )
    response = ANON.request(
        "/markup/preview",
        "POST",
        [
            ("markup", "lorcode"),
            (
                "text",
                "[user]crane2000[/user][user]bird50[/user]"
                "[user]missing_fixture_user[/user]",
            ),
        ],
    )
    require(response.status == 200, f"MemberTag preview returned {response.status}")
    rendered = str(json.loads(response.text).get("html", ""))
    require(
        '<span style="white-space: nowrap"><img src="/img/tuxlor.png">'
        '<a style="text-decoration: none" '
        'href="http://localhost:8181/people/crane2000/profile">crane2000</a></span>'
        in rendered,
        "existing MemberTag does not match Java DOM/canonical profile URL",
    )
    require(
        '<span style="white-space: nowrap"><img src="/img/tuxlor.png"><s>'
        '<a style="text-decoration: none" '
        'href="http://localhost:8181/people/bird50/profile">bird50</a></s></span>'
        in rendered,
        "blocked MemberTag is not linked and struck like Java",
    )
    require(
        " <s>missing_fixture_user</s>" in rendered,
        "missing MemberTag is not rendered as Java's failed lookup",
    )

    markdown_response = ANON.request(
        "/markup/preview",
        "POST",
        [
            ("markup", "markdown"),
            ("text", "@raven1000 @bird50 @missing_fixture_user"),
        ],
    )
    require(
        markdown_response.status == 200,
        f"Markdown LorUser preview returned {markdown_response.status}",
    )
    markdown_rendered = str(json.loads(markdown_response.text).get("html", ""))
    require(
        '<span style="white-space: nowrap">'
        '<a href="http://localhost:8181/people/raven1000/profile" '
        'class="mention">@raven1000</a></span>'
        in markdown_rendered,
        "existing Markdown LorUser does not match Java DOM/profile URL",
    )
    require(
        '<span style="white-space: nowrap"><s>'
        '<a href="http://localhost:8181/people/bird50/profile" '
        'class="mention">@bird50</a></s></span>'
        in markdown_rendered,
        "blocked Markdown LorUser is not linked and struck like Java",
    )
    require(
        "<s>@missing_fixture_user</s>" in markdown_rendered,
        "missing Markdown LorUser is not struck like Java",
    )

    forum_topic = topic("/forum/games/9101003")
    require(
        "/people/crane2000/profile" in forum_topic
        and "/people/bird50/profile" in forum_topic
        and "<s>missing_fixture_user</s>" in forum_topic,
        "topic consumer does not use DB-aware MemberTag rendering",
    )
    news_topic = topic("/news/russia/9101002")
    require(
        "/people/crane2000/profile" in news_topic,
        "comment consumer does not use DB-aware MemberTag rendering",
    )
    markdown_topic = topic("/forum/linux-org-ru/9101010")
    require(
        'class="mention">@raven1000</a>' in markdown_topic
        and 'class="mention">@bird50</a>' in markdown_topic
        and "<s>@missing_fixture_user</s>" in markdown_topic,
        "topic consumer does not use DB-aware Markdown LorUser rendering",
    )
    require(
        'class="mention">@raven1000</a>' in news_topic,
        "comment consumer does not use DB-aware Markdown LorUser rendering",
    )
    require(
        "tuxlor.png" in topic("/people/crane2000/"),
        "user topic-list card does not use DB-aware MemberTag rendering",
    )
    require(
        "tuxlor.png" in topic("/people/finch50/profile"),
        "profile userinfo does not use DB-aware MemberTag rendering",
    )
    require(
        "tuxlor.png" in require_html(ANON.request("/"), "MemberTag main page"),
        "main-page topic card does not use DB-aware MemberTag rendering",
    )
    rss = ANON.request("/section-rss.jsp?section=1")
    require(rss.status == 200, f"MemberTag RSS returned {rss.status}")
    require(
        "tuxlor.png" in rss.text and "/people/crane2000/profile" in rss.text,
        "RSS consumer does not use DB-aware MemberTag rendering",
    )


@test("poll rendering and pending visibility")
def polls() -> None:
    results = topic("/polls/polls/9101007?results=true")
    require("poll-result" in results and "Всего голосов: 8" in results, "poll results are incomplete")
    require(re.search(r'class="msg-text".*class="poll-result"', results, re.S) is not None, "poll is outside the original msg-text container")
    percentages = [int(value) for value in re.findall(r"\((\d+)%\)", results)]
    require(percentages and max(percentages) <= 100, "poll contains impossible percentage")
    poll_feed = require_html(ANON.request("/polls/"), "poll feed")
    require('href="/view-all.jsp?section=5"' in poll_feed and "Неподтверждённые: 1" in poll_feed, "pending poll count is absent")
    require('name="vote"' in poll_feed and "Прошёл меньшую часть" in poll_feed, "poll variants are absent from feed")
    require("Для участия в опросе" in poll_feed, "anonymous poll login notice is absent")
    feed_percentages = [int(value) for value in re.findall(r"\((\d+)%\)", poll_feed)]
    require(feed_percentages and max(feed_percentages) <= 100, "legacy poll feed contains impossible percentage")
    notice_position = poll_feed.index("Для участия в опросе")
    results_position = poll_feed.index("Результаты", notice_position)
    require(notice_position < results_position, "anonymous notice must precede the results link")
    voter = Client(BASE)
    voter.login("lark70")
    voter_feed = require_html(voter.request("/polls/"), "authorized poll feed")
    require('action="/vote.jsp"' in voter_feed and 'name="csrf"' in voter_feed, "authorized poll form is absent from feed")
    previous_voter = Client(BASE)
    previous_voter.login("swift45")
    previous_feed = require_html(previous_voter.request("/polls/"), "voted poll feed")
    require("poll-result" in previous_feed and "poll-selected" in previous_feed, "voted poll results are absent from feed")
    public_pending = require_html(
        ANON.request("/polls/polls/9101008"), "public pending poll preview"
    )
    require("ожидает подтверждения" in public_pending, "public pending poll notice is absent")
    author = Client(BASE)
    author.login("albatross3000")
    pending = require_html(author.request("/polls/polls/9101008"), "pending poll owner")
    require("ожидает подтверждения" in pending, "pending poll notice is absent")


@test("premoderation queue uses complete original-style cards")
def premoderation_queue() -> None:
    cases = (
        (1, 9101001, "Краткая тестовая новость", None),
        (3, 9101017, "Неподтверждённая галерея должна отображаться", "medium-image-container"),
        (5, 9101008, "Неподтверждённый опрос должен быть виден", "poll-uncommited"),
        (6, 9101018, "Очередь премодерации должна сохранять", None),
    )
    for section_id, topic_id, body_fragment, required_dom in cases:
        page = require_html(
            ANON.request(f"/view-all.jsp?section={section_id}"),
            f"premoderation section {section_id}",
        )
        card = re.search(
            rf'<article class="news" id="topic-{topic_id}">(.*?)</article>',
            page,
            re.S,
        )
        require(card is not None, f"topic {topic_id} is not rendered as a news card")
        require(body_fragment in card.group(1), f"topic {topic_id} body is absent")
        require("(не подтверждено)" in card.group(1), f"topic {topic_id} pending marker is absent")
        if required_dom is not None:
            require(required_dom in card.group(1), f"topic {topic_id} misses {required_dom}")
        require("Подтвердить" not in card.group(1), "anonymous user sees commit action")

    all_pending = require_html(ANON.request("/view-all.jsp"), "all pending topics")
    for topic_id in (9101001, 9101008, 9101017, 9101018):
        require(f'id="topic-{topic_id}"' in all_pending, f"global queue misses {topic_id}")

    own_corrector = Client(BASE)
    own_corrector.login("ibis_corrector")
    own_page = require_html(own_corrector.request("/view-all.jsp?section=3"), "own pending gallery")
    own_card = re.search(r'id="topic-9101017"(.*?)</article>', own_page, re.S)
    require(own_card is not None, "corrector pending gallery is absent")
    require("Править" in own_card.group(1), "author edit action is absent")
    require(
        "Удалить" not in own_card.group(1),
        "author can delete a pending topic after the month fixture added replies",
    )
    require("Подтвердить" not in own_card.group(1), "corrector can commit their own topic")

    other_corrector = Client(BASE)
    other_corrector.login("tern_corrector")
    other_page = require_html(other_corrector.request("/view-all.jsp?section=3"), "pending gallery moderation")
    other_card = re.search(r'id="topic-9101017"(.*?)</article>', other_page, re.S)
    require(other_card is not None and "Подтвердить" in other_card.group(1), "other corrector cannot commit topic")

    news_queue = require_html(ANON.request("/view-all.jsp?section=1"), "pending news link")
    require("(linux.org.ru)" in news_queue, "external news link has no original short host")
    require('href="/tag/prod-ready"' in news_queue, "news tag URL differs")


@test("comments, nesting, reactions and closed-topic controls")
def comments_and_reactions() -> None:
    forum = topic("/forum/games/9101003")
    require('id="comment-9102004"' in forum and 'id="comment-9102016"' in forum, "fixture comments missing")
    require('class="userpic"><img class="photo"' in forum, "comment userpic column is absent")
    require("message-w-userpic" in forum, "comment body does not reserve userpic space")
    require("<h2>Комментарии:" not in forum, "non-original comment count heading is present")
    reply = topic("/news/russia/9101002")
    require("Ответ на:" in reply and "от lark70" in reply, "reply context is absent")
    leaf = re.search(r'id="comment-9102003"(.*?)</article>', reply, re.S)
    require(
        leaf is not None
        and re.search(r'<a[^>]*>Показать ответ', leaf.group(1)) is None,
        "leaf comment offers nonexistent answers",
    )
    reactor = Client(BASE)
    reactor.login("swift45")
    comment_form_topic = topic("/forum/games/9101003", reactor)
    require(
        "Пустая строка (два раза Enter)" in comment_form_topic,
        "original comment markup help is absent for an eligible registered user",
    )
    require(
        'href="/help/markdown.md"' in comment_form_topic,
        "comment Markdown help link is absent for an eligible registered user",
    )
    reaction_page = topic("/forum/linux-org-ru/9101010")
    for reaction in ("🤡", "👍", "🔥"):
        require(reaction in reaction_page, f"topic reaction is absent: {reaction}")
    authenticated_reactions = topic("/forum/linux-org-ru/9101010", reactor)
    require("zero-reactions" in authenticated_reactions, "hidden zero-reaction choices are absent")
    require("reaction-show" in authenticated_reactions, "reaction reveal control is absent")
    closed = topic("/forum/linux-org-ru/9101013")
    require("Ответить" not in closed, "closed topic still offers a reply action")


@test("topic page original-compatible metadata and scripts")
def topic_dom_contract() -> None:
    page = topic("/forum/games/9101003")
    require(re.search(r'<div class="messages"[^>]*>\s*<article class="msg"', page) is not None, "topic is outside .messages")
    require('<link rel="canonical"' in page, "canonical link is absent")
    require('property="og:title"' in page, "OpenGraph title is absent")
    require('property="og:description"' in page, "OpenGraph description is absent")
    require('property="og:tag"' in page, "OpenGraph tag metadata is absent")
    require(
        'property="article:section" content="Форум: Games"' in page,
        "article section metadata does not include section and group",
    )
    require("initNextPrevKeys()" in page, "topic keyboard navigation is not initialized")
    require('id="interpage"' in page and "init_interpage_adv(ads)" in page, "interpage ad hook is absent")
    require(
        "/js/add-form.js" in page and 'id="commentForm"' in page,
        "unrestricted anonymous topic omits Java reply controls",
    )
    require("/login.jsp?from=" in page, "header login link does not preserve current URL")
    require(
        re.search(r'>oriole300</a>\s*<span class="stars">★★★</span>', page) is not None,
        "topic author score stars are absent",
    )
    require(
        re.search(r'>crane2000</a>\s*<span class="stars">★★★★★</span>', page) is not None,
        "comment author score stars are absent",
    )

    page_minus_one = ANON.request("/forum/games/9101003/page-1")
    require(page_minus_one.status == 302, "page-1 does not redirect to the canonical topic")
    require(
        page_minus_one.headers.get("Location", "").endswith("/forum/games/9101003"),
        "page-1 redirect target is not the canonical topic",
    )


@test("topic scrollers, ignored subtree filter and deleted-comments POST contract")
def topic_navigation_filter_deleted_contract() -> None:
    scroller_path = db(
        "SELECT '/forum/'||g.urlname||'/'||t.id FROM topics t "
        "JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section "
        "WHERE s.id=2 AND s.scroll_mode='GROUP' AND NOT t.deleted AND NOT t.draft "
        "AND NOT t.sticky AND t.stat1>0 AND EXISTS (SELECT 1 FROM topics n "
        "WHERE n.groupid=t.groupid AND NOT n.deleted AND NOT n.draft AND NOT n.sticky "
        "AND n.postdate<>t.postdate) ORDER BY t.id LIMIT 1"
    )
    require(scroller_path != "", "no DB-derived forum topic is available for scroller coverage")
    scroller_page = topic(scroller_path)
    require('class="scroller-row"' in scroller_page, "topic scroller DOM is absent")
    require('class="scroller-arrow"' in scroller_page, "topic scroller has no adjacent link")

    ignored_fixture = db(
        "SELECT viewer.nick||'|'||"
        "CASE s.id WHEN 1 THEN '/news/' WHEN 2 THEN '/forum/' WHEN 3 THEN '/gallery/' "
        "WHEN 5 THEN '/polls/' WHEN 6 THEN '/articles/' END||g.urlname||'/'||t.id||'|'||"
        "parent.id||'|'||child.id FROM ignore_list il "
        "JOIN users viewer ON viewer.id=il.userid "
        "JOIN comments parent ON parent.userid=il.ignored AND NOT parent.deleted "
        "JOIN comments child ON child.replyto=parent.id AND NOT child.deleted "
        "JOIN topics t ON t.id=parent.topic AND t.id=child.topic "
        "JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section "
        "WHERE viewer.id BETWEEN 9100001 AND 9100050 "
        "AND NOT t.deleted AND NOT t.draft AND COALESCE(t.postscore,-9999)<>10002 "
        "AND (SELECT count(*) FROM comments direct_child "
        "WHERE direct_child.replyto=parent.id AND NOT direct_child.deleted)=1 "
        "ORDER BY t.id,parent.id LIMIT 1"
    )
    require(ignored_fixture != "", "no DB-derived ignored reply subtree is available")
    viewer_nick, ignored_path, parent_id, child_id = ignored_fixture.split("|")
    viewer = Client(BASE)
    viewer.login(viewer_nick)
    filtered_page = topic(ignored_path, viewer)
    require(f'id="comment-{parent_id}"' not in filtered_page, "ignored parent is still rendered")
    require(f'id="comment-{child_id}"' not in filtered_page, "ignored reply subtree is still rendered")
    shown_page = topic(f"{ignored_path}?filter=show", viewer)
    require(f'id="comment-{parent_id}"' in shown_page, "filter=show does not restore ignored parent")
    require(f'id="comment-{child_id}"' in shown_page, "filter=show does not restore ignored child")
    single_answer_hrefs = {
        html.unescape(value)
        for value in re.findall(
            r'<a href="([^"]+)" data-samepage="(?:true|false)">\s*Показать ответ\s*</a>',
            shown_page,
        )
    }
    require(
        f"{ignored_path}?filter=show&cid={child_id}" in single_answer_hrefs,
        "single-answer link does not preserve filter=show",
    )

    eligible = Client(BASE)
    eligible.login("crane2000")
    live_page = topic("/forum/games/9101003", eligible)
    require(
        'action="/forum/games/9101003" method="POST"' in live_page
        and 'name="deleted" value="1"' in live_page,
        "deleted-comments action is not the source POST form",
    )
    deleted_get = eligible.request("/forum/games/9101003?deleted=1")
    require(deleted_get.status == 302, "eligible non-moderator GET deleted view is not canonicalized")
    deleted_post = require_html(
        eligible.request(
            "/forum/games/9101003",
            "POST",
            [("deleted", "1")],
        ),
        "eligible deleted-comments POST",
    )
    require('id="comment-9102015"' in deleted_post, "deleted-comments POST omits deleted comment")
    deleted_card = re.search(r'id="comment-9102015"(.*?)</article>', deleted_post, re.S)
    require(
        deleted_card is not None and "👍" in deleted_card.group(1),
        "deleted comment loses its reaction display",
    )


@test("score and role authorization")
def authorization() -> None:
    low = Client(BASE)
    low.login("swift45")
    low_talks = require_html(low.request("/add.jsp?group=8404&noinfo=1"), "score45 talks form")
    require("disabled" in low_talks and "score" in low_talks, "score45 posting restriction is absent")

    boundary = Client(BASE)
    boundary.login("finch50")
    boundary_talks = require_html(boundary.request("/add.jsp?group=8404&noinfo=1"), "score50 talks form")
    require('<button type="submit" class="btn btn-primary">' in boundary_talks, "score50 cannot post to Talks")
    require('data-format-mode="lorcode"' in boundary_talks, "saved LORCODE mode is not used by topic form")
    require('href="/help/lorcode.md"' in boundary_talks, "LORCODE help is absent from topic form")
    require('name="draft"' in boundary_talks, "authorized topic form has no draft action")
    require("не более 5" in boundary_talks and "не более 3" in boundary_talks, "tag limits are absent from topic form")
    boundary_settings = require_html(boundary.request("/people/finch50/settings"), "score50 settings")
    require(
        re.search(r'<input[^>]*id="hideAdsense"[^>]*disabled', boundary_settings) is not None,
        "score50 can enable the one-green-star advertising option",
    )

    anonymous_form = require_html(ANON.request("/add.jsp?group=8404&noinfo=1"), "anonymous topic form")
    require('name="draft"' not in anonymous_form, "anonymous topic form exposes draft action")

    ordinary = Client(BASE)
    ordinary.login("raven1000")
    require(ordinary.request("/commit.jsp?msgid=9101001").status == 403, "ordinary user can open commit form")

    own_corrector = Client(BASE)
    own_corrector.login("ibis_corrector")
    require(own_corrector.request("/commit.jsp?msgid=9101017").status == 403, "corrector can commit own pending topic")

    other_corrector = Client(BASE)
    other_corrector.login("ibis_corrector")
    commit_form = other_corrector.request("/commit.jsp?msgid=9101001")
    require(
        commit_form.status == 200
        and 'action="edit.jsp"' in commit_form.text
        and 'name="commit"' in commit_form.text,
        "corrector cannot review another author through the source-compatible edit form",
    )

    moderator = Client(BASE)
    moderator.login("hawk_moderator")
    require(moderator.request("/commit.jsp?msgid=9101001").status == 200, "moderator cannot open commit form")
    single_group = moderator.request("/add-section.jsp?section=5")
    require(
        single_group.status == 302
        and single_group.headers.get("Location") == "/add.jsp?group=19387",
        "single-group add-section redirect differs from SectionController",
    )
    groupmod = require_html(
        moderator.request("/groupmod.jsp?group=126"),
        "moderator group settings",
    )
    require('id="groupModForm" action="groupmod.jsp" method="POST"' in groupmod, "groupmod form contract differs")
    for field in (
        'name="group" value="126"',
        'name="title"',
        'name="info"',
        'name="urlName"',
        'name="longinfo"',
        'name="resolvable"',
        'name="csrf"',
    ):
        require(field in groupmod, f"groupmod field is absent: {field}")
    require(
        moderator.request("/groupmod.jsp").status == 404
        and moderator.request("/groupmod.jsp?group=invalid").status == 404,
        "groupmod required numeric binding differs from the original bad-parameter contract",
    )
    require(ordinary.request("/groupmod.jsp?group=126").status == 403, "ordinary user can edit group")

    same_ip = require_html(
        # Comment 9102014 is inside SameIPController's five-day window.
        # Comment 9102004 at 5 days 20 hours is intentionally outside it and
        # therefore cannot exercise the JSP's `not empty comments` delete form.
        moderator.request("/sameip.jsp?ip=198.51.100.14"),
        "moderator same-IP workflow",
    )
    for fragment in (
        '<form action="sameip.jsp">',
        'action="delip.jsp"',
        'action="banip.jsp"',
        'name="ip" value="198.51.100.14"',
        'name="ban_time"',
        'name="ban_mode"',
        'name="allow_posting"',
        'name="captcha_required"',
        'name="csrf"',
        "function banTimeChange",
        "/admin/geoip?ip=198.51.100.14",
        "/people/eagle_moderator/profile",
        "Вожусь с Terraform: тест локального mirror",
    ):
        require(fragment in same_ip, f"same-IP workflow fragment is absent: {fragment}")
    require(
        "Re: Terraform mirror" not in same_ip,
        "same-IP result uses the comment title instead of the Java topic title",
    )
    same_ip_root = require_html(moderator.request("/sameip.jsp"), "same-IP search form")
    require(
        'action="delip.jsp"' not in same_ip_root and 'action="banip.jsp"' not in same_ip_root,
        "same-IP mutation controls are visible without an exact IP",
    )
    require_html(
        moderator.request("/sameip.jsp?mask=33"),
        "same-IP ignores mask without an IP",
    )
    invalid_ip = moderator.request("/sameip.jsp?ip=not-an-ip")
    require(
        invalid_ip.status == 500 and "not ip" in invalid_ip.text,
        "same-IP invalid address does not preserve BadInputException 500",
    )
    invalid_mask = moderator.request("/sameip.jsp?ip=198.51.100.4&mask=33")
    require(
        invalid_mask.status == 500 and "bad mask" in invalid_mask.text,
        "same-IP invalid mask does not preserve BadInputException 500",
    )
    require(ordinary.request("/sameip.jsp?ip=198.51.100.4").status == 403, "ordinary user can open same-IP moderation")


@test("explicit Java user errors keep visible 500 responses without mutation")
def explicit_user_error_contracts() -> None:
    for path in ("/tracker?offset=-1", "/tracker/?offset=301"):
        response = ANON.request(path)
        require(
            response.status == 500 and "Некорректное значение offset" in response.text,
            f"tracker invalid offset differs from UserErrorException: {path}",
        )

    moderator = Client(BASE)
    moderator.login("hawk_moderator")
    invalid_domain_offset = moderator.request("/admin/email-domains?offset=-1")
    require(
        invalid_domain_offset.status == 500 and "Wrong offset" in invalid_domain_offset.text,
        "email-domain offset does not preserve BadInputException 500",
    )
    invalid_domain = moderator.request(
        "/admin/email-domains/add", "POST", [("domain", "invalid_domain!")]
    )
    require(
        invalid_domain.status == 500 and "Invalid domain" in invalid_domain.text,
        "email-domain add validation does not preserve BadInputException 500",
    )
    delete_legacy_shape = moderator.request(
        "/admin/email-domains/delete", "POST", [("domain", "invalid_domain!")]
    )
    require(
        delete_legacy_shape.status == 302
        and delete_legacy_shape.headers.get("Location") == "/admin/email-domains",
        "email-domain delete applies the add-only regex or has a wrong redirect",
    )

    owner = Client(BASE)
    owner.login("crane2000")
    settings_before = db("SELECT settings::text FROM user_settings WHERE id=9100009")
    invalid_settings = owner.request(
        "/people/crane2000/settings",
        "POST",
        [
            ("topics", "31"),
            ("messages", "500"),
            ("style", "tango-light"),
            ("format_mode", "markdown"),
            ("avatar", "empty"),
            ("trackerMode", "main"),
        ],
    )
    require(
        invalid_settings.status == 500
        and "некорректное число тем" in invalid_settings.text
        and db("SELECT settings::text FROM user_settings WHERE id=9100009") == settings_before,
        "invalid settings do not fail atomically with BadInputException 500",
    )
    own_remark = owner.request("/people/crane2000/remark")
    require(
        own_remark.status == 500 and "Нельзя оставить заметку самому себе" in own_remark.text,
        "self-remark does not preserve UserErrorException 500",
    )
    require_html(
        owner.request("/people/crane2000/remarks?offset=-1&sort=99"),
        "empty remarks skip offset/sort validation",
    )


@test("group list uses original compact DOM")
def group_dom() -> None:
    # The forum JSP renders the denormalized groups.stat3 value.  In Java it
    # is refreshed by StatUpdater.updateGroupStats() (initial delay 5 min,
    # then hourly), not by GroupController/GroupListDao on every request.
    # Compose deliberately disables all background/external jobs, so run the
    # same database maintenance function here before checking its HTTP view.
    db("SELECT stat_update2()")

    forum_index = require_html(ANON.request("/forum/"), "forum index")
    forum_groups = db(
        "SELECT id||'|'||urlname||'|'||stat3 FROM groups WHERE section=2 ORDER BY id"
    ).splitlines()
    rendered_groups = re.findall(
        r'<li><a class="navLink" href="/forum/([^"/]+)">', forum_index
    )
    non_tech = {8404, 4068, 9326, 19405}
    expected_groups = [
        urllib.parse.quote(row.split("|", 2)[1], safe="")
        for row in forum_groups
        if int(row.split("|", 2)[0]) not in non_tech
    ] + [
        urllib.parse.quote(row.split("|", 2)[1], safe="")
        for row in forum_groups
        if int(row.split("|", 2)[0]) in non_tech
    ]
    require(
        rendered_groups == expected_groups,
        "forum group order differs from GroupDao/SectionController",
    )
    for row in forum_groups:
        _, urlname, stat3 = row.split("|", 2)
        encoded_urlname = urllib.parse.quote(urlname, safe="")
        require(
            re.search(
                rf'href="/forum/{re.escape(encoded_urlname)}"[^>]*>.*?</a>\s*'
                rf'\({re.escape(stat3)} за сутки\)',
                forum_index,
                re.S,
            )
            is not None,
            f"forum index does not render groups.stat3 for {urlname}",
        )

    page = require_html(ANON.request("/forum/games"), "group topics")
    require('class="group-item"' in page, "group page does not use group-item rows")
    require('class="tracker-item"' not in page, "group page incorrectly uses tracker rows")
    new_card = re.search(r'<a href="([^"]+)" class="group-item">.*?Проходите ли вы игры', page, re.S)
    require(new_card is not None, "new-topic group row is absent")
    require("lastmod=" not in new_card.group(1), "new-topic mode incorrectly opens the last comment")
    active = require_html(ANON.request("/forum/games?lastmod=true"), "active group topics")
    active_card = re.search(
        r'<a href="([^"]+)" class="group-item">.*?Проходите ли вы игры', active, re.S
    )
    require(active_card is not None, "active group row is absent")
    require("lastmod=9102016" in active_card.group(1), "active mode does not open the last comment")

    mismatched = db(
        "WITH expected AS (SELECT g.id,"
        "COALESCE(sum(t.stat3) FILTER (WHERE NOT t.deleted "
        "AND t.lastmod>CURRENT_TIMESTAMP-'2 days'::interval),0)::bigint+"
        "count(t.id) FILTER (WHERE NOT t.deleted "
        "AND t.postdate>CURRENT_TIMESTAMP-'1 day'::interval)::bigint AS stat3 "
        "FROM groups g LEFT JOIN topics t ON t.groupid=g.id GROUP BY g.id) "
        "SELECT count(*) FROM groups g JOIN expected e USING(id) "
        "WHERE g.stat3::bigint<>e.stat3"
    )
    require(mismatched == "0", "group counters differ from Java stat_update2 semantics")

    # The month-scale fixture promises content in every live group.  Exercise
    # the actual anonymous HTTP predicate instead of inferring visibility from
    # the activity counter (the original does not make that implication).
    for row in forum_groups:
        _, urlname, _ = row.split("|", 2)
        encoded_urlname = urllib.parse.quote(urlname, safe="")
        group_page = require_html(
            ANON.request(f"/forum/{encoded_urlname}"), f"forum group {urlname}"
        )
        require(
            'class="group-item"' in group_page,
            f"month fixture has no anonymously displayable topic in {urlname}",
        )


@test("archive index matches original controls and visibility")
def archive_index() -> None:
    gallery = require_html(ANON.request("/gallery/archive/"), "gallery archive")
    for fragment in (
        'action="/search.jsp"',
        'name="range"',
        'name="section" value="gallery"',
        'class="btn btn-selected" href="/gallery/archive/"',
    ):
        require(fragment in gallery, f"gallery archive contract is absent: {fragment}")
    latest_gallery_year, latest_gallery_month = db(
        "SELECT EXTRACT(YEAR FROM max(t.postdate))::int||'|'||"
        "EXTRACT(MONTH FROM max(t.postdate))::int FROM topics t "
        "JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section "
        "WHERE s.id=3 AND NOT t.deleted AND (t.moderate OR NOT s.moderate)"
    ).split("|", 1)
    require(
        f"/gallery/archive/{latest_gallery_year}/{latest_gallery_month}/" in gallery,
        "latest gallery archive month URL has no canonical trailing slash",
    )
    # TopicService.getUncommitedCount is deliberately live rather than part of
    # monthly_stats.  Stateful browser compatibility runs can leave another
    # valid pending gallery topic behind, so a hard-coded fixture count makes
    # this test order-dependent.  Use the exact TopicDao predicate from the
    # original and require the archive navigation to expose that value.
    pending_gallery = int(
        db(
            "SELECT count(*) FROM topics,groups,sections WHERE section=sections.id "
            "AND sections.moderate AND NOT draft AND topics.groupid=groups.id "
            "AND NOT deleted AND NOT topics.moderate "
            "AND postdate>(CURRENT_TIMESTAMP-'3 month'::interval) AND section=3"
        )
    )
    rendered_pending = re.search(r"Неподтверждённые: (\d+)", gallery)
    require(
        (rendered_pending is None and pending_gallery == 0)
        or (
            rendered_pending is not None
            and int(rendered_pending.group(1)) == pending_gallery
        ),
        "archive premoderation count differs from original TopicDao semantics",
    )
    require("(36)" not in gallery, "uncommitted gallery topics leaked into archive statistics")

    forum = require_html(ANON.request("/forum/games/archive/"), "forum archive")
    require('href="/forum/games?lastmod=true"' in forum, "forum archive active tab is absent")
    require('name="group" value="games"' in forum, "forum archive search group is absent")
    latest_forum_year, latest_forum_month = db(
        "SELECT EXTRACT(YEAR FROM max(t.postdate))::int||'|'||"
        "EXTRACT(MONTH FROM max(t.postdate))::int FROM topics t "
        "JOIN groups g ON g.id=t.groupid JOIN sections s ON s.id=g.section "
        "WHERE s.id=2 AND g.urlname='games' AND NOT t.deleted "
        "AND (t.moderate OR NOT s.moderate)"
    ).split("|", 1)
    require(
        f"/forum/games/{latest_forum_year}/{latest_forum_month}/" in forum,
        "latest forum archive month URL differs",
    )


@test("public page and form DOM contracts")
def public_page_contracts() -> None:
    add_without_group = ANON.request("/add.jsp")
    require(
        add_without_group.status == 302
        and add_without_group.headers.get("Location") == "/add-section.jsp",
        "add.jsp without group has a non-Spring redirect contract",
    )
    invalid_group_offset = ANON.request("/forum/games?offset=-1")
    require(
        invalid_group_offset.status == 404
        and "Bad format of &#39;offset&#39; offset не может быть отрицательным"
        in invalid_group_offset.text,
        "negative group offset differs from ServletParameterBadValueException",
    )
    require(
        ANON.request("/forum/games/1989/1/").status == 404
        and ANON.request("/forum/games/2026/13/").status == 404,
        "archive calendar bounds do not use the bad-parameter 404 contract",
    )

    forum = require_html(ANON.request("/forum/"), "forum index")
    for heading in ("Технический форум", "Остальное", "Лента форума", "RSS подписки"):
        require(f"<h1>{heading}</h1>" in forum, f"forum heading is absent: {heading}")
    require('class="navLink"' in forum, "forum group navigation contract is absent")
    require('href="/section-rss.jsp?section=2"' in forum, "forum RSS URL differs")

    search = require_html(ANON.request("/search.jsp"), "search form")
    require('action="/search.jsp"' in search, "search form action differs")
    for field in ('name="q"', 'name="range"', 'name="interval"', 'name="user"', 'name="usertopic"'):
        require(field in search, f"search form field is absent: {field}")

    login = require_html(ANON.request("/login.jsp?from=/forum/"), "login form")
    for field in ('name="redirectUrl" value="/forum/"', 'name="nick"', 'name="passwd"', 'name="csrf"'):
        require(field in login, f"login form field is absent: {field}")

    register = require_html(ANON.request("/register.jsp"), "registration form")
    for field in ('id="registerForm"', 'name="nick"', 'name="email"', 'name="password"', 'name="password2"', 'name="rules" value="okay"'):
        require(field in register, f"registration field is absent: {field}")
    require('href="/help/rules.md"' in register, "registration rules URL differs")
    require(
        'equalTo: "#password"' in register and 'remote: "/check-login"' in register,
        "registration client validation differs from the original JSP",
    )

    rules = require_html(ANON.request("/help/rules.md"), "forum rules")
    require("Правил" in rules, "forum rules content is absent")
    require('rel="search" title="Search L.O.R." href="/search.jsp"' in rules, "global search relation is absent")
    require(re.search(r'<base href="https?://[^\"]+:8181/">', rules) is not None, "original-compatible base URL is absent")
    require('href="/help/markdown.md">Разметка Markdown</a>' in rules, "anonymous footer markup help is absent")


@test("representative internal links, forms and rendered assets stay resolvable")
def representative_interface_integrity() -> None:
    owner = Client(BASE)
    owner.login("crane2000")
    settings = require_html(
        owner.request("/people/crane2000/settings"),
        "interface-integrity settings",
    )
    documents = {
        "/": require_html(ANON.request("/"), "interface-integrity home"),
        "/forum/": require_html(ANON.request("/forum/"), "interface-integrity forum"),
        "/tag/prod-ready": require_html(
            ANON.request("/tag/prod-ready"), "interface-integrity aggregate tag"
        ),
        "/tag/prod-ready?section=1": require_html(
            ANON.request("/tag/prod-ready?section=1"),
            "interface-integrity news tag",
        ),
        "/tag/prod-ready?section=2": require_html(
            ANON.request("/tag/prod-ready?section=2"),
            "interface-integrity forum tag",
        ),
        "/forum/games/9101003": topic("/forum/games/9101003"),
        "/people/crane2000/profile": require_html(
            ANON.request("/people/crane2000/profile"),
            "interface-integrity profile",
        ),
        "/login.jsp?from=/forum/": require_html(
            ANON.request("/login.jsp?from=/forum/"),
            "interface-integrity login",
        ),
        "/search.jsp": require_html(
            ANON.request("/search.jsp"), "interface-integrity search"
        ),
        "/people/crane2000/settings": settings,
    }

    expected_links = {
        "/": ("/news/", "/gallery/", "/polls/", "/articles/", "/forum/", "/tracker/", "/tags"),
        "/forum/": ("/forum/games", "/forum/lenta"),
        "/tag/prod-ready": ("/news/russia/9101002",),
        "/tag/prod-ready?section=1": ("/tag/prod-ready", "/news/russia/9101002"),
        "/tag/prod-ready?section=2": ("/tag/prod-ready", "/forum/games/9101003"),
        "/forum/games/9101003": ("/forum/games", "/people/oriole300/profile"),
        "/people/crane2000/profile": ("/people/crane2000/",),
        "/login.jsp?from=/forum/": ("/register.jsp", "/lostpwd.jsp"),
        "/search.jsp": (),
        "/people/crane2000/settings": (
            "/people/crane2000/edit",
            "/addphoto.jsp",
            "/user-filter",
        ),
    }
    safe_targets: dict[str, Client] = {}
    for source, vec_targets in expected_links.items():
        body = documents[source]
        source_client = owner if source == "/people/crane2000/settings" else ANON
        for target in vec_targets:
            require(
                f'href="{target}' in body,
                f"representative internal link is absent: {source} -> {target}",
            )
            safe_targets[target] = source_client

    for target, client in sorted(safe_targets.items()):
        response = client.request(target)
        require(response.status == 200, f"representative internal link is not 200: {target}")
        require(
            response.headers.get("Content-Type", "").startswith("text/html"),
            f"representative internal link is not HTML: {target}",
        )

    form_documents = {
        "/login.jsp?from=/forum/": documents["/login.jsp?from=/forum/"],
        "/search.jsp": documents["/search.jsp"],
        "/forum/games/9101003": documents["/forum/games/9101003"],
        "/people/crane2000/settings": settings,
    }
    form_action_pattern = re.compile(
        r'<form\b[^>]*\baction=(?:"([^"]*)"|\'([^\']*)\'|([^\s>]+))',
        re.I,
    )
    form_action_paths: dict[str, list[str]] = {}
    for source, body in form_documents.items():
        actions = [next(value for value in match if value != "") for match in form_action_pattern.findall(body)]
        require(actions, f"representative page has no expected form: {source}")
        form_action_paths[source] = []
        for action in actions:
            path = local_form_action_path(source, action)
            require(
                path is not None,
                f"form action is not a resolved local target: {source} -> {action}",
            )
            form_action_paths[source].append(path)
    require(
        '/login_process' in form_action_paths["/login.jsp?from=/forum/"],
        "login form action is not /login_process",
    )
    require(
        '/search.jsp' in form_action_paths["/search.jsp"],
        "search form action is not /search.jsp",
    )
    require(
        '/add_comment.jsp' in form_action_paths["/forum/games/9101003"],
        "topic reply form action is not /add_comment.jsp",
    )
    require(
        '/people/crane2000/settings'
        in form_action_paths["/people/crane2000/settings"],
        "settings form action is not the owner settings route",
    )

    asset_paths = {
        "/favicon.ico",
        "/manifest.json",
        "/tango/combined.css",
        # Historical Java demo/import rows still use these pre-2016 names;
        # current source ships their replacement images under new filenames.
        "/tango/img/kde-logo-new2.png",
        "/tango/img/klogo.png",
        "/tango/img/money-logo.png",
        "/tango/img/red-copyright.png",
        "/js/script.min.js",
        "/js/lor.js",
        "/js/plugins.js",
        "/js/highlight.min.js",
        "/js/realtime.js",
        "/webjars/jquery/3.7.1/jquery.min.js",
    }
    for body in documents.values():
        for reference in re.findall(r'(?:href|src)="(/[^"]+)"', body):
            path = urllib.parse.urlsplit(reference).path
            if path.startswith(
                (
                    "/img/",
                    "/images/",
                    "/photos/",
                    "/font/",
                    "/js/",
                    "/tango/",
                    "/black/",
                    "/white2/",
                    "/waltz/",
                    "/zomg_ponies/",
                    "/webjars/",
                )
            ):
                asset_paths.add(reference)

    asset_responses = {}
    for path in sorted(asset_paths):
        response = ANON.request(path)
        require(response.status == 200, f"rendered local asset is not 200: {path}")
        require(len(response.body) > 0, f"rendered local asset is empty: {path}")
        asset_responses[path] = response

    for legacy_path, current_path in (
        ("/tango/img/kde-logo-new2.png", "/tango/img/klogo.png"),
        ("/tango/img/money-logo.png", "/tango/img/red-copyright.png"),
    ):
        require(
            asset_responses[legacy_path].headers.get("Content-Type") == "image/png"
            and asset_responses[legacy_path].body == asset_responses[current_path].body,
            f"historical group-image alias does not serve its current PNG: {legacy_path}",
        )

    require(
        asset_responses["/tango/combined.css"].headers.get("X-Frame-Options")
        == "SAMEORIGIN"
        and asset_responses["/tango/combined.css"].headers.get("X-XSS-Protection")
        is None,
        "security-excluded static asset headers differ from the Java MVC interceptor",
    )


@test("saved theme and authenticated tracker matrix")
def theme_and_tracker_matrix() -> None:
    users = (
        ("swift45", "tango-auto", "/tango/combined.css", "auto", "markdown"),
        ("finch50", "tango-light", "/tango/combined.css", "light", "lorcode"),
        ("lark70", "tango", "/tango/combined.css", "dark", "ntobr"),
        ("robin201", "black", "/black/combined.css", None, "markdown"),
        ("oriole300", "white2", "/white2/combined.css", None, "markdown"),
        ("falcon500", "waltz", "/waltz/combined.css", None, "lorcode"),
        ("heron750", "zomg_ponies", "/zomg_ponies/combined.css", None, "markdown"),
    )
    for nick, style, stylesheet, color_mode, format_mode in users:
        client = Client(BASE)
        client.login(nick)
        page = require_html(client.request(f"/people/{nick}/profile"), f"{style} profile")
        require(f'data-style="{style}"' in page, f"saved style is not applied: {style}")
        require(f'href="{stylesheet}" data-lor-theme-stylesheet' in page, f"stylesheet differs: {style}")
        if color_mode is not None:
            require(f'data-theme="{color_mode}"' in page, f"color mode differs: {style}")
        else:
            html_open = re.search(r"<html[^>]*>", page)
            require(html_open is not None and "data-theme=" not in html_open.group(0), f"legacy theme has tango mode: {style}")
        expected_help = {
            "markdown": "Разметка Markdown",
            "lorcode": "Разметка LORCODE",
        }.get(format_mode)
        if expected_help is not None:
            require(expected_help in page, f"footer markup help differs: {style}")
        else:
            require("Разметка Markdown" not in page and "Разметка LORCODE" not in page, f"legacy mode has unrelated help: {style}")

    tracker_client = Client(BASE)
    tracker_client.login("raven1000")
    settings = require_html(tracker_client.request("/people/raven1000/settings"), "settings form")
    require('id="profileForm"' in settings, "settings form id differs")
    for field in (
        'name="style"',
        'name="format_mode"',
        'name="topics"',
        'name="messages"',
        'name="trackerMode"',
        'name="avatar"',
    ):
        require(field in settings, f"settings field is absent: {field}")
    edit = require_html(tracker_client.request("/people/raven1000/edit"), "profile edit form")
    require('id="editRegForm"' in edit, "profile edit form id differs")
    for field in (
        'name="name"',
        'name="password"',
        'name="password2"',
        'name="url"',
        'name="email"',
        'name="town"',
        'name="info"',
        'name="infoMarkup"',
        'name="oldpass"',
    ):
        require(field in edit, f"profile edit field is absent: {field}")
    tracker = require_html(tracker_client.request("/tracker"), "authenticated tracker")
    require("<h1>Активные топики</h1>" in tracker, "tracker heading is absent")
    require('class="tracker-item"' in tracker, "new tracker DOM is absent")
    for filter_label in ("основные", "все", "без talks", "тех. форум"):
        require(filter_label in tracker, f"tracker filter is absent: {filter_label}")


@test("image delete lifecycle matches Java permissions DOM history and redirect")
def image_delete_lifecycle() -> None:
    image_id = 9104002
    topic_id = 9101006
    initial_deleted = db(f"SELECT deleted::text FROM images WHERE id={image_id}")
    initial_lastmod = db(f"SELECT lastmod::text FROM topics WHERE id={topic_id}")
    initial_history = {
        int(value)
        for value in db(
            "SELECT COALESCE(string_agg(id::text,',' ORDER BY id),'') "
            "FROM edit_info "
            f"WHERE msgid={topic_id} AND object_type='TOPIC'::edit_event_type"
        ).split(",")
        if value
    }

    try:
        anonymous = ANON.request(f"/delete_image?id={image_id}")
        require(anonymous.status == 403, "anonymous image delete form must be forbidden")

        author = Client(BASE)
        author.login("raven1000")
        own_committed = author.request(f"/delete_image?id={image_id}")
        require(
            own_committed.status == 403,
            "author may not edit a committed premoderated gallery topic",
        )

        moderator = Client(BASE)
        moderator.login("hawk_moderator")
        edit = require_html(
            moderator.request(f"/edit.jsp?msgid={topic_id}"),
            "gallery edit existing images",
        )
        for current_image_id in (9104002, 9104003, 9104004):
            require(
                f'href="/images/{current_image_id}/original.png"' in edit
                and f'href="/delete_image?id={current_image_id}"' in edit,
                f"existing image {current_image_id} or its delete link is absent",
            )

        form = require_html(
            moderator.request(f"/delete_image?id={image_id}"),
            "delete image form",
        )
        for fragment in (
            "<h1>Удаление изображения</h1>",
            'class="medium-image-container"',
            f'src="/images/{image_id}/1000px.jpg"',
            '<form method="POST" action="/delete_image">',
            'name="csrf"',
            f'name="id" value="{image_id}"',
            'type="submit" class="btn btn-danger" value="Удалить"',
        ):
            require(fragment in form, f"delete-image DOM contract is absent: {fragment}")

        single_image = moderator.request("/delete_image?id=9104001")
        require(
            single_image.status == 403,
            "the only prepared image in an imagepost section must not be deletable",
        )

        old_lastmod_ms = int(
            db(
                "SELECT floor(extract(epoch FROM lastmod)*1000)::bigint "
                f"FROM topics WHERE id={topic_id}"
            )
        )
        old_history_count = len(initial_history)
        deleted = moderator.request(
            "/delete_image",
            "POST",
            [("id", str(image_id))],
        )
        require(
            deleted.status == 302,
            f"delete image must use RedirectView 302, got {deleted.status}",
        )
        require(
            deleted.headers.get("Location")
            == f"/gallery/screenshots/{topic_id}?lastmod={old_lastmod_ms}",
            "delete image canonical forceLastmod redirect differs: "
            f"{deleted.headers.get('Location')}",
        )
        require(
            db(f"SELECT deleted::text FROM images WHERE id={image_id}") == "true",
            "image soft-delete was not persisted",
        )
        require(
            int(
                db(
                    "SELECT floor(extract(epoch FROM lastmod)*1000)::bigint "
                    f"FROM topics WHERE id={topic_id}"
                )
            )
            > old_lastmod_ms,
            "topic lastmod was not updated",
        )
        require(
            int(
                db(
                    "SELECT count(*) FROM edit_info "
                    f"WHERE msgid={topic_id} AND object_type='TOPIC'::edit_event_type"
                )
            )
            == old_history_count + 1,
            "image deletion did not create exactly one topic history row",
        )
        require(
            db(
                "SELECT editor::text || '|' || oldaddimages::text "
                "FROM edit_info "
                f"WHERE msgid={topic_id} AND object_type='TOPIC'::edit_event_type "
                "ORDER BY id DESC LIMIT 1"
            )
            == "9100013|{9104002,9104003,9104004}",
            "edit_info does not contain the pre-delete ordered image snapshot",
        )
    finally:
        current_history = {
            int(value)
            for value in db(
                "SELECT COALESCE(string_agg(id::text,',' ORDER BY id),'') "
                "FROM edit_info "
                f"WHERE msgid={topic_id} AND object_type='TOPIC'::edit_event_type"
            ).split(",")
            if value
        }
        created_history = sorted(current_history - initial_history)
        delete_history = "SELECT 1"
        if created_history:
            delete_history = (
                "DELETE FROM edit_info "
                f"WHERE msgid={topic_id} AND id IN ({','.join(map(str, created_history))})"
            )
        db(
            "BEGIN; "
            f"UPDATE images SET deleted={initial_deleted} WHERE id={image_id}; "
            f"UPDATE topics SET lastmod='{initial_lastmod}'::timestamptz WHERE id={topic_id}; "
            f"{delete_history}; "
            "COMMIT"
        )


@test("setpostscore matches Java binding form permissions and option delta")
def set_post_score_lifecycle() -> None:
    topic_id = 9101003
    premoderated_topic_id = 9101006

    def options_state(current_topic_id: int) -> tuple[str, str, str, str]:
        value = db(
            "SELECT COALESCE(postscore,-9999)::text||'|'||sticky::text||'|'||"
            "notop::text||'|'||lastmod::text "
            f"FROM topics WHERE id={current_topic_id}"
        )
        parts = value.split("|", 3)
        require(len(parts) == 4, f"topic options state is absent: {current_topic_id}")
        return parts[0], parts[1], parts[2], parts[3]

    initial_states = {
        current_topic_id: options_state(current_topic_id)
        for current_topic_id in (topic_id, premoderated_topic_id)
    }
    try:
        require(
            ANON.request(f"/setpostscore.jsp?msgid={topic_id}").status == 403,
            "anonymous user can open moderator topic options",
        )
        require(
            ANON.request("/setpostscore.jsp").status == 400,
            "missing GET msgid does not use Spring's 400 binding response",
        )
        for value in ("x", "2147483648"):
            require(
                ANON.request(f"/setpostscore.jsp?msgid={value}").status == 400,
                f"invalid GET msgid does not use Spring's 400 binding response: {value}",
            )

        corrector = Client(BASE)
        corrector.login("tern_corrector")
        require(
            corrector.request(f"/setpostscore.jsp?msgid={topic_id}").status == 403,
            "corrector can open moderator topic options",
        )
        require(
            corrector.request(
                "/setpostscore.jsp",
                "POST",
                [("msgid", str(topic_id)), ("postscore", "50")],
            ).status
            == 403,
            "corrector can mutate moderator topic options",
        )

        moderator = Client(BASE)
        moderator.login("hawk_moderator")
        form = require_html(
            moderator.request(f"/setpostscore.jsp?msgid={topic_id}"),
            "setpostscore form",
        )
        for fragment in (
            "<h1>Смена режима параметров сообщения</h1>",
            '<form method=POST action="setpostscore.jsp">',
            'name="csrf"',
            f"name=msgid value=\"{topic_id}\"",
            '<select name="postscore">',
            '<option selected value="-9999">без ограничений</option>',
            'value="-50">для зарегистрированных</option>',
            'value="10002">без комментариев</option>',
            'name="sticky"',
            'name="notop"',
            'class="btn btn-primary">Изменить</button>',
        ):
            require(fragment in form, f"setpostscore form contract is absent: {fragment}")

        premoderated_form = require_html(
            moderator.request(
                f"/setpostscore.jsp?msgid={premoderated_topic_id}"
            ),
            "premoderated setpostscore form",
        )
        require(
            'name="sticky"' not in premoderated_form,
            "premoderated form exposes the sticky checkbox",
        )
        require(
            'name="notop"' in premoderated_form,
            "premoderated form hides the notop checkbox",
        )

        missing_postscore = moderator.request(
            "/setpostscore.jsp",
            "POST",
            [("msgid", str(topic_id)), ("score", "100")],
        )
        require(
            missing_postscore.status == 400,
            "legacy score alias incorrectly replaces required postscore",
        )
        malformed_boolean = moderator.request(
            "/setpostscore.jsp",
            "POST",
            [
                ("msgid", str(topic_id)),
                ("postscore", "-9999"),
                ("sticky", "invalid"),
            ],
        )
        require(
            malformed_boolean.status == 400,
            "malformed Spring Boolean does not use the 400 binding response",
        )
        require(
            moderator.request(
                "/setpostscore.jsp",
                "POST",
                [("msgid", str(topic_id)), ("postscore", "2147483648")],
            ).status
            == 400,
            "overflowing POST postscore does not use the 400 binding response",
        )
        require(
            moderator.request(
                "/setpostscore.jsp",
                "POST",
                [("msgid", "2147483647"), ("postscore", "50")],
            ).status
            == 404,
            "unknown topic does not use the not-found response",
        )
        require(
            moderator.request(
                f"/setpostscore.jsp?msgid={topic_id}",
                "PUT",
            ).status
            == 405,
            "unsupported setpostscore method is not rejected with 405",
        )
        for invalid_postscore in (-10000, -9998, -51, 10003):
            invalid = moderator.request(
                "/setpostscore.jsp",
                "POST",
                [
                    ("msgid", str(topic_id)),
                    ("postscore", str(invalid_postscore)),
                ],
            )
            require(
                invalid.status == 500
                and f"invalid postscore {invalid_postscore}" in invalid.text,
                f"invalid postscore error differs: {invalid_postscore}",
            )
        require(
            options_state(topic_id) == initial_states[topic_id],
            "rejected requests mutated topic options",
        )

        old_lastmod = initial_states[topic_id][3]
        changed = moderator.request(
            "/setpostscore.jsp",
            "POST",
            [
                ("msgid", str(topic_id)),
                ("postscore", "1234"),
                ("sticky", "on"),
                ("notop", "on"),
            ],
        )
        changed_html = require_html(changed, "setpostscore action-done")
        for fragment in (
            "<p>Данные изменены</p>",
            "Установлен новый уровень записи: <b>Ограничение на отправку комментариев</b>: "
            "только для зарегистрированных пользователей, score>=1234<br>",
            "Новое значение sticky: true<br>",
            "Новое значение notop: true<br>",
            f'<a href="/forum/games/{topic_id}">Продолжить</a>',
        ):
            require(fragment in changed_html, f"action-done delta is absent: {fragment}")
        current = options_state(topic_id)
        require(current[:3] == ("1234", "true", "true"), "topic option delta was not persisted")
        require(current[3] != old_lastmod, "topic lastmod was not updated")

        no_op_lastmod = current[3]
        no_op = require_html(
            moderator.request(
                "/setpostscore.jsp",
                "POST",
                [
                    ("msgid", str(topic_id)),
                    ("postscore", "1234"),
                    ("sticky", "on"),
                    ("notop", "on"),
                ],
            ),
            "setpostscore no-op",
        )
        require(re.search(r"<p>\s*</p>", no_op) is not None, "no-op action-done is not empty")
        require(
            options_state(topic_id)[3] != no_op_lastmod,
            "no-op did not unconditionally update lastmod",
        )

        sticky_only = require_html(
            moderator.request(
                "/setpostscore.jsp",
                "POST",
                [("msgid", str(topic_id)), ("postscore", "1234")],
            ),
            "setpostscore sticky/notop-only delta",
        )
        require(
            "Новое значение sticky: false<br>" in sticky_only
            and "Новое значение notop: false<br>" in sticky_only,
            "absent checkbox defaults were not applied",
        )

        crafted_premoderated = moderator.request(
            "/setpostscore.jsp",
            "POST",
            [
                ("msgid", str(premoderated_topic_id)),
                ("postscore", initial_states[premoderated_topic_id][0]),
                ("sticky", "on"),
            ],
        )
        require_html(crafted_premoderated, "crafted premoderated sticky update")
        require(
            options_state(premoderated_topic_id)[1] == "true",
            "server incorrectly rejects sticky for a premoderated topic",
        )
    finally:
        restore_statements = []
        for current_topic_id, state in initial_states.items():
            postscore, sticky, notop, lastmod = state
            restore_statements.append(
                "UPDATE topics SET "
                f"postscore={postscore},sticky={sticky},notop={notop},"
                f"lastmod='{lastmod}'::timestamptz WHERE id={current_topic_id}"
            )
        db("BEGIN; " + "; ".join(restore_statements) + "; COMMIT")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:8181")
    return parser.parse_args()


def main() -> int:
    global BASE, ANON
    args = parse_args()
    BASE = args.base.rstrip("/") + "/"
    ANON = Client(BASE)
    failures: list[str] = []
    for name, function in TESTS:
        try:
            function()
            print(f"PASS {name}")
        except Exception as error:
            failures.append(name)
            print(f"FAIL {name}: {error}", file=sys.stderr)
    print(f"prod_ready_test: {len(TESTS)-len(failures)}/{len(TESTS)} passed")
    if failures:
        print("failed: " + ", ".join(failures), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
