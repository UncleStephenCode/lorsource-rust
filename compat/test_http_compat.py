#!/usr/bin/env python3
"""HTTP compatibility smoke tests for old lorsource -> Rust port.

Usage without old app:
  NEW_BASE_URL=http://localhost:8080 python3 compat/test_http_compat.py

Usage with both apps:
  OLD_BASE_URL=http://localhost:8081 NEW_BASE_URL=http://localhost:8080 \
    python3 compat/test_http_compat.py

The test intentionally compares coarse behaviour first: status class, redirect
path and content-type family. Body equality is not expected because JSP and
Askama markup differ during the port.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path


@dataclass
class Response:
    status: int
    content_type: str
    location_path: str | None
    body_prefix: bytes


def request(base: str, path: str, method: str) -> Response:
    url = urllib.parse.urljoin(base.rstrip("/") + "/", path.lstrip("/"))
    opener = urllib.request.build_opener(NoRedirectHandler)
    req = urllib.request.Request(url, method=method.upper(), headers={"User-Agent": "lorsource-rust-compat/1"})
    try:
        with opener.open(req, timeout=15) as resp:
            return Response(resp.status, resp.headers.get("content-type", ""), None, resp.read(256))
    except urllib.error.HTTPError as e:
        location = e.headers.get("location")
        location_path = urllib.parse.urlparse(location).path if location else None
        return Response(e.code, e.headers.get("content-type", ""), location_path, e.read(256))


class NoRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):  # type: ignore[override]
        return None


def status_class(status: int) -> int:
    return status // 100


def content_family(content_type: str) -> str:
    return content_type.split(";", 1)[0].strip().split("/", 1)[0]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--matrix", type=Path, default=Path(__file__).with_name("endpoints.json"))
    parser.add_argument("--old", default=os.environ.get("OLD_BASE_URL"))
    parser.add_argument("--new", default=os.environ.get("NEW_BASE_URL", "http://localhost:8080"))
    args = parser.parse_args()

    matrix = json.loads(args.matrix.read_text(encoding="utf-8"))
    failures: list[str] = []

    for case in matrix:
        method = case.get("method", "GET")
        new_resp = request(args.new, case["new"], method)
        expected = case.get("new_expected_status")
        if expected is not None and new_resp.status != expected:
            failures.append(f"{case['name']}: new status {new_resp.status}, expected {expected}")
            continue
        if expected is None and new_resp.status == 404:
            failures.append(f"{case['name']}: new endpoint unexpectedly 404: {case['new']}")
            continue
        if args.old and case.get("compare", True):
            old_resp = request(args.old, case["old"], method)
            if status_class(old_resp.status) != status_class(new_resp.status):
                failures.append(f"{case['name']}: status class old={old_resp.status} new={new_resp.status}")
            if old_resp.location_path and new_resp.location_path and old_resp.location_path != new_resp.location_path:
                failures.append(f"{case['name']}: redirect old={old_resp.location_path} new={new_resp.location_path}")
            if old_resp.status < 400 and new_resp.status < 400 and content_family(old_resp.content_type) != content_family(new_resp.content_type):
                failures.append(f"{case['name']}: content type old={old_resp.content_type} new={new_resp.content_type}")
        print(f"{case['name']}: {method} {case['new']} -> {new_resp.status} {new_resp.content_type}")

    if failures:
        print("\nFAILURES:", file=sys.stderr)
        for failure in failures:
            print("- " + failure, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
