#!/usr/bin/env python3
"""Stateful migration-regression checks against a disposable Java-schema DB.

The caller must explicitly opt into mutations and provide two pre-created test
accounts. CI creates them in its throw-away Compose volume; this script never
silently writes to an operator database.
"""

from __future__ import annotations

import json
import html
import os
import re
import subprocess
import struct
import sys
import time
import urllib.parse
import urllib.error
import urllib.request
import zlib

from stateful_database import psql_target
from test_http_compat import HttpClient, response_value
from write_flow_html import visible_text, visible_topic_title


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def post(client: HttpClient, path: str, values: list[tuple[str, str]]):
    return client.request(path, "POST", urllib.parse.urlencode(values))


def png(width: int, height: int, rgba: tuple[int, int, int, int]) -> bytes:
    def chunk(kind: bytes, payload: bytes) -> bytes:
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))

    rows = b"".join(b"\x00" + bytes(rgba) * width for _ in range(height))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows, 9))
        + chunk(b"IEND", b"")
    )


def post_multipart(
    client: HttpClient,
    path: str,
    values: list[tuple[str, str]],
    files: list[tuple[str, str, str, bytes]],
):
    boundary = "----lorsource-compat-" + str(int(time.time() * 1_000_000))
    parts: list[bytes] = []
    values = [*values, ("csrf", client.ensure_csrf())]
    for name, value in values:
        parts.extend(
            [
                f"--{boundary}\r\n".encode(),
                f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode(),
                value.encode(),
                b"\r\n",
            ]
        )
    for name, filename, content_type, payload in files:
        parts.extend(
            [
                f"--{boundary}\r\n".encode(),
                (
                    f'Content-Disposition: form-data; name="{name}"; filename="{filename}"\r\n'
                    f"Content-Type: {content_type}\r\n\r\n"
                ).encode(),
                payload,
                b"\r\n",
            ]
        )
    parts.append(f"--{boundary}--\r\n".encode())
    request = urllib.request.Request(
        urllib.parse.urljoin(client.base, path.lstrip("/")),
        data=b"".join(parts),
        method="POST",
        headers={
            "User-Agent": "lorsource-rust-compat/2",
            "Content-Type": f"multipart/form-data; boundary={boundary}",
        },
    )
    try:
        with client.opener.open(request, timeout=30) as response:
            return response_value(response.status, response.headers, response.read(1_048_576))
    except urllib.error.HTTPError as error:
        return response_value(error.code, error.headers, error.read(1_048_576))


def login(base: str, nick: str, password: str) -> HttpClient:
    client = HttpClient(base)
    response = post(
        client,
        "/login_process",
        [("nick", nick), ("passwd", password), ("redirectUrl", "/forum/")],
    )
    require(response.status == 302, f"login for {nick} returned {response.status}")
    require(client.cookie("remember_me") is not None, f"login for {nick} set no remember_me")
    return client


def text(response) -> str:
    return response.body.decode("utf-8", errors="replace")


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


def selected_radio_value(page: str, name: str) -> str:
    match = re.search(
        rf'<input[^>]*name="{re.escape(name)}"[^>]*value="([^"]+)"[^>]*checked',
        page,
    )
    require(match is not None, f"settings form has no selected {name} value")
    return html.unescape(match.group(1))


def wait_for_topic_interval(last_created_at: float) -> None:
    """Respect AddTopicFloodCache's original 30-second per-IP contract."""
    interval = float(os.environ.get("WRITE_FLOW_POST_INTERVAL_SECONDS", "30.5"))
    remaining = interval - (time.monotonic() - last_created_at)
    if remaining > 0:
        time.sleep(remaining)


