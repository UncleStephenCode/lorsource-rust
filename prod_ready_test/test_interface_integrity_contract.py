#!/usr/bin/env python3
"""Pure regressions for conditional interface-integrity expectations."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from test_port import profile_topic_history_targets


class ProfileTopicHistoryTargetsTest(unittest.TestCase):
    def test_accepts_the_java_empty_opensearch_branch(self) -> None:
        body = "<h2>Сообщения пользователя</h2><ul></ul>"
        self.assertEqual(profile_topic_history_targets(body, "crane2000"), ())

    def test_returns_the_history_target_with_section_statistics(self) -> None:
        body = (
            '<a href="/people/crane2000/?section=5">1</a>'
            '<a href="/people/crane2000/">Темы</a>'
        )
        self.assertEqual(
            profile_topic_history_targets(body, "crane2000"),
            ("/people/crane2000/",),
        )

    def test_rejects_dom_branches_that_do_not_match_whois_jsp(self) -> None:
        with self.assertRaisesRegex(
            AssertionError,
            "profile topic-history link and per-section statistics diverge",
        ):
            profile_topic_history_targets(
                '<a href="/people/crane2000/">Темы</a>', "crane2000"
            )


if __name__ == "__main__":
    unittest.main()
