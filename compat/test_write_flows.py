#!/usr/bin/env python3
"""Stateful migration-regression checks against a disposable Java-schema DB.

The caller must explicitly opt into mutations and provide two pre-created test
accounts. CI creates them in its throw-away Compose volume; this script never
silently writes to an operator database.
"""

from __future__ import annotations

import json
import os
import re
import struct
import sys
import time
import urllib.parse
import urllib.error
import urllib.request
import zlib

from test_http_compat import HttpClient, response_value


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

    base = os.environ.get("NEW_BASE_URL", "http://127.0.0.1:8181")
    author_nick = os.environ["WRITE_FLOW_AUTHOR_NICK"]
    author_password = os.environ["WRITE_FLOW_AUTHOR_PASSWORD"]
    reactor_nick = os.environ["WRITE_FLOW_REACTOR_NICK"]
    reactor_password = os.environ["WRITE_FLOW_REACTOR_PASSWORD"]
    group_id = os.environ.get("WRITE_FLOW_GROUP_ID", "126")

    author = login(base, author_nick, author_password)
    reactor = login(base, reactor_nick, reactor_password)

    form = author.request(f"/add.jsp?group={group_id}", "GET")
    require(form.status == 200, f"GET /add.jsp returned {form.status}")
    require('name="tags"' in text(form), "topic form has no tags field")

    suffix = str(int(time.time() * 1000))
    title = f"Rust compatibility write flow {suffix}"
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
    require(title in topic_html and body in topic_html, "created title/body are missing")
    require('href="/tag/rust-port-ci"' in topic_html, "first comma-separated tag is missing")
    require('href="/tag/compatibility"' in topic_html, "second comma-separated tag is missing")
    require("rust-port-ci%2C" not in topic_html, "comma-separated tags were stored as one tag")

    # Java commits first and sends a persistent ActiveMQ message. Rust uses a
    # persistent filesystem spool with the same asynchronous contract. Allow
    # both the worker and OpenSearch's refresh interval to elapse, then prove
    # the committed write becomes searchable.
    search_path = "/search.jsp?" + urllib.parse.urlencode({"q": title})
    search_html = ""
    for _ in range(20):
        search_response = author.request(search_path, "GET")
        search_html = text(search_response)
        if search_response.status == 200 and title in search_html:
            break
        time.sleep(1)
    require(title in search_html, "durable search queue did not index the created topic")

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
    require(reactor_nick in text(reaction_list), "authoritative reaction list omits reactor")

    notifications = author.request("/notifications?filter=reaction", "GET")
    notifications_html = text(notifications)
    require(notifications.status == 200, f"reaction notifications returned {notifications.status}")
    require("no-store" in notifications.cache_control, "notifications response is cacheable")
    require(title in notifications_html, "reaction notification omits topic title")
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
        author,
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

    removed = post(
        reactor,
        "/reactions/ajax",
        [("topic", str(topic_id)), ("reaction", "🎉-false")],
    )
    require(removed.status == 200 and json.loads(text(removed)) == {"count": 0}, "reaction removal failed")
    after_removal = author.request("/notifications?filter=reaction", "GET")
    require(after_removal.status == 200, "notifications failed after reaction removal")
    require(title not in text(after_removal),
            "read notification for a removed reaction remains visible")

    # Gallery has two distinct original DOM modes: a responsive single image
    # and a Swiffy slider for several images. Exercise the real multipart
    # pipeline and the Java FIT_TO_WIDTH derivative layout for both.
    image = png(400, 800, (0, 128, 255, 128))
    gallery_topics: list[int] = []
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
            preview_url = f"/gallery-uploads/preview/{hidden.group(1)}"
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
