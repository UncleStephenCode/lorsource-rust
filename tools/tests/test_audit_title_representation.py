from __future__ import annotations

import csv
import datetime as dt
import hashlib
import json
import os
import re
import stat
import sys
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import audit_title_representation as audit  # noqa: E402


MARKER = "clone-2026-08-15-a1b2c3d4"
SYSTEM_IDENTIFIER = "7612345678901234567"


def target_identity() -> dict[str, object]:
    return {
        "record_type": "target",
        "application_name": audit.APPLICATION_NAME,
        "database": "lor_title_clone",
        "current_user": "lor_title_auditor",
        "server_address": "127.0.0.1",
        "server_port": 5432,
        "server_version_num": "170006",
        "system_identifier": SYSTEM_IDENTIFIER,
        "transaction_read_only": "on",
        "database_comment": audit.CLONE_COMMENT_PREFIX + MARKER,
        "role_flags": {
            "rolsuper": False,
            "rolcreaterole": False,
            "rolcreatedb": False,
            "rolreplication": False,
            "rolbypassrls": False,
        },
        "mutation_privileges": False,
        "select_privileges": True,
        "relation_contract": deepcopy(audit.EXPECTED_RELATION_CONTRACT),
        "row_security": {
            "comments": False,
            "edit_info": False,
            "topics": False,
        },
        "column_contract": deepcopy(audit.EXPECTED_COLUMN_CONTRACT),
    }


def connection() -> audit.DatabaseTarget:
    return audit.DatabaseTarget(
        env={},
        database="lor_title_clone",
        role="lor_title_auditor",
        requested_host="clone.example",
        requested_port=5432,
    )


def windows() -> list[audit.DeploymentWindow]:
    return audit.parse_deployment_windows(
        {
            "schema_version": 1,
            "windows": [
                {
                    "name": "java-era",
                    "start": None,
                    "end": "2026-08-01T00:00:00Z",
                },
                {
                    "name": "rust-canary",
                    "start": "2026-08-01T00:00:00Z",
                    "end": "2026-08-10T00:00:00Z",
                },
                {
                    "name": "rust-era",
                    "start": "2026-08-12T00:00:00Z",
                    "end": None,
                },
            ],
        }
    )


def row(
    source: str,
    row_id: int,
    title: str,
    written_at: str,
    *,
    entity_id: int | None = None,
    entity_kind: str | None = None,
) -> dict[str, object]:
    if entity_kind is None:
        entity_kind = "COMMENT" if source == "comments.title" else "TOPIC"
    return {
        "record_type": "row",
        "source": source,
        "pipeline": (
            "history_title_snapshot_maketitle_or_raw"
            if source == "edit_info.oldtitle"
            else "current_title_storage"
        ),
        "row_id": row_id,
        "entity_id": row_id if entity_id is None else entity_id,
        "entity_kind": entity_kind,
        "written_at": written_at,
        "title": title,
    }


