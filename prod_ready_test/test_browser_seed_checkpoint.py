#!/usr/bin/env python3
"""Fast regressions for browser-seed crash recovery state."""

from __future__ import annotations

import json
import tempfile
import unittest
from html.parser import HTMLParser
from pathlib import Path
import sys


HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from browser_seed import (
    BrowserSeed,
    LORCODE_BOLD_SELECTOR,
    comment_author_selector,
    commit_link_selector,
    content_fingerprint,
    empty_checkpoint,
    needs_flood_wait,
    read_checkpoint,
    write_checkpoint,
)


class CommentSignatureParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.in_sign = False
        self.sign_author_hrefs: list[str] = []

    def handle_starttag(
        self, tag: str, attributes: list[tuple[str, str | None]]
    ) -> None:
        attrs = dict(attributes)
        if tag == "div" and "sign" in (attrs.get("class") or "").split():
            self.in_sign = True
        elif self.in_sign and tag == "a" and attrs.get("href"):
            self.sign_author_hrefs.append(str(attrs["href"]))

    def handle_endtag(self, tag: str) -> None:
        if tag == "div" and self.in_sign:
            self.in_sign = False


class BrowserSeedCheckpointTest(unittest.TestCase):
    def test_resume_recognizes_actual_comment_author_signature(self) -> None:
        actual_comment_dom = (
            '<article class="msg" id="comment-9125019">'
            '<div class="msg_body"><div class="msg-text">body</div>'
            '<div class="sign"><a href="/people/crane2000/profile">'
            "crane2000</a></div></div></article>"
        )
        parser = CommentSignatureParser()
        parser.feed(actual_comment_dom)

        self.assertNotIn("itemprop", actual_comment_dom)
        self.assertEqual(parser.sign_author_hrefs, ["/people/crane2000/profile"])
        self.assertEqual(
            comment_author_selector("crane2000"),
            '.sign > a[href="/people/crane2000/profile"]',
        )

    def test_resume_commit_selector_does_not_match_uncommit(self) -> None:
        selector = commit_link_selector(9125001)
        self.assertEqual(
            selector,
            '#topic-9125001 a[href="/commit.jsp?msgid=9125001"], '
            '#topic-9125001 a[href="commit.jsp?msgid=9125001"]',
        )
        self.assertNotIn("uncommit.jsp", selector)
        self.assertNotIn("href$=", selector)

    def test_lorcode_bold_contract_uses_original_b_element(self) -> None:
        self.assertEqual(LORCODE_BOLD_SELECTOR, "b")

    def test_resume_waits_before_its_first_new_write(self) -> None:
        self.assertFalse(needs_flood_wait(0, False))
        self.assertTrue(needs_flood_wait(0, True))
        self.assertTrue(needs_flood_wait(1, False))

    def test_round_trip_preserves_observed_content(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "checkpoint.json"
            state = empty_checkpoint()
            state["topics"] = {
                "forum-markdown": {
                    "id": 9125001,
                    "url": "/forum/linux-org-ru/9125001",
                }
            }
            state["comments"] = {
                "root": {"id": 9125002, "topic": "forum-markdown"}
            }

            write_checkpoint(path, state)

            self.assertEqual(read_checkpoint(path), state)
            self.assertFalse(path.with_name(f".{path.name}.tmp").exists())

    def test_corrupt_or_foreign_checkpoint_is_not_resumed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "checkpoint.json"
            path.write_text("not-json", encoding="utf-8")
            self.assertEqual(read_checkpoint(path), empty_checkpoint())

            path.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "content_fingerprint": "different-content",
                        "topics": {"stale": {}},
                        "comments": {},
                    }
                ),
                encoding="utf-8",
            )
            state = read_checkpoint(path)
            self.assertEqual(state["content_fingerprint"], content_fingerprint())
            self.assertEqual(state["topics"], {})

    def test_close_actor_releases_page_context_and_format_cache(self) -> None:
        class FakeContext:
            closed = False

            def close(self) -> None:
                self.closed = True

        seed = object.__new__(BrowserSeed)
        context = FakeContext()
        seed.pages = {"swift45": object()}
        seed.contexts = {"swift45": context}
        seed.current_formats = {"swift45": "markdown"}

        BrowserSeed.close_actor(seed, "swift45")

        self.assertTrue(context.closed)
        self.assertEqual(seed.pages, {})
        self.assertEqual(seed.contexts, {})
        self.assertEqual(seed.current_formats, {})


if __name__ == "__main__":
    unittest.main()
