"""Small DOM helpers shared by the stateful write-flow and its unit tests."""

from __future__ import annotations

from html.parser import HTMLParser


class VisibleTextParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.parts: list[str] = []

    def handle_data(self, data: str) -> None:
        self.parts.append(data)


def visible_text(page: str) -> str:
    parser = VisibleTextParser()
    parser.feed(page)
    parser.close()
    return "".join(parser.parts)


class TopicTitleParser(HTMLParser):
    def __init__(self, topic_url: str) -> None:
        super().__init__(convert_charrefs=True)
        self.topic_url = topic_url
        self.h1_depth = 0
        self.in_topic_link = False
        self.parts: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag == "h1":
            self.h1_depth += 1
        elif tag == "a" and self.h1_depth > 0 and dict(attrs).get("href") == self.topic_url:
            self.in_topic_link = True

    def handle_endtag(self, tag: str) -> None:
        if tag == "a":
            self.in_topic_link = False
        elif tag == "h1" and self.h1_depth > 0:
            self.h1_depth -= 1

    def handle_data(self, data: str) -> None:
        if self.in_topic_link:
            self.parts.append(data)


def visible_topic_title(page: str, topic_url: str) -> str:
    parser = TopicTitleParser(topic_url)
    parser.feed(page)
    parser.close()
    return "".join(parser.parts)