class TitleClassifierTest(unittest.TestCase):
    def test_guava_storage_alphabet(self) -> None:
        raw = "A & B < C > D \"quoted\" and 'single'"
        encoded = (
            "A &amp; B &lt; C &gt; D &quot;quoted&quot; and &#39;single&#39;"
        )

        self.assertEqual(encoded, audit.canonical_storage_escape(raw))
        self.assertEqual(raw, audit.decode_one_entity_layer(encoded))

    def test_golden_primary_categories(self) -> None:
        vectors = {
            audit.CATEGORY_PLAIN: ("Plain title", "Кириллица — 2026"),
            audit.CATEGORY_CANONICAL: (
                "A &amp; B",
                "&lt;b&gt;",
                "&quot;q&quot; &#39;x&#39;",
            ),
            audit.CATEGORY_RAW: (
                "it's raw",
                "A <b> & B \"q\"",
                "A ' &nbsp; &amp;lt;",
            ),
            audit.CATEGORY_OTHER: (
                "&nbsp;",
                "&#160;",
                "&#xA0;",
                "&#039;",
                "&copy;",
                "&apos;",
                "&bogus;",
                "&a;",
                "&_legacy;",
                "&123;",
                "&amp; &nbsp;",
            ),
            audit.CATEGORY_AMBIGUOUS: (
                "&amp;lt;",
                "&amp;amp;",
                "&amp;nbsp;",
                "&amp;#39;",
                "&#38;quot;",
                "&#x26;#39;",
                "&amp;amp;lt;",
            ),
        }

        for expected, titles in vectors.items():
            for title in titles:
                with self.subTest(title=title):
                    self.assertEqual(expected, audit.classify_title(title).category)

    def test_entity_ampersand_is_not_counted_as_raw(self) -> None:
        classified = audit.classify_title("&amp; &lt; &bogus;")

        self.assertEqual(audit.CATEGORY_OTHER, classified.category)
        self.assertEqual((), classified.raw_five)
        self.assertTrue(classified.mixed_representation)
        self.assertEqual(("&amp;:1", "&lt;:1"), classified.canonical_entities)
        self.assertEqual(("&bogus;:1",), classified.other_entities)

    def test_raw_precedence_keeps_all_secondary_evidence(self) -> None:
        classified = audit.classify_title("A ' &nbsp; &amp;lt;")

        self.assertEqual(audit.CATEGORY_RAW, classified.category)
        self.assertEqual(("single_quote:1",), classified.raw_five)
        self.assertEqual(("&amp;:1",), classified.canonical_entities)
        self.assertEqual(("&nbsp;:1",), classified.other_entities)
        self.assertTrue(classified.double_encoded)
        self.assertTrue(classified.mixed_representation)

    def test_all_five_raw_characters_are_counted_independently(self) -> None:
        classified = audit.classify_title("& < > \" '")

        self.assertEqual(audit.CATEGORY_RAW, classified.category)
        self.assertEqual(
            (
                "ampersand:1",
                "double_quote:1",
                "greater_than:1",
                "less_than:1",
                "single_quote:1",
            ),
            classified.raw_five,
        )

    def test_java_html4_one_layer_boundaries(self) -> None:
        self.assertEqual("&lt;", audit.decode_one_entity_layer("&amp;lt;"))
        self.assertEqual("&amp;", audit.decode_one_entity_layer("&amp;amp;"))
        self.assertEqual("&apos;", audit.decode_one_entity_layer("&apos;"))
        self.assertEqual("&NewLine;", audit.decode_one_entity_layer("&NewLine;"))
        self.assertEqual("\x0b", audit.decode_one_entity_layer("&#11;"))
        self.assertEqual("<", audit.decode_one_entity_layer("&#x3c;"))

    def test_one_layer_reescape_is_stable_for_ambiguous_guava_text(self) -> None:
        for value in ("&amp;lt;", "&amp;amp;", "&amp;#39;"):
            with self.subTest(value=value):
                self.assertEqual(
                    value,
                    audit.canonical_storage_escape(audit.decode_one_entity_layer(value)),
                )


class DeploymentWindowsTest(unittest.TestCase):
    def test_half_open_boundaries_and_gap(self) -> None:
        parsed = windows()
        utc = dt.timezone.utc

        self.assertEqual(
            "java-era",
            audit.deployment_window_for(dt.datetime(2026, 7, 31, 23, 59, tzinfo=utc), parsed),
        )
        self.assertEqual(
            "rust-canary",
            audit.deployment_window_for(dt.datetime(2026, 8, 1, tzinfo=utc), parsed),
        )
        self.assertEqual(
            "unassigned",
            audit.deployment_window_for(dt.datetime(2026, 8, 11, tzinfo=utc), parsed),
        )
        self.assertEqual(
            "rust-era",
            audit.deployment_window_for(dt.datetime(2026, 8, 12, tzinfo=utc), parsed),
        )

    def test_overlapping_windows_are_rejected(self) -> None:
        document = {
            "schema_version": 1,
            "windows": [
                {
                    "name": "first",
                    "start": "2026-08-01T00:00:00Z",
                    "end": "2026-08-10T00:00:00Z",
                },
                {
                    "name": "second",
                    "start": "2026-08-09T00:00:00Z",
                    "end": None,
                },
            ],
        }

        with self.assertRaisesRegex(audit.AuditError, "overlap"):
            audit.parse_deployment_windows(document)

    def test_naive_timestamp_is_rejected(self) -> None:
        document = {
            "schema_version": 1,
            "windows": [
                {"name": "bad", "start": "2026-08-01T00:00:00", "end": None}
            ],
        }

        with self.assertRaisesRegex(audit.AuditError, "RFC3339"):
            audit.parse_deployment_windows(document)


