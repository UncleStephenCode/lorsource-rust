from __future__ import annotations

import email.message
import http.cookiejar
import importlib.util
import sys
import unittest
import urllib.parse
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "compat/test_http_compat.py"
SPEC = importlib.util.spec_from_file_location("test_http_compat_script", MODULE_PATH)
assert SPEC and SPEC.loader
compat = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = compat
SPEC.loader.exec_module(compat)


class StubResponse:
    def __init__(self, status: int, content_type: str, body: bytes) -> None:
        self.status = status
        self.headers = email.message.Message()
        self.headers["Content-Type"] = content_type
        self._body = body

    def __enter__(self):
        return self

    def __exit__(self, *_args: object) -> None:
        pass

    def read(self, _limit: int) -> bytes:
        return self._body


def csrf_cookie() -> http.cookiejar.Cookie:
    return http.cookiejar.Cookie(
        version=0,
        name="CSRF_TOKEN",
        value="compat-token",
        port=None,
        port_specified=False,
        domain="example.test",
        domain_specified=True,
        domain_initial_dot=False,
        path="/",
        path_specified=True,
        secure=False,
        expires=None,
        discard=True,
        comment=None,
        comment_url=None,
        rest={},
        rfc2109=False,
    )


class StubOpener:
    def __init__(self, client) -> None:
        self.client = client

    def open(self, request, timeout: int):
        del timeout
        if request.get_method() == "GET":
            self.client.cookies.set_cookie(csrf_cookie())
            return StubResponse(200, "text/html; charset=utf-8", b'<input name="csrf">')
        form = urllib.parse.parse_qs((request.data or b"").decode("utf-8"))
        if form.get("csrf") != ["compat-token"]:
            return StubResponse(403, "text/plain", b"forbidden")
        return StubResponse(200, "application/json", b'{"html":"ok"}')


def client():
    value = compat.HttpClient("http://example.test")
    value.opener = StubOpener(value)
    return value


class HttpCompatibilityClientTest(unittest.TestCase):
    def test_post_bootstraps_cookie_and_injects_csrf(self) -> None:
        http_client = client()

        response = http_client.request("/submit", "POST", "value=one")

        self.assertEqual(200, response.status)
        self.assertEqual(b'{"html":"ok"}', response.body)

    def test_omitted_csrf_exercises_negative_contract(self) -> None:
        http_client = client()

        response = http_client.request("/submit", "POST", "value=one", csrf_mode="omit")

        self.assertEqual(403, response.status)

    def test_redirect_target_preserves_query_for_exact_comparison(self) -> None:
        headers = email.message.Message()
        headers["Location"] = "/target?from=compat#comment-42"
        response = compat.response_value(302, headers, b"")

        self.assertEqual(302, response.status)
        self.assertEqual("/target", response.location_path)
        self.assertEqual("/target?from=compat", response.location_target)
        self.assertEqual("comment-42", response.location_fragment)

    def test_declarative_body_content_type_and_status_contract(self) -> None:
        headers = email.message.Message()
        headers["Content-Type"] = "text/html; charset=utf-8"
        headers["Set-Cookie"] = "CSRF_TOKEN=compat-token; Path=/"
        headers["Cache-Control"] = "max-age=3600"
        response = compat.response_value(200, headers, b'<form><input name="csrf"></form>')
        case = {
            "name": "form",
            "new_expected_status": 200,
            "new_expected_content_type": "text/html",
            "new_body_contains": ['name="csrf"'],
            "new_body_not_contains": ["internal error"],
            "new_expected_cookie_names": ["CSRF_TOKEN"],
            "new_expected_cache_control": "max-age=3600",
        }

        self.assertEqual([], compat.validate_expected(case, "new", response))

    def test_exact_cookie_contract_can_assert_an_empty_static_response(self) -> None:
        response = compat.response_value(200, email.message.Message(), b"asset")
        case = {
            "name": "direct static",
            "new_expected_cookie_names_exact": [],
        }
        self.assertEqual([], compat.validate_expected(case, "new", response))

        response.set_cookie_names = frozenset({"CSRF_TOKEN"})
        self.assertEqual(1, len(compat.validate_expected(case, "new", response)))

    def test_machine_report_omits_response_body_and_keeps_protocol_fields(self) -> None:
        headers = email.message.Message()
        headers["Content-Type"] = "text/html; charset=utf-8"
        headers["Location"] = "/target?from=compat"
        headers["Set-Cookie"] = "CSRF_TOKEN=secret-value; Path=/"
        response = compat.response_value(302, headers, b"private body")

        self.assertEqual(
            {
                "status": 302,
                "content_type": "text/html; charset=utf-8",
                "location": "/target?from=compat",
                "location_raw": "/target?from=compat",
                "set_cookie_names": ["CSRF_TOKEN"],
                "cache_control": "",
            },
            compat.report_response(response),
        )


if __name__ == "__main__":
    unittest.main()
