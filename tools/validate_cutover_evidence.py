#!/usr/bin/env python3
"""Validate production cutover evidence without reading deployment secrets."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import re
import sys
import urllib.parse
from pathlib import Path
from typing import Any
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError


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
    "scheduler_timezone_configured",
    "legacy_jdbc_timezone_configured",
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
OPERATIONS_KEYS = {"production_clone", "scheduler", "search_cutover", "lifecycle"}
SEARCH_ARTIFACT_COMMON_KEYS = {
    "schema_version",
    "kind",
    "rehearsal_id",
    "captured_at",
    "image_digest",
    "database_snapshot_id",
    "database_wal_position",
    "mode",
    "java_writers_stopped",
    "java_consumers_stopped",
    "rust_spool_pending",
    "rust_spool_processing",
}
SEARCH_DRAIN_KEYS = {"queue_name", "ready_messages", "inflight_messages"}
SEARCH_REINDEX_KEYS = {
    "full_reindex_completed",
    "legacy_queue_disposition_recorded",
    "expected_documents",
    "indexed_documents",
    "reconciliation_passed",
    "representative_queries_checked",
    "opensearch_snapshot_id",
    "expected_id_set_sha256",
    "indexed_id_set_sha256",
    "expected_content_sha256",
    "indexed_content_sha256",
}
EMPTY_SHA256 = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"


class EvidenceError(ValueError):
    pass


def parse_strict_json(payload: bytes, label: str) -> Any:
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError as error:
        raise EvidenceError(f"{label} must be strict UTF-8 JSON: {error}") from error

    def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        value: dict[str, Any] = {}
        for key, item in pairs:
            if key in value:
                raise EvidenceError(f"{label} contains duplicate object key {key!r}")
            value[key] = item
        return value

    def reject_non_finite_constant(value: str) -> None:
        raise EvidenceError(f"{label} contains non-standard JSON constant {value}")

    try:
        return json.loads(
            text,
            object_pairs_hook=reject_duplicate_keys,
            parse_constant=reject_non_finite_constant,
        )
    except json.JSONDecodeError as error:
        raise EvidenceError(f"{label} must be strict UTF-8 JSON: {error}") from error


def load_document(path: Path) -> dict[str, Any]:
    try:
        payload = path.read_bytes()
    except OSError as error:
        raise EvidenceError(f"{path}: cannot read JSON evidence: {error}") from error
    value = parse_strict_json(payload, f"{path}: JSON evidence")
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


def validate_artifact_digest(name: str, value: object) -> str:
    if not isinstance(value, str) or not IMAGE_DIGEST_RE.fullmatch(value):
        raise EvidenceError(f"{name} must be sha256 followed by 64 lowercase hex characters")
    return value


def load_search_artifact(path: Path) -> tuple[dict[str, Any], str]:
    try:
        payload = path.read_bytes()
    except OSError as error:
        raise EvidenceError(f"{path}: cannot read retained search evidence artifact: {error}") from error
    if not payload:
        raise EvidenceError(f"{path}: retained search evidence artifact must not be empty")
    value = parse_strict_json(payload, f"{path}: retained search evidence artifact")
    if not isinstance(value, dict):
        raise EvidenceError(f"{path}: retained search evidence artifact root must be an object")
    return value, "sha256:" + hashlib.sha256(payload).hexdigest()


def validate_non_negative_int(name: str, value: object) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise EvidenceError(f"{name} must be a non-negative integer")
    return value


def validate_all_true(name: str, value: object, expected: set[str]) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != expected:
        raise EvidenceError(f"{name} must contain the exact required checks")
    failed = sorted(key for key, result in value.items() if result is not True)
    if failed:
        raise EvidenceError(f"{name} checks did not pass: {failed}")
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


def validate_iana_timezone(name: str, value: object) -> str:
    if not isinstance(value, str) or not value or value.startswith("/") or ".." in value:
        raise EvidenceError(f"{name} must be a valid IANA timezone name")
    try:
        ZoneInfo(value)
    except (ZoneInfoNotFoundError, ValueError) as error:
        raise EvidenceError(f"{name} must be a valid IANA timezone name") from error
    return value


def validate_search_cutover(
    value: object,
    now: dt.datetime,
    max_age: dt.timedelta,
    search_artifact: dict[str, Any],
    search_artifact_sha256: str,
    *,
    rehearsal_id: str,
    image_digest: str,
    snapshot_id: str,
    wal_position: str,
) -> None:
    if not isinstance(value, dict):
        raise EvidenceError("operations.search_cutover must be an object")
    common = {
        "mode",
        "checked_at",
        "java_writers_stopped",
        "java_consumers_stopped",
        "rust_spool_pending",
        "rust_spool_processing",
        "artifact_sha256",
    }
    mode = value.get("mode")
    if mode == "activemq-drained":
        expected = common | {"queue_name", "ready_messages", "inflight_messages"}
    elif mode == "full-reindex":
        expected = common | SEARCH_REINDEX_KEYS
    else:
        raise EvidenceError(
            "operations.search_cutover.mode must be activemq-drained or full-reindex"
        )
    if set(value) != expected:
        raise EvidenceError(
            f"operations.search_cutover {mode} evidence has missing or unexpected fields"
        )
    parse_timestamp("operations.search_cutover.checked_at", value["checked_at"], now, max_age)
    for name in ("java_writers_stopped", "java_consumers_stopped"):
        if value[name] is not True:
            raise EvidenceError(f"operations.search_cutover.{name} must be true")
    for name in ("rust_spool_pending", "rust_spool_processing"):
        if validate_non_negative_int(f"operations.search_cutover.{name}", value[name]) != 0:
            raise EvidenceError(f"operations.search_cutover.{name} must be zero")
    artifact_sha256 = validate_artifact_digest(
        "operations.search_cutover.artifact_sha256", value["artifact_sha256"]
    )
    if artifact_sha256 != search_artifact_sha256:
        raise EvidenceError(
            "operations.search_cutover artifact digest does not match the retained probe/reindex artifact"
        )

    artifact_expected = SEARCH_ARTIFACT_COMMON_KEYS | (
        SEARCH_DRAIN_KEYS if mode == "activemq-drained" else SEARCH_REINDEX_KEYS
    )
    if set(search_artifact) != artifact_expected:
        raise EvidenceError(
            f"retained search artifact {mode} has missing or unexpected fields"
        )
    if search_artifact["schema_version"] != 1 or search_artifact["kind"] != "search-cutover":
        raise EvidenceError(
            "retained search artifact must use schema_version 1 and kind search-cutover"
        )
    artifact_rehearsal = validate_identifier(
        "search artifact rehearsal_id", search_artifact["rehearsal_id"]
    )
    if artifact_rehearsal != rehearsal_id:
        raise EvidenceError("retained search artifact rehearsal_id does not match the evidence set")
    parse_timestamp(
        "search artifact captured_at", search_artifact["captured_at"], now, max_age
    )
    if search_artifact["captured_at"] != value["checked_at"]:
        raise EvidenceError(
            "retained search artifact captured_at must match operations.search_cutover.checked_at"
        )
    for name, expected_value in (
        ("image_digest", image_digest),
        ("database_snapshot_id", snapshot_id),
        ("database_wal_position", wal_position),
    ):
        if search_artifact[name] != expected_value:
            raise EvidenceError(
                f"retained search artifact {name} does not match the cutover evidence set"
            )
    artifact_cross_checks = {
        "mode",
        "java_writers_stopped",
        "java_consumers_stopped",
        "rust_spool_pending",
        "rust_spool_processing",
        *(SEARCH_DRAIN_KEYS if mode == "activemq-drained" else SEARCH_REINDEX_KEYS),
    }
    mismatches = sorted(
        name for name in artifact_cross_checks if search_artifact[name] != value[name]
    )
    if mismatches:
        raise EvidenceError(
            "retained search artifact disagrees with operations.search_cutover: "
            f"{mismatches}"
        )

    if mode == "activemq-drained":
        if value["queue_name"] != "lor.searchQueue":
            raise EvidenceError("operations.search_cutover.queue_name must be lor.searchQueue")
        for name in ("ready_messages", "inflight_messages"):
            if validate_non_negative_int(f"operations.search_cutover.{name}", value[name]) != 0:
                raise EvidenceError(
                    "ActiveMQ cutover requires zero ready and inflight search messages"
                )
        return

    for name in (
        "full_reindex_completed",
        "legacy_queue_disposition_recorded",
        "reconciliation_passed",
    ):
        if value[name] is not True:
            raise EvidenceError(f"operations.search_cutover.{name} must be true")
    expected_documents = validate_non_negative_int(
        "operations.search_cutover.expected_documents", value["expected_documents"]
    )
    indexed_documents = validate_non_negative_int(
        "operations.search_cutover.indexed_documents", value["indexed_documents"]
    )
    if expected_documents <= 0 or indexed_documents != expected_documents:
        raise EvidenceError(
            "full reindex requires a positive exact expected/indexed document reconciliation"
        )
    representative_queries = validate_non_negative_int(
        "operations.search_cutover.representative_queries_checked",
        value["representative_queries_checked"],
    )
    if representative_queries <= 0:
        raise EvidenceError("full reindex requires representative query checks")
    validate_identifier(
        "operations.search_cutover.opensearch_snapshot_id", value["opensearch_snapshot_id"]
    )
    for name in (
        "expected_id_set_sha256",
        "indexed_id_set_sha256",
        "expected_content_sha256",
        "indexed_content_sha256",
    ):
        digest = validate_artifact_digest(
            f"operations.search_cutover.{name}", value[name]
        )
        if digest == EMPTY_SHA256 or digest == "sha256:" + "0" * 64:
            raise EvidenceError(
                f"operations.search_cutover.{name} must describe a non-empty reconciliation set"
            )
    if value["expected_id_set_sha256"] != value["indexed_id_set_sha256"]:
        raise EvidenceError("full reindex requires identical expected/indexed ID-set digests")
    if value["expected_content_sha256"] != value["indexed_content_sha256"]:
        raise EvidenceError("full reindex requires identical expected/indexed content digests")


def validate_operations(
    document: dict[str, Any],
    now: dt.datetime,
    max_age: dt.timedelta,
    search_artifact: dict[str, Any],
    search_artifact_sha256: str,
    *,
    rehearsal_id: str,
    image_digest: str,
    snapshot_id: str,
    wal_position: str,
) -> None:
    evidence = document["evidence"]
    if set(evidence) != OPERATIONS_KEYS:
        raise EvidenceError("operations: evidence must contain the exact operational sections")

    validate_all_true(
        "operations.production_clone",
        evidence["production_clone"],
        {
            "restore_verified",
            "liquibase_validate_passed",
            "runtime_schema_contract_passed",
            "java_rust_comparison_passed",
        },
    )

    scheduler = evidence["scheduler"]
    expected_scheduler = {
        "original_java_timezone",
        "rust_scheduler_timezone",
        "legacy_jdbc_timezone",
        "timezone_match_verified",
        "active_scheduler_instances",
        "single_scheduler_verified",
    }
    if not isinstance(scheduler, dict) or set(scheduler) != expected_scheduler:
        raise EvidenceError("operations.scheduler must contain the exact scheduler evidence")
    timezones = [
        validate_iana_timezone(f"operations.scheduler.{name}", scheduler[name])
        for name in (
            "original_java_timezone",
            "rust_scheduler_timezone",
            "legacy_jdbc_timezone",
        )
    ]
    if len(set(timezones)) != 1 or scheduler["timezone_match_verified"] is not True:
        raise EvidenceError(
            "scheduler and legacy JDBC timezones must match the evidenced Java JVM timezone"
        )
    if (
        validate_non_negative_int(
            "operations.scheduler.active_scheduler_instances",
            scheduler["active_scheduler_instances"],
        )
        != 1
        or scheduler["single_scheduler_verified"] is not True
    ):
        raise EvidenceError("operations.scheduler must prove exactly one active scheduler")

    validate_search_cutover(
        evidence["search_cutover"],
        now,
        max_age,
        search_artifact,
        search_artifact_sha256,
        rehearsal_id=rehearsal_id,
        image_digest=image_digest,
        snapshot_id=snapshot_id,
        wal_position=wal_position,
    )
    validate_all_true(
        "operations.lifecycle",
        evidence["lifecycle"],
        {
            "sigterm_drain_passed",
            "restart_health_passed",
            "rollback_switch_passed",
            "post_rollback_smoke_passed",
        },
    )


def validate_all(
    config_path: Path,
    media_path: Path,
    external_path: Path,
    operations_path: Path,
    search_artifact_path: Path,
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
        ("operations", load_document(operations_path)),
    ]
    search_artifact, search_artifact_sha256 = load_search_artifact(search_artifact_path)
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
    rehearsal_id = next(iter(rehearsal_ids))
    validate_config(documents[0][1])
    validate_media(documents[1][1])
    validate_external(documents[2][1], now, max_age)
    validate_operations(
        documents[3][1],
        now,
        max_age,
        search_artifact,
        search_artifact_sha256,
        rehearsal_id=rehearsal_id,
        image_digest=image_digest,
        snapshot_id=snapshot_id,
        wal_position=wal_position,
    )
    return rehearsal_id


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--media", required=True, type=Path)
    parser.add_argument("--external", required=True, type=Path)
    parser.add_argument("--operations", required=True, type=Path)
    parser.add_argument("--search-artifact", required=True, type=Path)
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
            args.operations,
            args.search_artifact,
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
