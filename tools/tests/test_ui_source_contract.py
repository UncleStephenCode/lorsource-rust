import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


class UiSourceContractTests(unittest.TestCase):
    def test_every_literal_static_asset_reference_resolves(self) -> None:
        prefixes = (
            "img/",
            "js/",
            "font/",
            "webjars/",
            "black/",
            "tango/",
            "white2/",
            "waltz/",
            "zomg_ponies/",
            "adv/",
            "qrerror/",
        )
        missing: list[str] = []
        sources = list((ROOT / "templates").glob("*.html"))
        sources.append(ROOT / "src/theme_middleware.rs")
        sources.extend((ROOT / "static").rglob("*.css"))
        for source in sources:
            text = source.read_text(encoding="utf-8")
            references = re.findall(
                r'(?:src|href)=["\'](/[^"\'?#{}% ]+)|url\(["\']?(/[^)"\'?#{}% ]+)',
                text,
            )
            for pair in references:
                url = next(value for value in pair if value)
                relative = url.lstrip("/")
                if relative.startswith(prefixes) and not (ROOT / "static" / relative).is_file():
                    missing.append(f"{source.relative_to(ROOT)}: {url}")
        self.assertEqual(missing, [])

    def test_rss_discovery_uses_original_page_specific_endpoints(self) -> None:
        base = (ROOT / "templates/base.html").read_text(encoding="utf-8")
        self.assertNotIn('href="/rss"', base)
        expected = {
            "main_page.html": "/section-rss.jsp?section=1",
            "groups.html": "/section-rss.jsp?section=2",
            "group_topics.html":
                "/section-rss.jsp?section={{ group.section }}&amp;group={{ group.id }}",
        }
        for template, href in expected.items():
            text = (ROOT / "templates" / template).read_text(encoding="utf-8")
            self.assertIn('rel="alternate"', text)
            self.assertIn(f'href="{href}"', text)
        section_list = (ROOT / "templates/index.html").read_text(encoding="utf-8")
        self.assertIn('rel="alternate" href="{{ url }}"', section_list)
        self.assertIn("nav.rss_url", section_list)

    def test_theme_shell_keeps_original_css_hooks(self) -> None:
        base = (ROOT / "templates/base.html").read_text(encoding="utf-8")
        middleware = (ROOT / "src/theme_middleware.rs").read_text(encoding="utf-8")
        self.assertIn('<main id="bd">', base)
        for marker in ("LOR_THEME_HEADER", "LOR_THEME_FOOTER", "LOR_BASE_URL", "LOR_TIMEZONE"):
            self.assertIn(marker, base)
        for hook in ('id="hd"', 'id="sitetitle"', 'id="topProfile"', 'id="ft"'):
            self.assertIn(hook, middleware)


if __name__ == "__main__":
    unittest.main()
