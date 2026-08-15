#!/usr/bin/env python3
"""Validate production cutover evidence without reading deployment secrets."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import re
import sys
import urllib.parse
from pathlib import Path
from typing import Any


IMAGE_DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
WAL_POSITION_RE = re.compile(r"^[0-9A-F]+/[0-9A-F]+$")
SAFE_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:/-]{7,127}$")
PLACEHOLDER_PARTS = {
    "demo",
    "example",
    "fake",
    "local",
    "placeholder",
    "sample",
    "test",
    "tbd",
    "todo",
    "unknown",
}
COMMON_KEYS = {
    "schema_version",
    "kind",
    "rehearsal_id",
    "captured_at",
    "image_digest",
    "database_snapshot_id",
    "database_wal_position",
    "status",
    "evidence",
}
CONFIG_CHECKS = {
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
MEDIA_BOOL_CHECKS = {
    "read_probe_passed",
    "write_probe_passed",
    "atomic_rename_probe_passed",
    "cleanup_probe_passed",
    "ownership_survives_restart",
    "backup_restore_probe_passed",
}
REQUIRED_ADAPTERS = {
    "opensearch",
    "smtp",
    "captcha",
    "geoip",
    "tor_exit_list",
    "disposable_email_domains",
    "telegram",
}


class EvidenceError(ValueError):
    pass


def load_document(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvidenceError(f"{path}: cannot read JSON evidence: {error}") from error
    if not isinstance(value, dict):
        raise EvidenceError(f"{path}: evidence root must be a JSON object")
    return value


def validate_identifier(name: str, value: object) -> str:
    if not isinstance(value, str) or not SAFE_ID_RE.fullmatch(value):
        raise EvidenceError(f"{name} must be an explicit 8-128 character identifier")
    components = {part.lower() for part in re.split(r"[^A-Za-z0-9]+", value) if part}
    if components & PLACEHOLDER_PARTS:
        raise EvidenceError(f"{name} contains a placeholder marker")
    return value


def parse_timestamp(name: str, value: object, now: dt.datetime, max_age: dt.timedelta) -> dt.datetime:
    if not isinstance(value, str):
        raise EvidenceError(f"{name} must be an RFC3339 timestamp")
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise EvidenceError(f"{name} must be an RFC3339 timestamp") from error
    if parsed.tzinfo is None:
        raise EvidenceError(f"{name} must include a timezone")
    parsed = parsed.astimezone(dt.timezone.utc)
    if parsed > now + dt.timedelta(minutes=5):
        raise EvidenceError(f"{name} is in the future")
    if now - parsed > max_age:
        raise EvidenceError(f"{name} is older than the allowed evidence age")
    return parsed


def validate_common(
    document: dict[str, Any],
    *,
    kind: str,
    image_digest: str,
    snapshot_id: str,
    wal_position: str,
    now: dt.datetime,
    max_age: dt.timedelta,
) -> str:
    if set(document) != COMMON_KEYS:
        missing = sorted(COMMON_KEYS - set(document))
        extra = sorted(set(document) - COMMON_KEYS)
        raise EvidenceError(f"{kind}: invalid top-level keys; missing={missing}, extra={extra}")
    if document["schema_version"] != 1 or document["kind"] != kind:
        raise EvidenceError(f"{kind}: unsupported schema_version or kind")
    if document["status"] != "passed":
        raise EvidenceError(f"{kind}: status must be passed")
    rehearsal_id = validate_identifier(f"{kind}.rehearsal_id", document["rehearsal_id"])
    parse_timestamp(f"{kind}.captured_at", document["captured_at"], now, max_age)
    if document["image_digest"] != image_digest:
        raise EvidenceError(f"{kind}: image digest does not match CUTOVER_IMAGE_DIGEST")
    if document["database_snapshot_id"] != snapshot_id:
        raise EvidenceError(f"{kind}: snapshot id does not match CUTOVER_SNAPSHOT_ID")
    if document["database_wal_position"] != wal_position:
        raise EvidenceError(f"{kind}: WAL position does not match CUTOVER_WAL_POSITION")
    if not isinstance(document["evidence"], dict):
        raise EvidenceError(f"{kind}: evidence must be a JSON object")
    return rehearsal_id


def validate_config(document: dict[str, Any]) -> None:
    evidence = document["evidence"]
    if set(evidence) != CONFIG_CHECKS:
        raise EvidenceError("configuration: evidence must contain the exact production checks")
    failed = sorted(name for name, value in evidence.items() if value is not True)
    if failed:
        raise EvidenceError(f"configuration: checks did not pass: {failed}")


def validate_media(document: dict[str, Any]) -> None:
    evidence = document["evidence"]
    expected = {
        "upload_root",
        "runtime_uid",
        "runtime_gid",
        "directories",
        "representative_files_checked",
        "storage_snapshot_id",
        *MEDIA_BOOL_CHECKS,
    }
    if set(evidence) != expected:
        raise EvidenceError("media: evidence has missing or unexpected checks")
    upload_root = evidence["upload_root"]
    if not isinstance(upload_root, str) or not upload_root.startswith("/") or upload_root in {"/", "/tmp"}:
        raise EvidenceError("media.upload_root must be an explicit dedicated absolute path")
    if evidence["runtime_uid"] != 8181 or evidence["runtime_gid"] != 8181:
        raise EvidenceError("media runtime ownership must match UID/GID 8181")
    directories = evidence["directories"]
    if (
        not isinstance(directories, list)
        or any(not isinstance(value, str) for value in directories)
        or set(directories) != {"photos", "gallery", "images"}
    ):
        raise EvidenceError("media directories must cover photos, gallery and images")
    if not isinstance(evidence["representative_files_checked"], int) or evidence["representative_files_checked"] <= 0:
        raise EvidenceError("media requires at least one representative file check")
    validate_identifier("media.storage_snapshot_id", evidence["storage_snapshot_id"])
    failed = sorted(name for name in MEDIA_BOOL_CHECKS if evidence[name] is not True)
    if failed:
        raise EvidenceError(f"media checks did not pass: {failed}")


def validate_endpoint(name: str, value: object) -> None:
    if not isinstance(value, str):
        raise EvidenceError(f"external.{name}.endpoint must be a redacted URL")
    try:
        parsed = urllib.parse.urlsplit(value)
        hostname = parsed.hostname
    except ValueError as error:
        raise EvidenceError(f"external.{name}.endpoint is malformed") from error
    if parsed.scheme not in {"http", "https", "smtp"} or not hostname:
        raise EvidenceError(f"external.{name}.endpoint must use http, https or smtp")
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise EvidenceError(f"external.{name}.endpoint must not contain credentials, query or fragment")


def validate_external(document: dict[str, Any], now: dt.datetime, max_age: dt.timedelta) -> None:
    adapters = document["evidence"]
    if set(adapters) != REQUIRED_ADAPTERS:
        raise EvidenceError("external: all required adapters must have explicit evidence")
    for name, adapter in adapters.items():
        if not isinstance(adapter, dict):
            raise EvidenceError(f"external.{name} must be an object")
        allowed_statuses = {"passed", "disabled"} if name == "telegram" else {"passed"}
        if adapter.get("status") not in allowed_statuses:
            raise EvidenceError(f"external.{name}.status must be one of {sorted(allowed_statuses)}")
        expected = {"status", "checked_at", "endpoint", "contract_verified"}
        if adapter.get("status") == "disabled":
            expected.add("disabled_reason")
        if set(adapter) != expected:
            raise EvidenceError(f"external.{name} has missing or unexpected fields")
        parse_timestamp(f"external.{name}.checked_at", adapter["checked_at"], now, max_age)
        validate_endpoint(name, adapter["endpoint"])
        if adapter["status"] == "passed" and adapter["contract_verified"] is not True:
            raise EvidenceError(f"external.{name} contract was not verified")
        if adapter["status"] == "disabled":
            if adapter["contract_verified"] is not False:
                raise EvidenceError("disabled Telegram evidence must not claim a verified contract")
            if not isinstance(adapter["disabled_reason"], str) or len(adapter["disabled_reason"].strip()) < 8:
                raise EvidenceError("disabled Telegram evidence requires an explicit reason")


def validate_all(
    config_path: Path,
    media_path: Path,
    external_path: Path,
    image_digest: str,
    snapshot_id: str,
    wal_position: str,
    max_age_hours: float,
    *,
    now: dt.datetime | None = None,
) -> str:
    if not IMAGE_DIGEST_RE.fullmatch(image_digest):
        raise EvidenceError("image digest must be sha256 followed by 64 lowercase hex characters")
    validate_identifier("snapshot id", snapshot_id)
    if not WAL_POSITION_RE.fullmatch(wal_position):
        raise EvidenceError("WAL position must be a PostgreSQL LSN such as 16/B374D848")
    if max_age_hours <= 0:
        raise EvidenceError("maximum evidence age must be positive")
    now = (now or dt.datetime.now(dt.timezone.utc)).astimezone(dt.timezone.utc)
    max_age = dt.timedelta(hours=max_age_hours)
    documents = [
        ("configuration", load_document(config_path)),
        ("media", load_document(media_path)),
        ("external-adapters", load_document(external_path)),
    ]
    rehearsal_ids = {
        validate_common(
            document,
            kind=kind,
            image_digest=image_digest,
            snapshot_id=snapshot_id,
            wal_position=wal_position,
            now=now,
            max_age=max_age,
        )
        for kind, document in documents
    }
    if len(rehearsal_ids) != 1:
        raise EvidenceError("all evidence files must use the same rehearsal_id")
    validate_config(documents[0][1])
    validate_media(documents[1][1])
    validate_external(documents[2][1], now, max_age)
    return rehearsal_ids.pop()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--media", required=True, type=Path)
    parser.add_argument("--external", required=True, type=Path)
    parser.add_argument("--image-digest", required=True)
    parser.add_argument("--snapshot-id", required=True)
    parser.add_argument("--wal-position", required=True)
    parser.add_argument("--max-age-hours", type=float, default=168)
    args = parser.parse_args()
    try:
        rehearsal_id = validate_all(
            args.config,
            args.media,
            args.external,
            args.image_digest,
            args.snapshot_id,
            args.wal_position,
            args.max_age_hours,
        )
    except EvidenceError as error:
        print(f"cutover evidence rejected: {error}", file=sys.stderr)
        return 1
    print(f"Cutover evidence validated for rehearsal {rehearsal_id}.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
