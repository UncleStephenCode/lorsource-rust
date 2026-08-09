#!/usr/bin/env python3
"""State/HTTP/DOM regression suite for the prod_ready_test fixture."""

from __future__ import annotations

import argparse
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
        page = require_html(ANON.request(f"/people/{user['nick']}/profile"), user["nick"])
        require(user["nick"] in page, f"profile nick missing: {user['nick']}")
    legacy = require_html(ANON.request("/people/albatross3000/profile"), "legacy profile")
    require("<script>alert(1)</script>" not in legacy, "profile stored XSS was not sanitized")


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


@test("canonical content routes")
def canonical_routes() -> None:
    for fixture in MANIFEST["topics"]:
        page = topic(fixture["path"])
        require(str(fixture["id"]) in page, f"topic id missing: {fixture['path']}")
    expanded_cut = topic("/news/russia/9101002")
    require('id="cut"' in expanded_cut and "Эта часть должна быть скрыта" in expanded_cut, "topic cut is not expanded")

    news_feed = require_html(ANON.request("/news/"), "news cut preview")
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
    anonymous_home = require_html(ANON.request("/"), "anonymous home page")
    anonymous_card = re.search(
        r'<article class="news" id="topic-9101012">(.*?)</article>',
        anonymous_home,
        re.S,
    )
    require(anonymous_card is not None, "restricted news is absent from home page")
    require(
        "comment-message.jsp?topic=9101012" not in anonymous_card.group(1),
        "anonymous user sees a forbidden add-comment action on home page",
    )
    require(
        "comment-message.jsp?topic=" not in anonymous_home,
        "anonymous user sees an add-comment action in the topic feed",
    )
    anonymous_topic = topic("/forum/games/9101003")
    require(
        re.search(r">\s*Ответить\s*</a>", anonymous_topic) is None,
        "anonymous user sees a reply action on the topic page",
    )
    require(
        'id="commentForm"' not in anonymous_topic,
        "anonymous user receives the inline comment form",
    )
    require(
        "/js/add-form.js" not in anonymous_topic,
        "anonymous topic page loads the reply-form script",
    )

    registered = Client(BASE)
    registered.login("lark70")
    registered_home = require_html(registered.request("/"), "registered home page")
    registered_card = re.search(
        r'<article class="news" id="topic-9101012">(.*?)</article>',
        registered_home,
        re.S,
    )
    require(registered_card is not None, "restricted news is absent for registered viewer")
    require(
        'href="/news/opensource/9101012#comments"' in registered_card.group(1)
        or "comment-message.jsp?topic=9101012" in registered_card.group(1),
        "eligible registered user does not see the news comment action",
    )
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


@test("comment forms, preview and reply target validation")
def comment_form_contracts() -> None:
    client = Client(BASE)
    client.login("crane2000")

    topic_form = require_html(
        client.request("/comment-message.jsp?topic=9101003"),
        "top-level comment form",
    )
    require('id="topic-9101003"' in topic_form, "top-level form omits topic context")
    require(
        'name="replyto" value="0"' in topic_form,
        "top-level form does not preserve the original zero reply target",
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
    require("Править" in own_card.group(1) and "Удалить" in own_card.group(1), "author actions are absent")
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
    require("/js/add-form.js" not in page, "anonymous page loads authenticated reply controls")
    require("/login.jsp?from=" in page, "header login link does not preserve current URL")
    require(
        re.search(r'>oriole300</a>\s*<span class="stars">★★★</span>', page) is not None,
        "topic author score stars are absent",
    )
    require(
        re.search(r'>crane2000</a>\s*<span class="stars">★★★★★</span>', page) is not None,
        "comment author score stars are absent",
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
    own_corrector.login("tern_corrector")
    require(own_corrector.request("/commit.jsp?msgid=9101012").status == 403, "corrector can commit own topic")

    other_corrector = Client(BASE)
    other_corrector.login("ibis_corrector")
    commit_form = other_corrector.request("/commit.jsp?msgid=9101001")
    require(commit_form.status == 200 and 'action="/commit.jsp"' in commit_form.text, "corrector cannot review another author")

    moderator = Client(BASE)
    moderator.login("hawk_moderator")
    require(moderator.request("/commit.jsp?msgid=9101001").status == 200, "moderator cannot open commit form")
    require(moderator.request("/groupmod.jsp?group=126").status == 200, "moderator cannot edit group")
    require(ordinary.request("/groupmod.jsp?group=126").status == 403, "ordinary user can edit group")


@test("group list uses original compact DOM")
def group_dom() -> None:
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

    stale = db(
        "SELECT count(*) FROM groups g WHERE g.section=2 AND g.stat3<>0 "
        "AND NOT EXISTS(SELECT 1 FROM topics t WHERE t.groupid=g.id AND NOT t.deleted "
        "AND (t.postdate>CURRENT_TIMESTAMP-'1 day'::interval "
        "OR (t.lastmod>CURRENT_TIMESTAMP-'2 days'::interval AND t.stat3>0)))"
    )
    require(stale == "0", "forum index contains activity counters with no displayable content")


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
    require("/gallery/archive/2026/8/" in gallery, "archive month URL has no canonical trailing slash")
    require("Неподтверждённые: 1" in gallery, "archive misses premoderation count")
    require("(36)" not in gallery, "uncommitted gallery topics leaked into archive statistics")

    forum = require_html(ANON.request("/forum/games/archive/"), "forum archive")
    require('href="/forum/games?lastmod=true"' in forum, "forum archive active tab is absent")
    require('name="group" value="games"' in forum, "forum archive search group is absent")
    require("/forum/games/2026/8/" in forum, "forum archive month URL differs")


@test("public page and form DOM contracts")
def public_page_contracts() -> None:
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

    rules = require_html(ANON.request("/help/rules.md"), "forum rules")
    require("Правил" in rules, "forum rules content is absent")
    require('rel="search" title="Search L.O.R." href="/search.jsp"' in rules, "global search relation is absent")
    require(re.search(r'<base href="https?://[^\"]+:8181/">', rules) is not None, "original-compatible base URL is absent")
    require('href="/help/markdown.md">Разметка Markdown</a>' in rules, "anonymous footer markup help is absent")


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
