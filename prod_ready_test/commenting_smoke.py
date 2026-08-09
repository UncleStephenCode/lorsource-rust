#!/usr/bin/env python3
"""Browser regression for the complete comment lifecycle (registration excluded)."""

from __future__ import annotations

import argparse
import json
import time
from pathlib import Path

from browser_seed import BrowserSeed, require


TOPIC = {
    "key": "commenting-smoke",
    "id": 9101003,
    "url": "/forum/games/9101003",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:8181")
    parser.add_argument("--headed", action="store_true")
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("/tmp/lorsource-commenting-smoke"),
    )
    return parser.parse_args()


def run() -> dict[str, object]:
    args = parse_args()
    from playwright.sync_api import sync_playwright

    with sync_playwright() as playwright:
        seed = BrowserSeed(playwright, args.base, args.headed, args.output)
        try:
            guest = seed.browser.new_context(viewport={"width": 1440, "height": 1000})
            guest_page = guest.new_page()
            seed.goto(guest_page, str(TOPIC["url"]))
            require(
                guest_page.get_by_role("link", name="Ответить", exact=True).count() == 0,
                "anonymous viewer sees a forbidden Reply action",
            )
            require(
                guest_page.locator("#commentForm").count() == 0,
                "anonymous viewer receives the inline comment form",
            )
            guest.close()

            crane = seed.page_for("crane2000")
            seed.goto(crane, str(TOPIC["url"]))
            root_reply = crane.locator(
                f'#topic-{TOPIC["id"]} .reply a[href^="comment-message.jsp"]'
            )
            require(root_reply.count() == 1, "top-level Reply action is absent")
            root_reply.click()
            preview_text = "**browser comment preview**"
            crane.locator('#commentForm textarea[name="msg"]').fill(preview_text)
            preview_tab = crane.locator('.markup-tabs__tab[data-tab="preview"]')
            preview_tab.wait_for(state="visible")
            with crane.expect_response(lambda response: "/markup/preview" in response.url) as response:
                preview_tab.click()
            require(response.value.ok, f"preview returned HTTP {response.value.status}")
            require(
                "<strong>browser comment preview</strong>"
                in crane.locator(".markup-preview").inner_html(),
                "Markdown preview differs from the editor mode",
            )

            marker = time.time_ns()
            root = seed.add_comment(
                crane,
                TOPIC,
                f"Browser root comment {marker}: **Markdown** and @robin201.",
            )
            time.sleep(4.2)
            robin = seed.page_for("robin201")
            child = seed.add_comment(
                robin,
                TOPIC,
                f"Browser nested reply {marker}: reply context and lifecycle.",
                int(root["id"]),
            )

            seed.goto(robin, str(TOPIC["url"]))
            child_scope = robin.locator(f'#comment-{child["id"]}')
            require(child_scope.count() == 1, "new nested comment is absent")
            edit_link = child_scope.locator('a[href^="/edit_comment?"]')
            delete_link = child_scope.locator('a[href^="/delete_comment.jsp?"]')
            warning_link = child_scope.locator('a[href^="/post-warning?"]')
            require(edit_link.count() == 1, "fresh leaf comment has no Edit action")
            require(delete_link.count() == 1, "fresh leaf comment has no Delete action")
            require(warning_link.count() == 1, "eligible user has no moderator-warning action")

            edit_link.click()
            robin.wait_for_load_state("domcontentloaded")
            edited_text = f"Browser edited nested reply {marker}."
            edit_form = robin.locator('#commentForm[action="/edit_comment"]')
            edit_form.locator('textarea[name="msg"]').fill(edited_text)
            edit_form.locator('button[type="submit"]:not([name="preview"])').click()
            robin.wait_for_load_state("domcontentloaded")
            require(edited_text in robin.locator("body").inner_text(), "edited comment is not rendered")

            seed.goto(crane, str(TOPIC["url"]))
            root_scope = crane.locator(f'#comment-{root["id"]}')
            require(
                root_scope.locator('a[href^="/delete_comment.jsp?"]').count() == 0,
                "comment with a live reply can be deleted by its author",
            )

            seed.goto(robin, str(TOPIC["url"]))
            child_scope = robin.locator(f'#comment-{child["id"]}')
            child_scope.locator('a[href^="/delete_comment.jsp?"]').click()
            robin.wait_for_load_state("domcontentloaded")
            delete_form = robin.locator('form[action="/delete_comment.jsp"]')
            require(delete_form.count() == 1, "Delete form is absent")
            delete_reason = f"browser lifecycle {marker}"
            delete_form.locator('input[name="reason"]').fill(delete_reason)
            delete_form.locator('button[type="submit"]').click()
            robin.wait_for_load_state("domcontentloaded")
            require("Удалено успешно" in robin.locator("body").inner_text(), "Delete did not complete")

            moderator = seed.page_for("hawk_moderator")
            seed.goto(moderator, f'{TOPIC["url"]}?deleted=true')
            deleted_scope = moderator.locator(f'#comment-{child["id"]}')
            require(deleted_scope.count() == 1, "moderator cannot see the deleted comment")
            deleted_text = deleted_scope.inner_text()
            require(delete_reason in deleted_text, "delete reason is absent")
            require(edited_text in deleted_text, "deleted comment body is hidden from moderator")
            undelete_link = deleted_scope.locator('a[href^="/undelete_comment?"]')
            require(
                undelete_link.count() == 0,
                "moderator can restore a comment deleted by its own author",
            )

            time.sleep(4.2)
            victim_page = seed.page_for("oriole300")
            victim = seed.add_comment(
                victim_page,
                TOPIC,
                f"Browser moderator-delete target {marker}.",
            )
            seed.goto(moderator, str(TOPIC["url"]))
            victim_scope = moderator.locator(f'#comment-{victim["id"]}')
            victim_scope.locator('a[href^="/delete_comment.jsp?"]').click()
            moderator.wait_for_load_state("domcontentloaded")
            moderator_delete_form = moderator.locator('form[action="/delete_comment.jsp"]')
            moderator_reason = f"browser moderator lifecycle {marker}"
            moderator_delete_form.locator('input[name="reason"]').fill(moderator_reason)
            moderator_delete_form.locator('button[type="submit"]').click()
            moderator.wait_for_load_state("domcontentloaded")
            require("Удалено успешно" in moderator.locator("body").inner_text(), "moderator Delete failed")

            senior = seed.page_for("eagle_moderator")
            seed.goto(senior, f'{TOPIC["url"]}?deleted=true')
            victim_deleted_scope = senior.locator(f'#comment-{victim["id"]}')
            require(victim_deleted_scope.count() == 1, "deleted moderator target is absent")
            require(moderator_reason in victim_deleted_scope.inner_text(), "moderator delete reason is absent")
            victim_deleted_scope.locator('a[href^="/undelete_comment?"]').click()
            senior.wait_for_load_state("domcontentloaded")
            undelete_form = senior.locator('form[action="/undelete_comment"]')
            require(undelete_form.count() == 1, "Restore form is absent")
            undelete_form.locator('button[type="submit"]').click()
            senior.wait_for_load_state("domcontentloaded")
            require(
                str(victim["body"]) in senior.locator("body").inner_text(),
                "restored moderator target is absent",
            )

            seed.add_reaction(robin, TOPIC, int(victim["id"]), "👍")

            result = {
                "topic": TOPIC["url"],
                "root_comment": root["id"],
                "nested_comment": child["id"],
                "restored_comment": victim["id"],
                "preview": "passed",
                "edit": "passed",
                "delete_restore": "passed",
                "reaction": "passed",
                "anonymous_controls": "passed",
            }
            args.output.mkdir(parents=True, exist_ok=True)
            (args.output / "result.json").write_text(
                json.dumps(result, ensure_ascii=False, indent=2) + "\n",
                encoding="utf-8",
            )
            return result
        finally:
            seed.close()


def main() -> int:
    print(json.dumps(run(), ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
