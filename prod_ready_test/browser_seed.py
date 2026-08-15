#!/usr/bin/env python3
"""Create and benchmark the seven-day LOR fixture through real browser workflows."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
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
CHECKPOINT_VERSION = 1
MIN_FREE_DISK_BYTES = 1024 * 1024 * 1024
LORCODE_BOLD_SELECTOR = "b"


def content_fingerprint() -> str:
    payload = json.dumps(CONTENT, ensure_ascii=False, sort_keys=True).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def empty_checkpoint() -> dict[str, object]:
    return {
        "version": CHECKPOINT_VERSION,
        "content_fingerprint": content_fingerprint(),
        "topics": {},
        "comments": {},
    }


def read_checkpoint(path: Path) -> dict[str, object]:
    if not path.exists():
        return empty_checkpoint()
    try:
        state = json.loads(path.read_text("utf-8"))
    except (OSError, json.JSONDecodeError):
        return empty_checkpoint()
    if not isinstance(state, dict):
        return empty_checkpoint()
    if state.get("version") != CHECKPOINT_VERSION:
        return empty_checkpoint()
    if state.get("content_fingerprint") != content_fingerprint():
        return empty_checkpoint()
    if not isinstance(state.get("topics"), dict) or not isinstance(
        state.get("comments"), dict
    ):
        return empty_checkpoint()
    return state


def write_checkpoint(path: Path, state: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp")
    temporary.write_text(
        json.dumps(state, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def needs_flood_wait(created_count: int, existing_seen: bool) -> bool:
    """Keep a resume inside the same conservative flood-control envelope."""
    return created_count > 0 or existing_seen


def commit_link_selector(topic_id: int) -> str:
    """Match both valid Commit href forms, never the Uncommit suffix."""
    scope = f"#topic-{topic_id}"
    query = f"commit.jsp?msgid={topic_id}"
    return f'{scope} a[href="/{query}"], {scope} a[href="{query}"]'


def comment_author_selector(nick: str) -> str:
    """Comments expose their author through ``.sign``, without itemprop."""
    return f'.sign > a[href="/people/{nick}/profile"]'


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
    def __init__(
        self,
        playwright,
        base: str,
        headed: bool,
        output: Path,
        checkpoint: Path | None = None,
        max_actor_contexts: int | None = None,
    ) -> None:
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
        self.checkpoint = checkpoint
        self.checkpoint_state = (
            read_checkpoint(checkpoint) if checkpoint is not None else empty_checkpoint()
        )
        self.max_actor_contexts = max_actor_contexts
        self.ensure_free_disk("Chrome startup")
        self.browser = playwright.chromium.launch(
            executable_path=executable,
            headless=not headed,
            args=["--no-sandbox", "--disable-dev-shm-usage", "--disable-background-networking"],
        )
        self.contexts = {}
        self.pages = {}
        self.topics: dict[str, dict[str, object]] = dict(
            self.checkpoint_state["topics"]
        )
        self.comments: dict[str, dict[str, object]] = dict(
            self.checkpoint_state["comments"]
        )
        self.current_formats: dict[str, str] = {}
        self.metrics: list[dict[str, object]] = []
        self.peak_actor_contexts = 0
        self.closing = False
        self.browser.on(
            "disconnected",
            lambda _: None
            if self.closing
            else self.write_failure_diagnostic("browser disconnected"),
        )

    def close(self) -> None:
        self.closing = True
        self.close_all_actors()
        if self.browser.is_connected():
            self.browser.close()

    def ensure_free_disk(self, operation: str) -> None:
        usage = shutil.disk_usage(self.output)
        require(
            usage.free >= MIN_FREE_DISK_BYTES,
            f"{operation}: only {usage.free // (1024 * 1024)} MiB are free on "
            f"{self.output}; Chrome requires at least "
            f"{MIN_FREE_DISK_BYTES // (1024 * 1024)} MiB",
        )

    def write_failure_diagnostic(self, reason: object) -> None:
        try:
            usage = shutil.disk_usage(self.output)
            diagnostic = {
                "reason": str(reason),
                "free_disk_mib": usage.free // (1024 * 1024),
                "active_actors": sorted(self.contexts),
                "peak_actor_contexts": self.peak_actor_contexts,
                "topics_checkpointed": sorted(self.topics),
                "comments_checkpointed": sorted(self.comments),
            }
            (self.output / "browser-seed-failure.json").write_text(
                json.dumps(diagnostic, ensure_ascii=False, indent=2) + "\n",
                encoding="utf-8",
            )
        except OSError:
            pass

    def save_checkpoint(self) -> None:
        if self.checkpoint is None:
            return
        self.checkpoint_state["topics"] = self.topics
        self.checkpoint_state["comments"] = self.comments
        write_checkpoint(self.checkpoint, self.checkpoint_state)

    def close_actor(self, nick: str) -> None:
        self.pages.pop(nick, None)
        context = self.contexts.pop(nick, None)
        self.current_formats.pop(nick, None)
        if context is not None:
            try:
                context.close()
            except Exception:
                if self.browser.is_connected():
                    raise

    def close_all_actors(self) -> None:
        for nick in list(self.contexts):
            self.close_actor(nick)

    def metric(self, name: str, started: float, **details: object) -> None:
        self.metrics.append(
            {
                "name": name,
                "duration_ms": round((time.perf_counter() - started) * 1000, 2),
                **details,
            }
        )

    def page_for(self, nick: str):
        if nick in self.pages:
            return self.pages[nick]
        if (
            self.max_actor_contexts is not None
            and len(self.contexts) >= self.max_actor_contexts
        ):
            raise RuntimeError(
                f"actor context limit ({self.max_actor_contexts}) reached before "
                f"opening {nick}; close an inactive actor first"
            )
        self.ensure_free_disk(f"opening browser context for {nick}")
        context = self.browser.new_context(
            viewport={"width": 1440, "height": 1100},
            locale="ru-RU",
            timezone_id="Europe/Moscow",
        )
        page = context.new_page()
        page.on("crash", lambda _: self.write_failure_diagnostic(f"page crashed for {nick}"))
        page.set_default_timeout(30_000)
        page.set_default_navigation_timeout(45_000)
        self.contexts[nick] = context
        self.pages[nick] = page
        self.peak_actor_contexts = max(self.peak_actor_contexts, len(self.contexts))
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
        started = time.perf_counter()
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
        self.metric("login", started, actor=nick)

    def set_format_mode(self, page, nick: str, mode: str) -> None:
        if self.current_formats.get(nick) == mode:
            return
        started = time.perf_counter()
        self.goto(page, f"/people/{nick}/settings")
        form = page.locator('#profileForm[action^="/people/"]')
        require(form.count() == 1, f"settings form is absent for {nick}")
        radio = form.locator(f'input[name="format_mode"][value="{mode}"]')
        require(radio.count() == 1, f"format mode {mode} is unavailable for {nick}")
        radio.check()
        form.locator('button[type="submit"]').click()
        page.wait_for_load_state("domcontentloaded")
        self.current_formats[nick] = mode
        self.metric("settings.format", started, actor=nick, mode=mode)

    def discover_add_url(self, page, group_path: str) -> str:
        self.goto(page, group_path)
        links = page.locator('a[href^="/add.jsp?group="]')
        require(links.count() > 0, f"group page has no Add link: {group_path}")
        href = links.first.get_attribute("href")
        require(bool(href), f"empty Add link: {group_path}")
        return str(href)

    @staticmethod
    def topic_record(
        item: dict[str, object], topic_id: int, url: str
    ) -> dict[str, object]:
        return {
            "key": item["key"],
            "id": topic_id,
            "url": url,
            "title": item["title"],
            "moderation": str(item["moderation"]),
            "format": str(item.get("format", "markdown")),
            "poll_variants": [str(value) for value in item.get("poll_variants", [])],
            "multiselect": bool(item.get("multiselect", False)),
        }

    def checkpointed_topic_is_live(
        self, page, item: dict[str, object], topic: dict[str, object]
    ) -> bool:
        try:
            topic_id = int(topic["id"])
            topic_url = str(topic["url"])
        except (KeyError, TypeError, ValueError):
            return False
        expected_prefix = str(item["group_path"]).rstrip("/") + "/"
        if not topic_url.startswith(expected_prefix):
            return False
        try:
            self.goto(page, topic_url)
        except RuntimeError:
            return False
        node = page.locator(f"#topic-{topic_id}")
        if node.count() != 1:
            return False
        title = node.locator("h1").first
        return title.count() == 1 and title.inner_text().strip() == str(item["title"])

    def restore_topics(self, page) -> None:
        items = {str(item["key"]): item for item in CONTENT["topics"]}
        restored: dict[str, dict[str, object]] = {}
        for key, topic in self.topics.items():
            item = items.get(key)
            if item is None or not isinstance(topic, dict):
                continue
            if self.checkpointed_topic_is_live(page, item, topic):
                restored[key] = self.topic_record(
                    item,
                    int(topic["id"]),
                    str(topic["url"]),
                )
                print(f"RESUMED {key} {topic['url']}", flush=True)
        if len(restored) != len(self.topics):
            valid_topic_keys = set(restored)
            self.comments = {
                key: comment
                for key, comment in self.comments.items()
                if isinstance(comment, dict) and comment.get("topic") in valid_topic_keys
            }
        self.topics = restored
        self.save_checkpoint()

    def create_topic(self, page, item: dict[str, object], image_offset: int) -> dict[str, object]:
        started = time.perf_counter()
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
        poll_variants = [str(value) for value in item.get("poll_variants", [])]
        for index, label in enumerate(poll_variants):
            variant = form.locator(f'input[name="poll[{index}]"]')
            require(variant.count() == 1, f"{item['key']}: poll variant #{index} is absent")
            variant.fill(label)
        if bool(item.get("multiselect", False)):
            multiselect = form.locator('input[name="multiSelect"]')
            require(multiselect.count() == 1, f"{item['key']}: multiselect control is absent")
            multiselect.check()
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
        match = re.search(
            r"/(news|gallery|articles|forum|polls)/[^/]+/(\d+)",
            urlparse(page.url).path,
        )
        if match is None:
            confirmation_link = page.get_by_role("link", name="Перейти к сообщению")
            if confirmation_link.count() == 1:
                confirmation_link.click()
                page.wait_for_load_state("domcontentloaded")
                match = re.search(
                    r"/(news|gallery|articles|forum|polls)/[^/]+/(\d+)",
                    urlparse(page.url).path,
                )
        page_text = page.locator("body").inner_text()
        require(
            match is not None,
            f"{item['key']}: unexpected create redirect {page.url}; {page_text[:1000]!r}",
        )
        topic_id = int(match.group(2))
        result = self.topic_record(item, topic_id, urlparse(page.url).path)
        self.metric(
            "topic.create",
            started,
            key=item["key"],
            content_type=str(item["group_path"]).split("/")[1],
            topic_id=topic_id,
        )
        return result

    def commit_topic(self, page, topic: dict[str, object]) -> None:
        started = time.perf_counter()
        self.goto(page, f"/commit.jsp?msgid={topic['id']}")
        form = page.locator('form[action="edit.jsp"]')
        require(form.count() == 1, f"commit form is absent for topic {topic['id']}")
        form.locator('button[type="submit"][name="commit"]').click()
        page.wait_for_load_state("domcontentloaded")
        require("jump-message.jsp" not in page.url, f"commit redirect was not resolved: {page.url}")
        self.metric("topic.commit", started, key=topic["key"], topic_id=topic["id"])

    def ensure_topic_committed(self, page, topic: dict[str, object]) -> None:
        self.goto(page, str(topic["url"]))
        commit_link = page.locator(commit_link_selector(int(topic["id"])))
        if commit_link.count() == 0:
            self.metric(
                "topic.commit.resume",
                time.perf_counter(),
                key=topic["key"],
                topic_id=topic["id"],
            )
            return
        self.commit_topic(page, topic)

    def find_existing_comment(
        self,
        page,
        topic: dict[str, object],
        item: dict[str, object],
        expected_reply: int | None,
    ) -> dict[str, object] | None:
        self.goto(page, str(topic["url"]))
        candidates: list[tuple[int, object]] = []
        for node in page.locator('#comments article.msg[id^="comment-"]').all():
            match = re.fullmatch(r"comment-(\d+)", node.get_attribute("id") or "")
            if match is None:
                continue
            if node.locator(comment_author_selector(str(item["author"]))).count() != 1:
                continue
            body = node.locator(".msg-text")
            if body.count() != 1 or str(item["body"]) not in body.inner_text():
                continue
            candidates.append((int(match.group(1)), node))
        if not candidates:
            return None
        comment_id, node = max(candidates, key=lambda candidate: candidate[0])
        if expected_reply is not None:
            context = node.locator(":scope > .title")
            if (
                context.count() != 1
                or context.locator(f'a[href*="cid={expected_reply}"]').count() != 1
            ):
                return None
        return {
            "id": comment_id,
            "url": f'{topic["url"]}?cid={comment_id}',
            "topic": topic["key"],
            "reply_to": expected_reply,
            "body": item["body"],
        }

    def add_comment(
        self,
        page,
        topic: dict[str, object],
        body: str,
        reply_to: int | None = None,
    ) -> dict[str, object]:
        from playwright.sync_api import TimeoutError as PlaywrightTimeoutError

        started = time.perf_counter()
        self.goto(page, str(topic["url"]))
        topic_id = int(topic["id"])
        if reply_to is not None:
            reply_link = page.locator(
                f'#comment-{reply_to} .reply a[href^="add_comment.jsp"]'
            )
        else:
            reply_link = page.locator(
                f'#topic-{topic_id} .reply a[href^="comment-message.jsp"]'
            )
        require(
            reply_link.count() == 1,
            f"inline reply link is absent for target {reply_to or topic_id}",
        )
        original_url = page.url
        reply_link.click()
        page.wait_for_timeout(100)
        require(
            page.url == original_url,
            f"reply click unexpectedly navigated away from the topic: {page.url}",
        )
        form = page.locator('#commentForm[action="/add_comment.jsp"]')
        require(form.count() == 1, f"comment form is absent for topic {topic['id']}")
        form_container = page.locator("#comment-form-container")
        require(
            form_container.count() == 1
            and not form_container.is_hidden()
            and "comment-form-inline-visible"
            in (form_container.get_attribute("class") or ""),
            f"inline comment form did not open for target {reply_to or topic_id}",
        )
        hidden_reply = form.locator('input[name="replyto"]')
        expected_reply = str(reply_to or 0)
        require(
            hidden_reply.count() == 1 and hidden_reply.input_value() == expected_reply,
            f"reply target {expected_reply} is not preserved by the inline form",
        )
        form.locator('textarea[name="msg"]').fill(body)
        with page.expect_response(
            lambda response: "/add_comment_ajax" in response.url
        ) as response_info:
            form.locator("button.btn-primary").click()
        require(
            response_info.value.ok,
            f"comment AJAX failed: HTTP {response_info.value.status}",
        )
        try:
            page.wait_for_url(
                re.compile(rf"{re.escape(str(topic['url']))}\?cid=\d+$"),
                timeout=10_000,
            )
        except PlaywrightTimeoutError as error:
            errors = form.locator("div[error]").all_inner_texts()
            raise RuntimeError(f"comment AJAX did not redirect; errors={errors}") from error
        match = re.search(r"[?&]cid=(\d+)", page.url)
        require(match is not None, f"created comment URL has no cid: {page.url}")
        result = {
            "id": int(match.group(1)),
            "url": page.url,
            "topic": topic["key"],
            "reply_to": reply_to,
            "body": body,
        }
        self.metric(
            "comment.reply" if reply_to is not None else "comment.create",
            started,
            topic_id=topic["id"],
            comment_id=result["id"],
        )
        return result

    def add_reaction(
        self,
        page,
        topic: dict[str, object],
        comment_id: int | None = None,
        emoji: str = "👍",
    ) -> None:
        started = time.perf_counter()
        self.goto(page, str(topic["url"]))
        scope = page if comment_id is None else page.locator(f"#comment-{comment_id}")
        if comment_id is not None:
            require(scope.count() == 1, f"reaction target comment {comment_id} is absent")
        selected = scope.locator(
            f'form.reactions-form button[name="reaction"][value^="{emoji}-false"]'
        ).first
        if selected.count() == 1:
            count_text = selected.locator(".reaction-count").inner_text()
            require(int(count_text) >= 1, "persisted reaction has an invalid count")
            self.metric(
                "reaction.resume",
                started,
                topic_id=topic["id"],
                comment_id=comment_id,
                emoji=emoji,
            )
            return
        button = scope.locator(
            f'form.reactions-form button[name="reaction"][value^="{emoji}-true"]'
        ).first
        require(button.count() == 1, f"reaction button is absent for target {comment_id or topic['id']}")
        require(not button.is_visible(), "empty reaction choices must initially be hidden")
        reveal = (
            page.locator("#topicMenu a.reaction-show")
            if comment_id is None
            else scope.locator(".reply a.reaction-show")
        )
        require(reveal.count() == 1, f"reaction reveal link is absent for target {comment_id or topic['id']}")
        page.wait_for_function("window.jQuery && typeof window.jQuery === 'function'")
        page.wait_for_timeout(500)
        reveal.click()
        button.wait_for(state="visible")
        with page.expect_response(lambda response: "/reactions/ajax" in response.url) as response_info:
            button.click()
        require(response_info.value.ok, f"reaction AJAX failed: HTTP {response_info.value.status}")
        selected = scope.locator(
            f'form.reactions-form button[name="reaction"][value^="{emoji}-false"]'
        ).first
        selected.wait_for(state="visible")
        require(
            int(selected.locator(".reaction-count").inner_text()) >= 1,
            "reaction count was not updated after AJAX submit",
        )
        self.metric(
            "reaction.comment" if comment_id is not None else "reaction.topic",
            started,
            topic_id=topic["id"],
            comment_id=comment_id,
            emoji=emoji,
        )

    def vote(self, page, topic: dict[str, object], variants: list[int], actor: str) -> None:
        started = time.perf_counter()
        self.goto(page, str(topic["url"]))
        form = page.locator('form[action="/vote.jsp"]')
        if form.count() == 0:
            require(
                page.locator(".poll-result").count() == 1,
                f"poll form and persisted results are absent for {topic['key']} and {actor}",
            )
            self.metric(
                "poll.vote.resume",
                started,
                actor=actor,
                topic_id=topic["id"],
                choices=variants,
            )
            return
        inputs = form.locator('input[name="vote"]')
        require(inputs.count() == len(topic["poll_variants"]), "poll variant count differs")
        for index in variants:
            require(0 <= index < inputs.count(), f"invalid poll variant index {index}")
            inputs.nth(index).check()
        form.locator('button[type="submit"]').click()
        page.wait_for_load_state("domcontentloaded")
        require(page.locator(".poll-result").count() == 1, "poll results are absent after vote")
        self.metric(
            "poll.vote.multi" if topic["multiselect"] else "poll.vote.single",
            started,
            actor=actor,
            topic_id=topic["id"],
            choices=variants,
        )

    def verify_poll_results(
        self,
        page,
        topic: dict[str, object],
        expected_votes: list[int],
        expected_people: int,
    ) -> None:
        started = time.perf_counter()
        self.goto(page, f"{topic['url']}?results=true")
        rows = page.locator(".poll-result li")
        require(rows.count() == len(expected_votes), f"{topic['key']}: result row count differs")
        actual: dict[str, tuple[int, int]] = {}
        for row in rows.all():
            label = row.locator(".penguin_label").inner_text().strip()
            match = re.search(r"(\d+)\s*\((\d+)%\)", row.locator(".penguin_percent").inner_text())
            require(match is not None, f"{topic['key']}: malformed result for {label}")
            actual[label] = (int(match.group(1)), int(match.group(2)))
        divisor = expected_people if expected_people else sum(expected_votes)
        for label, votes in zip(topic["poll_variants"], expected_votes, strict=True):
            expected_percent = round(100 * votes / divisor) if divisor else 0
            require(actual.get(label) == (votes, expected_percent), f"{topic['key']}: {label}={actual.get(label)}, expected {(votes, expected_percent)}")
            require(actual[label][1] <= 100, f"{topic['key']}: impossible percentage for {label}")
        summary = page.locator(".poll-sum").inner_text()
        require(f"Всего голосов: {sum(expected_votes)}" in summary, "poll total votes differs")
        if topic["multiselect"]:
            require(
                f"всего проголосовавших: {expected_people}" in summary,
                "multiselect voter total differs",
            )
        self.metric("poll.results", started, topic_id=topic["id"])

    def verify_created_content(self, page) -> None:
        started = time.perf_counter()
        for topic in self.topics.values():
            self.goto(page, str(topic["url"]))
            topic_node = page.locator(f"#topic-{topic['id']}")
            require(topic_node.count() == 1, f"{topic['key']}: topic is absent from canonical page")
            require(
                topic_node.locator("h1").first.inner_text().strip() == str(topic["title"]),
                f"{topic['key']}: canonical title differs",
            )

        markdown = self.topics["forum-markdown"]
        self.goto(page, str(markdown["url"]))
        markdown_body = page.locator(f"#topic-{markdown['id']} .msg-text")
        require(
            markdown_body.locator("strong").filter(has_text="Markdown").count() == 1,
            "Markdown bold markup is not rendered",
        )
        require(
            markdown_body.locator("code").filter(has_text="inline code").count() == 1,
            "Markdown inline code is not rendered",
        )
        require(markdown_body.locator("li").count() == 2, "Markdown list is not rendered")

        lorcode = self.topics["forum-lorcode"]
        self.goto(page, str(lorcode["url"]))
        lorcode_body = page.locator(f"#topic-{lorcode['id']} .msg-text")
        require(
            lorcode_body.locator(LORCODE_BOLD_SELECTOR)
            .filter(has_text="LORCODE")
            .count()
            == 1,
            "LORCODE bold markup is not rendered",
        )
        require(
            lorcode_body.locator("blockquote").filter(has_text="цитата").count() == 1,
            "LORCODE quote is not rendered",
        )
        require(
            lorcode_body.locator("code").filter(has_text="echo lor").count() == 1,
            "LORCODE code is not rendered",
        )

        linebreak = self.topics["forum-linebreak"]
        self.goto(page, str(linebreak["url"]))
        require(
            page.locator(f"#topic-{linebreak['id']} .msg-text br").count() >= 2,
            "User line break mode does not preserve single newlines",
        )

        gallery_multi = self.topics["alt-mobile-gallery"]
        self.goto(page, str(gallery_multi["url"]))
        require(
            page.locator(
                f"#topic-{gallery_multi['id']} .slider-parent .swiffy-slider"
            ).count()
            == 1,
            "multi-image gallery has no original-compatible slider",
        )
        require(
            page.locator(
                f"#topic-{gallery_multi['id']} .slider-container > *"
            ).count()
            == 2,
            "multi-image gallery image count differs",
        )

        gallery_single = self.topics["hyprland-gallery"]
        self.goto(page, str(gallery_single["url"]))
        single = page.locator(f"#topic-{gallery_single['id']}")
        require(
            single.locator(".medium-image-container").count() == 1,
            "single-image gallery has no responsive image container",
        )
        require(
            single.locator(".swiffy-slider").count() == 0,
            "single-image gallery incorrectly uses a slider",
        )

        thread = self.topics["forum-markdown"]
        self.goto(page, str(thread["url"]))
        for key in ("root", "reply", "nested"):
            comment = self.comments[key]
            require(
                page.locator(f"#comment-{comment['id']}").count() == 1,
                f"thread misses comment {key}",
            )
        for key, parent in (("reply", "root"), ("nested", "reply")):
            comment = self.comments[key]
            parent_comment = self.comments[parent]
            context = page.locator(f"#comment-{comment['id']} > .title")
            require(
                context.count() == 1 and "Ответ на:" in context.inner_text(),
                f"reply context is absent for {key}",
            )
            require(
                context.locator(f'a[href*="cid={parent_comment["id"]}"]').count()
                == 1,
                f"reply {key} points to the wrong parent",
            )
        root = page.locator(f"#comment-{self.comments['root']['id']}")
        require(
            root.locator(
                'form.reactions-form button[name="reaction"][value^="🎉-"] .reaction-count'
            ).inner_text()
            == "1",
            "comment reaction is absent after reload",
        )

        news = self.topics["esp32-news"]
        self.goto(page, str(news["url"]))
        require(
            page.locator(
                'form.reactions-form button[name="reaction"][value^="👍-"] .reaction-count'
            ).first.inner_text()
            == "1",
            "topic reaction is absent after reload",
        )

        self.goto(page, "/view-all.jsp?section=1")
        pending = self.topics["pending-news"]
        pending_card = page.locator(f"#topic-{pending['id']}")
        require(pending_card.count() == 1, "pending news is absent from moderation queue")
        require(
            "не подтверждено" in pending_card.inner_text(),
            "pending news has no moderation marker",
        )

        self.goto(page, "/gallery/")
        require(
            page.locator(f"#topic-{gallery_multi['id']} .swiffy-slider").count() == 1,
            "gallery feed misses multi-image slider",
        )
        require(
            page.locator(
                f"#topic-{gallery_single['id']} .medium-image-container"
            ).count()
            == 1,
            "gallery feed misses single image",
        )
        page.screenshot(path=self.output / "created-gallery-desktop.png", full_page=True)
        self.metric(
            "content.verify",
            started,
            topics=len(self.topics),
            comments=len(self.comments),
        )

    def benchmark_pages(self, page) -> None:
        paths = [
            "/",
            "/news/",
            "/forum/",
            "/gallery/",
            "/polls/",
            "/articles/",
            "/tracker/",
            "/gallery/archive/",
            "/people/raven1000/",
            "/search.jsp?range=COMMENTS&user=crane2000&sort=DATE",
            *(str(topic["url"]) for topic in self.topics.values()),
        ]
        for path in paths:
            started = time.perf_counter()
            self.goto(page, path)
            duration_ms = (time.perf_counter() - started) * 1000
            require(duration_ms < 10_000, f"benchmark page exceeded 10s: {path}")
            self.metric("page.get", started, path=path)

    def benchmark_summary(self) -> dict[str, object]:
        durations = sorted(float(item["duration_ms"]) for item in self.metrics)
        if not durations:
            return {"operations": 0, "p50_ms": 0, "p95_ms": 0, "max_ms": 0}
        p50 = durations[(len(durations) - 1) // 2]
        p95 = durations[min(len(durations) - 1, int(len(durations) * 0.95))]
        return {
            "operations": len(durations),
            "p50_ms": p50,
            "p95_ms": p95,
            "max_ms": durations[-1],
        }

    def wait_for_comment_history(self, page, nick: str, expected_bodies: list[str]) -> None:
        path = f"/search.jsp?range=COMMENTS&user={nick}&sort=DATE"
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
        for section in ("1", "2", "3", "5", "6"):
            require(
                author_page.locator(f'a[href="/people/raven1000/?section={section}"]').count() == 1,
                f"author history section {section} is absent",
            )
        require('href="/tag/plain%20text"' in author_page.content(), "encoded profile tag URL is absent")
        author_page.screenshot(path=self.output / "author-topics-desktop.png", full_page=True)

        self.goto(author_page, "/people/raven1000/?section=3")
        filtered = author_page.locator("body").inner_text()
        require("Альт мобильный" in filtered, "gallery filter misses gallery topic")
        require("ESP32-S3" not in filtered, "gallery filter leaks news topics")

        expected = [
            str(item["body"])
            for item in CONTENT["comments"]
            if str(item["author"]) == str(CONTENT["commenter"])
        ]
        self.wait_for_comment_history(commenter_page, str(CONTENT["commenter"]), expected)
        search_body = commenter_page.locator("body").inner_text()
        count_match = re.search(r"Всего найдено (\d+)", search_body)
        require(
            count_match is not None and int(count_match.group(1)) >= len(expected),
            "comment history count differs",
        )
        commenter_page.screenshot(path=self.output / "comment-history-desktop.png", full_page=True)

        author_page.set_viewport_size({"width": 390, "height": 844})
        self.goto(author_page, "/people/raven1000/")
        author_page.screenshot(path=self.output / "author-topics-mobile.png", full_page=True)
        commenter_page.set_viewport_size({"width": 390, "height": 844})
        self.goto(commenter_page, "/search.jsp?range=COMMENTS&user=crane2000&sort=DATE")
        commenter_page.screenshot(path=self.output / "comment-history-mobile.png", full_page=True)

    def verify_archive_comments_and_forum(self, page) -> None:
        gallery = self.topics["alt-mobile-gallery"]
        self.goto(page, str(gallery["url"]))
        comment = page.locator("#comments article.msg").first
        require(comment.count() == 1, "browser-created gallery comment is absent")
        require(comment.locator(".userpic img.photo").count() == 1, "comment userpic column is absent")
        require(comment.locator(".msg_body.message-w-userpic").count() == 1, "comment body does not reserve avatar space")
        require(page.locator("#comments > h2").count() == 0, "non-original comments heading is present")
        page.screenshot(path=self.output / "topic-comments-desktop.png", full_page=True)

        self.goto(page, "/gallery/archive/")
        require(page.locator('form[action="/search.jsp"]').count() == 1, "archive search form is absent")
        now = datetime.now(timezone.utc)
        current_archive = f"/gallery/archive/{now.year}/{now.month}/"
        require(
            page.locator(f'a[href="{current_archive}"]').count() == 1,
            "current archive month is absent",
        )
        page.screenshot(path=self.output / "gallery-archive-desktop.png", full_page=True)

        self.goto(page, "/forum/")
        forum_text = page.locator("body").inner_text()
        for stale_group in ("Lor-source (11 за сутки)", "Mobile (16 за сутки)", "Multimedia (7 за сутки)"):
            require(stale_group not in forum_text, f"stale forum counter remains: {stale_group}")

    def run(self) -> dict[str, object]:
        author = str(CONTENT["author"])
        moderator = str(CONTENT["moderator"])
        commenter = str(CONTENT["commenter"])
        author_page = self.page_for(author)
        self.restore_topics(author_page)

        image_offset = 0
        created_topics = 0
        resumed_topics = bool(self.topics)
        for item in CONTENT["topics"]:
            key = str(item["key"])
            if key in self.topics:
                image_offset += int(item.get("images", 0))
                continue
            if needs_flood_wait(created_topics, resumed_topics):
                print(f"WAIT topic flood interval {TOPIC_INTERVAL_SECONDS}s", flush=True)
                time.sleep(TOPIC_INTERVAL_SECONDS)
            self.set_format_mode(
                author_page,
                author,
                str(item.get("format", "markdown")),
            )
            topic = self.create_topic(author_page, item, image_offset)
            self.topics[key] = topic
            self.save_checkpoint()
            image_offset += int(item.get("images", 0))
            created_topics += 1
            print(f"CREATED {topic['key']} {topic['url']}", flush=True)
        self.close_all_actors()

        moderator_page = self.page_for(moderator)
        for topic in self.topics.values():
            if topic["moderation"] == "commit":
                self.ensure_topic_committed(moderator_page, topic)
                print(f"COMMITTED {topic['key']}")
        self.close_all_actors()

        comment_urls: list[str] = []
        created_comments = 0
        existing_comments_seen = bool(self.comments)
        for item in CONTENT["comments"]:
            topic = self.topics[str(item["topic"])]
            actor = str(item["author"])
            actor_page = self.page_for(actor)
            reply_key = item.get("reply_to")
            reply_to = self.comments[str(reply_key)]["id"] if reply_key is not None else None
            key = str(item["key"])
            comment = self.find_existing_comment(actor_page, topic, item, reply_to)
            if comment is None:
                if needs_flood_wait(created_comments, existing_comments_seen):
                    print(
                        f"WAIT comment flood interval {COMMENT_INTERVAL_SECONDS}s",
                        flush=True,
                    )
                    time.sleep(COMMENT_INTERVAL_SECONDS)
                comment = self.add_comment(actor_page, topic, str(item["body"]), reply_to)
                created_comments += 1
                print(f"COMMENTED {topic['key']} {item['key']}", flush=True)
            else:
                existing_comments_seen = True
                print(f"RESUMED COMMENT {topic['key']} {item['key']}", flush=True)
            self.comments[key] = comment
            self.save_checkpoint()
            comment_urls.append(str(comment["url"]))
            self.close_actor(actor)

        commenter_page = self.page_for(commenter)
        self.add_reaction(commenter_page, self.topics["esp32-news"], emoji="👍")
        self.close_all_actors()
        author_page = self.page_for(author)
        self.add_reaction(
            author_page,
            self.topics["forum-markdown"],
            int(self.comments["root"]["id"]),
            "🎉",
        )
        print("REACTED topic and comment", flush=True)
        self.close_all_actors()

        for topic_key, actors in CONTENT["poll_votes"].items():
            topic = self.topics[str(topic_key)]
            for actor, choices in actors.items():
                actor_name = str(actor)
                self.vote(
                    self.page_for(actor_name),
                    topic,
                    [int(value) for value in choices],
                    actor_name,
                )
                print(f"VOTED {topic_key} {actor}", flush=True)
                self.close_actor(actor_name)

        author_page = self.page_for(author)
        for topic_key, actors in CONTENT["poll_votes"].items():
            topic = self.topics[str(topic_key)]
            expected_votes = [0] * len(topic["poll_variants"])
            for choices in actors.values():
                for index in choices:
                    expected_votes[int(index)] += 1
            self.verify_poll_results(author_page, topic, expected_votes, len(actors))

        self.verify_created_content(author_page)
        self.verify_archive_comments_and_forum(author_page)
        commenter_page = self.page_for(commenter)
        self.verify_histories(author_page, commenter_page)
        self.benchmark_pages(author_page)
        self.save_checkpoint()
        return {
            "source_window_hours": CONTENT["source_window_hours"],
            "topics": list(self.topics.values()),
            "comment_urls": comment_urls,
            "comments": list(self.comments.values()),
            "metrics": self.metrics,
            "benchmark": self.benchmark_summary(),
            "artifacts": str(self.output),
            "peak_actor_contexts": self.peak_actor_contexts,
            "checkpoint": str(self.checkpoint) if self.checkpoint is not None else None,
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
    parser.add_argument(
        "--checkpoint",
        type=Path,
        default=Path("/tmp/prod_ready_browser_seed_checkpoint.json"),
        help="atomic resume state; only browser-observed topic/comment IDs are stored",
    )
    parser.add_argument(
        "--restart",
        action="store_true",
        help="discard the browser resume checkpoint after a fresh fixture seed",
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

    output = args.output.resolve()
    (output / "browser-seed-failure.json").unlink(missing_ok=True)
    checkpoint = args.checkpoint.resolve()
    if args.restart:
        checkpoint.unlink(missing_ok=True)
    with sync_playwright() as playwright:
        seed = BrowserSeed(
            playwright,
            args.base,
            args.headed,
            output,
            checkpoint=checkpoint,
            max_actor_contexts=2,
        )
        try:
            result = seed.run()
        except Exception as error:
            seed.write_failure_diagnostic(error)
            raise
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
