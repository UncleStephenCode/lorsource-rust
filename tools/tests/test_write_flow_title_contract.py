from __future__ import annotations

import sys
import unittest
from pathlib import Path


COMPAT = Path(__file__).resolve().parents[2] / "compat"
sys.path.insert(0, str(COMPAT))

import write_flow_html  # noqa: E402


class WriteFlowTitleContractTest(unittest.TestCase):
    def test_extracts_single_decoded_topic_title(self) -> None:
        topic_url = "/forum/test/42"
        page = (
            '<h1><a href="/unrelated">Wrong</a></h1>'
            f'<h1><a href="{topic_url}">'
            "A &amp; B &lt; C &gt; D &quot;quoted&quot; &#39;apostrophe&#39;"
            "</a></h1>"
        )

        self.assertEqual(
            'A & B < C > D "quoted" \'apostrophe\'',
            write_flow_html.visible_topic_title(page, topic_url),
        )

    def test_double_escaped_title_remains_observable_as_a_failure(self) -> None:
        topic_url = "/forum/test/42"
        page = f'<h1><a href="{topic_url}">A &amp;amp; B</a></h1>'

        self.assertEqual(
            "A &amp; B",
            write_flow_html.visible_topic_title(page, topic_url),
        )


if __name__ == "__main__":
    unittest.main()
