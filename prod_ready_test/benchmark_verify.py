#!/usr/bin/env python3
"""Verify the seven-day fixture in PostgreSQL and benchmark public HTML reads."""

from __future__ import annotations

import argparse
import json
import math
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from urllib.request import Request, urlopen


HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
CONTENT = json.loads((HERE / "browser_content.json").read_text("utf-8"))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def query(sql: str) -> list[list[str]]:
    result = subprocess.run(
        [
            "docker",
            "compose",
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
            "-F",
            "\t",
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            sql,
        ],
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return [line.split("\t") for line in result.stdout.splitlines() if line]


def percentile(values: list[float], fraction: float) -> float:
    require(bool(values), "cannot calculate a percentile for an empty sample")
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, math.ceil(len(ordered) * fraction) - 1)]


def verify_database(browser: dict[str, object]) -> dict[str, object]:
    day_rows = query(
        "SELECT floor(extract(epoch FROM (CURRENT_TIMESTAMP-t.postdate))/86400)::integer AS day,"
        "count(*) FROM topics t WHERE t.userid BETWEEN 9100001 AND 9100014 "
        "AND t.postdate BETWEEN CURRENT_TIMESTAMP-interval '7 days' AND CURRENT_TIMESTAMP "
        "GROUP BY day ORDER BY day"
    )
    day_counts = {int(day): int(count) for day, count in day_rows}
    require(set(range(7)).issubset(day_counts), f"seven-day topic depth is incomplete: {day_counts}")

    section_rows = query(
        "SELECT g.section,count(*) FROM topics t JOIN groups g ON g.id=t.groupid "
        "WHERE t.userid BETWEEN 9100001 AND 9100014 "
        "AND t.postdate >= CURRENT_TIMESTAMP-interval '7 days' "
        "GROUP BY g.section ORDER BY g.section"
    )
    section_counts = {int(section): int(count) for section, count in section_rows}
    require(
        {1, 2, 3, 5, 6}.issubset(section_counts),
        f"not every content section is represented: {section_counts}",
    )

    topics = {str(item["key"]): item for item in browser["topics"]}
    expected_markup = {
        "forum-markdown": "MARKDOWN",
        "forum-lorcode": "BBCODE_TEX",
        "forum-linebreak": "BBCODE_ULB",
    }
    topic_ids = ",".join(str(topic["id"]) for topic in topics.values())
    markup_rows = query(
        f"SELECT m.id,m.markup::text FROM msgbase m WHERE m.id IN ({topic_ids}) ORDER BY m.id"
    )
    markup_by_id = {int(topic_id): markup for topic_id, markup in markup_rows}
    for key, markup in expected_markup.items():
        require(
            markup_by_id.get(int(topics[key]["id"])) == markup,
            f"{key}: stored markup differs from {markup}",
        )

    comments = {str(item["body"]): item for item in browser["comments"]}
    comment_ids = ",".join(str(comment["id"]) for comment in comments.values())
    comment_rows = query(
        f"SELECT c.id,c.topic,COALESCE(c.replyto,0),c.reactions::text "
        f"FROM comments c WHERE c.id IN ({comment_ids}) ORDER BY c.id"
    )
    require(len(comment_rows) == len(comments), "not every browser-created comment was persisted")
    comment_by_id = {int(row[0]): row for row in comment_rows}
    for comment in comments.values():
        row = comment_by_id[int(comment["id"])]
        require(int(row[1]) == int(topics[str(comment["topic"])]["id"]), "comment topic differs")
        require(int(row[2]) == int(comment["reply_to"] or 0), "comment reply target differs")

    root = next(item for item in comments.values() if item["body"].startswith("Корневой"))
    reaction_rows = query(
        "SELECT topic_id,COALESCE(comment_id,0),reaction,count(*) FROM reactions_log "
        f"WHERE topic_id IN ({int(topics['esp32-news']['id'])},{int(topics['forum-markdown']['id'])}) "
        "GROUP BY topic_id,comment_id,reaction ORDER BY topic_id,comment_id,reaction"
    )
    reaction_set = {(int(topic), int(comment), reaction, int(count)) for topic, comment, reaction, count in reaction_rows}
    require(
        (int(topics["esp32-news"]["id"]), 0, "👍", 1) in reaction_set,
        "topic reaction log is incomplete",
    )
    require(
        (int(topics["forum-markdown"]["id"]), int(root["id"]), "🎉", 1)
        in reaction_set,
        "comment reaction log is incomplete",
    )

    poll_report: dict[str, object] = {}
    for key, actors in CONTENT["poll_votes"].items():
        topic = topics[key]
        poll_header = query(
            f"SELECT p.id,p.multiselect,count(DISTINCT vu.userid) "
            f"FROM polls p LEFT JOIN vote_users vu ON vu.vote=p.id WHERE p.topic={int(topic['id'])} "
            "GROUP BY p.id,p.multiselect"
        )
        require(len(poll_header) == 1, f"{key}: poll row is absent")
        poll_id, multiselect, people = poll_header[0]
        variant_rows = query(
            f"SELECT pv.label,pv.votes,count(vu.userid) FROM polls_variants pv "
            f"LEFT JOIN vote_users vu ON vu.variant_id=pv.id WHERE pv.vote={int(poll_id)} "
            "GROUP BY pv.id,pv.label,pv.votes ORDER BY pv.id"
        )
        expected_votes = [0] * len(topic["poll_variants"])
        for choices in actors.values():
            for index in choices:
                expected_votes[int(index)] += 1
        actual_votes = [int(row[1]) for row in variant_rows]
        logged_votes = [int(row[2]) for row in variant_rows]
        require(actual_votes == expected_votes, f"{key}: stored poll counters differ")
        require(logged_votes == expected_votes, f"{key}: vote log and counters differ")
        require(int(people) == len(actors), f"{key}: unique voter count differs")
        divisor = len(actors) if multiselect == "t" else sum(expected_votes)
        percentages = [round(100 * count / divisor) if divisor else 0 for count in expected_votes]
        require(max(percentages, default=0) <= 100, f"{key}: impossible percentage")
        poll_report[key] = {
            "multiselect": multiselect == "t",
            "votes": actual_votes,
            "people": int(people),
            "percentages": percentages,
        }

    return {
        "topic_day_buckets": day_counts,
        "section_counts": section_counts,
        "created_markups": expected_markup,
        "created_comments": len(comment_rows),
        "created_reactions": 2,
        "polls": poll_report,
    }


