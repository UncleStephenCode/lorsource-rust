#!/usr/bin/env python3
"""Read-only inventory of mixed Java/Rust title representations.

The command deliberately has no migration mode.  It connects only through an
explicit, owner-only URL file, verifies an operator-installed clone marker and
stable PostgreSQL identity, and executes its catalog probe and data scan in
READ ONLY transactions.  Title values are represented by hashes in artifacts;
operators can review the exact rows on the same clone by source and id.
"""

from __future__ import annotations

import argparse
import contextlib
import csv
import datetime as dt
import hashlib
import html.entities
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import urllib.parse
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Iterator, Mapping, Sequence


AUDIT_CONFIRMATION = "read-only-title-audit"
APPLICATION_NAME = "lorsource-title-representation-audit"
CLONE_COMMENT_PREFIX = "lorsource-title-audit-clone:"
OUTPUT_JSON = "title-representation-audit.json"
OUTPUT_CSV = "title-representation-rows.csv"
OUTPUT_SUMMARY = "title-representation-summary.txt"

CATEGORY_PLAIN = "plain"
CATEGORY_RAW = "raw_five_chars"
CATEGORY_CANONICAL = "canonical_entities"
CATEGORY_OTHER = "other_named_or_numeric_entities"
CATEGORY_AMBIGUOUS = "double_encoded_or_ambiguous"
CATEGORIES = (
    CATEGORY_PLAIN,
    CATEGORY_RAW,
    CATEGORY_CANONICAL,
    CATEGORY_OTHER,
    CATEGORY_AMBIGUOUS,
)

CANONICAL_ENTITIES = ("&amp;", "&quot;", "&#39;", "&lt;", "&gt;")
CANONICAL_ENTITY_SET = set(CANONICAL_ENTITIES)
RAW_FIVE_NAMES = {
    "&": "ampersand",
    "<": "less_than",
    ">": "greater_than",
    '"': "double_quote",
    "'": "single_quote",
}
ENTITY_RE = re.compile(r"&(?:#[xX][0-9A-Fa-f]+|#[0-9]+|[A-Za-z0-9_]+);")
DOUBLE_ENCODED_RE = re.compile(
    r"(?:&(?:amp|AMP);|&#0*38;|&#[xX]0*26;)"
    r"(?:#[xX][0-9A-Fa-f]+|#[0-9]+|[A-Za-z0-9_]+);"
)
SAFE_MARKER_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{7,127}$")
SYSTEM_IDENTIFIER_RE = re.compile(r"^[0-9]{8,32}$")
WINDOW_NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
RFC3339_RE = re.compile(
    r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}"
    r"(?:\.[0-9]{1,6})?(?:Z|[+-][0-9]{2}:[0-9]{2})$"
)
FORBIDDEN_MARKER_PARTS = {"live", "prod", "production"}

SOURCE_ORDER = {"topics.title": 1, "comments.title": 2, "edit_info.oldtitle": 3}

EXPECTED_COLUMN_CONTRACT = {
    "comments.id": {"udt_name": "int4", "nullable": False},
    "comments.postdate": {"udt_name": "timestamptz", "nullable": False},
    "comments.title": {"udt_name": "varchar", "nullable": False},
    "edit_info.editdate": {"udt_name": "timestamptz", "nullable": False},
    "edit_info.id": {"udt_name": "int4", "nullable": False},
    "edit_info.msgid": {"udt_name": "int4", "nullable": False},
    "edit_info.object_type": {"udt_name": "edit_event_type", "nullable": False},
    "edit_info.oldtitle": {"udt_name": "text", "nullable": True},
    "topics.id": {"udt_name": "int4", "nullable": False},
    "topics.postdate": {"udt_name": "timestamptz", "nullable": False},
    "topics.title": {"udt_name": "varchar", "nullable": False},
}
EXPECTED_RELATION_CONTRACT = {
    "comments": {
        "relkind": "r",
        "relpersistence": "p",
        "relrowsecurity": False,
        "relforcerowsecurity": False,
    },
    "edit_info": {
        "relkind": "r",
        "relpersistence": "p",
        "relrowsecurity": False,
        "relforcerowsecurity": False,
    },
    "topics": {
        "relkind": "r",
        "relpersistence": "p",
        "relrowsecurity": False,
        "relforcerowsecurity": False,
    },
}

TARGET_KEYS = {
    "application_name",
    "column_contract",
    "current_user",
    "database",
    "database_comment",
    "mutation_privileges",
    "record_type",
    "relation_contract",
    "role_flags",
    "row_security",
    "select_privileges",
    "server_address",
    "server_port",
    "server_version_num",
    "system_identifier",
    "transaction_read_only",
}
ROW_KEYS = {
    "entity_id",
    "entity_kind",
    "pipeline",
    "record_type",
    "row_id",
    "source",
    "title",
    "written_at",
}


class AuditError(ValueError):
    """A fail-closed configuration, target, query or artifact error."""


@dataclass(frozen=True)
class TitleClassification:
    category: str
    raw_five: tuple[str, ...]
    canonical_entities: tuple[str, ...]
    other_entities: tuple[str, ...]
    double_encoded: bool
    mixed_representation: bool
    decoded_once: str


@dataclass(frozen=True)
class DeploymentWindow:
    name: str
    start: dt.datetime | None
    end: dt.datetime | None

    def contains(self, value: dt.datetime) -> bool:
        return (self.start is None or value >= self.start) and (
            self.end is None or value < self.end
        )


