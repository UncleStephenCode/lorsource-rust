#!/usr/bin/env python3
"""HTTP compatibility smoke tests for old lorsource -> Rust port.

Usage without old app:
  NEW_BASE_URL=http://localhost:8181 python3 compat/test_http_compat.py

Usage with both apps:
  OLD_BASE_URL=http://localhost:8081 NEW_BASE_URL=http://localhost:8181 \
    python3 compat/test_http_compat.py

The test intentionally compares coarse behaviour first: status class, redirect
path and content-type family. Body equality is not expected because JSP and
Askama markup differ during the port.
"""
from __future__ import annotations

import argparse
import datetime
import http.cookiejar
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
    location_target: str | None
    location_fragment: str | None
    location_raw: str | None
    body: bytes
    set_cookie_names: frozenset[str]
    cache_control: str
    allow: str
    content_length: str


class HttpClient:
    """Small stateful browser substitute with cookies and disabled redirects."""

    def __init__(self, base: str) -> None:
        self.base = base.rstrip("/") + "/"
        self.cookies = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            NoRedirectHandler,
            urllib.request.HTTPCookieProcessor(self.cookies),
        )

    def cookie(self, name: str) -> str | None:
        return next((cookie.value for cookie in self.cookies if cookie.name == name), None)

    def ensure_csrf(self) -> str:
        token = self.cookie("CSRF_TOKEN")
        if token:
            return token.strip('"')
        response = self.request("/", "GET")
        if response.status >= 400:
            raise RuntimeError(f"CSRF bootstrap GET / failed with {response.status}")
        token = self.cookie("CSRF_TOKEN")
        if not token:
            raise RuntimeError("CSRF bootstrap GET / did not set CSRF_TOKEN")
        return token.strip('"')

    def request(
        self,
        path: str,
        method: str,
        data: str | None = None,
        csrf_mode: str = "auto",
        extra_headers: dict[str, str] | None = None,
    ) -> Response:
        method = method.upper()
        if method == "POST" and csrf_mode == "query":
            parsed_path = urllib.parse.urlsplit(path)
            query_pairs = urllib.parse.parse_qsl(
                parsed_path.query, keep_blank_values=True
            )
            if not any(key == "csrf" for key, _value in query_pairs):
                query_pairs.append(("csrf", self.ensure_csrf()))
            path = urllib.parse.urlunsplit(
                (
                    parsed_path.scheme,
                    parsed_path.netloc,
                    parsed_path.path,
                    urllib.parse.urlencode(query_pairs),
                    parsed_path.fragment,
                )
            )
        elif method == "POST" and csrf_mode != "omit":
            pairs = urllib.parse.parse_qsl(data or "", keep_blank_values=True)
            if not any(key == "csrf" for key, _value in pairs):
                token = "invalid-csrf-token" if csrf_mode == "invalid" else self.ensure_csrf()
                pairs.append(("csrf", token))
            data = urllib.parse.urlencode(pairs)

        url = urllib.parse.urljoin(self.base, path.lstrip("/"))
        headers = {"User-Agent": "lorsource-rust-compat/2"}
        headers.update(extra_headers or {})
        body = None
        if data is not None:
            body = data.encode("utf-8")
            headers["Content-Type"] = "application/x-www-form-urlencoded"
        request = urllib.request.Request(url, data=body, method=method, headers=headers)
        try:
            with self.opener.open(request, timeout=15) as response:
                return response_value(response.status, response.headers, response.read(1_048_576))
        except urllib.error.HTTPError as error:
            return response_value(error.code, error.headers, error.read(1_048_576))


def response_value(status: int, headers, body: bytes) -> Response:
    location = headers.get("location")
    parsed_location = urllib.parse.urlparse(location) if location else None
    location_path = parsed_location.path if parsed_location else None
    location_target = None
    if parsed_location:
        location_target = parsed_location.path
        if parsed_location.query:
            location_target += "?" + parsed_location.query
    cookie_names = frozenset(
        value.split("=", 1)[0].strip()
        for value in (headers.get_all("set-cookie") or [])
        if "=" in value
    )
    return Response(
        status=status,
        content_type=headers.get("content-type", ""),
        location_path=location_path,
        location_target=location_target,
        location_fragment=parsed_location.fragment if parsed_location else None,
        location_raw=location,
        body=body,
        set_cookie_names=cookie_names,
        cache_control=headers.get("cache-control", ""),
        allow=headers.get("allow", ""),
        content_length=headers.get("content-length", ""),
    )


class NoRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):  # type: ignore[override]
        return None


def status_class(status: int) -> int:
    return status // 100


def content_family(content_type: str) -> str:
    return content_type.split(";", 1)[0].strip().split("/", 1)[0]


def media_type(content_type: str) -> str:
    return content_type.split(";", 1)[0].strip().lower()


def report_response(response: Response) -> dict[str, object]:
    return {
        "status": response.status,
        "content_type": response.content_type,
        "location": response.location_target,
        "location_raw": response.location_raw,
        "set_cookie_names": sorted(response.set_cookie_names),
        "cache_control": response.cache_control,
        "allow": response.allow,
        "content_length": response.content_length,
    }


def expected(case: dict[str, object], side: str, name: str):
    return case.get(f"{side}_{name}", case.get(name))


def validate_expected(
    case: dict[str, object], side: str, response: Response
) -> list[str]:
    label = f"{case['name']}: {side}"
    failures: list[str] = []
    expected_status = expected(case, side, "expected_status")
    if expected_status is not None and response.status != expected_status:
        failures.append(f"{label} status {response.status}, expected {expected_status}")

    expected_type = expected(case, side, "expected_content_type")
    if expected_type is not None and media_type(response.content_type) != str(expected_type):
        failures.append(
            f"{label} content type {response.content_type!r}, expected {expected_type!r}"
        )

    expected_location = expected(case, side, "expected_location")
    if expected_location is not None and response.location_target != expected_location:
        failures.append(
            f"{label} redirect {response.location_target!r}, expected {expected_location!r}"
        )

    body_text = response.body.decode("utf-8", errors="replace")
    for fragment in expected(case, side, "body_contains") or []:
        if str(fragment) not in body_text:
            failures.append(f"{label} body is missing {fragment!r}")
    for fragment in expected(case, side, "body_not_contains") or []:
        if str(fragment) in body_text:
            failures.append(f"{label} body unexpectedly contains {fragment!r}")

    expected_cookies = expected(case, side, "expected_cookie_names") or []
    missing_cookies = set(map(str, expected_cookies)) - response.set_cookie_names
    if missing_cookies:
        failures.append(f"{label} missing Set-Cookie values {sorted(missing_cookies)!r}")
    exact_cookies = expected(case, side, "expected_cookie_names_exact")
    if exact_cookies is not None:
        exact_cookie_set = frozenset(map(str, exact_cookies))
        if response.set_cookie_names != exact_cookie_set:
            failures.append(
                f"{label} Set-Cookie values {sorted(response.set_cookie_names)!r}, "
                f"expected exactly {sorted(exact_cookie_set)!r}"
            )
    expected_cache_control = expected(case, side, "expected_cache_control")
    if expected_cache_control is not None and response.cache_control != str(expected_cache_control):
        failures.append(
            f"{label} Cache-Control {response.cache_control!r}, expected {expected_cache_control!r}"
        )
    expected_location_raw = expected(case, side, "expected_location_raw")
    if expected_location_raw is not None and response.location_raw != str(expected_location_raw):
        failures.append(
            f"{label} raw redirect {response.location_raw!r}, expected {expected_location_raw!r}"
        )
    expected_allow = expected(case, side, "expected_allow")
    if expected_allow is not None and response.allow != str(expected_allow):
        failures.append(f"{label} Allow {response.allow!r}, expected {expected_allow!r}")
    expected_content_length = expected(case, side, "expected_content_length")
    if expected_content_length is not None and response.content_length != str(expected_content_length):
        failures.append(
            f"{label} Content-Length {response.content_length!r}, expected {expected_content_length!r}"
        )
    return failures