def main() -> int:
    if os.environ.get("WRITE_FLOW_ALLOW_MUTATION") != "yes":
        print("WRITE_FLOW_ALLOW_MUTATION=yes is required", file=sys.stderr)
        return 2
    verify_database_target()

    base = os.environ.get("NEW_BASE_URL", "http://127.0.0.1:8181")
    author_nick = os.environ["WRITE_FLOW_AUTHOR_NICK"]
    author_password = os.environ["WRITE_FLOW_AUTHOR_PASSWORD"]
    reactor_nick = os.environ["WRITE_FLOW_REACTOR_NICK"]
    reactor_password = os.environ["WRITE_FLOW_REACTOR_PASSWORD"]
    group_id = os.environ.get("WRITE_FLOW_GROUP_ID", "126")

    author = login(base, author_nick, author_password)
    reactor = login(base, reactor_nick, reactor_password)

    # Exercise every server-side theme against an authenticated session.  The
    # original themes depend on different header DOMs, not just different CSS
    # files, so checking the saved hstore value alone is insufficient.
    reactor_settings_path = f"/people/{urllib.parse.quote(reactor_nick)}/settings"
    settings_form = reactor.request(reactor_settings_path, "GET")
    settings_html = text(settings_form)
    require(settings_form.status == 200, "settings form is unavailable")
    original_settings = {
        name: selected_radio_value(settings_html, name)
        for name in ("style", "topics", "messages", "trackerMode", "avatar", "format_mode")
    }
    checked_settings = [
        name
        for name in (
            "photos",
            "hideAdsense",
            "mainGallery",
            "oldTracker",
            "oldNotifications",
            "reactionNotification",
        )
        if re.search(rf'<input[^>]*name="{name}"[^>]*checked', settings_html)
    ]

    def save_theme(style: str) -> None:
        values = [
            ("style", style),
            ("topics", original_settings["topics"]),
            ("messages", original_settings["messages"]),
            ("trackerMode", original_settings["trackerMode"]),
            ("avatar", original_settings["avatar"]),
            ("format_mode", original_settings["format_mode"]),
            *((name, "on") for name in checked_settings),
        ]
        saved = post(reactor, reactor_settings_path, values)
        require(saved.status == 302, f"saving theme {style} returned {saved.status}")

    theme_contracts = {
        "tango": ('data-style="tango" data-theme="dark"', "/tango/combined.css", 'id="sitetitle"'),
        "tango-light": (
            'data-style="tango-light" data-theme="light"',
            "/tango/combined.css",
            'id="sitetitle"',
        ),
        "tango-auto": (
            'data-style="tango-auto" data-theme="auto"',
            "/tango/combined.css",
            'id="sitetitle"',
        ),
        # Theme.BLACK uses the dedicated head-main.jsp logo on the main page;
        # lor-new.png belongs to the non-main head.jsp variant.
        "black": (
            'data-style="black"',
            "/black/combined.css",
            "/black/lorlogo-try.png",
        ),
        "white2": ('data-style="white2"', "/white2/combined.css", 'id="hdtux"'),
        "waltz": ('data-style="waltz"', "/waltz/combined.css", 'id="sitetitle"'),
        "zomg_ponies": (
            'data-style="zomg_ponies"',
            "/zomg_ponies/combined.css",
            "PONY.ORG.RU",
        ),
    }
    for style, fragments in theme_contracts.items():
        save_theme(style)
        themed_home = reactor.request("/", "GET")
        themed_html = text(themed_home)
        require(
            themed_home.status == 200
            and all(fragment in themed_html for fragment in fragments)
            and '<main id="bd">' in themed_html
            and 'id="ft"' in themed_html,
            f"theme {style} does not expose its original stylesheet/header DOM contract",
        )
    save_theme(original_settings["style"])

    reactor_edit_path = f"/people/{urllib.parse.quote(reactor_nick)}/edit"
    edit_form = reactor.request(reactor_edit_path, "GET")
    edit_form_html = text(edit_form)
    email_match = re.search(r'id="email"[^>]*value="([^"]*)"', edit_form_html)
    require(
        edit_form.status == 200
        and "no-store" in edit_form.cache_control
        and 'name="info"' in edit_form_html
        and 'name="infoMarkup"' in edit_form_html
        and 'name="oldpass"' in edit_form_html
        and email_match is not None,
        "edit-profile form does not match the Java field/cache contract",
    )
    reactor_email = html.unescape(email_match.group(1))
    profile_info = f"**profile compatibility {int(time.time() * 1000)}**"
    profile_saved = post(
        reactor,
        reactor_edit_path,
        [
            ("email", reactor_email),
            ("info", profile_info),
            ("infoMarkup", "markdown"),
            ("oldpass", reactor_password),
        ],
    )
    require(profile_saved.status == 302, f"profile update returned {profile_saved.status}")
    reactor_profile_path = f"/people/{urllib.parse.quote(reactor_nick)}/profile"
    updated_profile = text(reactor.request(reactor_profile_path, "GET"))
    require(
        "<strong>profile compatibility" in updated_profile,
        "profile info or selected Markdown mode was not persisted",
    )
    profile_cleared = post(
        reactor,
        reactor_edit_path,
        [
            ("email", reactor_email),
            ("info", ""),
            ("infoMarkup", "markdown"),
            ("oldpass", reactor_password),
        ],
    )
    require(profile_cleared.status == 302, "profile cleanup failed")

    # Profile-side private state uses the original browser forms: remarks are
    # keyed by `text`, ignore actions by `add`/`del`, and user-filter is HTML
    # with no-store headers rather than the old Rust JSON shortcut.
    author_profile_path = f"/people/{urllib.parse.quote(author_nick)}/profile"
    author_remark_path = f"/people/{urllib.parse.quote(author_nick)}/remark"
    reactor_view_of_author = reactor.request(author_profile_path, "GET")
    reactor_view_html = text(reactor_view_of_author)
    author_id_match = re.search(r"<b>ID:</b>\s*(\d+)", reactor_view_html)
    require(
        reactor_view_of_author.status == 200 and author_id_match is not None,
        "profile does not expose the canonical user ID field",
    )
    author_id = author_id_match.group(1)
    remark_text = f"compat remark {int(time.time() * 1000)}"
    saved_remark = post(
        reactor,
        author_remark_path,
        [("text", remark_text)],
    )
    require(
        saved_remark.status == 302
        and saved_remark.location_target == author_profile_path,
        "remark form did not redirect to the target profile",
    )
    remarked_profile = reactor.request(author_profile_path, "GET")
    require(remark_text in text(remarked_profile), "saved private remark is absent from profile")

    ignored = post(
        reactor,
        "/user-filter/ignore-user",
        [("nick", author_nick), ("add", "")],
    )
    require(
        ignored.status == 302 and ignored.location_target == "/user-filter",
        "ignore-user add did not use the Java browser redirect",
    )
    filter_page = reactor.request("/user-filter", "GET")
    filter_html = text(filter_page)
    require(
        filter_page.status == 200
        and filter_page.content_type.startswith("text/html")
        and "no-store" in filter_page.cache_control
        and author_nick in filter_html,
        "user-filter is not a private non-cacheable HTML page",
    )
    unignored = post(
        reactor,
        "/user-filter/ignore-user",
        [("id", author_id), ("del", "")],
    )
    require(unignored.status == 302, "ignore-user cleanup failed")
    deleted_remark = post(
        reactor,
        author_remark_path,
        [("text", "")],
    )
    require(deleted_remark.status == 302, "remark cleanup failed")
    require(
        remark_text not in text(reactor.request(author_profile_path, "GET")),
        "empty remark did not delete the private note",
    )

    form = author.request(f"/add.jsp?group={group_id}", "GET")
    require(form.status == 200, f"GET /add.jsp returned {form.status}")
    require('name="tags"' in text(form), "topic form has no tags field")

    suffix = str(int(time.time() * 1000))
    title = f"Rust compatibility write flow {suffix} & < > \"quoted\" 'apostrophe'"
    visible_title = (
        f"Rust compatibility write flow {suffix} & < > «quoted» 'apostrophe'"
    )
    stored_title = (
        f"Rust compatibility write flow {suffix} &amp; &lt; &gt; "
        "&quot;quoted&quot; &#39;apostrophe&#39;"
    )
    body = f"Transactional topic body {suffix}"
    created = post(
        author,
        "/add.jsp",
        [
            ("group", group_id),
            ("title", title),
            ("msg", body),
            ("tags", "rust-port-ci, compatibility"),
            ("allowAnonymous", "true"),
        ],
    )
    require(created.status == 303, f"topic creation returned {created.status}: {text(created)[:500]}")
    require(created.location_target is not None, "topic creation has no canonical redirect")
    match = re.fullmatch(r"/forum/[^/]+/(\d+)", created.location_target)
    require(match is not None, f"unexpected topic redirect {created.location_target!r}")
    topic_id = int(match.group(1))
    last_topic_created = time.monotonic()

    topic = author.request(created.location_target, "GET")
    topic_html = text(topic)
    require(topic.status == 200, f"canonical topic returned {topic.status}")
    require(
        visible_topic_title(topic_html, created.location_target) == visible_title,
        "created title is missing or is visibly double-escaped",
    )
    require(body in visible_text(topic_html), "created body is missing")
    require(
        db(f"SELECT title FROM topics WHERE id={topic_id}") == stored_title,
        "topic title bytes differ from Java's HTML-escaped storage contract",
    )
    require('href="/tag/rust-port-ci"' in topic_html, "first comma-separated tag is missing")
    require('href="/tag/compatibility"' in topic_html, "second comma-separated tag is missing")
    require("rust-port-ci%2C" not in topic_html, "comma-separated tags were stored as one tag")

    favorite_tags = post(
        author,
        "/user-filter/favorite-tag",
        [("tagName", "rust-port-ci, compatibility"), ("add", "")],
    )
    require(
        favorite_tags.status == 302 and favorite_tags.location_target == "/user-filter",
        "HTML favorite-tag form did not split and redirect",
    )
    favorite_filter = text(author.request("/user-filter", "GET"))
    require(
        "rust-port-ci" in favorite_filter and "compatibility" in favorite_filter,
        "comma-separated favorite tags were not stored separately",
    )
    for tag_name in ("rust-port-ci", "compatibility"):
        deleted_favorite = post(
            author,
            "/user-filter/favorite-tag",
            [("tagName", tag_name), ("del", "")],
        )
        require(deleted_favorite.status == 302, f"favorite tag cleanup failed for {tag_name}")

    # Java commits first and sends a persistent ActiveMQ message. Rust uses a
    # persistent filesystem spool with the same asynchronous contract. Allow
    # both the worker and OpenSearch's refresh interval to elapse, then prove
    # the committed write becomes searchable.
    search_path = "/search.jsp?" + urllib.parse.urlencode({"q": title})
    search_html = ""
    for _ in range(20):
        search_response = author.request(search_path, "GET")
        search_html = text(search_response)
        if search_response.status == 200 and visible_title in visible_text(search_html):
            break
        time.sleep(1)
    require(
        visible_title in visible_text(search_html),
        "durable search queue did not index the created topic",
    )

    reactor_view = reactor.request(created.location_target, "GET")
    reactor_html = text(reactor_view)
    require('class="reaction-show"' in reactor_html, "empty reaction picker has no reveal control")
    require('value="🎉-true"' in reactor_html, "reaction choices are not rendered")

    reacted = post(
        reactor,
        "/reactions/ajax",
        [("topic", str(topic_id)), ("reaction", "🎉-true")],
    )
    require(reacted.status == 200, f"adding topic reaction returned {reacted.status}: {text(reacted)}")
    require(json.loads(text(reacted)) == {"count": 1}, "reaction count did not become one")

    reacted_view = reactor.request(created.location_target, "GET")
    reacted_html = text(reacted_view)
    require('value="🎉-false"' in reacted_html, "selected reaction is not rendered as removable")
    require('class="reaction reaction-show-list"' in reacted_html, "reaction author-list link is missing")
    require('class="reaction reaction-show"' in reacted_html, "zero-reaction hide/reveal control is missing")

    reaction_list = reactor.request(f"/reactions?topic={topic_id}", "GET")
    require(reaction_list.status == 200, f"reaction list returned {reaction_list.status}")
    reaction_list_html = text(reaction_list)
    require(reactor_nick in reaction_list_html, "authoritative reaction list omits reactor")
    require(
        "<!doctype html>" in reaction_list_html
        and "<title>Реакция на сообщение</title>" in reaction_list_html
        and f'id="topic-{topic_id}"' in reaction_list_html
        and 'class="reactions-form" action="/reactions" method="POST"' in reaction_list_html
        and f'class="btn btn-primary" href="{created.location_target}"' in reaction_list_html,
        "topic reaction GET is not the original full-page topic/reactions view",
    )
    anonymous_reaction = HttpClient(base).request(f"/reactions?topic={topic_id}", "GET")
    require(
        anonymous_reaction.status == 302
        and anonymous_reaction.location_target == created.location_target,
        "anonymous topic reaction GET does not use the original 302 canonical redirect",
    )
    msgid_alias = HttpClient(base).request(f"/reactions?msgid={topic_id}", "GET")
    require(msgid_alias.status == 400, "non-original msgid alias is accepted by reaction GET")

    notification_feed = author.request(
        "/show-replies.jsp?" + urllib.parse.urlencode({"output": "rss", "nick": author_nick}),
        "GET",
    )
    notification_feed_xml = text(notification_feed)
    require(
        notification_feed.status == 200
        and notification_feed.content_type.startswith("application/rss+xml")
        and f"Уведомления пользователя {author_nick}" in notification_feed_xml
        and f"@{reactor_nick} поставил 🎉" in notification_feed_xml
        and body in notification_feed_xml,
        "notification RSS omits Java-compatible reaction note or rendered message body",
    )

    notifications = author.request("/notifications?filter=reaction", "GET")
    notifications_html = text(notifications)
    require(notifications.status == 200, f"reaction notifications returned {notifications.status}")
    require("no-store" in notifications.cache_control, "notifications response is cacheable")
    require(
        visible_title in visible_text(notifications_html),
        "reaction notification omits topic title",
    )
    require(reactor_nick in notifications_html and "🎉" in notifications_html,
            "reaction notification omits current reactor/reaction")
    click_ids = re.search(
        r'name="firstId" value="(\d+)".*?name="lastId" value="(\d+)"',
        notifications_html,
        re.S,
    )
    require(click_ids is not None, "reaction notification has no grouped click range")
    clicked = post(
        author,
        "/notifications-click",
        [("firstId", click_ids.group(1)), ("lastId", click_ids.group(2))],
    )
    require(clicked.status == 302, f"reaction notification click returned {clicked.status}")

    comment = post(
        reactor,
        "/add_comment_ajax",
        [("topic", str(topic_id)), ("replyto", "0"), ("msg", f"Comment body {suffix}")],
    )
    require(comment.status == 200, f"comment creation returned {comment.status}: {text(comment)}")
    comment_payload = json.loads(text(comment))
    require("url" in comment_payload and f"/forum/" in comment_payload["url"], "comment URL is not canonical")
    # Java returns topic.getLink + "?cid=<id>". The browser then follows the
    # controller's page-aware jump to the canonical topic URL with an anchor;
    # HttpClient deliberately disables redirects, so reproduce that one hop.
    comment_jump = author.request(comment_payload["url"], "GET")
    require(comment_jump.status == 302, f"comment jump returned {comment_jump.status}")
    require(comment_jump.location_target is not None, "comment jump has no target")
    comment_page = author.request(comment_jump.location_target, "GET")
    require(comment_page.status == 200 and f"Comment body {suffix}" in text(comment_page), "created comment is missing")

    comment_id_match = re.search(r"[?&]cid=(\d+)", comment_payload["url"])
    require(comment_id_match is not None, "comment URL omits the Java cid parameter")
    comment_id = comment_id_match.group(1)
    comment_reactions = author.request(
        f"/reactions?topic=1&comment={comment_id}", "GET"
    )
    comment_reactions_html = text(comment_reactions)
    require(
        comment_reactions.status == 200
        and "<title>Реакция на комментарий</title>" in comment_reactions_html
        and f'id="comment-{comment_id}"' in comment_reactions_html
        and f"Comment body {suffix}" in comment_reactions_html
        and f'name="comment" value="{comment_id}"' in comment_reactions_html,
        "comment reaction GET does not reproduce reaction-comment.jsp",
    )
    anonymous_comment_reaction = HttpClient(base).request(
        f"/reactions?comment={comment_id}", "GET"
    )
    require(
        anonymous_comment_reaction.status == 302
        and anonymous_comment_reaction.location_target == comment_payload["url"],
        "anonymous comment reaction GET does not use the original 302 cid redirect",
    )
    tracker = author.request("/tracker/?filter=all", "GET")
    tracker_html = text(tracker)
    require(
        tracker.status == 200 and visible_title in visible_text(tracker_html),
        "created topic is absent from tracker",
    )
    tracker_item = re.search(
        rf'<a href="([^"]*lastmod={comment_id_match.group(1)})" class="tracker-item">(.*?)</a>',
        tracker_html,
        re.S,
    )
    require(tracker_item is not None, "tracker does not link to the last visible comment")
    require(
        reactor_nick in tracker_item.group(2),
        "tracker shows the topic author instead of the last-comment author",
    )

    anonymous = HttpClient(base)
    legacy_tracker = anonymous.request("/tracker.jsp", "GET")
    require(
        legacy_tracker.status == 302
        and legacy_tracker.location_target == "/tracker/?filter=all",
        f"anonymous legacy tracker redirect differs from Java: {legacy_tracker.location_target!r}",
    )

    removed = post(
        reactor,
        "/reactions/ajax",
        [("topic", str(topic_id)), ("reaction", "🎉-false")],
    )
    require(removed.status == 200 and json.loads(text(removed)) == {"count": 0}, "reaction removal failed")
    after_removal = author.request("/notifications?filter=reaction", "GET")
    require(after_removal.status == 200, "notifications failed after reaction removal")
    require(
        visible_title not in visible_text(text(after_removal)),
        "read notification for a removed reaction remains visible",
    )

    # Gallery has two distinct original DOM modes: a responsive single image
    # and a Swiffy slider for several images. Exercise the real multipart
    # pipeline and the Java FIT_TO_WIDTH derivative layout for both.
    image = png(400, 800, (0, 128, 255, 128))
    gallery_topics: list[int] = []
    protected_gallery_topic: int | None = None
    protected_gallery_image: str | None = None
    for image_count, expected_fragment, forbidden_fragment in [
        (1, 'class="medium-image-container"', 'class="swiffy-slider'),
        (2, 'class="swiffy-slider', 'class="medium-image-container"'),
    ]:
        wait_for_topic_interval(last_topic_created)
        gallery_title = f"Rust gallery {image_count} image mode {suffix}"
        gallery_values = [
            ("group", os.environ.get("WRITE_FLOW_GALLERY_GROUP_ID", "4962")),
            ("title", gallery_title),
            ("msg", f"Gallery body {image_count} {suffix}"),
            ("tags", f"rust-gallery-{suffix}-{image_count}"),
        ]
        gallery_files = [
            ("images", f"image-{index}.png", "image/png", image)
            for index in range(image_count)
        ]
        if image_count == 1:
            preview = post_multipart(
                author,
                "/add.jsp",
                [*gallery_values, ("preview", "")],
                gallery_files,
            )
            preview_html = text(preview)
            require(preview.status == 200, f"gallery preview returned {preview.status}")
            hidden = re.search(
                r'name="uploadedImages\[0\]" value="([\w.-]+)"', preview_html
            )
            require(hidden is not None, "gallery preview has no reusable hidden filename")
            preview_url = f"/gallery/preview/{hidden.group(1)}"
            anonymous_preview = HttpClient(base).request(preview_url, "GET")
            require(
                anonymous_preview.status == 403,
                "anonymous user can read a staged gallery preview",
            )
            preview_image = author.request(preview_url, "GET")
            require(
                preview_image.status == 200 and preview_image.content_type.startswith("image/"),
                "staged gallery preview is not served",
            )
            gallery_values.append(("uploadedImages[0]", hidden.group(1)))
            gallery_files = []
        gallery = post_multipart(
            author,
            "/add.jsp",
            gallery_values,
            gallery_files,
        )
        require(gallery.status == 200, f"gallery creation returned {gallery.status}: {text(gallery)[:500]}")
        link = re.search(r'href="(/gallery/[^/]+/(\d+))"', text(gallery))
        require(
            link is not None,
            f"moderated gallery result has no canonical topic link: {text(gallery)[:5000]}",
        )
        gallery_topics.append(int(link.group(2)))
        last_topic_created = time.monotonic()
        gallery_page = author.request(link.group(1), "GET")
        gallery_html = text(gallery_page)
        require(
            gallery_page.status == 200 and gallery_title in gallery_html,
            "created gallery topic is missing",
        )
        require(expected_fragment in gallery_html, f"gallery {image_count}-image DOM mode is missing")
        require(forbidden_fragment not in gallery_html, f"gallery {image_count}-image DOM modes are mixed")
        image_urls = set(re.findall(r'/images/\d+/1000px\.jpg', gallery_html))
        require(len(image_urls) == image_count, f"gallery rendered {len(image_urls)} of {image_count} images")
        for image_url in image_urls:
            derivative = author.request(image_url, "GET")
            require(
                derivative.status == 200 and derivative.content_type.startswith("image/jpeg"),
                f"gallery derivative {image_url} is not served",
            )
        if image_count == 1:
            protected_gallery_topic = int(link.group(2))
            protected_gallery_image = next(iter(image_urls))

    require(protected_gallery_topic is not None, "protected gallery topic was not recorded")
    require(protected_gallery_image is not None, "protected gallery image was not recorded")
    public_image = HttpClient(base).request(protected_gallery_image, "GET")
    require(
        public_image.status == 200 and public_image.content_type.startswith("image/jpeg"),
        "visible gallery image is not public",
    )
    deleted_gallery = post(
        author,
        "/delete.jsp",
        [("msgid", str(protected_gallery_topic)), ("reason", "media visibility regression")],
    )
    require(deleted_gallery.status == 200, f"gallery deletion returned {deleted_gallery.status}")
    hidden_image = HttpClient(base).request(protected_gallery_image, "GET")
    require(
        hidden_image.status == 403,
        "anonymous direct URL exposed an image from a deleted topic",
    )
    author_history_image = author.request(protected_gallery_image, "GET")
    require(
        author_history_image.status == 200,
        "topic author cannot access an image from their recently deleted topic",
    )

    print(
        f"write flow passed: topic={topic_id} gallery={','.join(map(str, gallery_topics))} "
        f"redirect={created.location_target}"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, KeyError) as error:
        print(f"write flow failed: {error}", file=sys.stderr)
        raise SystemExit(1)
