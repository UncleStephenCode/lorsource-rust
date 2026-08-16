from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
import subprocess
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
        self.search_artifact = self.root / "search-cutover.json"

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

    @staticmethod
    def sha256(label: str) -> str:
        return "sha256:" + hashlib.sha256(label.encode("utf-8")).hexdigest()

    def bind_search_artifact(self, documents) -> None:
        search_cutover = documents[3]["evidence"]["search_cutover"]
        artifact = {
            "schema_version": 1,
            "kind": "search-cutover",
            "rehearsal_id": self.rehearsal,
            "captured_at": search_cutover["checked_at"],
            "image_digest": self.digest,
            "database_snapshot_id": self.snapshot,
            "database_wal_position": self.wal,
            **{
                name: value
                for name, value in search_cutover.items()
                if name not in {"checked_at", "artifact_sha256"}
            },
        }
        payload = json.dumps(artifact, sort_keys=True).encode("utf-8")
        self.search_artifact.write_bytes(payload)
        search_cutover["artifact_sha256"] = "sha256:" + hashlib.sha256(payload).hexdigest()

    def rewrite_search_artifact(self, documents, artifact: dict[str, object]) -> None:
        payload = json.dumps(artifact, sort_keys=True).encode("utf-8")
        self.search_artifact.write_bytes(payload)
        documents[3]["evidence"]["search_cutover"]["artifact_sha256"] = (
            "sha256:" + hashlib.sha256(payload).hexdigest()
        )

    def full_reindex_evidence(self) -> dict[str, object]:
        id_set_digest = self.sha256("canonical sorted expected and indexed message IDs")
        content_digest = self.sha256("canonical expected and indexed search documents")
        return {
            "mode": "full-reindex",
            "checked_at": "2026-08-08T11:45:00Z",
            "java_writers_stopped": True,
            "java_consumers_stopped": True,
            "rust_spool_pending": 0,
            "rust_spool_processing": 0,
            "artifact_sha256": "sha256:" + "f" * 64,
            "full_reindex_completed": True,
            "legacy_queue_disposition_recorded": True,
            "expected_documents": 5946,
            "indexed_documents": 5946,
            "reconciliation_passed": True,
            "representative_queries_checked": 12,
            "opensearch_snapshot_id": "search-snapshot-20260808-001",
            "expected_id_set_sha256": id_set_digest,
            "indexed_id_set_sha256": id_set_digest,
            "expected_content_sha256": content_digest,
            "indexed_content_sha256": content_digest,
        }

    def documents(
        self,
    ) -> tuple[dict[str, object], dict[str, object], dict[str, object], dict[str, object]]:
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
                "scheduler_timezone_configured",
                "legacy_jdbc_timezone_configured",
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
        operations = {
            "production_clone": {
                "restore_verified": True,
                "liquibase_validate_passed": True,
                "runtime_schema_contract_passed": True,
                "java_rust_comparison_passed": True,
            },
            "scheduler": {
                "original_java_timezone": "Europe/Moscow",
                "rust_scheduler_timezone": "Europe/Moscow",
                "legacy_jdbc_timezone": "Europe/Moscow",
                "timezone_match_verified": True,
                "active_scheduler_instances": 1,
                "single_scheduler_verified": True,
            },
            "search_cutover": {
                "mode": "activemq-drained",
                "checked_at": "2026-08-08T11:45:00Z",
                "java_writers_stopped": True,
                "java_consumers_stopped": True,
                "rust_spool_pending": 0,
                "rust_spool_processing": 0,
                "artifact_sha256": "sha256:" + "f" * 64,
                "queue_name": "lor.searchQueue",
                "ready_messages": 0,
                "inflight_messages": 0,
            },
            "lifecycle": {
                "sigterm_drain_passed": True,
                "restart_health_passed": True,
                "rollback_switch_passed": True,
                "post_rollback_smoke_passed": True,
            },
        }
        documents = (
            self.common("configuration", config_checks),
            self.common("media", media),
            self.common("external-adapters", adapters),
            self.common("operations", operations),
        )
        self.bind_search_artifact(documents)
        return documents

    def write_documents(self, documents):
        paths = tuple(
            self.root / name
            for name in ("config.json", "media.json", "external.json", "operations.json")
        )
        for path, document in zip(paths, documents, strict=True):
            path.write_text(json.dumps(document), encoding="utf-8")
        return paths

    def validate(self, documents) -> str:
        paths = self.write_documents(documents)
        return validate_all(
            *paths,
            self.search_artifact,
            self.digest,
            self.snapshot,
            self.wal,
            24,
            now=self.now,
        )

    def run_gate_toggle_check(self, **overrides: str) -> subprocess.CompletedProcess[str]:
        root = Path(__file__).resolve().parents[2]
        environment = os.environ.copy()
        for name in (
            "CUTOVER_REQUIRE_RELEASE_EVIDENCE",
            "CUTOVER_VALIDATE_DB",
            "CUTOVER_WRITE_FLOW",
            "CUTOVER_MODERATION_FLOW",
            "CUTOVER_DEVELOPER_DRY_RUN",
        ):
            environment.pop(name, None)
        environment.update(
            {
                "ORIGINAL_ROOT": str(root.parent / "lorsource-java"),
                "OLD_BASE_URL": "http://127.0.0.1:9",
                **overrides,
            }
        )
        return subprocess.run(
            [str(root / "scripts" / "run-cutover-gate.sh")],
            cwd=root,
            env=environment,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_accepts_bound_fresh_complete_evidence(self) -> None:
        self.assertEqual(self.validate(self.documents()), self.rehearsal)

    def test_gate_rejects_a_misspelled_boolean_instead_of_skipping(self) -> None:
        result = self.run_gate_toggle_check(CUTOVER_REQUIRE_RELEASE_EVIDENCE="treu")
        self.assertEqual(result.returncode, 2)
        self.assertIn("must be exactly true, false, 1 or 0", result.stderr)

    def test_gate_requires_explicit_developer_mode_for_any_skip(self) -> None:
        result = self.run_gate_toggle_check(CUTOVER_VALIDATE_DB="0")
        self.assertEqual(result.returncode, 2)
        self.assertIn("only with CUTOVER_DEVELOPER_DRY_RUN=true", result.stderr)

    def test_gate_validates_the_exact_bytes_retained_in_its_evidence_directory(self) -> None:
        root = Path(__file__).resolve().parents[2]
        fake_bin = self.root / "fake-bin"
        fake_bin.mkdir()
        fake_python = fake_bin / "python3"
        fake_python.write_text(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CUTOVER_FAKE_PYTHON_ARGS\"\nexit 77\n",
            encoding="utf-8",
        )
        fake_python.chmod(0o755)
        arguments_path = self.root / "validator-arguments.txt"
        evidence_dir = self.root / "retained"
        source_paths = []
        for name in ("config", "media", "external", "operations", "search"):
            path = self.root / f"source-{name}.json"
            path.write_bytes(f"source bytes for {name}\n".encode("utf-8"))
            source_paths.append(path)

        environment = os.environ.copy()
        environment.update(
            {
                "PATH": f"{fake_bin}:{environment['PATH']}",
                "ORIGINAL_ROOT": str(root.parent / "lorsource-java"),
                "OLD_BASE_URL": "http://127.0.0.1:9",
                "EVIDENCE_DIR": str(evidence_dir),
                "CUTOVER_IMAGE_DIGEST": self.digest,
                "CUTOVER_SNAPSHOT_ID": self.snapshot,
                "CUTOVER_WAL_POSITION": self.wal,
                "CUTOVER_CONFIG_MANIFEST": str(source_paths[0]),
                "CUTOVER_MEDIA_EVIDENCE": str(source_paths[1]),
                "CUTOVER_EXTERNAL_EVIDENCE": str(source_paths[2]),
                "CUTOVER_OPERATIONS_EVIDENCE": str(source_paths[3]),
                "CUTOVER_SEARCH_EVIDENCE_ARTIFACT": str(source_paths[4]),
                "CUTOVER_FAKE_PYTHON_ARGS": str(arguments_path),
                "CUTOVER_REQUIRE_RELEASE_EVIDENCE": "true",
                "CUTOVER_VALIDATE_DB": "true",
                "CUTOVER_WRITE_FLOW": "true",
                "CUTOVER_MODERATION_FLOW": "true",
                "CUTOVER_DEVELOPER_DRY_RUN": "false",
            }
        )
        result = subprocess.run(
            [str(root / "scripts" / "run-cutover-gate.sh")],
            cwd=root,
            env=environment,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 77, result.stdout + result.stderr)

        retained_names = (
            "config-manifest.redacted.json",
            "media-rehearsal.json",
            "external-adapters.json",
            "operations.json",
            "search-cutover-artifact.json",
        )
        validator_arguments = arguments_path.read_text(encoding="utf-8").splitlines()
        for option, source_path, retained_name in zip(
            ("--config", "--media", "--external", "--operations", "--search-artifact"),
            source_paths,
            retained_names,
            strict=True,
        ):
            retained_path = evidence_dir / retained_name
            self.assertEqual(
                validator_arguments[validator_arguments.index(option) + 1],
                str(retained_path),
            )
            self.assertEqual(retained_path.read_bytes(), source_path.read_bytes())

    def test_rejects_placeholder_snapshot(self) -> None:
        paths = self.write_documents(self.documents())
        with self.assertRaisesRegex(EvidenceError, "placeholder"):
            validate_all(
                *paths,
                self.search_artifact,
                self.digest,
                "local-test-snapshot",
                self.wal,
                24,
                now=self.now,
            )

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

    def test_rejects_nonempty_legacy_activemq_queue(self) -> None:
        documents = self.documents()
        documents[3]["evidence"]["search_cutover"]["ready_messages"] = 1
        self.bind_search_artifact(documents)
        with self.assertRaisesRegex(EvidenceError, "requires zero"):
            self.validate(documents)

    def test_rejects_unbound_search_artifact(self) -> None:
        documents = self.documents()
        documents[3]["evidence"]["search_cutover"]["artifact_sha256"] = (
            "sha256:" + "e" * 64
        )
        with self.assertRaisesRegex(EvidenceError, "artifact digest does not match"):
            self.validate(documents)

    def test_rejects_artifact_claim_that_disagrees_with_operations_json(self) -> None:
        documents = self.documents()
        artifact = json.loads(self.search_artifact.read_text(encoding="utf-8"))
        artifact["ready_messages"] = 999
        self.rewrite_search_artifact(documents, artifact)
        with self.assertRaisesRegex(EvidenceError, "disagrees.*ready_messages"):
            self.validate(documents)

    def test_rejects_empty_or_unparseable_search_artifact(self) -> None:
        for payload, message in ((b"", "must not be empty"), (b"ready=0", "strict UTF-8 JSON")):
            with self.subTest(payload=payload):
                documents = self.documents()
                self.search_artifact.write_bytes(payload)
                with self.assertRaisesRegex(EvidenceError, message):
                    self.validate(documents)

    def test_rejects_duplicate_keys_and_non_standard_numbers_in_strict_json(self) -> None:
        for payload, message in (
            (
                b'{"schema_version":1,"schema_version":1}',
                "duplicate object key 'schema_version'",
            ),
            (b'{"schema_version":NaN}', "non-standard JSON constant NaN"),
            (b'{"schema_version":Infinity}', "non-standard JSON constant Infinity"),
        ):
            with self.subTest(payload=payload):
                documents = self.documents()
                self.search_artifact.write_bytes(payload)
                with self.assertRaisesRegex(EvidenceError, message):
                    self.validate(documents)

    def test_rejects_search_artifact_from_another_snapshot(self) -> None:
        documents = self.documents()
        artifact = json.loads(self.search_artifact.read_text(encoding="utf-8"))
        artifact["database_snapshot_id"] = "prodclone-20260808-OTHER"
        self.rewrite_search_artifact(documents, artifact)
        with self.assertRaisesRegex(EvidenceError, "database_snapshot_id does not match"):
            self.validate(documents)

    def test_accepts_forced_full_reindex_with_exact_reconciliation(self) -> None:
        documents = self.documents()
        documents[3]["evidence"]["search_cutover"] = self.full_reindex_evidence()
        self.bind_search_artifact(documents)
        self.assertEqual(self.validate(documents), self.rehearsal)

    def test_rejects_full_reindex_count_mismatch(self) -> None:
        documents = self.documents()
        search_cutover = self.full_reindex_evidence()
        search_cutover["indexed_documents"] = 5945
        documents[3]["evidence"]["search_cutover"] = search_cutover
        self.bind_search_artifact(documents)
        with self.assertRaisesRegex(EvidenceError, "exact expected/indexed"):
            self.validate(documents)

    def test_rejects_full_reindex_id_or_content_digest_mismatch(self) -> None:
        for field, message in (
            ("indexed_id_set_sha256", "ID-set digests"),
            ("indexed_content_sha256", "content digests"),
        ):
            with self.subTest(field=field):
                documents = self.documents()
                search_cutover = self.full_reindex_evidence()
                search_cutover[field] = self.sha256(f"mismatching {field}")
                documents[3]["evidence"]["search_cutover"] = search_cutover
                self.bind_search_artifact(documents)
                with self.assertRaisesRegex(EvidenceError, message):
                    self.validate(documents)

    def test_rejects_empty_reindex_reconciliation_digest(self) -> None:
        documents = self.documents()
        search_cutover = self.full_reindex_evidence()
        empty_digest = "sha256:" + hashlib.sha256(b"").hexdigest()
        search_cutover["expected_content_sha256"] = empty_digest
        search_cutover["indexed_content_sha256"] = empty_digest
        documents[3]["evidence"]["search_cutover"] = search_cutover
        self.bind_search_artifact(documents)
        with self.assertRaisesRegex(EvidenceError, "non-empty reconciliation set"):
            self.validate(documents)

    def test_rejects_unproved_timezone_or_scheduler_cardinality(self) -> None:
        documents = self.documents()
        scheduler = documents[3]["evidence"]["scheduler"]
        scheduler["legacy_jdbc_timezone"] = "UTC"
        scheduler["active_scheduler_instances"] = 2
        with self.assertRaisesRegex(EvidenceError, "timezones must match"):
            self.validate(documents)

    def test_rejects_missing_sigterm_or_rollback_evidence(self) -> None:
        documents = self.documents()
        documents[3]["evidence"]["lifecycle"]["rollback_switch_passed"] = False
        with self.assertRaisesRegex(EvidenceError, "checks did not pass"):
            self.validate(documents)


if __name__ == "__main__":
    unittest.main()
