from __future__ import annotations

import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "compat"))

import stateful_database  # noqa: E402


class ModerationDatabaseTargetTest(unittest.TestCase):
    def tearDown(self) -> None:
        stateful_database.psql_target.cache_clear()

    def test_compose_remains_the_safe_default(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=True):
            command, child_env, expected = stateful_database.psql_target()

        self.assertEqual(command[:3], ["docker", "compose", "exec"])
        self.assertIsNone(child_env)
        self.assertIsNone(expected)

    def test_external_target_requires_disposable_marker(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "database-url"
            path.write_text("postgres://user:password@db/rehearsal\n", encoding="utf-8")
            path.chmod(stat.S_IRUSR | stat.S_IWUSR)
            with mock.patch.dict(
                os.environ,
                {
                    "STATEFUL_DATABASE_URL_FILE": str(path),
                    "STATEFUL_EXPECTED_DATABASE": "rehearsal",
                },
                clear=True,
            ):
                with self.assertRaisesRegex(AssertionError, "IS_DISPOSABLE"):
                    stateful_database.psql_target()

    def test_private_url_file_becomes_libpq_environment(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "database-url"
            path.write_text(
                "postgresql://test%2Duser:s3cr%25t@db.example:5544/lorsource_rehearsal?sslmode=verify-full\n",
                encoding="utf-8",
            )
            path.chmod(stat.S_IRUSR | stat.S_IWUSR)
            with mock.patch.dict(
                os.environ,
                {
                    "STATEFUL_DATABASE_URL_FILE": str(path),
                    "STATEFUL_DATABASE_IS_DISPOSABLE": "yes",
                    "STATEFUL_EXPECTED_DATABASE": "lorsource_rehearsal",
                },
                clear=True,
            ):
                command, child_env, expected = stateful_database.psql_target()

        self.assertEqual(command, ["psql"])
        self.assertEqual(expected, "lorsource_rehearsal")
        assert child_env is not None
        self.assertEqual(child_env["PGHOST"], "db.example")
        self.assertEqual(child_env["PGPORT"], "5544")
        self.assertEqual(child_env["PGUSER"], "test-user")
        self.assertEqual(child_env["PGPASSWORD"], "s3cr%t")
        self.assertEqual(child_env["PGDATABASE"], "lorsource_rehearsal")
        self.assertEqual(child_env["PGSSLMODE"], "verify-full")
        self.assertNotIn("s3cr", " ".join(command))

    def test_external_url_file_must_be_private(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "database-url"
            path.write_text("postgres://user:password@db/rehearsal\n", encoding="utf-8")
            path.chmod(0o644)
            with mock.patch.dict(
                os.environ,
                {
                    "STATEFUL_DATABASE_URL_FILE": str(path),
                    "STATEFUL_DATABASE_IS_DISPOSABLE": "yes",
                    "STATEFUL_EXPECTED_DATABASE": "rehearsal",
                },
                clear=True,
            ):
                with self.assertRaisesRegex(AssertionError, "group/other"):
                    stateful_database.psql_target()


if __name__ == "__main__":
    unittest.main()
