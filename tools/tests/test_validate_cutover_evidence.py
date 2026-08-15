from __future__ import annotations

import datetime as dt
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from validate_cutover_evidence import EvidenceError, validate_all  # noqa: E402


class CutoverEvidenceTest(unittest.TestCase):
    def setUp(self) -> None:
        self.now = dt.datetime(2026, 8, 8, 12, 0, tzinfo=dt.timezone.utc)
        self.digest = "sha256:" + "a" * 64
        self.snapshot = "prodclone-20260808-001"
        self.wal = "16/B374D848"
        self.rehearsal = "prod-rehearsal-20260808-001"
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def common(self, kind: str, evidence: object) -> dict[str, object]:
        return {
            "schema_version": 1,
            "kind": kind,
            "rehearsal_id": self.rehearsal,
            "captured_at": "2026-08-08T11:30:00Z",
            "image_digest": self.digest,
            "database_snapshot_id": self.snapshot,
            "database_wal_position": self.wal,
            "status": "passed",
            "evidence": evidence,
        }

    def documents(self) -> tuple[dict[str, object], dict[str, object], dict[str, object]]:
        config_checks = {
            name: True
            for name in {
                "lor_env_production",
                "public_https",
                "websocket_wss_same_authority",
                "runtime_database_role_least_privilege",
                "java_site_secret_continuity_verified",
                "secret_values_redacted",
                "trusted_proxy_cidrs_configured",
                "opensearch_configured",
                "captcha_configured",
                "smtp_configured",
                "admin_email_configured",
                "dev_bypasses_disabled",
                "one_active_background_scheduler",
                "telegram_proxy_configured_if_enabled",
            }
        }
        media = {
            "upload_root": "/srv/lorsource/uploads",
            "runtime_uid": 8181,
            "runtime_gid": 8181,
            "directories": ["photos", "gallery", "images"],
            "representative_files_checked": 30,
            "storage_snapshot_id": "media-snapshot-20260808-001",
            "read_probe_passed": True,
            "write_probe_passed": True,
            "atomic_rename_probe_passed": True,
            "cleanup_probe_passed": True,
            "ownership_survives_restart": True,
            "backup_restore_probe_passed": True,
        }
        adapters = {}
        for name in {
            "opensearch",
            "smtp",
            "captcha",
            "geoip",
            "tor_exit_list",
            "disposable_email_domains",
            "telegram",
        }:
            adapters[name] = {
                "status": "passed",
                "checked_at": "2026-08-08T11:40:00Z",
                "endpoint": "smtp://mail.internal:25" if name == "smtp" else f"https://{name}.internal",
                "contract_verified": True,
            }
        return (
            self.common("configuration", config_checks),
            self.common("media", media),
            self.common("external-adapters", adapters),
        )

    def write_documents(self, documents):
        paths = tuple(self.root / name for name in ("config.json", "media.json", "external.json"))
        for path, document in zip(paths, documents, strict=True):
            path.write_text(json.dumps(document), encoding="utf-8")
        return paths

    def validate(self, documents) -> str:
        paths = self.write_documents(documents)
        return validate_all(
            *paths,
            self.digest,
            self.snapshot,
            self.wal,
            24,
            now=self.now,
        )

    def test_accepts_bound_fresh_complete_evidence(self) -> None:
        self.assertEqual(self.validate(self.documents()), self.rehearsal)

    def test_rejects_placeholder_snapshot(self) -> None:
        paths = self.write_documents(self.documents())
        with self.assertRaisesRegex(EvidenceError, "placeholder"):
            validate_all(*paths, self.digest, "local-test-snapshot", self.wal, 24, now=self.now)

    def test_rejects_stale_evidence(self) -> None:
        documents = self.documents()
        for document in documents:
            document["captured_at"] = "2026-08-01T00:00:00Z"
        with self.assertRaisesRegex(EvidenceError, "older"):
            self.validate(documents)

    def test_rejects_digest_mismatch(self) -> None:
        documents = self.documents()
        documents[1]["image_digest"] = "sha256:" + "b" * 64
        with self.assertRaisesRegex(EvidenceError, "does not match"):
            self.validate(documents)

    def test_rejects_obsolete_distinct_cookie_secret_claim(self) -> None:
        documents = self.documents()
        configuration = documents[0]["evidence"]
        del configuration["java_site_secret_continuity_verified"]
        configuration["cookie_and_site_secrets_distinct"] = True
        with self.assertRaisesRegex(EvidenceError, "exact production checks"):
            self.validate(documents)

    def test_rejects_credential_bearing_endpoint(self) -> None:
        documents = self.documents()
        documents[2]["evidence"]["opensearch"]["endpoint"] = "https://user:secret@search.internal"
        with self.assertRaisesRegex(EvidenceError, "must not contain credentials"):
            self.validate(documents)

    def test_rejects_missing_external_adapter(self) -> None:
        documents = self.documents()
        del documents[2]["evidence"]["captcha"]
        with self.assertRaisesRegex(EvidenceError, "all required adapters"):
            self.validate(documents)

    def test_rejects_malformed_media_directories_without_traceback(self) -> None:
        documents = self.documents()
        documents[1]["evidence"]["directories"] = [{"name": "photos"}]
        with self.assertRaisesRegex(EvidenceError, "media directories"):
            self.validate(documents)

    def test_rejects_malformed_external_url_without_traceback(self) -> None:
        documents = self.documents()
        documents[2]["evidence"]["geoip"]["endpoint"] = "https://[invalid"
        with self.assertRaisesRegex(EvidenceError, "malformed"):
            self.validate(documents)

    def test_allows_explicitly_disabled_telegram(self) -> None:
        documents = self.documents()
        documents[2]["evidence"]["telegram"] = {
            "status": "disabled",
            "checked_at": "2026-08-08T11:40:00Z",
            "endpoint": "https://api.telegram.org",
            "contract_verified": False,
            "disabled_reason": "Telegram publishing is disabled by deployment policy",
        }
        self.assertEqual(self.validate(documents), self.rehearsal)


if __name__ == "__main__":
    unittest.main()