def fetch(base: str, path: str) -> tuple[str, float, int]:
    started = time.perf_counter()
    request = Request(base.rstrip("/") + path, headers={"User-Agent": "prod-ready-benchmark/1"})
    with urlopen(request, timeout=15) as response:
        payload = response.read()
        status = response.status
        content_type = response.headers.get("Content-Type", "")
    duration_ms = (time.perf_counter() - started) * 1000
    require(status == 200, f"benchmark GET {path}: HTTP {status}")
    require("text/html" in content_type, f"benchmark GET {path}: not HTML")
    require(bool(payload), f"benchmark GET {path}: empty response")
    return path, duration_ms, len(payload)


def benchmark_reads(base: str, browser: dict[str, object]) -> dict[str, object]:
    paths = [
        "/",
        "/news/",
        "/forum/",
        "/gallery/",
        "/polls/",
        "/articles/",
        "/tracker/",
        "/gallery/archive/",
        "/people/raven1000/",
        "/search.jsp?range=COMMENTS&user=crane2000&sort=DATE",
        *(str(topic["url"]) for topic in browser["topics"] if topic["moderation"] != "pending"),
    ]
    jobs = paths * 3
    started = time.perf_counter()
    results: list[tuple[str, float, int]] = []
    with ThreadPoolExecutor(max_workers=8) as pool:
        futures = [pool.submit(fetch, base, path) for path in jobs]
        for future in as_completed(futures):
            results.append(future.result())
    elapsed = time.perf_counter() - started
    durations = [duration for _, duration, _ in results]
    require(percentile(durations, 0.95) < 10_000, "public GET p95 exceeded 10 seconds")
    return {
        "requests": len(results),
        "concurrency": 8,
        "elapsed_seconds": round(elapsed, 3),
        "requests_per_second": round(len(results) / elapsed, 2),
        "p50_ms": round(percentile(durations, 0.50), 2),
        "p95_ms": round(percentile(durations, 0.95), 2),
        "max_ms": round(max(durations), 2),
        "bytes": sum(size for _, _, size in results),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:8181")
    parser.add_argument(
        "--browser-result",
        type=Path,
        default=Path("/tmp/prod_ready_browser_seed_result.json"),
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("/tmp/prod_ready_7d_benchmark.json"),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    browser = json.loads(args.browser_result.read_text("utf-8"))
    report = {
        "window_hours": 168,
        "registration_tested": False,
        "database": verify_database(browser),
        "browser": browser["benchmark"],
        "public_read_load": benchmark_reads(args.base, browser),
    }
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", "utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    print(f"benchmark report: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
