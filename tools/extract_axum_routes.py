#!/usr/bin/env python3
"""Extract Axum route declarations from the Rust port.

Besides inline ``get(...).post(...)`` expressions, the application keeps
several compatibility-sensitive method matrices in named ``MethodRouter``
builders.  Treating those calls as ``ANY`` hides accidental extra methods, so
the extractor resolves same-tree ``module::builder()`` calls as well.
"""
from __future__ import annotations

import argparse
import csv
import json
import re
from pathlib import Path


CFG_TEST_ATTRIBUTE = re.compile(r"(?m)^[ \t]*#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\][ \t]*")


def _raw_string_end(text: str, start: int) -> int | None:
    """Return the end of a Rust raw string beginning at ``start``."""
    i = start
    if text.startswith("br", i) or text.startswith("cr", i):
        i += 2
    elif text.startswith("r", i):
        i += 1
    else:
        return None
    hashes = 0
    while i < len(text) and text[i] == "#":
        hashes += 1
        i += 1
    if i >= len(text) or text[i] != '"':
        return None
    terminator = '"' + "#" * hashes
    end = text.find(terminator, i + 1)
    return len(text) if end < 0 else end + len(terminator)


def _item_end(text: str, start: int) -> int:
    """Find a cfg-gated Rust item's end without counting lexical braces."""
    i = start
    block_comment_depth = 0
    while i < len(text):
        if block_comment_depth:
            if text.startswith("/*", i):
                block_comment_depth += 1
                i += 2
            elif text.startswith("*/", i):
                block_comment_depth -= 1
                i += 2
            else:
                i += 1
            continue
        if text.startswith("//", i):
            newline = text.find("\n", i + 2)
            i = len(text) if newline < 0 else newline + 1
            continue
        if text.startswith("/*", i):
            block_comment_depth = 1
            i += 2
            continue
        raw_end = _raw_string_end(text, i)
        if raw_end is not None:
            i = raw_end
            continue
        if text[i] == '"':
            i += 1
            while i < len(text):
                if text[i] == "\\":
                    i += 2
                elif text[i] == '"':
                    i += 1
                    break
                else:
                    i += 1
            continue
        if text[i] == "'":
            # A Rust lifetime has no closing quote. Treat this as a character
            # literal only when a closing quote occurs on the same line.
            j = i + 1
            escaped = False
            while j < len(text) and text[j] not in "\r\n":
                if not escaped and text[j] == "'":
                    i = j + 1
                    break
                if not escaped and text[j] == "\\":
                    escaped = True
                else:
                    escaped = False
                j += 1
            else:
                i += 1
            continue
        if text[i] == "{":
            _body, end = capture_block(text, i)
            return end
        if text[i] == ";":
            return i + 1
        i += 1
    return len(text)


def _without_non_code(text: str) -> str:
    """Mask Rust comments and strings so attributes are found only in code."""
    chars = list(text)

    def mask(start: int, end: int) -> None:
        for pos in range(start, end):
            if chars[pos] not in "\r\n":
                chars[pos] = " "

    i = 0
    block_comment_depth = 0
    block_start = 0
    while i < len(text):
        if block_comment_depth:
            if text.startswith("/*", i):
                block_comment_depth += 1
                i += 2
            elif text.startswith("*/", i):
                block_comment_depth -= 1
                i += 2
                if not block_comment_depth:
                    mask(block_start, i)
            else:
                i += 1
            continue
        if text.startswith("//", i):
            end = text.find("\n", i + 2)
            end = len(text) if end < 0 else end
            mask(i, end)
            i = end
            continue
        if text.startswith("/*", i):
            block_start = i
            block_comment_depth = 1
            i += 2
            continue
        raw_end = _raw_string_end(text, i)
        if raw_end is not None:
            mask(i, raw_end)
            i = raw_end
            continue
        if text[i] == '"':
            start = i
            i += 1
            while i < len(text):
                if text[i] == "\\":
                    i += 2
                elif text[i] == '"':
                    i += 1
                    break
                else:
                    i += 1
            mask(start, min(i, len(text)))
            continue
        i += 1
    if block_comment_depth:
        mask(block_start, len(text))
    return "".join(chars)


def without_cfg_test_items(text: str) -> str:
    """Mask ``#[cfg(test)]`` items while preserving production line numbers."""
    ranges: list[tuple[int, int]] = []
    search_text = _without_non_code(text)
    cursor = 0
    while match := CFG_TEST_ATTRIBUTE.search(search_text, cursor):
        end = _item_end(text, match.end())
        ranges.append((match.start(), end))
        cursor = max(end, match.end())
    if not ranges:
        return text
    chars = list(text)
    for start, end in ranges:
        for i in range(start, end):
            if chars[i] not in "\r\n":
                chars[i] = " "
    return "".join(chars)


def capture_call(text: str, open_pos: int) -> tuple[str, int]:
    depth = 0
    quote = False
    esc = False
    for i in range(open_pos, len(text)):
        c = text[i]
        if quote:
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                quote = False
        else:
            if c == '"':
                quote = True
            elif c == '(':
                depth += 1
            elif c == ')':
                depth -= 1
                if depth == 0:
                    return text[open_pos + 1 : i], i + 1
    return text[open_pos + 1 :], len(text)


