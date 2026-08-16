#!/usr/bin/env python3
"""Regression guard for the current Java JSON/AJAX and ANY-method surface."""

from __future__ import annotations

import json
import os
import unittest
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
# `run-compatibility-suite.sh` regenerates this file from `ORIGINAL_ROOT`
# immediately before the source-contract tests run.
JAVA_ROUTES = ROOT / "docs/generated/original_routes.json"


def java_root_from_environment() -> Path:
    """Use the pinned Java checkout selected by CI, with the local sibling fallback."""
    return Path(
        os.environ.get("ORIGINAL_ROOT", str(ROOT.parent / "lorsource-java"))
    ).resolve()


JAVA_ROOT = java_root_from_environment()


class JavaApiSourceContractTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.routes = json.loads(JAVA_ROUTES.read_text(encoding="utf-8"))

    def test_java_source_root_honors_the_workflow_checkout(self) -> None:
        selected = ROOT / ".ci" / "pinned-java-contract"
        with patch.dict(os.environ, {"ORIGINAL_ROOT": str(selected)}):
            self.assertEqual(selected.resolve(), java_root_from_environment())

    def test_response_body_inventory_is_explicit_and_stable(self) -> None:
        actual = {
            (
                row["path"],
                tuple(row["methods"]),
                tuple(row["params"]),
                tuple(row["headers"]),
                row["controller"],
                row["handler"],
            )
            for row in self.routes
            if row["response_body"]
        }
        expected = {
            ("/add_comment_ajax", ("POST",), (), (), "AddCommentController", "addCommentAjax"),
            ("/admin/geoip", ("GET",), (), (), "GeoLocationController", "geoip"),
            ("/check-login", ("ANY",), (), (), "RegisterController", "ajaxLoginCheck"),
            ("/markup/preview", ("POST",), (), (), "MarkupPreviewController", "preview"),
            ("/memories.jsp", ("POST",), ("add",), (), "MemoriesController", "add"),
            ("/memories.jsp", ("POST",), ("remove",), (), "MemoriesController", "remove"),
            (
                "/notifications-click/ajax",
                ("POST",),
                (),
                (),
                "UserEventController",
                "clickNotificationsAjax",
            ),
            (
                "/notifications-count",
                ("GET",),
                (),
                (),
                "UserEventApiController",
                "getEventsCount",
            ),
            (
                "/notifications-reset",
                ("POST",),
                (),
                (),
                "UserEventApiController",
                "resetNotifications",
            ),
            (
                "/people/{nick}/profile",
                ("GET", "HEAD"),
                ("year-stats",),
                (),
                "WhoisController",
                "yearStats",
            ),
            (
                "/reactions/ajax",
                ("POST",),
                ("comment",),
                (),
                "ReactionController",
                "setCommentReactionAjax",
            ),
            (
                "/reactions/ajax",
                ("POST",),
                ("!comment",),
                (),
                "ReactionController",
                "setTopicReactionAjax",
            ),
            ("/tags", ("ANY",), ("term",), (), "TagController", "showTagListHandlerJSON"),
            (
                "/user-filter/favorite-tag",
                ("POST",),
                ("add",),
                ("Accept=application/json",),
                "UserFilterController",
                "favoriteTagAddJSON",
            ),
            (
                "/user-filter/favorite-tag",
                ("POST",),
                ("del",),
                ("Accept=application/json",),
                "UserFilterController",
                "favoriteTagDelJSON",
            ),
            (
                "/user-filter/ignore-tag",
                ("POST",),
                ("add",),
                ("Accept=application/json",),
                "UserFilterController",
                "ignoreTagAddJSON",
            ),
            (
                "/user-filter/ignore-tag",
                ("POST",),
                ("del",),
                ("Accept=application/json",),
                "UserFilterController",
                "ignoreTagDelJSON",
            ),
            (
                "/yandex-tableau",
                ("GET",),
                (),
                (),
                "UserEventApiController",
                "getYandexWidget",
            ),
        }
        self.assertEqual(expected, actual)

    def test_high_risk_binding_and_content_type_contracts(self) -> None:
        by_handler = {row["handler"]: row for row in self.routes}

        ajax_comment = by_handler["addCommentAjax"]
        self.assertEqual(
            ["msg", "nick", "original", "password", "preview", "replyto", "topic"],
            ajax_comment["form_fields"],
        )
        self.assertEqual(["application/json; charset=UTF-8"], ajax_comment["produces"])

        preview = by_handler["preview"]
        self.assertEqual(
            [("text", False), ("markup", False)],
            [(item["name"], item["required"]) for item in preview["request_params"]],
        )
        self.assertEqual(["application/json; charset=UTF-8"], preview["produces"])

        check_login = by_handler["ajaxLoginCheck"]
        self.assertEqual(
            [("nick", True)],
            [(item["name"], item["required"]) for item in check_login["request_params"]],
        )

        for handler, target in (
            ("setCommentReactionAjax", "comment"),
            ("setTopicReactionAjax", "topic"),
        ):
            self.assertEqual(
                [(target, True), ("reaction", True)],
                [
                    (item["name"], item["required"])
                    for item in by_handler[handler]["request_params"]
                ],
            )

    def test_data_feed_and_legacy_redirects_really_are_any_method(self) -> None:
        expected = {
            ("/check-login", "ajaxLoginCheck"),
            ("/group-lastmod.jsp", "topicsLastmod"),
            ("/group.jsp", "topics"),
            ("/people/{nick}", "showUserTopicsRssGone"),
            ("/people/{nick}", "showUserTopics"),
            ("/section-rss.jsp", "showRSS"),
            ("/tags", "showTagListHandlerJSON"),
            ("/tags.jsp", "oldTagsRedirectHandler"),
            ("/tracker.jsp", "trackerOldUrl"),
            ("/view-message.jsp", "getMessageOld"),
            ("/view-section.jsp", "oldLink"),
        }
        actual = {
            (row["path"], row["handler"])
            for row in self.routes
            if row["methods"] == ["ANY"] and (row["path"], row["handler"]) in expected
        }
        self.assertEqual(expected, actual)

    def test_public_form_credentials_keep_java_default_markdown_profile(self) -> None:
        auth_util = (JAVA_ROOT / "src/main/scala/ru/org/linux/auth/AuthUtil.scala").read_text(
            encoding="utf-8"
        )
        default_profile = (
            JAVA_ROOT / "src/main/scala/ru/org/linux/site/DefaultProfile.scala"
        ).read_text(encoding="utf-8")
        rust_profile = (ROOT / "src/profile.rs").read_text(encoding="utf-8")
        rust_comments = (ROOT / "src/routes/comments.rs").read_text(encoding="utf-8")
        rust_topics = (ROOT / "src/routes/topics.rs").read_text(encoding="utf-8")

        self.assertIn("profile = Profile.DEFAULT", auth_util)
        self.assertIn(
            "builder.put(FormatModeProperty, MarkupType.Markdown.formId)", default_profile
        )
        self.assertIn('pub const DEFAULT_FORMAT_MODE: &str = "markdown";', rust_profile)
        self.assertIn("crate::profile::DEFAULT_FORMAT_MODE.into()", rust_comments)
        self.assertIn("credentialed public-form posts use Profile.DEFAULT", rust_topics)

    def test_comment_queue_and_realtime_side_effect_order_matches_controllers(self) -> None:
        rust = (ROOT / "src/routes/comments.rs").read_text(encoding="utf-8")
        create = rust.split("async fn insert_comment(", 1)[1].split(
            "async fn locate_topic_or_comment", 1
        )[0]
        self.assertLess(create.index("tx.commit().await?"), create.index(".vUpdateComments(&[id])"))
        self.assertLess(
            create.index(".vUpdateComments(&[id])"),
            create.index("state.realtime.vNotifyNewComment"),
        )
        self.assertLess(
            create.index("state.realtime.vNotifyNewComment"),
            create.index("state.realtime.vNotifyEvents"),
        )
        self.assertNotIn("search_index::index_comment", create)

        edit = rust.split("pub async fn edit_comment(", 1)[1].split(
            "pub async fn delete_comment_form", 1
        )[0]
        self.assertLess(
            edit.index("tx.commit().await?"), edit.index(".vUpdateComments(&[form.original])")
        )
        self.assertNotIn("state.realtime", edit)
        self.assertNotIn("INSERT INTO user_events", edit)

    def test_topic_create_queue_is_fallible_and_precedes_realtime_for_non_drafts(self) -> None:
        rust = (ROOT / "src/routes/topics.rs").read_text(encoding="utf-8")
        create = rust.split("pub async fn create_topic(", 1)[1].split(
            "struct ModeratedTopicTemplate", 1
        )[0]
        commit = create.index("tx.commit().await?")
        non_draft = create.index("if !is_draft {", commit)
        queued = create.index(".vUpdateMessage(id, false)", non_draft)
        realtime = create.index("state.realtime.vNotifyEvents", queued)
        self.assertLess(commit, non_draft)
        self.assertLess(non_draft, queued)
        self.assertLess(queued, realtime)
        self.assertNotIn("search_index::index_topic", create)

    def test_firewall_and_servlet_parameter_defaults_are_kept_together(self) -> None:
        firewall = (ROOT / "src/http_method_firewall.rs").read_text(encoding="utf-8")
        form = (ROOT / "src/form.rs").read_text(encoding="utf-8")
        main = (ROOT / "src/main.rs").read_text(encoding="utf-8")

        for method in ("DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST", "PUT"):
            self.assertIn(f"Method::{method}", firewall)
        for forbidden in ("%3b", "%2f", "%5c", "%00", "%0a", "%0d", "%25", "%2e"):
            self.assertIn(forbidden, firewall)
        self.assertIn("(0x20..=0x7e).contains(iByte)", firewall)
        self.assertIn(r'^\p{Assigned}*$', form)
        self.assertIn("!sName.chars().any(char::is_control)", form)
        self.assertIn("middleware::from_fn(http_method_firewall::apply)", main)

    def test_csrf_body_parameters_require_a_servlet_form_media_type(self) -> None:
        csrf = (ROOT / "src/csrf.rs").read_text(encoding="utf-8")
        client = (ROOT / "compat/test_http_compat.py").read_text(encoding="utf-8")
        writes = (ROOT / "compat/test_write_flows.py").read_text(encoding="utf-8")

        self.assertIn('eq_ignore_ascii_case("application/x-www-form-urlencoded")', csrf)
        self.assertIn('eq_ignore_ascii_case("multipart/form-data")', csrf)
        self.assertIn("EnPostParameterBody::None", csrf)
        self.assertIn('headers.setdefault("Content-Type"', client)
        self.assertIn('"/logout_all_sessions"', writes)
        self.assertIn("rejected non-form csrf bodies mutated token_generation", writes)

    def test_view_message_uses_layered_read_model_and_exact_binding(self) -> None:
        legacy = (ROOT / "src/routes/legacy.rs").read_text(encoding="utf-8")
        repository = (ROOT / "src/domain/topic/repository.rs").read_text(encoding="utf-8")
        postgres = (ROOT / "src/infra/postgres/topic_repository.rs").read_text(
            encoding="utf-8"
        )
        handler = legacy.split("pub async fn legacy_view_message(", 1)[1].split(
            "mod legacy_view_message_tests", 1
        )[0]

        self.assertIn("stLegacyTopicRedirect", repository)
        self.assertIn("stLegacyTopicRedirect", postgres)
        self.assertIn("servlet_request_parameters", handler)
        self.assertNotIn("sqlx::", handler)
        self.assertIn("sRedirectViewQueryValue", legacy)

    def test_current_java_edit_comment_does_not_add_new_ref_events(self) -> None:
        java = (
            JAVA_ROOT
            / "src/main/scala/ru/org/linux/comment/CommentCreateService.scala"
        ).read_text(encoding="utf-8")
        edit = java.split("def edit(oldComment", 1)[1].split(
            "private def addEditHistoryItem", 1
        )[0]
        self.assertLess(
            edit.index("msgbaseDao.updateMessage(oldComment.id, commentBody)"),
            edit.index("val messageText = msgbaseDao.getMessageText(oldComment.id)"),
        )

    def test_user_identity_binders_keep_java_exact_nick_semantics(self) -> None:
        java_dao = (
            JAVA_ROOT / "src/main/scala/ru/org/linux/user/UserDao.scala"
        ).read_text(encoding="utf-8")
        java_events = (
            JAVA_ROOT / "src/main/scala/ru/org/linux/user/UserEventController.scala"
        ).read_text(encoding="utf-8")
        java_search = (
            JAVA_ROOT / "src/main/scala/ru/org/linux/search/SearchController.scala"
        ).read_text(encoding="utf-8")
        rust_repository = (
            ROOT / "src/infra/postgres/user_identity_repository.rs"
        ).read_text(encoding="utf-8")
        rust_legacy = (ROOT / "src/routes/legacy.rs").read_text(encoding="utf-8")
        rust_search = (ROOT / "src/routes/search.rs").read_text(encoding="utf-8")
        rust_auth = (ROOT / "src/routes/auth.rs").read_text(encoding="utf-8")

        self.assertIn("SELECT id FROM users WHERE nick=${nick}", java_dao)
        self.assertIn("currentUser.user.nick == nick", java_events)
        self.assertIn("userService.getUser(nick)", java_events)
        self.assertIn(
            "binder.registerCustomEditor(classOf[User], new UserPropertyEditor(userService))",
            java_search,
        )
        for sql_name in (
            "S_EXACT_IDENTITY",
            "S_ACTIVATION_IDENTITY",
            "S_PASSWORD_RESET_IDENTITY",
        ):
            sql = rust_repository.split(f"const {sql_name}", 1)[1].split(";", 1)[0]
            self.assertIn("WHERE nick=$1", sql)
            self.assertNotIn("lower(nick)", sql.lower())
        show_replies = rust_legacy.split("pub async fn show_replies_jsp(", 1)[1].split(
            "fn sCanonicalUserEventFilter(", 1
        )[0]
        self.assertEqual(2, show_replies.count("optExactIdentity(&nick)"))
        self.assertIn("optActivationIdentity(nick)", rust_legacy)
        self.assertIn("optExactIdentity(&sRequestedUser)", rust_search)
        self.assertIn("optPasswordResetIdentity(nick)", rust_auth)

    def test_activation_and_reset_bind_required_servlet_parameters_before_db_work(self) -> None:
        rust_legacy = (ROOT / "src/routes/legacy.rs").read_text(encoding="utf-8")
        activation = rust_legacy.split("pub async fn activate_post(", 1)[1].split(
            "fn render_activation_form(", 1
        )[0]
        rust_auth = (ROOT / "src/routes/auth.rs").read_text(encoding="utf-8")
        reset = rust_auth.split("pub async fn reset_password_with_code(", 1)[1].split(
            "fn generate_java_like_password(", 1
        )[0]

        self.assertIn("servlet_request_parameters(stRequest)", activation)
        for parameter in ("activation", "nick", "passwd"):
            self.assertIn(
                f"Required request parameter '{parameter}' is missing", activation
            )
        self.assertLess(
            activation.index("optActivationIdentity(nick)"),
            activation.index("verify_login(&state.pool, &stActivationUser.sNick"),
        )
        self.assertIn("servlet_request_parameters(stRequest)", reset)
        for parameter in ("nick", "code"):
            self.assertIn(f"Required request parameter '{parameter}' is missing", reset)
        self.assertNotIn(".trim()", activation)
        self.assertNotIn(".trim()", reset)

    def test_login_failure_is_recorded_before_the_delayed_response(self) -> None:
        rust = (ROOT / "src/routes/auth.rs").read_text(encoding="utf-8")
        helper = rust.split("async fn vRecordLoginFailureBeforeDelay", 1)[1].split(
            "pub async fn login(", 1
        )[0]
        self.assertLess(helper.index("vRecordFailedAttempt"), helper.index("futDelay.await"))
        self.assertIn(
            "failed_login_is_visible_to_concurrent_requests_before_delay_finishes", rust
        )

    def test_registration_mailbox_and_deregister_fields_match_java_contracts(self) -> None:
        java_validator = (
            JAVA_ROOT
            / "src/main/scala/ru/org/linux/user/RegisterRequestValidator.scala"
        ).read_text(encoding="utf-8")
        java_deregister = (
            JAVA_ROOT / "src/main/scala/ru/org/linux/user/DeregisterController.scala"
        ).read_text(encoding="utf-8")
        rust_address = (ROOT / "src/domain/email/address.rs").read_text(
            encoding="utf-8"
        )
        rust_legacy = (ROOT / "src/routes/legacy.rs").read_text(encoding="utf-8")

        self.assertIn("new InternetAddress(form.getEmail, true)", java_validator)
        self.assertIn("topPrivateDomain.toString", java_validator)
        self.assertIn("var acceptBlock: Boolean", java_deregister)
        self.assertIn("var acceptOneway: Boolean", java_deregister)
        self.assertIn("psl::domain", rust_address)
        self.assertIn("bSingleAddressPhrase", rust_address)
        deregister = rust_legacy.split("pub struct DeregisterForm", 1)[1].split(
            "pub async fn deregister_form", 1
        )[0]
        self.assertIn('#[serde(rename = "acceptBlock")]', deregister)
        self.assertIn('#[serde(rename = "acceptOneway")]', deregister)
        self.assertNotIn('alias = "accept_block"', deregister)
        self.assertNotIn('alias = "accept_oneway"', deregister)

    def test_tag_and_moderation_queue_sends_are_fallible_and_source_ordered(self) -> None:
        tags = (ROOT / "src/routes/tags.rs").read_text(encoding="utf-8")
        change = tags.split("pub async fn change_tag(", 1)[1].split(
            "async fn vReindexTopicIds", 1
        )[0]
        self.assertLess(change.index("vReindexTopicIds"), change.index("tx.commit().await?"))
        delete = tags.split("pub async fn delete_tag(", 1)[1]
        first_commit = delete.index("tx.commit().await?")
        self.assertLess(first_commit, delete.index("vReindexTopicIds", first_commit))
        merge_commit = delete.rindex("tx.commit().await?")
        self.assertLess(merge_commit, delete.index("reindex_topics_with_tag", merge_commit))
        self.assertIn("cQueue.vUpdateMessage(*iTopicId, true).await?", tags)
        self.assertNotIn("search_index::index_topic", tags)

        admin = (ROOT / "src/routes/admin.rs").read_text(encoding="utf-8")
        ip_delete = (ROOT / "src/application/admin/ip_mass_delete.rs").read_text(
            encoding="utf-8"
        )
        self.assertIn("cQueue.vUpdateMessage(*iTopicId, true).await?", admin)
        self.assertIn("cQueue.vUpdateComments(&stDelete.vecCommentIds).await?", admin)
        self.assertIn("self.oReindexQueue.vUpdateMessage(*iTopicId, true).await?", ip_delete)
        self.assertIn(".vUpdateComments(&stResult.vecDeletedCommentIds)", ip_delete)
        self.assertNotIn("search_index::index_topic", admin)
        self.assertNotIn("search_index::index_comment", admin)


if __name__ == "__main__":
    unittest.main()
