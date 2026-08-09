#!/usr/bin/env python3
"""Load deterministic LOR-like data into the disposable Compose instance."""

from __future__ import annotations

import argparse
import io
import json
import os
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SEED_SQL = Path(__file__).with_name("seed.sql")
CONFIRMATION = "seed-disposable-compose-lor"
IMAGE_IDS = {
    9104001: (26, 80, 125, "single desktop"),
    9104002: (52, 101, 164, "slider desktop"),
    9104003: (84, 48, 107, "slider monitor"),
    9104004: (46, 125, 91, "slider game"),
    9104005: (128, 77, 30, "corrector workplace"),
    9104006: (37, 92, 122, "pending gallery preview"),
}
PHOTO_IDS = {
    9100011: (30, 110, 170, "TC"),
    9100012: (145, 72, 120, "IC"),
    9100013: (130, 70, 25, "HM"),
    9100014: (45, 95, 55, "EM"),
}
FIXTURE_NICKS = (
    "swift45",
    "finch50",
    "lark70",
    "robin201",
    "oriole300",
    "falcon500",
    "heron750",
    "raven1000",
    "crane2000",
    "albatross3000",
    "tern_corrector",
    "ibis_corrector",
    "hawk_moderator",
    "eagle_moderator",
)


def run(
    command: list[str],
    *,
    input_bytes: bytes | None = None,
    capture: bool = False,
) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        command,
        cwd=ROOT,
        input=input_bytes,
        check=True,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
    )


def compose(*arguments: str, input_bytes: bytes | None = None, capture: bool = False):
    return run(
        ["docker", "compose", *arguments],
        input_bytes=input_bytes,
        capture=capture,
    )


def query(sql: str) -> str:
    result = compose(
        "exec",
        "-T",
        "postgres",
        "psql",
        "-X",
        "-U",
        "postgres",
        "-d",
        "lor",
        "-At",
        "-v",
        "ON_ERROR_STOP=1",
        "-c",
        sql,
        capture=True,
    )
    return result.stdout.decode("utf-8").strip()


def verify_target() -> None:
    services = compose("ps", "--status", "running", "--services", capture=True)
    running = set(services.stdout.decode("utf-8").split())
    missing = {"postgres", "app"} - running
    if missing:
        raise RuntimeError(
            "Compose services are not running: " + ", ".join(sorted(missing))
        )

    identity = query(
        "SELECT current_database()||'|'||current_user||'|'||"
        "(to_regclass('public.databasechangelog') IS NOT NULL)::text"
    )
    if identity != "lor|postgres|true":
        raise RuntimeError(f"unexpected database target: {identity!r}")
    if query("SELECT count(*) FROM sections") != "5":
        raise RuntimeError("unexpected section catalog; refusing to mutate the database")


def load_sql(accounts_only: bool) -> None:
    arguments = [
        "exec",
        "-T",
        "postgres",
        "psql",
        "-X",
        "-U",
        "postgres",
        "-d",
        "lor",
        "-v",
        "ON_ERROR_STOP=1",
    ]
    if accounts_only:
        arguments.extend(("-v", "accounts_only=true"))
    arguments.extend((
        "-f",
        "-",
    ))
    compose(
        *arguments,
        input_bytes=SEED_SQL.read_bytes(),
    )


def reset_search_fixture() -> None:
    """Remove only fixture-authored documents left by earlier UI runs."""

    payload = json.dumps(
        {
            "query": {
                "bool": {
                    "should": [
                        {"terms": {"author": FIXTURE_NICKS}},
                        {"terms": {"topic_author": FIXTURE_NICKS}},
                    ],
                    "minimum_should_match": 1,
                }
            }
        },
        separators=(",", ":"),
    ).encode("utf-8")
    response = compose(
        "exec",
        "-T",
        "opensearch",
        "curl",
        "-sS",
        "-XPOST",
        "http://localhost:9200/messages/_delete_by_query?refresh=true&conflicts=proceed",
        "-H",
        "Content-Type: application/json",
        "--data-binary",
        "@-",
        "-w",
        "\n%{http_code}",
        input_bytes=payload,
        capture=True,
    )
    body, _, status = response.stdout.decode("utf-8").rpartition("\n")
    if status not in {"200", "404"}:
        raise RuntimeError(f"OpenSearch fixture cleanup failed: HTTP {status}: {body[:500]}")
    if status == "200":
        result = json.loads(body)
        print(f"OpenSearch fixture cleanup: {result.get('deleted', 0)} documents")