def capture_block(text: str, open_pos: int) -> tuple[str, int]:
    """Return a brace-delimited Rust block, ignoring lexical braces."""
    depth = 0
    i = open_pos
    block_comment_depth = 0
    while i < len(text):
        if block_comment_depth:
            if text.startswith("/*", i):
                block_comment_depth += 1
                i += 2
            elif text.startswith("*/", i):
                block_comment_depth -= 1
                i += 2
            else:
                i += 1
            continue
        if text.startswith("//", i):
            newline = text.find("\n", i + 2)
            i = len(text) if newline < 0 else newline + 1
            continue
        if text.startswith("/*", i):
            block_comment_depth = 1
            i += 2
            continue
        raw_end = _raw_string_end(text, i)
        if raw_end is not None:
            i = raw_end
            continue
        if text[i] == '"':
            i += 1
            while i < len(text):
                if text[i] == "\\":
                    i += 2
                elif text[i] == '"':
                    i += 1
                    break
                else:
                    i += 1
            continue
        if text[i] == "'":
            j = i + 1
            escaped = False
            while j < len(text) and text[j] not in "\r\n":
                if not escaped and text[j] == "'":
                    i = j + 1
                    break
                if not escaped and text[j] == "\\":
                    escaped = True
                else:
                    escaped = False
                j += 1
            else:
                i += 1
            continue
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[open_pos + 1 : i], i + 1
        i += 1
    return text[open_pos + 1 :], len(text)


def methods(expr: str) -> list[str]:
    result = []
    for name, verb in [("get", "GET"), ("post", "POST"), ("put", "PUT"), ("delete", "DELETE"), ("patch", "PATCH")]:
        if re.search(rf"\b{name}\s*\(", expr) or re.search(rf"\.{name}\s*\(", expr):
            result.append(verb)
    return result or ["ANY"]


def method_router_builders(route_files: list[Path]) -> dict[tuple[str, str], str]:
    """Index public named MethodRouter builders by Rust module and function."""
    builders: dict[tuple[str, str], str] = {}
    declaration = re.compile(
        r"\bpub(?:\(crate\))?\s+fn\s+(\w+)\s*\([^)]*\)\s*"
        r"->\s*(?:[\w:]+::)?MethodRouter(?:\s*<[^>{}]+>)?\s*\{",
        flags=re.S,
    )
    for file in route_files:
        text = without_cfg_test_items(file.read_text(errors="ignore"))
        search_text = _without_non_code(text)
        for match in declaration.finditer(search_text):
            body, _end = capture_block(text, match.end() - 1)
            builders[(file.stem, match.group(1))] = body
    return builders


def resolved_methods(
    expr: str, builders: dict[tuple[str, str], str]
) -> list[str]:
    wrapper = re.fullmatch(r"\s*auto\s*\((.*)\)\s*", expr, flags=re.S)
    if wrapper is not None:
        return resolved_methods(wrapper.group(1), builders)
    direct = methods(expr)
    if direct != ["ANY"]:
        return direct
    call = re.match(r"\s*(\w+)::(\w+)\s*\(\s*\)", expr)
    if call is None:
        return direct
    body = builders.get((call.group(1), call.group(2)))
    return methods(body) if body is not None else direct


def split_first_arg(body: str) -> tuple[str, str] | None:
    m = re.match(r'\s*"([^"]+)"\s*,\s*(.*)$', body, flags=re.S)
    if not m:
        return None
    expr = m.group(2).strip()
    if expr.endswith(","):
        expr = expr[:-1].rstrip()
    return m.group(1), expr


def extract_routes(root: Path) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    route_files = sorted((root / "src/routes").glob("*.rs"))
    builders = method_router_builders(route_files)
    for file in route_files:
        text = without_cfg_test_items(file.read_text(errors="ignore"))
        search_text = _without_non_code(text)
        for m in re.finditer(r"\.route\s*\(", search_text):
            body, _end = capture_call(text, m.end() - 1)
            parsed = split_first_arg(body)
            if not parsed:
                continue
            path, expr = parsed
            rows.append(
                {
                    "path": path,
                    "methods": resolved_methods(expr, builders),
                    "handler": re.sub(r"\s+", " ", expr),
                    "source": str(file.relative_to(root)),
                    "line": text.count("\n", 0, m.start()) + 1,
                }
            )
    return rows


def write_csv(rows: list[dict[str, object]], output: Path) -> None:
    with output.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(
            fh,
            fieldnames=["methods", "path", "handler", "source", "line"],
            lineterminator="\n",
        )
        writer.writeheader()
        for row in rows:
            writer.writerow({**row, "methods": ",".join(row["methods"])})


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path, nargs="?", default=Path.cwd())
    parser.add_argument("--json", type=Path)
    parser.add_argument("--csv", type=Path)
    args = parser.parse_args()
    routes = extract_routes(args.root)
    if args.json:
        args.json.write_text(json.dumps(routes, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if args.csv:
        write_csv(routes, args.csv)
    if not args.json and not args.csv:
        print(json.dumps(routes, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