def compare_responses(
    case: dict[str, object], old_response: Response, new_response: Response
) -> list[str]:
    failures: list[str] = []
    exact = bool(case.get("compare_exact", False))
    if exact and old_response.status != new_response.status:
        failures.append(
            f"{case['name']}: status old={old_response.status} new={new_response.status}"
        )
    elif not exact and status_class(old_response.status) != status_class(new_response.status):
        failures.append(
            f"{case['name']}: status class old={old_response.status} new={new_response.status}"
        )

    old_location = old_response.location_target if exact else old_response.location_path
    new_location = new_response.location_target if exact else new_response.location_path
    if old_location is not None or new_location is not None:
        if old_location != new_location:
            failures.append(
                f"{case['name']}: redirect old={old_location!r} new={new_location!r}"
            )

    if old_response.status < 400 and new_response.status < 400:
        old_type = media_type(old_response.content_type) if exact else content_family(old_response.content_type)
        new_type = media_type(new_response.content_type) if exact else content_family(new_response.content_type)
        if old_type != new_type:
            failures.append(
                f"{case['name']}: content type old={old_response.content_type} new={new_response.content_type}"
            )

    if case.get("compare_cookies") and old_response.set_cookie_names != new_response.set_cookie_names:
        failures.append(
            f"{case['name']}: cookies old={sorted(old_response.set_cookie_names)!r} "
            f"new={sorted(new_response.set_cookie_names)!r}"
        )
    if case.get("compare_cache_control") and old_response.cache_control != new_response.cache_control:
        failures.append(
            f"{case['name']}: Cache-Control old={old_response.cache_control!r} "
            f"new={new_response.cache_control!r}"
        )
    if case.get("compare_location_raw") and old_response.location_raw != new_response.location_raw:
        failures.append(
            f"{case['name']}: raw redirect old={old_response.location_raw!r} "
            f"new={new_response.location_raw!r}"
        )
    if case.get("compare_allow") and old_response.allow != new_response.allow:
        failures.append(
            f"{case['name']}: Allow old={old_response.allow!r} new={new_response.allow!r}"
        )
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--matrix", type=Path, default=Path(__file__).with_name("endpoints.json"))
    parser.add_argument("--old", default=os.environ.get("OLD_BASE_URL"))
    parser.add_argument("--new", default=os.environ.get("NEW_BASE_URL", "http://localhost:8181"))
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()

    matrix = json.loads(args.matrix.read_text(encoding="utf-8"))
    failures: list[str] = []
    new_client = HttpClient(args.new)
    old_client = HttpClient(args.old) if args.old else None
    results: list[dict[str, object]] = []

    for case in matrix:
        if case.get("fresh_session"):
            case_new_client = HttpClient(args.new)
            case_old_client = HttpClient(args.old) if args.old else None
        else:
            case_new_client = new_client
            case_old_client = old_client
        method = case.get("method", "GET")
        data = case.get("data")
        csrf_mode = str(case.get("csrf_mode", "auto"))
        case_headers = {str(key): str(value) for key, value in case.get("headers", {}).items()}
        new_resp = case_new_client.request(case["new"], method, data, csrf_mode, case_headers)
        result: dict[str, object] = {
            "name": case["name"],
            "method": method,
            "new_path": case["new"],
            "new": report_response(new_resp),
        }
        failures.extend(validate_expected(case, "new", new_resp))
        if expected(case, "new", "expected_status") is None and new_resp.status == 404:
            failures.append(f"{case['name']}: new endpoint unexpectedly 404: {case['new']}")
        if case_old_client and case.get("compare", True):
            old_resp = case_old_client.request(case["old"], method, data, csrf_mode, case_headers)
            result["old_path"] = case["old"]
            result["old"] = report_response(old_resp)
            failures.extend(validate_expected(case, "old", old_resp))
            failures.extend(compare_responses(case, old_resp, new_resp))
        results.append(result)
        print(f"{case['name']}: {method} {case['new']} -> {new_resp.status} {new_resp.content_type}")

    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(
            json.dumps(
                {
                    "generated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    "matrix": str(args.matrix),
                    "old_base_url": args.old,
                    "new_base_url": args.new,
                    "passed": not failures,
                    "failures": failures,
                    "cases": results,
                },
                ensure_ascii=False,
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    if failures:
        print("\nFAILURES:", file=sys.stderr)
        for failure in failures:
            print("- " + failure, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
