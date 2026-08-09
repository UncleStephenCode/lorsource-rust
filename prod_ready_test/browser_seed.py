#!/usr/bin/env python3
"""Create the 24-hour LOR fixture through the real browser/UI workflows."""

from __future__ import annotations

import argparse
import io
import json
import re
import shutil
import sys
import time
from pathlib import Path
from urllib.parse import urljoin, urlparse


HERE = Path(__file__).resolve().parent
CONTENT = json.loads((HERE / "browser_content.json").read_text("utf-8"))
PASSWORD = "Birds-ProdReady-2026"
TOPIC_INTERVAL_SECONDS = 31
COMMENT_INTERVAL_SECONDS = 4


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def image_payload(index: int) -> dict[str, object]:
    try:
        from PIL import Image, ImageDraw
    except ImportError as error:
        raise RuntimeError("Pillow is required for browser gallery uploads") from error

    colors = ((32, 74, 113), (80, 43, 99), (35, 105, 73), (121, 68, 31))
    image = Image.new("RGB", (1280, 720), colors[index % len(colors)])
    draw = ImageDraw.Draw(image)
    draw.rectangle((55, 55, 1225, 665), fill=(25, 30, 33), outline=(114, 159, 207), width=5)
    draw.text((95, 100), f"LOR browser seed / image {index + 1}", fill=(238, 238, 236))
    draw.text((95, 145), "OpenSUSE / ARM64 / production compatibility", fill=(138, 226, 52))
    buffer = io.BytesIO()
    image.save(buffer, format="PNG", optimize=True)
    return {
        "name": f"lor-browser-seed-{index + 1}.png",
        "mimeType": "image/png",
        "buffer": buffer.getvalue(),
    }


