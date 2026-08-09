"""Fail-closed PostgreSQL target selection for stateful rehearsals."""

from __future__ import annotations

import os
import urllib.parse
from functools import lru_cache
from pathlib import Path


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


@lru_cache(maxsize=1)
def psql_target() -> tuple[list[str], dict[str, str] | None, str | None]:
    """Resolve a Compose or explicitly guarded external `psql` target."""

    url_file = os.environ.get("STATEFUL_DATABASE_URL_FILE")
    if not url_file:
        return (
            [
                "docker",
                "compose",
                "exec",
                "-T",
                "postgres",
                "psql",
                "-U",
                "postgres",
                "-d",
                "lor",
            ],
            None,
            None,
        )

    require(
        os.environ.get("STATEFUL_DATABASE_IS_DISPOSABLE") == "yes",
        "STATEFUL_DATABASE_IS_DISPOSABLE=yes is required for an external stateful database",
    )
    expected_database = os.environ.get("STATEFUL_EXPECTED_DATABASE", "")
    require(bool(expected_database), "STATEFUL_EXPECTED_DATABASE is required")

    path = Path(url_file)
    require(path.is_file(), f"stateful database URL file does not exist: {path}")
    require(
        path.stat().st_mode & 0o077 == 0,
        "stateful database URL file must not be accessible by group/other",
    )
    lines = path.read_text(encoding="utf-8").splitlines()
    require(
        len(lines) == 1 and bool(lines[0]),
        "stateful database URL file must contain exactly one non-empty line",
    )
    parsed = urllib.parse.urlsplit(lines[0])
    require(
        parsed.scheme in {"postgres", "postgresql"},
        "unsupported PostgreSQL URL scheme",
    )
    database = urllib.parse.unquote(parsed.path.removeprefix("/"))
    require(
        bool(parsed.hostname and parsed.username and database),
        "incomplete PostgreSQL URL",
    )
    require(
        database == expected_database,
        f"database URL targets {database!r}, expected {expected_database!r}",
    )

    child_env = os.environ.copy()
    child_env.update(
        {
            "PGHOST": parsed.hostname or "",
            "PGPORT": str(parsed.port or 5432),
            "PGUSER": urllib.parse.unquote(parsed.username or ""),
            "PGPASSWORD": urllib.parse.unquote(parsed.password or ""),
            "PGDATABASE": database,
        }
    )
    allowed_options = {
        "sslmode": "PGSSLMODE",
        "sslrootcert": "PGSSLROOTCERT",
        "sslcert": "PGSSLCERT",
        "sslkey": "PGSSLKEY",
        "target_session_attrs": "PGTARGETSESSIONATTRS",
    }
    for name, values in urllib.parse.parse_qs(
        parsed.query, keep_blank_values=True
    ).items():
        require(
            name in allowed_options and len(values) == 1,
            f"unsupported PostgreSQL URL option: {name}",
        )
        child_env[allowed_options[name]] = values[0]
    return (["psql"], child_env, expected_database)