class DatabaseTargetTest(unittest.TestCase):
    def test_private_url_is_converted_to_clean_libpq_environment(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "database-url"
            path.write_text(
                "postgresql://lor%5Ftitle%5Fauditor:s3cr%25t@clone.example:5544/"
                "lor_title_clone?sslmode=verify-full\n",
                encoding="utf-8",
            )
            path.chmod(stat.S_IRUSR | stat.S_IWUSR)
            with mock.patch.dict(
                os.environ,
                {"PGPASSWORD": "inherited-secret", "PGHOST": "wrong", "KEEP_ME": "yes"},
                clear=True,
            ):
                target = audit.load_database_target(
                    path,
                    expected_database="lor_title_clone",
                    expected_role="lor_title_auditor",
                )

        self.assertEqual("clone.example", target.env["PGHOST"])
        self.assertEqual("5544", target.env["PGPORT"])
        self.assertEqual("lor_title_auditor", target.env["PGUSER"])
        self.assertEqual("s3cr%t", target.env["PGPASSWORD"])
        self.assertEqual("verify-full", target.env["PGSSLMODE"])
        self.assertEqual("yes", target.env["KEEP_ME"])
        self.assertIn("default_transaction_read_only=on", target.env["PGOPTIONS"])

    def test_url_file_permissions_are_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "database-url"
            path.write_text(
                "postgresql://auditor:secret@clone/lor_title_clone\n",
                encoding="utf-8",
            )
            path.chmod(0o644)

            with self.assertRaisesRegex(audit.AuditError, "group/other"):
                audit.load_database_target(
                    path,
                    expected_database="lor_title_clone",
                    expected_role="auditor",
                )

    @unittest.skipUnless(hasattr(os, "O_NOFOLLOW"), "requires O_NOFOLLOW")
    def test_url_file_symlink_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "real"
            target.write_text(
                "postgresql://auditor:secret@clone/lor_title_clone\n",
                encoding="utf-8",
            )
            target.chmod(0o600)
            link = Path(directory) / "link"
            link.symlink_to(target)

            with self.assertRaisesRegex(audit.AuditError, "securely read"):
                audit.load_database_target(
                    link,
                    expected_database="lor_title_clone",
                    expected_role="auditor",
                )

    def test_wrong_database_or_unsupported_option_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "database-url"
            path.write_text(
                "postgresql://auditor:secret@clone/other?application_name=override\n",
                encoding="utf-8",
            )
            path.chmod(0o600)

            with self.assertRaisesRegex(audit.AuditError, "expected"):
                audit.load_database_target(
                    path,
                    expected_database="lor_title_clone",
                    expected_role="auditor",
                )


class TargetIdentityTest(unittest.TestCase):
    def validate(self, value: dict[str, object]) -> dict[str, object]:
        return audit.validate_target_identity(
            value,
            expected_database="lor_title_clone",
            expected_role="lor_title_auditor",
            expected_system_identifier=SYSTEM_IDENTIFIER,
            clone_marker=MARKER,
        )

    def test_exact_unprivileged_clone_identity_is_accepted(self) -> None:
        self.assertEqual(target_identity(), self.validate(target_identity()))

    def test_every_safety_boundary_fails_closed(self) -> None:
        mutations = (
            ("database_comment", "wrong"),
            ("database", "production"),
            ("system_identifier", "9999999999999999999"),
            ("transaction_read_only", "off"),
            ("mutation_privileges", True),
            ("select_privileges", False),
            ("server_address", "local-socket"),
            ("row_security", {"topics": False}),
            ("relation_contract", {}),
            ("column_contract", {}),
        )
        for key, replacement in mutations:
            with self.subTest(key=key):
                value = target_identity()
                value[key] = replacement
                with self.assertRaises(audit.AuditError):
                    self.validate(value)

        value = target_identity()
        value["role_flags"]["rolsuper"] = True
        with self.assertRaisesRegex(audit.AuditError, "unprivileged"):
            self.validate(value)

    def test_unknown_target_field_is_rejected(self) -> None:
        value = target_identity()
        value["unexpected"] = True

        with self.assertRaisesRegex(audit.AuditError, "unexpected"):
            self.validate(value)

    def test_clone_marker_cannot_claim_production(self) -> None:
        for marker in ("production-copy-123", "prod.20260815", "live_clone_123"):
            with self.subTest(marker=marker):
                with self.assertRaisesRegex(audit.AuditError, "live/production"):
                    audit._validate_clone_marker(marker)


class ArtifactTest(unittest.TestCase):
    def sample_rows(self) -> list[dict[str, object]]:
        return [
            row("topics.title", 1, "Plain secret title", "2026-07-31T23:59:00Z"),
            row("topics.title", 100_001, "A &amp; B", "2026-08-01T00:00:00Z"),
            row("comments.title", 2, "raw < secret", "2026-08-11T00:00:00Z"),
            row(
                "edit_info.oldtitle",
                10,
                "&amp;lt; snapshot",
                "2026-08-12T00:00:00Z",
                entity_id=1,
            ),
        ]

    def write(self, output: Path) -> dict[str, object]:
        return audit.write_audit_artifacts(
            target_identity=target_identity(),
            rows=self.sample_rows(),
            windows=windows(),
            windows_sha256=hashlib.sha256(b"deployment-windows").hexdigest(),
            id_bucket_size=100_000,
            output_dir=output,
            connection=connection(),
        )

    def test_artifacts_are_deterministic_hashed_and_do_not_export_titles(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            first = Path(directory) / "first"
            second = Path(directory) / "second"
            first_report = self.write(first)
            second_report = self.write(second)

            for name in (audit.OUTPUT_CSV, audit.OUTPUT_JSON, audit.OUTPUT_SUMMARY):
                first_bytes = (first / name).read_bytes()
                self.assertEqual(first_bytes, (second / name).read_bytes())
                self.assertEqual(0o600, stat.S_IMODE((first / name).stat().st_mode))
                for raw_title in (
                    b"Plain secret title",
                    b"A &amp; B",
                    b"raw < secret",
                    b"&amp;lt; snapshot",
                ):
                    self.assertNotIn(raw_title, first_bytes)

            self.assertEqual(first_report, second_report)
            self.assertEqual(4, first_report["totals"]["row_count"])
            csv_path = first / audit.OUTPUT_CSV
            self.assertEqual(
                hashlib.sha256(csv_path.read_bytes()).hexdigest(),
                first_report["totals"]["csv_sha256"],
            )
            with csv_path.open(encoding="utf-8", newline="") as stream:
                csv_rows = list(csv.DictReader(stream))
            self.assertEqual(4, len(csv_rows))
            self.assertEqual(
                [
                    audit.CATEGORY_PLAIN,
                    audit.CATEGORY_CANONICAL,
                    audit.CATEGORY_RAW,
                    audit.CATEGORY_AMBIGUOUS,
                ],
                [item["classification"] for item in csv_rows],
            )
            self.assertEqual("unassigned", csv_rows[2]["deployment_window"])
            self.assertEqual(
                "history_title_snapshot_maketitle_or_raw", csv_rows[3]["pipeline"]
            )
            parsed_json = json.loads((first / audit.OUTPUT_JSON).read_text(encoding="utf-8"))
            self.assertEqual("not_claimed", parsed_json["provenance"]["production_clone_evidence"])

    def test_existing_output_and_nondeterministic_row_order_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            occupied = Path(directory) / "occupied"
            occupied.mkdir()
            (occupied / "keep").write_text("operator data", encoding="utf-8")
            with self.assertRaisesRegex(audit.AuditError, "absent or empty"):
                self.write(occupied)
            self.assertEqual("operator data", (occupied / "keep").read_text(encoding="utf-8"))

            unordered = Path(directory) / "unordered"
            bad_rows = [
                row("topics.title", 2, "second", "2026-08-01T00:00:00Z"),
                row("topics.title", 1, "first", "2026-08-01T00:00:00Z"),
            ]
            with self.assertRaisesRegex(audit.AuditError, "deterministic"):
                audit.write_audit_artifacts(
                    target_identity=target_identity(),
                    rows=bad_rows,
                    windows=windows(),
                    windows_sha256="0" * 64,
                    id_bucket_size=100,
                    output_dir=unordered,
                    connection=connection(),
                )
            self.assertEqual([], list(unordered.iterdir()))


class SqlSafetyTest(unittest.TestCase):
    def test_database_sql_is_fixed_and_read_only(self) -> None:
        sql = audit.TARGET_PROBE_SQL + "\n" + audit.AUDIT_SQL
        without_literals = re.sub(r"'(?:''|[^'])*'", "''", sql)

        self.assertEqual(2, len(re.findall(r"BEGIN TRANSACTION[^;]+READ ONLY", sql)))
        self.assertEqual(2, len(re.findall(r"\bCOMMIT\b", without_literals, re.I)))
        self.assertIsNone(
            re.search(
                r"\b(?:INSERT|UPDATE|DELETE|ALTER|CREATE|DROP|TRUNCATE|MERGE|CALL|DO)\b",
                without_literals,
                re.I,
            )
        )
        self.assertIn("max(e.editdate)", audit.AUDIT_SQL)
        self.assertNotIn("lastmod", audit.AUDIT_SQL.lower())
        self.assertIn("ORDER BY t.id", audit.AUDIT_SQL)
        self.assertIn("ORDER BY c.id", audit.AUDIT_SQL)
        self.assertIn("ORDER BY e.id", audit.AUDIT_SQL)


if __name__ == "__main__":
    unittest.main()
