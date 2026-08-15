#!/usr/bin/env python3
"""Authenticated desktop/mobile regression matrix for all original LOR themes."""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path
from urllib.parse import urljoin, urlparse


PASSWORD = "Birds-ProdReady-2026"
DEFAULT_BASE = "http://localhost:8181"
ALLOWED_BASES = frozenset(
    {DEFAULT_BASE, "http://127.0.0.1:8181", "http://[::1]:8181"}
)
PAGES = {
    "profile": "/people/{nick}/profile",
    "settings": "/people/{nick}/settings",
    "topic": "/forum/games/9101003",
}
THEMES = (
    ("swift45", "tango-auto", "/tango/combined.css", "auto"),
    ("finch50", "tango-light", "/tango/combined.css", "light"),
    ("lark70", "tango", "/tango/combined.css", "dark"),
    ("robin201", "black", "/black/combined.css", None),
    ("oriole300", "white2", "/white2/combined.css", None),
    ("falcon500", "waltz", "/waltz/combined.css", None),
    ("heron750", "zomg_ponies", "/zomg_ponies/combined.css", None),
)
VIEWPORTS = {
    "desktop": {"width": 1440, "height": 1100},
    "mobile": {"width": 390, "height": 844},
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def local_base(value: str) -> str:
    if value not in ALLOWED_BASES:
        raise argparse.ArgumentTypeError(
            "--base must be exactly http://localhost:8181, "
            "http://127.0.0.1:8181, or http://[::1]:8181"
        )
    return value


def browser_executable() -> str:
    for name in ("google-chrome", "google-chrome-stable", "chromium"):
        if path := shutil.which(name):
            return path
    raise RuntimeError("Chrome/Chromium is not installed")


def absolute(base: str, path: str) -> str:
    return urljoin(base.rstrip("/") + "/", path.lstrip("/"))


class BrowserDiagnostics:
    def __init__(self, page, base: str) -> None:
        self.console_errors: list[str] = []
        self.page_errors: list[str] = []
        self.bad_local_responses: list[str] = []
        base_url = urlparse(base)
        self.origin = (base_url.scheme, base_url.netloc)
        page.on(
            "console",
            lambda message: self.console_errors.append(message.text)
            if message.type == "error"
            else None,
        )
        page.on("pageerror", lambda error: self.page_errors.append(str(error)))
        page.on("response", self._response)

    def _response(self, response) -> None:
        response_url = urlparse(response.url)
        if (response_url.scheme, response_url.netloc) == self.origin and response.status >= 400:
            self.bad_local_responses.append(f"HTTP {response.status} {response.url}")

    def reset(self) -> None:
        self.console_errors.clear()
        self.page_errors.clear()
        self.bad_local_responses.clear()

    def assert_clean(self, page, label: str) -> None:
        page.wait_for_timeout(100)
        require(not self.console_errors, f"{label}: console errors: {self.console_errors}")
        require(not self.page_errors, f"{label}: page errors: {self.page_errors}")
        require(
            not self.bad_local_responses,
            f"{label}: failed local resources: {self.bad_local_responses}",
        )


def goto(page, base: str, path: str) -> None:
    response = page.goto(absolute(base, path), wait_until="load")
    require(response is not None, f"{path}: browser returned no response")
    require(response.status < 400, f"{path}: HTTP {response.status}")


def login(page, base: str, nick: str) -> None:
    goto(page, base, "/login.jsp?from=/forum/")
    form = page.locator('form:has(input[name="nick"]):has(input[name="passwd"])')
    require(form.count() == 1, f"{nick}: login form is absent")
    form.locator('input[name="nick"]').fill(nick)
    form.locator('input[name="passwd"]').fill(PASSWORD)
    form.locator('button[type="submit"]').click()
    page.wait_for_load_state("domcontentloaded")
    require(
        page.locator(f'a[href="/people/{nick}/profile"]').count() > 0,
        f"{nick}: login failed at {page.url}",
    )


def verify_document(page, base: str, style: str, stylesheet: str, color_mode: str | None) -> None:
    root = page.locator("html")
    require(root.get_attribute("data-style") == style, f"{page.url}: wrong data-style")
    require(
        root.get_attribute("data-theme") == color_mode,
        f"{page.url}: wrong data-theme for {style}",
    )
    theme_link = page.locator("link[data-lor-theme-stylesheet]")
    require(theme_link.count() == 1, f"{page.url}: theme stylesheet hook is absent")
    require(
        theme_link.get_attribute("href") == stylesheet,
        f"{page.url}: wrong stylesheet for {style}",
    )
    css_response = page.context.request.get(absolute(base, stylesheet))
    require(css_response.status == 200, f"{stylesheet}: HTTP {css_response.status}")
    require(page.locator("main#bd").count() == 1, f"{page.url}: #bd is absent")
    require(page.locator("footer#ft").count() == 1, f"{page.url}: #ft is absent")
    if style == "black":
        require(
            page.locator("table.head").count() == 1,
            f"{page.url}: black non-main header is absent",
        )
    else:
        require(page.locator("#hd").count() == 1, f"{page.url}: #hd is absent")
    require(
        page.locator("#theme-indicator").count() == 1,
        f"{page.url}: theme indicator is absent",
    )
    require(
        "LOR_THEME_" not in page.content(),
        f"{page.url}: unexpanded theme marker",
    )
    broken_images = page.locator('img[src^="/"]').evaluate_all(
        """images => images
          .filter(image => image.complete && image.naturalWidth === 0)
          .map(image => image.getAttribute('src'))"""
    )
    require(not broken_images, f"{page.url}: broken local images: {broken_images}")


def settings_form_state(page) -> dict[str, object]:
    return page.locator("#profileForm").evaluate(
        """form => Object.fromEntries(Array.from(form.elements)
          .filter(field => field.name && field.name !== 'csrf')
          .filter(field => field.type !== 'radio' || field.checked)
          .map(field => [field.name,
            field.type === 'checkbox' ? field.checked : field.value]))"""
    )


def verify_local_storage_precedence(page, base: str, diagnostics: BrowserDiagnostics) -> None:
    diagnostics.reset()
    goto(page, base, "/people/swift45/profile")
    page.evaluate("localStorage.setItem('lor-theme', 'dark')")
    page.reload(wait_until="domcontentloaded")
    require(
        page.locator("html").get_attribute("data-style") == "tango-auto",
        "local override changed the persisted style",
    )
    require(
        page.locator("html").get_attribute("data-theme") == "dark",
        "valid local override was not applied",
    )

    goto(page, base, "/people/swift45/settings")
    before_settings = settings_form_state(page)
    page.wait_for_function(
        """() => {
          const form = document.querySelector('#profileForm');
          return form && typeof window.jQuery === 'function' &&
            window.jQuery._data(form, 'events')?.submit?.length > 0;
        }"""
    )
    page.locator('input[name="style"][value="tango-auto"]').check()
    page.locator('#profileForm button[type="submit"]').click()
    page.wait_for_load_state("domcontentloaded")
    require(
        page.evaluate("localStorage.getItem('lor-theme')") is None,
        "settings submit did not clear lor-theme",
    )
    require(
        page.locator("html").get_attribute("data-theme") == "auto",
        "saved tango-auto mode was not visible after clearing the override",
    )
    goto(page, base, "/people/swift45/settings")
    after_settings = settings_form_state(page)
    require(
        after_settings == before_settings,
        f"idempotent settings submit changed values: {before_settings} -> {after_settings}",
    )
    diagnostics.assert_clean(page, "localStorage/settings persistence")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default=DEFAULT_BASE, type=local_base)
    parser.add_argument("--output", type=Path, default=Path("/tmp/theme-browser-matrix"))
    parser.add_argument("--headed", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        from playwright.sync_api import sync_playwright
    except ImportError as error:
        raise RuntimeError(
            "Playwright is required: pip install -r prod_ready_test/requirements-browser.txt"
        ) from error

    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report: list[dict[str, object]] = []
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(
            executable_path=browser_executable(),
            headless=not args.headed,
            args=["--no-sandbox", "--disable-dev-shm-usage", "--disable-background-networking"],
        )
        try:
            guest_context = browser.new_context(
                viewport=VIEWPORTS["desktop"], locale="ru-RU", timezone_id="Europe/Moscow"
            )
            guest_context.add_cookies(
                [{"name": "lor_theme", "value": "black", "url": args.base}]
            )
            guest = guest_context.new_page()
            guest_diagnostics = BrowserDiagnostics(guest, args.base)
            for path in ("/people/swift45/profile", "/forum/games/9101003", "/login.jsp"):
                guest_diagnostics.reset()
                goto(guest, args.base, path)
                verify_document(guest, args.base, "tango-auto", "/tango/combined.css", "auto")
                guest_diagnostics.assert_clean(guest, f"guest {path}")
            guest_diagnostics.reset()
            forbidden = guest.goto(
                absolute(args.base, "/people/swift45/settings"),
                wait_until="load",
            )
            require(forbidden is not None and forbidden.status == 403, "guest settings is not 403")
            expected_forbidden = f"HTTP 403 {absolute(args.base, '/people/swift45/settings')}"
            guest_diagnostics.bad_local_responses = [
                failure
                for failure in guest_diagnostics.bad_local_responses
                if failure != expected_forbidden
            ]
            guest_diagnostics.console_errors = [
                error
                for error in guest_diagnostics.console_errors
                if error
                != "Failed to load resource: the server responded with a status of 403 (Forbidden)"
            ]
            verify_document(guest, args.base, "tango-auto", "/tango/combined.css", "auto")
            guest_diagnostics.assert_clean(guest, "guest settings 403")
            guest_context.close()

            for nick, style, stylesheet, color_mode in THEMES:
                context = browser.new_context(
                    viewport=VIEWPORTS["desktop"],
                    locale="ru-RU",
                    timezone_id="Europe/Moscow",
                )
                page = context.new_page()
                diagnostics = BrowserDiagnostics(page, args.base)
                page.set_default_timeout(30_000)
                login(page, args.base, nick)
                context.add_cookies(
                    [{"name": "lor_theme", "value": "zomg_ponies", "url": args.base}]
                )
                for viewport_name, viewport in VIEWPORTS.items():
                    page.set_viewport_size(viewport)
                    for page_name, path_pattern in PAGES.items():
                        path = path_pattern.format(nick=nick)
                        diagnostics.reset()
                        goto(page, args.base, path)
                        verify_document(page, args.base, style, stylesheet, color_mode)
                        destination = output / f"{style}-{page_name}-{viewport_name}.png"
                        page.screenshot(path=destination, full_page=True)
                        diagnostics.assert_clean(
                            page, f"{style}/{page_name}/{viewport_name}"
                        )
                        report.append(
                            {
                                "nick": nick,
                                "style": style,
                                "page": page_name,
                                "viewport": viewport_name,
                                "path": path,
                                "screenshot": str(destination),
                            }
                        )
                if nick == "swift45":
                    verify_local_storage_precedence(page, args.base, diagnostics)
                if nick == "robin201":
                    diagnostics.reset()
                    goto(page, args.base, "/people/robin201/profile")
                    page.evaluate("localStorage.setItem('lor-theme', 'light')")
                    page.reload(wait_until="domcontentloaded")
                    require(
                        page.locator("html").get_attribute("data-theme") == "light",
                        "head.jsp-compatible local override was not applied to a legacy theme",
                    )
                    page.evaluate("localStorage.removeItem('lor-theme')")
                    diagnostics.assert_clean(page, "black localStorage override")
                context.close()
        finally:
            browser.close()

    report_path = output / "report.json"
    report_path.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(f"theme browser matrix passed: {len(report)} page/viewports; {report_path}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RuntimeError as error:
        print(f"theme browser matrix failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