class BrowserSeed:
    def __init__(self, playwright, base: str, headed: bool, output: Path) -> None:
        executable = next(
            (
                path
                for name in ("google-chrome", "google-chrome-stable", "chromium")
                if (path := shutil.which(name))
            ),
            None,
        )
        require(executable is not None, "Chrome/Chromium is not installed")
        self.base = base.rstrip("/") + "/"
        self.output = output
        self.output.mkdir(parents=True, exist_ok=True)
        self.browser = playwright.chromium.launch(
            executable_path=executable,
            headless=not headed,
            args=["--no-sandbox", "--disable-dev-shm-usage", "--disable-background-networking"],
        )
        self.contexts = {}
        self.pages = {}
        self.topics: dict[str, dict[str, object]] = {}

    def close(self) -> None:
        self.browser.close()

    def page_for(self, nick: str):
        if nick in self.pages:
            return self.pages[nick]
        context = self.browser.new_context(
            viewport={"width": 1440, "height": 1100},
            locale="ru-RU",
            timezone_id="Europe/Moscow",
        )
        page = context.new_page()
        page.set_default_timeout(30_000)
        page.set_default_navigation_timeout(45_000)
        self.contexts[nick] = context
        self.pages[nick] = page
        self.login(page, nick)
        return page

    def absolute(self, path: str) -> str:
        return urljoin(self.base, path.lstrip("/"))

    def goto(self, page, path: str) -> None:
        from playwright.sync_api import Error as PlaywrightError

        target = self.absolute(path)
        preflight = page.context.request.get(target, fail_on_status_code=False)
        require(preflight.status < 500, f"{path}: HTTP {preflight.status}")
        response = None
        for attempt in range(3):
            try:
                response = page.goto(target, wait_until="domcontentloaded")
                break
            except PlaywrightError as error:
                if "ERR_ABORTED" not in str(error) or attempt == 2:
                    raise
                page.wait_for_timeout(750)
        if response is None:
            final_path = urlparse(page.url).path.rstrip("/") or "/"
            target_path = urlparse(target).path.rstrip("/") or "/"
            require(
                final_path == target_path,
                f"no browser response for {path}; final URL is {page.url}",
            )
        else:
            require(response.status < 500, f"{path}: HTTP {response.status}")

    def login(self, page, nick: str) -> None:
        self.goto(page, "/login.jsp?from=/forum/")
        base_href = page.locator("base").get_attribute("href")
        if base_href:
            canonical = urlparse(urljoin(page.url, base_href))
            current = urlparse(self.base)
            if (canonical.scheme, canonical.netloc) != (current.scheme, current.netloc):
                self.base = f"{canonical.scheme}://{canonical.netloc}/"
                self.goto(page, "/login.jsp?from=/forum/")
        form = page.locator('form:has(input[name="nick"]):has(input[name="passwd"])')
        require(form.count() == 1, "login form is absent")
        form.locator('input[name="nick"]').fill(nick)
        form.locator('input[name="passwd"]').fill(PASSWORD)
        form.locator('button[type="submit"]').click()
        page.wait_for_load_state("domcontentloaded")
        page_text = page.locator("body").inner_text()
        require(
            page.locator(f'a[href="/people/{nick}/profile"]').count() > 0,
            f"browser login failed for {nick}: {page.url}; {page_text[:500]!r}",
        )

    def discover_add_url(self, page, group_path: str) -> str:
        self.goto(page, group_path)
        links = page.locator('a[href^="/add.jsp?group="]')
        require(links.count() > 0, f"group page has no Add link: {group_path}")
        href = links.first.get_attribute("href")
        require(bool(href), f"empty Add link: {group_path}")
        return str(href)

    def create_topic(self, page, item: dict[str, object], image_offset: int) -> dict[str, object]:
        add_url = self.discover_add_url(page, str(item["group_path"]))
        self.goto(page, add_url)
        form = page.locator("#messageForm")
        require(form.count() == 1, f"topic form is absent for {item['key']}")
        form.locator('input[name="title"]').fill(str(item["title"]))
        form.locator('textarea[name="msg"]').fill(str(item["body"]))
        form.locator('input[name="tags"]').fill(str(item["tags"]))
        if item.get("url"):
            form.locator('input[name="url"]').fill(str(item["url"]))
            form.locator('input[name="linktext"]').fill(str(item.get("linktext", "Источник")))
        image_count = int(item.get("images", 0))
        image_inputs = form.locator('input[type="file"][name="images"]')
        require(
            image_inputs.count() >= image_count,
            f"{item['key']}: expected {image_count} image inputs, got {image_inputs.count()}",
        )
        for index in range(image_count):
            image_inputs.nth(index).set_input_files(image_payload(image_offset + index))
        form.locator("button.btn-primary").click()
        page.wait_for_load_state("domcontentloaded")
        error = page.locator(".error")
        require(error.count() == 0, f"{item['key']}: {error.all_inner_texts()}")
        match = re.search(r"/(news|gallery)/[^/]+/(\d+)", urlparse(page.url).path)
        if match is None:
            confirmation_link = page.get_by_role("link", name="Перейти к сообщению")
            if confirmation_link.count() == 1:
                confirmation_link.click()
                page.wait_for_load_state("domcontentloaded")
                match = re.search(r"/(news|gallery)/[^/]+/(\d+)", urlparse(page.url).path)
        page_text = page.locator("body").inner_text()
        require(
            match is not None,
            f"{item['key']}: unexpected create redirect {page.url}; {page_text[:1000]!r}",
        )
        topic_id = int(match.group(2))
        return {
            "key": item["key"],
            "id": topic_id,
            "url": urlparse(page.url).path,
            "title": item["title"],
            "commit": bool(item["commit"]),
        }

    def commit_topic(self, page, topic: dict[str, object]) -> None:
        self.goto(page, f"/commit.jsp?msgid={topic['id']}")
        form = page.locator('form[action="/commit.jsp"]')
        require(form.count() == 1, f"commit form is absent for topic {topic['id']}")
        form.locator('button[type="submit"]').click()
        page.wait_for_load_state("domcontentloaded")
        require("jump-message.jsp" not in page.url, f"commit redirect was not resolved: {page.url}")

    def add_comment(self, page, topic: dict[str, object], body: str) -> str:
        self.goto(page, f"/comment-message.jsp?topic={topic['id']}")
        form = page.locator('#commentForm[action="/add_comment.jsp"]')
        require(form.count() == 1, f"comment form is absent for topic {topic['id']}")
        form.locator('textarea[name="msg"]').fill(body)
        with page.expect_response(
            lambda response: "/add_comment_ajax" in response.url
        ) as response_info:
            form.locator("button.btn-primary").click()
        require(
            response_info.value.ok,
            f"comment AJAX failed: HTTP {response_info.value.status}",
        )
        page.wait_for_url(re.compile(rf"{re.escape(str(topic['url']))}\?cid=\d+$"))
        return page.url

    def add_reaction(self, page, topic: dict[str, object]) -> None:
        self.goto(page, str(topic["url"]))
        button = page.locator('form.reactions-form button[name="reaction"][value^="👍-true"]').first
        require(button.count() == 1, f"reaction button is absent for topic {topic['id']}")
        require(not button.is_visible(), "empty reaction choices must initially be hidden")
        reveal = page.locator("#topicMenu a.reaction-show")
        require(reveal.count() == 1, f"reaction reveal link is absent for topic {topic['id']}")
        page.wait_for_function("window.jQuery && typeof window.jQuery === 'function'")
        page.wait_for_timeout(500)
        reveal.click()
        button.wait_for(state="visible")
        with page.expect_response(lambda response: "/reactions/ajax" in response.url) as response_info:
            button.click()
        require(response_info.value.ok, f"reaction AJAX failed: HTTP {response_info.value.status}")
        selected = page.locator(
            'form.reactions-form button[name="reaction"][value^="👍-false"]'
        ).first
        selected.wait_for(state="visible")
        require(
            selected.locator(".reaction-count").inner_text() == "1",
            "reaction count was not updated after AJAX submit",
        )

    def wait_for_comment_history(self, page, expected_bodies: list[str]) -> None:
        path = "/search.jsp?range=COMMENTS&user=crane2000&sort=DATE"
        for _ in range(30):
            self.goto(page, path)
            text = page.locator("body").inner_text()
            if all(body in text for body in expected_bodies):
                return
            time.sleep(1)
        raise RuntimeError("browser-created comments did not appear in OpenSearch history")

    def verify_histories(self, author_page, commenter_page) -> None:
        self.goto(author_page, "/people/raven1000/")
        body = author_page.locator("body").inner_text()
        for topic in self.topics.values():
            require(str(topic["title"]) in body, f"author history misses {topic['key']}")
        require("(не подтверждено)" in body, "pending news marker is absent from author history")
        for section in ("1", "3"):
            require(
                author_page.locator(f'a[href="/people/raven1000/?section={section}"]').count() == 1,
                f"author history section {section} is absent",
            )
        require('href="/tag/%D0%BE%D1%84%D0%B8%D1%81%D0%BD%D0%BE%D0%B5%20%D0%BF%D0%BE"' in author_page.content(), "encoded profile tag URL is absent")
        author_page.screenshot(path=self.output / "author-topics-desktop.png", full_page=True)

        self.goto(author_page, "/people/raven1000/?section=3")
        filtered = author_page.locator("body").inner_text()
        require("Atomic Heart" in filtered, "gallery filter misses gallery topic")
        require("Wine 11.15" not in filtered, "gallery filter leaks news topics")

        expected = [str(item["body"]) for item in CONTENT["comments"]]
        self.wait_for_comment_history(commenter_page, expected)
        search_body = commenter_page.locator("body").inner_text()
        require("Всего найдено 4" in search_body, "comment history count differs")
        commenter_page.screenshot(path=self.output / "comment-history-desktop.png", full_page=True)

        author_page.set_viewport_size({"width": 390, "height": 844})
        self.goto(author_page, "/people/raven1000/")
        author_page.screenshot(path=self.output / "author-topics-mobile.png", full_page=True)
        commenter_page.set_viewport_size({"width": 390, "height": 844})
        self.goto(commenter_page, "/search.jsp?range=COMMENTS&user=crane2000&sort=DATE")
        commenter_page.screenshot(path=self.output / "comment-history-mobile.png", full_page=True)

    def verify_archive_comments_and_forum(self, page) -> None:
        gallery = self.topics["atomic-gallery"]
        self.goto(page, str(gallery["url"]))
        comment = page.locator("#comments article.msg").first
        require(comment.count() == 1, "browser-created gallery comment is absent")
        require(comment.locator(".userpic img.photo").count() == 1, "comment userpic column is absent")
        require(comment.locator(".msg_body.message-w-userpic").count() == 1, "comment body does not reserve avatar space")
        require(page.locator("#comments > h2").count() == 0, "non-original comments heading is present")
        page.screenshot(path=self.output / "topic-comments-desktop.png", full_page=True)

        self.goto(page, "/gallery/archive/")
        require(page.locator('form[action="/search.jsp"]').count() == 1, "archive search form is absent")
        require(page.locator(f'a[href="/gallery/archive/2026/8/"]').count() == 1, "current archive month is absent")
        page.screenshot(path=self.output / "gallery-archive-desktop.png", full_page=True)

        self.goto(page, "/forum/")
        forum_text = page.locator("body").inner_text()
        for stale_group in ("Lor-source (11 за сутки)", "Mobile (16 за сутки)", "Multimedia (7 за сутки)"):
            require(stale_group not in forum_text, f"stale forum counter remains: {stale_group}")

    def run(self) -> dict[str, object]:
        author_page = self.page_for(str(CONTENT["author"]))
        moderator_page = self.page_for(str(CONTENT["moderator"]))
        commenter_page = self.page_for(str(CONTENT["commenter"]))

        image_offset = 0
        for index, item in enumerate(CONTENT["topics"]):
            if index:
                print(f"WAIT topic flood interval {TOPIC_INTERVAL_SECONDS}s", flush=True)
                time.sleep(TOPIC_INTERVAL_SECONDS)
            topic = self.create_topic(author_page, item, image_offset)
            self.topics[str(item["key"])] = topic
            image_offset += int(item.get("images", 0))
            print(f"CREATED {topic['key']} {topic['url']}", flush=True)

        for topic in self.topics.values():
            if topic["commit"]:
                self.commit_topic(moderator_page, topic)
                print(f"COMMITTED {topic['key']}")

        comment_urls = []
        for index, item in enumerate(CONTENT["comments"]):
            if index:
                print(f"WAIT comment flood interval {COMMENT_INTERVAL_SECONDS}s", flush=True)
                time.sleep(COMMENT_INTERVAL_SECONDS)
            topic = self.topics[str(item["topic"])]
            comment_urls.append(self.add_comment(commenter_page, topic, str(item["body"])))
            print(f"COMMENTED {topic['key']}", flush=True)

        self.add_reaction(commenter_page, self.topics["polychromatic"])
        print("REACTED polychromatic 👍")
        self.verify_archive_comments_and_forum(author_page)
        self.verify_histories(author_page, commenter_page)
        return {
            "source_window_hours": CONTENT["source_window_hours"],
            "topics": list(self.topics.values()),
            "comment_urls": comment_urls,
            "artifacts": str(self.output),
        }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:8181")
    parser.add_argument("--headed", action="store_true")
    parser.add_argument("--output", type=Path, default=Path("/tmp/prod_ready_browser_seed"))
    parser.add_argument(
        "--result",
        type=Path,
        default=Path("/tmp/prod_ready_browser_seed_result.json"),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print(
            "Playwright is required: python3 -m pip install -r "
            "prod_ready_test/requirements-browser.txt",
            file=sys.stderr,
        )
        return 2

    with sync_playwright() as playwright:
        seed = BrowserSeed(playwright, args.base, args.headed, args.output.resolve())
        try:
            result = seed.run()
        finally:
            seed.close()
    args.result.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", "utf-8")
    print(f"browser seed result: {args.result}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(f"browser seed failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