def image_bytes(
    width: int,
    height: int,
    rgb: tuple[int, int, int],
    label: str,
    image_format: str,
) -> bytes:
    try:
        from PIL import Image, ImageDraw
    except ImportError as error:
        raise RuntimeError(
            "Pillow is required to generate gallery fixtures (python3-pillow)"
        ) from error

    image = Image.new("RGB", (width, height), rgb)
    draw = ImageDraw.Draw(image)
    margin = max(12, width // 40)
    draw.rounded_rectangle(
        (margin, margin, width - margin, height - margin),
        radius=max(12, width // 50),
        outline=(230, 235, 240),
        width=max(3, width // 250),
    )
    draw.rectangle(
        (margin * 2, margin * 3, width - margin * 2, height - margin * 3),
        fill=(20, 28, 31),
        outline=(114, 159, 207),
        width=max(2, width // 400),
    )
    draw.text((margin * 3, margin * 4), f"prod_ready_test / {label}", fill="white")
    draw.text(
        (margin * 3, margin * 4 + 28),
        f"{width}x{height}  Linux.org.ru Rust port",
        fill=(138, 226, 52),
    )
    buffer = io.BytesIO()
    save_args = {"quality": 88, "optimize": True} if image_format == "JPEG" else {}
    image.save(buffer, format=image_format, **save_args)
    return buffer.getvalue()


def put_file(path: str, payload: bytes) -> None:
    parent = str(Path(path).parent)
    compose("exec", "-T", "app", "mkdir", "-p", parent)
    compose(
        "exec",
        "-T",
        "app",
        "sh",
        "-c",
        f"umask 022; dd of='{path}' status=none",
        input_bytes=payload,
        capture=True,
    )


def load_media() -> None:
    for image_id, (*rgb_values, label) in IMAGE_IDS.items():
        rgb = tuple(rgb_values)
        original = image_bytes(1600, 900, rgb, label, "PNG")
        put_file(f"/app/uploads/images/{image_id}/original.png", original)
        for width in (500, 1000, 1500, 2000):
            height = width * 9 // 16
            derivative = image_bytes(width, height, rgb, label, "JPEG")
            put_file(f"/app/uploads/images/{image_id}/{width}px.jpg", derivative)

    for user_id, (*rgb_values, initials) in PHOTO_IDS.items():
        rgb = tuple(rgb_values)
        photo = image_bytes(300, 300, rgb, initials, "PNG")
        put_file(f"/app/uploads/photos/{user_id}.png", photo)


def print_summary(accounts_only: bool) -> None:
    if accounts_only:
        print(
            "prod_ready_test loaded: "
            + query("SELECT count(*) FROM users WHERE id BETWEEN 9100001 AND 9100014")
            + " accounts; content must be created by browser_seed.py"
        )
        print("All fixture passwords: Birds-ProdReady-2026")
        return
    summary = query(
        "SELECT "
        "(SELECT count(*) FROM users WHERE id BETWEEN 9100001 AND 9100014)||' users, '||"
        "(SELECT count(*) FROM topics WHERE id BETWEEN 9101001 AND 9101099)||' topics, '||"
        "(SELECT count(*) FROM comments WHERE id BETWEEN 9102001 AND 9102099)||' comments, '||"
        "(SELECT count(*) FROM images WHERE id BETWEEN 9104001 AND 9104099)||' images, '||"
        "(SELECT count(*) FROM polls WHERE id BETWEEN 9103001 AND 9103099)||' polls, '||"
        "(SELECT count(*) FROM reactions_log WHERE topic_id BETWEEN 9101001 AND 9101099)||' reactions'"
    )
    print(f"prod_ready_test loaded: {summary}")
    print("All fixture passwords: Birds-ProdReady-2026")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--confirm",
        help=f"required safety value: {CONFIRMATION}",
    )
    parser.add_argument(
        "--start",
        action="store_true",
        help="start the Compose services before seeding",
    )
    parser.add_argument(
        "--skip-media",
        action="store_true",
        help="load SQL only (gallery UI tests will intentionally fail)",
    )
    parser.add_argument(
        "--accounts-only",
        action="store_true",
        help="insert fixture accounts/settings only; create all content through browser_seed.py",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    confirmation = args.confirm or os.environ.get("PROD_READY_TEST_CONFIRM")
    if confirmation != CONFIRMATION:
        print(
            f"Refusing to mutate data. Pass --confirm {CONFIRMATION}",
            file=sys.stderr,
        )
        return 2
    if args.start:
        compose("up", "-d")
    verify_target()
    load_sql(args.accounts_only)
    reset_search_fixture()
    if not args.skip_media and not args.accounts_only:
        load_media()
    print_summary(args.accounts_only)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, subprocess.CalledProcessError) as error:
        print(f"prod_ready_test seed failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