@dataclass(frozen=True)
class DatabaseTarget:
    env: dict[str, str]
    database: str
    role: str
    requested_host: str
    requested_port: int


@dataclass
class GroupDigest:
    count: int = 0
    digest: object = None

    def __post_init__(self) -> None:
        if self.digest is None:
            self.digest = hashlib.sha256()

    def add(self, row_hash: str) -> None:
        self.count += 1
        self.digest.update(row_hash.encode("ascii"))
        self.digest.update(b"\n")

    def hexdigest(self) -> str:
        return self.digest.hexdigest()


def canonical_storage_escape(value: str) -> str:
    """Guava HtmlEscapers.htmlEscaper() mapping used by Java title writes."""

    out: list[str] = []
    for character in value:
        out.append(
            {
                "&": "&amp;",
                '"': "&quot;",
                "'": "&#39;",
                "<": "&lt;",
                ">": "&gt;",
            }.get(character, character)
        )
    return "".join(out)


def _decode_html4_entity(token: str) -> str:
    """Approximate Commons Text ``unescapeHtml4`` for one complete token.

    Python's :func:`html.unescape` implements HTML5 and therefore changes
    source-significant probes such as ``&apos;`` and ``&NewLine;`` which Java's
    HTML4 unescaper leaves alone.
    """

    if token.startswith("&#"):
        digits = token[3:-1] if token[2:3].lower() == "x" else token[2:-1]
        base = 16 if token[2:3].lower() == "x" else 10
        try:
            codepoint = int(digits, base)
            if 0 <= codepoint <= 0x10FFFF and not 0xD800 <= codepoint <= 0xDFFF:
                return chr(codepoint)
        except ValueError:
            pass
        return token
    codepoint = html.entities.name2codepoint.get(token[1:-1])
    return chr(codepoint) if codepoint is not None else token


def decode_one_entity_layer(value: str) -> str:
    """Apply one Java HTML4 entity-decoding layer to original input tokens."""

    return ENTITY_RE.sub(lambda match: _decode_html4_entity(match.group(0)), value)


def classify_title(value: str) -> TitleClassification:
    """Return one exclusive representation category and its observable signals."""

    entities = list(ENTITY_RE.finditer(value))
    canonical = Counter(
        match.group(0) for match in entities if match.group(0) in CANONICAL_ENTITY_SET
    )
    other = Counter(
        match.group(0) for match in entities if match.group(0) not in CANONICAL_ENTITY_SET
    )

    raw = Counter()
    entity_by_start = {match.start(): match for match in entities}
    position = 0
    while position < len(value):
        entity = entity_by_start.get(position)
        if entity is not None:
            position = entity.end()
            continue
        character = value[position]
        if character in RAW_FIVE_NAMES:
            raw[RAW_FIVE_NAMES[character]] += 1
        position += 1

    double_encoded = DOUBLE_ENCODED_RE.search(value) is not None
    mixed = sum(bool(signal) for signal in (raw, canonical, other)) > 1
    # Conservative, mutually-exclusive precedence.  Literal raw characters are
    # the strongest evidence even if entity-shaped text is present elsewhere.
    # A double candidate is not automatically corruption: ``&amp;lt;`` is also
    # valid Guava storage for a user who literally entered ``&lt;``.
    if raw:
        category = CATEGORY_RAW
    elif double_encoded:
        category = CATEGORY_AMBIGUOUS
    elif other:
        category = CATEGORY_OTHER
    elif canonical:
        category = CATEGORY_CANONICAL
    else:
        category = CATEGORY_PLAIN

    return TitleClassification(
        category=category,
        raw_five=tuple(f"{name}:{raw[name]}" for name in sorted(raw)),
        canonical_entities=tuple(f"{name}:{canonical[name]}" for name in sorted(canonical)),
        other_entities=tuple(f"{name}:{other[name]}" for name in sorted(other)),
        double_encoded=double_encoded,
        mixed_representation=mixed,
        decoded_once=decode_one_entity_layer(value),
    )


def _parse_rfc3339(name: str, value: object, *, nullable: bool) -> dt.datetime | None:
    if value is None and nullable:
        return None
    if not isinstance(value, str) or not RFC3339_RE.fullmatch(value):
        raise AuditError(f"{name} must be an RFC3339 timestamp")
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise AuditError(f"{name} must be an RFC3339 timestamp") from error
    if parsed.tzinfo is None:
        raise AuditError(f"{name} must include a UTC offset")
    return parsed.astimezone(dt.timezone.utc)


def parse_deployment_windows(document: object) -> list[DeploymentWindow]:
    if not isinstance(document, dict) or set(document) != {"schema_version", "windows"}:
        raise AuditError("deployment-window document must contain schema_version and windows")
    if document["schema_version"] != 1 or not isinstance(document["windows"], list):
        raise AuditError("deployment-window document must use schema_version 1 and a list")
    if not document["windows"]:
        raise AuditError("at least one deployment window is required")

    windows: list[DeploymentWindow] = []
    names: set[str] = set()
    for index, item in enumerate(document["windows"]):
        if not isinstance(item, dict) or set(item) != {"name", "start", "end"}:
            raise AuditError(f"windows[{index}] must contain exactly name, start and end")
        name = item["name"]
        if not isinstance(name, str) or not WINDOW_NAME_RE.fullmatch(name):
            raise AuditError(f"windows[{index}].name is invalid")
        if name == "unassigned" or name in names:
            raise AuditError(f"deployment window name is reserved or duplicated: {name}")
        names.add(name)
        start = _parse_rfc3339(f"windows[{index}].start", item["start"], nullable=True)
        end = _parse_rfc3339(f"windows[{index}].end", item["end"], nullable=True)
        if start is not None and end is not None and start >= end:
            raise AuditError(f"deployment window {name} has an empty/reversed range")
        windows.append(DeploymentWindow(name=name, start=start, end=end))

    windows.sort(
        key=lambda window: (
            window.start is not None,
            window.start or dt.datetime.min.replace(tzinfo=dt.timezone.utc),
            window.name,
        )
    )
    for previous, current in zip(windows, windows[1:]):
        if previous.end is None or current.start is None or current.start < previous.end:
            raise AuditError(
                f"deployment windows overlap: {previous.name} and {current.name}"
            )
    return windows


