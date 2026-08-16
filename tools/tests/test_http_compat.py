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
        self.last_request = None

    def open(self, request, timeout: int):
        del timeout
        self.last_request = request
        if request.get_method() == "GET":
            self.client.cookies.set_cookie(csrf_cookie())
            return StubResponse(200, "text/html; charset=utf-8", b'<input name="csrf">')
        query = urllib.parse.parse_qs(
            urllib.parse.urlsplit(request.full_url).query, keep_blank_values=True
        )
        form = urllib.parse.parse_qs((request.data or b"").decode("utf-8"))
        if query.get("csrf", form.get("csrf")) != ["compat-token"]:
            return StubResponse(403, "text/plain", b"forbidden")
        return StubResponse(200, "application/json", b'{"html":"ok"}')


def client():
    value = compat.HttpClient("http://example.test")
    value.opener = StubOpener(value)
    return value


class HttpCompatibilityClientTest(unittest.TestCase):
    def test_response_captures_security_and_expiry_headers(self) -> None:
        headers = email.message.Message()
        headers["Content-Type"] = "text/html"
        headers["X-Frame-Options"] = "DENY"
        headers["X-XSS-Protection"] = "0"
        headers["Expires"] = "Thu, 01 Jan 1970 00:00:00 GMT"

        response = compat.response_value(200, headers, b"ok")

        self.assertEqual("DENY", response.x_frame_options)
        self.assertEqual("0", response.x_xss_protection)
        self.assertEqual("Thu, 01 Jan 1970 00:00:00 GMT", response.expires)

    def test_post_bootstraps_cookie_and_injects_csrf(self) -> None:
        http_client = client()

        response = http_client.request("/submit", "POST", "value=one")

        self.assertEqual(200, response.status)
        self.assertEqual(b'{"html":"ok"}', response.body)

    def test_omitted_csrf_exercises_negative_contract(self) -> None:
        http_client = client()

        response = http_client.request("/submit", "POST", "value=one", csrf_mode="omit")

        self.assertEqual(403, response.status)

    def test_query_csrf_precedes_a_conflicting_form_value(self) -> None:
        http_client = client()

        response = http_client.request(
            "/submit?topic=42", "POST", "csrf=wrong", csrf_mode="query"
        )

        self.assertEqual(200, response.status)

    def test_explicit_non_form_content_type_is_preserved(self) -> None:
        http_client = client()

        http_client.request(
            "/submit",
            "POST",
            "csrf=compat-token",
            csrf_mode="omit",
            extra_headers={"Content-Type": "text/plain"},
        )

        self.assertEqual(
            "text/plain", http_client.opener.last_request.get_header("Content-type")
        )

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

    def test_exact_json_and_raw_content_type_contracts(self) -> None:
        headers = email.message.Message()
        headers["Content-Type"] = "application/json; charset=UTF-8"
        response = compat.response_value(
            200, headers, b'{"errors":["topic"],"preview":null}'
        )
        case = {
            "name": "ajax validation",
            "new_expected_content_type_raw": "application/json; charset=UTF-8",
            "new_expected_json": {"errors": ["topic"], "preview": None},
        }
        self.assertEqual([], compat.validate_expected(case, "new", response))

        case["new_expected_json"] = {"errors": [], "preview": None}
        self.assertEqual(1, len(compat.validate_expected(case, "new", response)))

    def test_differential_json_and_raw_content_type_comparison(self) -> None:
        old_headers = email.message.Message()
        old_headers["Content-Type"] = "application/json; charset=UTF-8"
        new_headers = email.message.Message()
        new_headers["Content-Type"] = "application/json"
        old_response = compat.response_value(200, old_headers, b'{"value":1}')
        new_response = compat.response_value(200, new_headers, b'{"value":2}')
        failures = compat.compare_responses(
            {
                "name": "json differential",
                "compare_exact": True,
                "compare_content_type_raw": True,
                "compare_json": True,
            },
            old_response,
            new_response,
        )
        self.assertEqual(2, len(failures))

    def test_exact_empty_firewall_body_contract(self) -> None:
        response = compat.response_value(400, email.message.Message(), b"")
        case = {
            "name": "firewall",
            "new_expected_status": 400,
            "new_expected_content_type_raw": "",
            "new_expected_body": "",
        }
        self.assertEqual([], compat.validate_expected(case, "new", response))

        nonempty = compat.response_value(400, email.message.Message(), b"rejected")
        self.assertEqual(1, len(compat.validate_expected(case, "new", nonempty)))

    def test_differential_exact_body_comparison(self) -> None:
        old_response = compat.response_value(400, email.message.Message(), b"")
        new_response = compat.response_value(400, email.message.Message(), b"rejected")
        failures = compat.compare_responses(
            {"name": "body differential", "compare_exact": True, "compare_body_exact": True},
            old_response,
            new_response,
        )
        self.assertEqual(1, len(failures))

    def test_known_difference_can_validate_each_side_without_comparing(self) -> None:
        case = {
            "name": "intentional method hardening",
            "old_expected_status": 200,
            "new_expected_status": 405,
            "skip_response_comparison": True,
        }
        old_response = compat.response_value(200, email.message.Message(), b"")
        new_response = compat.response_value(405, email.message.Message(), b"")
        self.assertEqual([], compat.validate_expected(case, "old", old_response))
        self.assertEqual([], compat.validate_expected(case, "new", new_response))
        self.assertTrue(case["skip_response_comparison"])

    def test_differential_probe_can_allow_an_unpinned_new_404(self) -> None:
        response = compat.response_value(404, email.message.Message(), b"")

        self.assertTrue(compat.unexpected_new_404({"name": "strict"}, response))
        self.assertFalse(
            compat.unexpected_new_404(
                {"name": "differential binder", "allow_new_404": True}, response
            )
        )
        self.assertFalse(
            compat.unexpected_new_404(
                {"name": "pinned", "expected_status": 404}, response
            )
        )

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
                "allow": "",
                "content_length": "",
                "x_frame_options": "",
                "x_xss_protection": "",
                "expires": "",
            },
            compat.report_response(response),
        )


if __name__ == "__main__":
    unittest.main()