def read_deployment_windows(path: Path) -> tuple[list[DeploymentWindow], str]:
    try:
        raw = path.read_bytes()
        document = json.loads(raw.decode("utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AuditError(f"cannot read deployment windows: {error}") from error
    return parse_deployment_windows(document), hashlib.sha256(raw).hexdigest()


def deployment_window_for(value: dt.datetime, windows: Sequence[DeploymentWindow]) -> str:
    matches = [window.name for window in windows if window.contains(value)]
    if len(matches) > 1:
        raise AuditError(f"timestamp unexpectedly belongs to several windows: {value.isoformat()}")
    return matches[0] if matches else "unassigned"


def _validate_clone_marker(marker: str) -> None:
    if not SAFE_MARKER_RE.fullmatch(marker):
        raise AuditError("clone marker must be an explicit 8-128 character identifier")
    parts = {part.lower() for part in re.split(r"[^A-Za-z0-9]+", marker) if part}
    if parts & FORBIDDEN_MARKER_PARTS:
        raise AuditError("clone marker must not identify a live/production database")


def load_database_target(
    path: Path,
    *,
    expected_database: str,
    expected_role: str,
) -> DatabaseTarget:
    descriptor: int | None = None
    try:
        flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
        descriptor = os.open(path, flags)
        file_stat = os.fstat(descriptor)
        if not stat.S_ISREG(file_stat.st_mode):
            raise AuditError("database URL file must be a regular non-symlink file")
        if file_stat.st_uid != os.getuid():
            raise AuditError("database URL file must be owned by the invoking user")
        if file_stat.st_mode & 0o077:
            raise AuditError("database URL file must not be accessible by group/other")
        with os.fdopen(descriptor, "r", encoding="utf-8") as stream:
            descriptor = None
            raw_url = stream.read(65_537)
    except (OSError, UnicodeDecodeError) as error:
        raise AuditError(f"cannot securely read database URL file: {error}") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)
    if len(raw_url) > 65_536:
        raise AuditError("database URL file is unexpectedly large")
    if raw_url.endswith("\n"):
        raw_url = raw_url[:-1]
    if not raw_url or "\n" in raw_url or "\r" in raw_url:
        raise AuditError("database URL file must contain exactly one non-empty line")
    try:
        parsed = urllib.parse.urlsplit(raw_url)
        port = parsed.port or 5432
    except ValueError as error:
        raise AuditError("database URL is malformed") from error
    if parsed.scheme not in {"postgres", "postgresql"} or parsed.fragment:
        raise AuditError("database URL must use postgres/postgresql and no fragment")
    database = urllib.parse.unquote(parsed.path.removeprefix("/"))
    role = urllib.parse.unquote(parsed.username or "")
    if not parsed.hostname or not database or "/" in database or not role:
        raise AuditError("database URL must contain one TCP host, role and database")
    if database != expected_database:
        raise AuditError(
            f"database URL targets {database!r}, expected {expected_database!r}"
        )
    if role != expected_role:
        raise AuditError(f"database URL role is {role!r}, expected {expected_role!r}")

    child_env = {key: value for key, value in os.environ.items() if not key.startswith("PG")}
    child_env.update(
        {
            "PGAPPNAME": APPLICATION_NAME,
            "PGCONNECT_TIMEOUT": "10",
            "PGDATABASE": database,
            "PGHOST": parsed.hostname,
            "PGOPTIONS": (
                "-c default_transaction_read_only=on "
                "-c lock_timeout=5000 "
                "-c idle_in_transaction_session_timeout=300000 "
                "-c search_path=pg_catalog,public"
            ),
            "PGPORT": str(port),
            "PGUSER": role,
        }
    )
    if parsed.password is not None:
        child_env["PGPASSWORD"] = urllib.parse.unquote(parsed.password)
    allowed_options = {
        "sslmode": "PGSSLMODE",
        "sslrootcert": "PGSSLROOTCERT",
        "sslcert": "PGSSLCERT",
        "sslkey": "PGSSLKEY",
        "target_session_attrs": "PGTARGETSESSIONATTRS",
    }
    for name, values in urllib.parse.parse_qs(parsed.query, keep_blank_values=True).items():
        if name not in allowed_options or len(values) != 1 or not values[0]:
            raise AuditError(f"unsupported/empty PostgreSQL URL option: {name}")
        child_env[allowed_options[name]] = values[0]
    return DatabaseTarget(
        env=child_env,
        database=database,
        role=role,
        requested_host=parsed.hostname,
        requested_port=port,
    )


_COLUMN_TUPLES = ",\n".join(
    f"        ('{key.split('.', 1)[0]}','{key.split('.', 1)[1]}')"
    for key in EXPECTED_COLUMN_CONTRACT
)
_MUTATION_PRIVILEGES = " OR ".join(
    f"has_table_privilege(current_user,'public.{table}','{privilege}')"
    for table in ("topics", "comments", "edit_info")
    for privilege in ("INSERT", "UPDATE", "DELETE", "TRUNCATE", "TRIGGER")
)
_SELECT_PRIVILEGES = " AND ".join(
    f"has_column_privilege(current_user,'public.{table}','{column}','SELECT')"
    for table, columns in {
        "topics": ("id", "title", "postdate"),
        "comments": ("id", "title", "postdate"),
        "edit_info": ("id", "msgid", "oldtitle", "editdate", "object_type"),
    }.items()
    for column in columns
)

TARGET_JSON_SQL = f"""
jsonb_build_object(
  'record_type','target',
  'application_name',current_setting('application_name'),
  'database',current_database(),
  'current_user',current_user,
  'server_address',COALESCE(inet_server_addr()::text,'local-socket'),
  'server_port',COALESCE(inet_server_port(),0),
  'server_version_num',current_setting('server_version_num'),
  'system_identifier',(SELECT system_identifier::text FROM pg_control_system()),
  'transaction_read_only',current_setting('transaction_read_only'),
  'database_comment',(
      SELECT shobj_description(oid,'pg_database')
        FROM pg_database WHERE datname=current_database()
  ),
  'role_flags',(
      SELECT jsonb_build_object(
        'rolsuper',rolsuper,'rolcreaterole',rolcreaterole,'rolcreatedb',rolcreatedb,
        'rolreplication',rolreplication,'rolbypassrls',rolbypassrls
      ) FROM pg_roles WHERE rolname=current_user
  ),
  'mutation_privileges',({_MUTATION_PRIVILEGES}),
  'select_privileges',({_SELECT_PRIVILEGES}),
  'relation_contract',(
      SELECT COALESCE(
        jsonb_object_agg(
          c.relname,
          jsonb_build_object(
            'relkind',c.relkind::text,
            'relpersistence',c.relpersistence::text,
            'relrowsecurity',c.relrowsecurity,
            'relforcerowsecurity',c.relforcerowsecurity
          ) ORDER BY c.relname
        ),'{{}}'::jsonb
      )
        FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
       WHERE n.nspname='public' AND c.relname IN ('topics','comments','edit_info')
  ),
  'row_security',(
      SELECT COALESCE(jsonb_object_agg(c.relname,c.relrowsecurity ORDER BY c.relname),'{{}}'::jsonb)
        FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
       WHERE n.nspname='public' AND c.relname IN ('topics','comments','edit_info')
  ),
  'column_contract',(
      SELECT COALESCE(
        jsonb_object_agg(
          c.table_name||'.'||c.column_name,
          jsonb_build_object('udt_name',c.udt_name,'nullable',c.is_nullable='YES')
          ORDER BY c.table_name,c.ordinal_position
        ),'{{}}'::jsonb
      )
        FROM information_schema.columns c
       WHERE c.table_schema='public'
         AND (c.table_name,c.column_name) IN (
{_COLUMN_TUPLES}
         )
  )
)"""

TARGET_PROBE_SQL = f"""\
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT ({TARGET_JSON_SQL})::text;
COMMIT;
"""

AUDIT_SQL = f"""\
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT ({TARGET_JSON_SQL})::text;
WITH latest_title_write AS (
  SELECT e.msgid,max(e.editdate) AS written_at
    FROM public.edit_info e
   WHERE e.object_type='TOPIC'::edit_event_type AND e.oldtitle IS NOT NULL
   GROUP BY e.msgid
)
SELECT jsonb_build_object(
         'record_type','row','source','topics.title','pipeline','current_title_storage',
         'row_id',t.id,'entity_id',t.id,'entity_kind','TOPIC',
         'written_at',to_char(COALESCE(w.written_at,t.postdate) AT TIME ZONE 'UTC',
                              'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
         'title',t.title
       )::text
  FROM public.topics t
  LEFT JOIN latest_title_write w ON w.msgid=t.id
 ORDER BY t.id;
WITH latest_title_write AS (
  SELECT e.msgid,max(e.editdate) AS written_at
    FROM public.edit_info e
   WHERE e.object_type='COMMENT'::edit_event_type AND e.oldtitle IS NOT NULL
   GROUP BY e.msgid
)
SELECT jsonb_build_object(
         'record_type','row','source','comments.title','pipeline','current_title_storage',
         'row_id',c.id,'entity_id',c.id,'entity_kind','COMMENT',
         'written_at',to_char(COALESCE(w.written_at,c.postdate) AT TIME ZONE 'UTC',
                              'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
         'title',c.title
       )::text
  FROM public.comments c
  LEFT JOIN latest_title_write w ON w.msgid=c.id
 ORDER BY c.id;
SELECT jsonb_build_object(
         'record_type','row','source','edit_info.oldtitle',
         'pipeline','history_title_snapshot_maketitle_or_raw',
         'row_id',e.id,'entity_id',e.msgid,'entity_kind',e.object_type::text,
         'written_at',to_char(e.editdate AT TIME ZONE 'UTC',
                              'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
         'title',e.oldtitle
       )::text
  FROM public.edit_info e
 WHERE e.oldtitle IS NOT NULL
 ORDER BY e.id;
COMMIT;
"""


def _psql_command(psql: str) -> list[str]:
    return [
        psql,
        "--no-psqlrc",
        "--quiet",
        "--tuples-only",
        "--no-align",
        "--set",
        "ON_ERROR_STOP=1",
        "--set",
        "VERBOSITY=terse",
    ]


def _psql_error(stderr: str) -> str:
    compact = " ".join(stderr.split())
    return compact[:2000] if compact else "psql exited without diagnostics"


def run_target_probe(target: DatabaseTarget, psql: str) -> dict[str, object]:
    try:
        result = subprocess.run(
            _psql_command(psql),
            input=TARGET_PROBE_SQL,
            text=True,
            encoding="utf-8",
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=target.env,
            check=False,
        )
    except OSError as error:
        raise AuditError(f"cannot execute psql target probe: {psql}") from error
    if result.returncode != 0:
        raise AuditError(f"read-only target probe failed: {_psql_error(result.stderr)}")
    lines = [line for line in result.stdout.splitlines() if line]
    if len(lines) != 1:
        raise AuditError("read-only target probe did not return exactly one identity row")
    try:
        value = json.loads(lines[0])
    except json.JSONDecodeError as error:
        raise AuditError("read-only target probe returned invalid JSON") from error
    if not isinstance(value, dict):
        raise AuditError("read-only target identity must be a JSON object")
    return value


def validate_target_identity(
    value: Mapping[str, object],
    *,
    expected_database: str,
    expected_role: str,
    expected_system_identifier: str,
    clone_marker: str,
) -> dict[str, object]:
    if set(value) != TARGET_KEYS:
        raise AuditError(
            "target identity has missing/unexpected fields: "
            f"missing={sorted(TARGET_KEYS-set(value))}, extra={sorted(set(value)-TARGET_KEYS)}"
        )
    if value["record_type"] != "target":
        raise AuditError("first audit record is not the target identity")
    expected_comment = CLONE_COMMENT_PREFIX + clone_marker
    checks = {
        "application_name": APPLICATION_NAME,
        "database": expected_database,
        "current_user": expected_role,
        "system_identifier": expected_system_identifier,
        "transaction_read_only": "on",
        "database_comment": expected_comment,
    }
    for key, expected in checks.items():
        if value[key] != expected:
            raise AuditError(f"target {key} mismatch: got {value[key]!r}, expected {expected!r}")
    if value["column_contract"] != EXPECTED_COLUMN_CONTRACT:
        raise AuditError("target title-column contract is missing or incompatible")
    if value["relation_contract"] != EXPECTED_RELATION_CONTRACT:
        raise AuditError("target audited relations are missing or incompatible")
    if value["row_security"] != {
        "comments": False,
        "edit_info": False,
        "topics": False,
    }:
        raise AuditError("target tables are missing or row-level security prevents a complete audit")
    flags = value["role_flags"]
    if not isinstance(flags, dict) or set(flags) != {
        "rolsuper",
        "rolcreaterole",
        "rolcreatedb",
        "rolreplication",
        "rolbypassrls",
    }:
        raise AuditError("target role flags are incomplete")
    if any(flag is not False for flag in flags.values()):
        raise AuditError("audit role must be unprivileged")
    if value["mutation_privileges"] is not False:
        raise AuditError("audit role must not have title-table mutation privileges")
    if value["select_privileges"] is not True:
        raise AuditError("audit role lacks a required title-column SELECT privilege")
    if not isinstance(value["server_version_num"], str) or not value[
        "server_version_num"
    ].isdigit():
        raise AuditError("target server_version_num is invalid")
    if (
        not isinstance(value["server_address"], str)
        or value["server_address"] == "local-socket"
        or not isinstance(value["server_port"], int)
        or not 1 <= value["server_port"] <= 65_535
    ):
        raise AuditError("target server address/port is invalid")
    return dict(value)


def _canonical_json(value: object) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _open_private_text(path: Path):
    descriptor = os.open(
        path,
        os.O_WRONLY
        | os.O_CREAT
        | os.O_EXCL
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_NOFOLLOW", 0),
        0o600,
    )
    return os.fdopen(descriptor, "w", encoding="utf-8", newline="")


def _publish_artifacts_exclusively(pairs: Sequence[tuple[Path, Path]]) -> None:
    """Publish a complete bundle without replacing an existing path."""

    published: list[Path] = []
    try:
        for temporary, final in pairs:
            os.link(temporary, final, follow_symlinks=False)
            published.append(final)
        for temporary, _final in pairs:
            temporary.unlink()
    except OSError as error:
        for final in published:
            with contextlib.suppress(FileNotFoundError):
                final.unlink()
        raise AuditError(f"cannot publish the private audit bundle: {error}") from error


def _group_add(groups: dict[tuple[object, ...], GroupDigest], key: tuple[object, ...], row_hash: str) -> None:
    groups.setdefault(key, GroupDigest()).add(row_hash)


CSV_FIELDS = (
    "source",
    "entity_kind",
    "pipeline",
    "row_id",
    "entity_id",
    "written_at",
    "date_utc",
    "deployment_window",
    "id_bucket_start",
    "id_bucket_end",
    "classification",
    "raw_five",
    "canonical_entities",
    "other_entities",
    "double_encoded",
    "mixed_representation",
    "unicode_chars",
    "utf8_bytes",
    "title_sha256",
    "decoded_once_sha256",
    "row_sha256",
)


def _group_rows(
    groups: Mapping[tuple[object, ...], GroupDigest], fields: Sequence[str]
) -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    for key in sorted(groups):
        group = groups[key]
        row = {name: value for name, value in zip(fields, key)}
        row.update({"row_count": group.count, "rows_sha256": group.hexdigest()})
        rows.append(row)
    return rows


def _validate_row_payload(value: object) -> dict[str, object]:
    if not isinstance(value, dict) or set(value) != ROW_KEYS or value.get("record_type") != "row":
        raise AuditError("audit stream contains an invalid row record")
    source = value["source"]
    if source not in SOURCE_ORDER:
        raise AuditError(f"audit stream contains an unknown source: {source!r}")
    if not isinstance(value["row_id"], int) or not isinstance(value["entity_id"], int):
        raise AuditError("audit row identifiers must be integers")
    if not isinstance(value["title"], str):
        raise AuditError("audit title must be non-null text")
    pipeline = value["pipeline"]
    expected_pipeline = (
        "history_title_snapshot_maketitle_or_raw"
        if source == "edit_info.oldtitle"
        else "current_title_storage"
    )
    if pipeline != expected_pipeline:
        raise AuditError(f"audit row has an invalid pipeline for {source}")
    entity_kind = value["entity_kind"]
    expected_kinds = (
        {"TOPIC", "COMMENT"}
        if source == "edit_info.oldtitle"
        else {"TOPIC"}
        if source == "topics.title"
        else {"COMMENT"}
    )
    if entity_kind not in expected_kinds:
        raise AuditError(f"audit row has an invalid entity kind for {source}")
    _parse_rfc3339("audit row written_at", value["written_at"], nullable=False)
    return value


def write_audit_artifacts(
    *,
    target_identity: Mapping[str, object],
    rows: Iterable[object],
    windows: Sequence[DeploymentWindow],
    windows_sha256: str,
    id_bucket_size: int,
    output_dir: Path,
    connection: DatabaseTarget,
) -> dict[str, object]:
    if id_bucket_size <= 0:
        raise AuditError("id bucket size must be positive")
    if output_dir.is_symlink():
        raise AuditError("output directory must not be a symlink")
    if output_dir.exists():
        if not output_dir.is_dir() or any(output_dir.iterdir()):
            raise AuditError("output directory must be absent or empty")
    else:
        output_dir.mkdir(mode=0o700, parents=True)
    os.chmod(output_dir, 0o700)

    csv_temp = output_dir / f".{OUTPUT_CSV}.tmp"
    json_temp = output_dir / f".{OUTPUT_JSON}.tmp"
    summary_temp = output_dir / f".{OUTPUT_SUMMARY}.tmp"
    final_paths = [output_dir / name for name in (OUTPUT_CSV, OUTPUT_JSON, OUTPUT_SUMMARY)]
    if any(path.exists() for path in (*final_paths, csv_temp, json_temp, summary_temp)):
        raise AuditError("audit output paths already exist")

    dataset_digest = hashlib.sha256()
    source_groups: dict[tuple[object, ...], GroupDigest] = {}
    kind_groups: dict[tuple[object, ...], GroupDigest] = {}
    category_groups: dict[tuple[object, ...], GroupDigest] = {}
    id_groups: dict[tuple[object, ...], GroupDigest] = {}
    date_groups: dict[tuple[object, ...], GroupDigest] = {}
    window_groups: dict[tuple[object, ...], GroupDigest] = {}
    row_count = 0
    previous_order: tuple[int, int] | None = None

    try:
        with _open_private_text(csv_temp) as stream:
            writer = csv.DictWriter(stream, fieldnames=CSV_FIELDS, lineterminator="\n")
            writer.writeheader()
            for raw_value in rows:
                value = _validate_row_payload(raw_value)
                source = str(value["source"])
                row_id = int(value["row_id"])
                order = (SOURCE_ORDER[source], row_id)
                if previous_order is not None and order <= previous_order:
                    raise AuditError("audit rows are not in deterministic source/id order")
                previous_order = order

                written_at = _parse_rfc3339(
                    "audit row written_at", value["written_at"], nullable=False
                )
                assert written_at is not None
                written_at_text = written_at.isoformat(timespec="microseconds").replace(
                    "+00:00", "Z"
                )
                title = str(value["title"])
                classification = classify_title(title)
                window = deployment_window_for(written_at, windows)
                bucket_start = (row_id // id_bucket_size) * id_bucket_size
                bucket_end = bucket_start + id_bucket_size - 1
                record: dict[str, object] = {
                    "source": source,
                    "entity_kind": value["entity_kind"],
                    "pipeline": value["pipeline"],
                    "row_id": row_id,
                    "entity_id": int(value["entity_id"]),
                    "written_at": written_at_text,
                    "date_utc": written_at.date().isoformat(),
                    "deployment_window": window,
                    "id_bucket_start": bucket_start,
                    "id_bucket_end": bucket_end,
                    "classification": classification.category,
                    "raw_five": "|".join(classification.raw_five),
                    "canonical_entities": "|".join(classification.canonical_entities),
                    "other_entities": "|".join(classification.other_entities),
                    "double_encoded": classification.double_encoded,
                    "mixed_representation": classification.mixed_representation,
                    "unicode_chars": len(title),
                    "utf8_bytes": len(title.encode("utf-8")),
                    "title_sha256": hashlib.sha256(title.encode("utf-8")).hexdigest(),
                    "decoded_once_sha256": hashlib.sha256(
                        classification.decoded_once.encode("utf-8")
                    ).hexdigest(),
                }
                row_hash = hashlib.sha256(_canonical_json(record)).hexdigest()
                record["row_sha256"] = row_hash
                writer.writerow(
                    {
                        key: (
                            "true"
                            if value is True
                            else "false"
                            if value is False
                            else value
                        )
                        for key, value in record.items()
                    }
                )
                dataset_digest.update(row_hash.encode("ascii"))
                dataset_digest.update(b"\n")
                row_count += 1

                _group_add(source_groups, (source,), row_hash)
                _group_add(kind_groups, (source, value["entity_kind"]), row_hash)
                _group_add(
                    category_groups,
                    (source, value["entity_kind"], classification.category),
                    row_hash,
                )
                _group_add(
                    id_groups,
                    (
                        source,
                        value["entity_kind"],
                        bucket_start,
                        bucket_end,
                        classification.category,
                    ),
                    row_hash,
                )
                _group_add(
                    date_groups,
                    (
                        source,
                        value["entity_kind"],
                        written_at.date().isoformat(),
                        classification.category,
                    ),
                    row_hash,
                )
                _group_add(
                    window_groups,
                    (source, value["entity_kind"], window, classification.category),
                    row_hash,
                )

        csv_sha256 = _sha256_file(csv_temp)
        schema_contract_sha256 = hashlib.sha256(
            _canonical_json(EXPECTED_COLUMN_CONTRACT)
        ).hexdigest()
        relation_contract_sha256 = hashlib.sha256(
            _canonical_json(EXPECTED_RELATION_CONTRACT)
        ).hexdigest()
        safe_target = {
            key: value
            for key, value in target_identity.items()
            if key != "record_type"
        }
        report: dict[str, object] = {
            "schema_version": 1,
            "audit_kind": "mixed_java_rust_title_representation",
            "safety": {
                "automatic_migration_available": False,
                "database_transactions": "REPEATABLE READ READ ONLY",
                "raw_title_values_exported": False,
                "target_verification": "explicit clone marker + database + role + system identifier",
            },
            "provenance": {
                "target": safe_target,
                "requested_endpoint": {
                    "host": connection.requested_host,
                    "port": connection.requested_port,
                    "database": connection.database,
                    "role": connection.role,
                },
                "column_contract_sha256": schema_contract_sha256,
                "relation_contract_sha256": relation_contract_sha256,
                "deployment_windows_sha256": windows_sha256,
                "sql_contract_sha256": hashlib.sha256(
                    (TARGET_PROBE_SQL + "\n" + AUDIT_SQL).encode("utf-8")
                ).hexdigest(),
                "tool_sha256": _sha256_file(Path(__file__).resolve()),
                "production_clone_evidence": "not_claimed",
            },
            "classification_contract": {
                "categories": list(CATEGORIES),
                "canonical_entities": list(CANONICAL_ENTITIES),
                "raw_five": RAW_FIVE_NAMES,
                "one_layer_only": True,
                "one_layer_decoder": "Apache Commons Text unescapeHtml4 compatible",
                "category_precedence": [
                    CATEGORY_RAW,
                    CATEGORY_AMBIGUOUS,
                    CATEGORY_OTHER,
                    CATEGORY_CANONICAL,
                    CATEGORY_PLAIN,
                ],
            },
            "totals": {
                "row_count": row_count,
                "rows_sha256": dataset_digest.hexdigest(),
                "csv_sha256": csv_sha256,
            },
            "groups": {
                "by_source": _group_rows(source_groups, ("source",)),
                "by_source_and_entity_kind": _group_rows(
                    kind_groups, ("source", "entity_kind")
                ),
                "by_classification": _group_rows(
                    category_groups, ("source", "entity_kind", "classification")
                ),
                "by_id_bucket": _group_rows(
                    id_groups,
                    (
                        "source",
                        "entity_kind",
                        "id_bucket_start",
                        "id_bucket_end",
                        "classification",
                    ),
                ),
                "by_utc_date": _group_rows(
                    date_groups,
                    ("source", "entity_kind", "date_utc", "classification"),
                ),
                "by_deployment_window": _group_rows(
                    window_groups,
                    (
                        "source",
                        "entity_kind",
                        "deployment_window",
                        "classification",
                    ),
                ),
            },
        }

        with _open_private_text(json_temp) as stream:
            json.dump(report, stream, ensure_ascii=False, sort_keys=True, indent=2)
            stream.write("\n")
        json_sha256 = _sha256_file(json_temp)
        category_totals = Counter()
        for (_source, _entity_kind, category), digest in category_groups.items():
            category_totals[category] += digest.count
        source_totals = {key[0]: digest.count for key, digest in source_groups.items()}
        summary_lines = [
            "LOR title representation audit (READ ONLY)",
            f"database: {connection.database}",
            f"role: {connection.role}",
            f"clone marker: {target_identity['database_comment']}",
            "production clone evidence: NOT CLAIMED",
            f"rows: {row_count}",
            f"rows sha256: {dataset_digest.hexdigest()}",
            f"csv sha256: {csv_sha256}",
            f"json sha256: {json_sha256}",
            "",
            "Rows by source:",
        ]
        summary_lines.extend(
            f"- {source}: {source_totals.get(source, 0)}" for source in SOURCE_ORDER
        )
        summary_lines.extend(("", "Rows by classification:"))
        summary_lines.extend(
            f"- {category}: {category_totals.get(category, 0)}" for category in CATEGORIES
        )
        summary_lines.extend(
            (
                "",
                "No UPDATE/INSERT/DELETE or automatic migration was performed.",
                "Review hashed row ids on the same verified clone before designing a migration.",
            )
        )
        with _open_private_text(summary_temp) as stream:
            stream.write("\n".join(summary_lines) + "\n")

        _publish_artifacts_exclusively(
            (
                (csv_temp, output_dir / OUTPUT_CSV),
                (json_temp, output_dir / OUTPUT_JSON),
                (summary_temp, output_dir / OUTPUT_SUMMARY),
            )
        )
        return report
    except Exception:
        for path in (csv_temp, json_temp, summary_temp):
            try:
                path.unlink()
            except FileNotFoundError:
                pass
        raise


def stream_database_rows(
    *,
    target: DatabaseTarget,
    psql: str,
    expected_identity: Mapping[str, object],
    expected_database: str,
    expected_role: str,
    expected_system_identifier: str,
    clone_marker: str,
) -> Iterator[dict[str, object]]:
    error_stream = tempfile.TemporaryFile(mode="w+t", encoding="utf-8")
    try:
        process = subprocess.Popen(
            _psql_command(psql),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=error_stream,
            text=True,
            encoding="utf-8",
            env=target.env,
        )
    except OSError as error:
        error_stream.close()
        raise AuditError(f"cannot execute psql title scan: {psql}") from error
    assert process.stdin is not None and process.stdout is not None
    identity_seen = False
    stream_exhausted = False
    stderr = ""
    return_code = -1
    try:
        try:
            process.stdin.write(AUDIT_SQL)
            process.stdin.close()
        except OSError as error:
            raise AuditError("failed to submit the fixed read-only query to psql") from error
        for raw_line in process.stdout:
            line = raw_line.rstrip("\n")
            if not line:
                raise AuditError("audit stream contains an unexpected blank line")
            try:
                value = json.loads(line)
            except json.JSONDecodeError as error:
                raise AuditError("audit stream contains invalid JSON") from error
            if not identity_seen:
                identity = validate_target_identity(
                    value,
                    expected_database=expected_database,
                    expected_role=expected_role,
                    expected_system_identifier=expected_system_identifier,
                    clone_marker=clone_marker,
                )
                if identity != dict(expected_identity):
                    raise AuditError("target identity changed between catalog probe and title scan")
                identity_seen = True
                continue
            yield _validate_row_payload(value)
        stream_exhausted = True
    finally:
        process.stdout.close()
        if not stream_exhausted and process.poll() is None:
            with contextlib.suppress(ProcessLookupError):
                process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                with contextlib.suppress(ProcessLookupError):
                    process.kill()
        return_code = process.wait()
        error_stream.seek(0)
        stderr = error_stream.read()
        error_stream.close()
    if return_code != 0:
        raise AuditError(f"read-only title scan failed: {_psql_error(stderr)}")
    if not identity_seen:
        raise AuditError("read-only title scan returned no verified target identity")


def run_audit(args: argparse.Namespace) -> dict[str, object]:
    if args.confirm != AUDIT_CONFIRMATION:
        raise AuditError(f"--confirm must equal {AUDIT_CONFIRMATION!r}")
    _validate_clone_marker(args.clone_marker)
    if not SYSTEM_IDENTIFIER_RE.fullmatch(args.expected_system_identifier):
        raise AuditError("expected system identifier must contain 8-32 decimal digits")
    if not args.expected_database or not args.expected_role:
        raise AuditError("expected database and role are required")
    if not 1_000 <= args.statement_timeout_ms <= 3_600_000:
        raise AuditError("statement timeout must be between 1000 and 3600000 ms")

    windows, windows_sha256 = read_deployment_windows(args.deployment_windows)
    target = load_database_target(
        args.database_url_file,
        expected_database=args.expected_database,
        expected_role=args.expected_role,
    )
    target.env["PGOPTIONS"] += f" -c statement_timeout={args.statement_timeout_ms}"
    probe = run_target_probe(target, args.psql)
    identity = validate_target_identity(
        probe,
        expected_database=args.expected_database,
        expected_role=args.expected_role,
        expected_system_identifier=args.expected_system_identifier,
        clone_marker=args.clone_marker,
    )
    with contextlib.closing(
        stream_database_rows(
            target=target,
            psql=args.psql,
            expected_identity=identity,
            expected_database=args.expected_database,
            expected_role=args.expected_role,
            expected_system_identifier=args.expected_system_identifier,
            clone_marker=args.clone_marker,
        )
    ) as rows:
        return write_audit_artifacts(
            target_identity=identity,
            rows=rows,
            windows=windows,
            windows_sha256=windows_sha256,
            id_bucket_size=args.id_bucket_size,
            output_dir=args.output_dir,
            connection=target,
        )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="READ-ONLY audit of topics/comments/edit-history title representation"
    )
    parser.add_argument("--database-url-file", required=True, type=Path)
    parser.add_argument("--expected-database", required=True)
    parser.add_argument("--expected-role", required=True)
    parser.add_argument("--expected-system-identifier", required=True)
    parser.add_argument("--clone-marker", required=True)
    parser.add_argument("--deployment-windows", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--id-bucket-size", type=int, default=100_000)
    parser.add_argument("--statement-timeout-ms", type=int, default=300_000)
    parser.add_argument("--psql", default="psql")
    parser.add_argument("--confirm", required=True)
    return parser


def main() -> int:
    try:
        args = build_parser().parse_args()
        report = run_audit(args)
        totals = report["totals"]
        print(
            f"READ-ONLY title audit complete: rows={totals['row_count']} "
            f"sha256={totals['rows_sha256']}"
        )
        print(f"Artifacts: {args.output_dir}")
        print("Production clone evidence: NOT CLAIMED")
        return 0
    except AuditError as error:
        print(f"Title audit refused: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
