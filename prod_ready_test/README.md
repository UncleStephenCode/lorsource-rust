# Production-readiness fixture

`prod_ready_test` loads an isolated, deterministic data set into the disposable
Docker Compose database and verifies the Rust port through its public HTTP and
HTML interfaces. It does not test registration: accounts are inserted directly
into the Java/Liquibase-compatible PostgreSQL schema.

There are two deliberately separate modes:

- the complete compatibility fixture uses `seed.sql` for deterministic edge
  cases and `month_scale.sql` for a rolling one-month load of exactly 50
  users, 1000 topics, and 5000 comments;
- browser-seed mode inserts accounts/settings only, then creates every topic,
  comment, image, moderation decision and reaction through the real site forms
  in Chrome via Playwright. No content row is inserted directly by that mode.
- seven-day benchmark mode combines historical SQL fixtures with fresh content
  created exclusively through the browser, then verifies persistence and runs a
  concurrent public-page read benchmark.

## Safety

The seed mutates the Compose database named `lor`. It refuses to run unless:

- the explicit confirmation value is supplied;
- the `postgres` and `app` services are running;
- the target is `lor`, the PostgreSQL role is `postgres`, and Liquibase metadata exists;
- the expected five-section catalog is present.

Fixture users are confined to `9100001..9100050`; the hand-authored and bulk
message/media namespaces are documented in `month_scale.sql`. Repeated runs
delete and recreate only fixture-owned rows and then synchronize affected
sequences. Do not run this against a production database.

## Run

From the repository root:

```bash
docker compose build app
python3 prod_ready_test/run_all.py --start
```

Include browser screenshots (Chrome/Chromium required):

```bash
python3 prod_ready_test/run_all.py --start --visual
```

Run the complete seven-day activity and performance benchmark:

```bash
python3 -m venv /tmp/lorsource-browser-venv
/tmp/lorsource-browser-venv/bin/pip install \
  -r prod_ready_test/requirements-browser.txt
/tmp/lorsource-browser-venv/bin/python prod_ready_test/run_all.py \
  --start --seven-day-benchmark
```

Run the focused browser lifecycle for comments against an already seeded and
running instance:

```bash
/tmp/lorsource-browser-venv/bin/python \
  prod_ready_test/commenting_smoke.py \
  --base http://127.0.0.1:8181
```

It verifies anonymous control visibility, the original inline Reply binding,
Markdown preview, a root comment and nested reply, edit permissions, the
author-delete/no-self-restore rule, moderator delete plus restoration by a
different moderator, and a comment reaction. Generated output is written to
`/tmp/lorsource-commenting-smoke`.

Run the deterministic Java-parity lifecycle for the legacy comment deletion
forms after seeding and rebuilding the current app image:

```bash
python3 prod_ready_test/comment_deletion_lifecycle.py \
  --base http://127.0.0.1:8181
```

This check uses the browser-facing `/delete_comment.jsp` and
`/undelete_comment` forms, validates their PreparedComment DOM, canonical
redirect, additive score/stat changes, `del_info`, the `new_event_t` unread
counter side effect, and exact moderator navigation. Its `finally` block
restores only fixture comment `9102004`, its author counters, its topic
counters/lastmod, and the exact DEL event created by this run, so repeat runs
are state-idempotent.

Run the focused Java-parity lifecycle for topic preview/edit, poll editing,
draft publishing, and moderation commit after seeding and rebuilding the
current app image:

```bash
python3 prod_ready_test/edit_topic_lifecycle.py \
  --base http://127.0.0.1:8181
```

The four reserved topics are installed through SQL, while every tested user
mutation is submitted through `/edit.jsp` (the commit review starts at
`/commit.jsp`). The check covers preview without persistence, the legacy 500
for missing/blank editable titles, edit history/tags/lastmod and mention
notifications, null poll-map preservation, poll variant replacement,
premoderated draft confirmation, and commit with group change plus additive
author bonus. Cleanup removes only its reserved IDs and reverses `topins_t`
group counters/memories, the commit bonus, and `new_event_t` unread deltas, so
the script also repairs a previously interrupted run.

Run the matching Java-parity lifecycle for topic deletion after the isolated
topic-deletion layer has been wired into the legacy routes and the app image
has been rebuilt:

```bash
python3 prod_ready_test/topic_deletion_lifecycle.py \
  --base http://127.0.0.1:8181
```

It exercises Spring-compatible binding failures, the complete delete and
undelete confirmation-page DOM, a cross-moderator score penalty, sticky
clearing, plain `del_info`, canonical `msgdel_t`/`msgundel_t` lastmod changes,
the `new_event_t` unread increment, visible Java-compatible exception pages,
repeat-delete immutability, and the action-done response/link contract.
The test also proves that delete/undelete does not repeat `topins_t` group or
memory effects. Its `finally` block restores the exact topic, author counters,
group counter, memories, and DEL event snapshot, making repeated runs safe on
the disposable fixture.

This mode first runs the deterministic HTTP/DB regression suite, then creates
fresh news, forum topics in all three markup modes, an article, both gallery
forms and both poll forms through Chrome. Multiple users create a three-level
comment thread, react to a topic and a comment, and vote. It creates screenshots
under `/tmp/prod_ready_browser_seed`, a browser result at
`/tmp/prod_ready_browser_seed_result.json`, and the combined performance and DB
report at `/tmp/prod_ready_7d_benchmark.json`. It also
asserts that the author's `/people/{nick}/` feed contains complete cards,
including a pending item, that the gallery section filter is isolated, and
that `/search.jsp?range=COMMENTS&user=...&sort=DATE` contains all comments made
through the browser in newest-first history.

`browser_seed.py` atomically checkpoints browser-observed topic and comment
IDs in `/tmp/prod_ready_browser_seed_checkpoint.json`. Re-running the same
command after a Chrome/process failure validates the stored canonical pages,
reuses matching content, discovers already submitted comments by author/body,
and treats existing reactions and votes as completed. It therefore resumes
without direct content-table writes and without creating a second copy of all
earlier topics. `run_all.py` supplies `--restart` because it always performs a
fresh scoped SQL seed first. The browser run keeps at most two authenticated
contexts alive, refuses to start with less than 1 GiB free on its output
filesystem, and writes crash details to
`/tmp/prod_ready_browser_seed/browser-seed-failure.json`.

To deliberately resume an interrupted standalone run, repeat it without
`--restart`:

```bash
/tmp/lorsource-browser-venv/bin/python prod_ready_test/browser_seed.py \
  --base http://127.0.0.1:8181
```

Use `--browser-seed` only for the narrower account-only/UI-only run. The exact
activity inventory and the explicit registration exclusion are recorded in
`activity_matrix.json`.

### Recorded disposable browser diagnostic (2026-08-15)

A clean browser-seed run and an immediate checkpoint-resume run both passed.
The database tuple before and after resume was unchanged at 10 topics, 5
comments, 2 reactions and 9 votes; peak authenticated contexts were 2. The
result is `/tmp/prod_ready_browser_seed_result.json`.

The seven-day verifier passed and wrote
`/tmp/prod_ready_7d_benchmark_final.json`. It recorded topic buckets for days
0..6 of `13,18,2,16,16,2,17`, section counts
`1:30, 2:41, 3:6, 5:4, 6:3`, single-poll percentages `[50,25,25]` from 4
voters, and multi-poll percentages `[33,100,33]` from 3 voters. Browser timing
covered 56 operations (p50 43.98 ms, p95 2896.99 ms, max 3137.98 ms); the
public-read sample used 57 requests at concurrency 8 (889.15 requests/s, p50
3.88 ms, p95 33.33 ms, max 39.62 ms). The report explicitly records
`registration_tested=false`.

This is a local disposable diagnostic, not production capacity or SLO
evidence. The sample does not reproduce the production proxy/TLS, database,
media, OpenSearch or network topology.

Screenshots are written to `/tmp/prod_ready_test_artifacts`. Individual stages:

```bash
python3 prod_ready_test/seed.py \
  --confirm seed-disposable-compose-lor
python3 prod_ready_test/test_port.py
python3 prod_ready_test/visual_smoke.py
```

The local Compose profile keeps `ENABLE_BACKGROUND_JOBS=false` so tests do not
contact external services. Before asserting forum activity counters,
`test_port.py` therefore invokes the same `stat_update2()` maintenance function
that Java runs after five minutes and then hourly. The HTTP assertion still
checks the persisted `groups.stat3` value used by the original JSP; it does not
replace it with a request-time approximation.

Pillow (`python3-pillow`) is required for deterministic gallery and profile
images. Pass `--skip-media` only when intentionally testing SQL without the
gallery UI assertions.

## Accounts

Every fixture account uses the development-only password:

```text
Birds-ProdReady-2026
```

The first ten ordinary users cover scores `45`, `50`, `70`, `201`, `300`,
`500`, `750`, `1000`, `2000`, and `3000`. Another 36 ordinary `bird15`…
`bird50` accounts provide the volume matrix. The additional role accounts are:

| Login | Role | Relevant flags |
|---|---|---|
| `tern_corrector` | corrector | `corrector=true`, `canmod=false` |
| `ibis_corrector` | corrector | `corrector=true`, `canmod=false` |
| `hawk_moderator` | moderator | `canmod=true`, `candel=false` |
| `eagle_moderator` | senior moderator | `canmod=true`, `candel=true` |

The complete machine-readable account and route list is in `manifest.json`.
Accounts have varied registration dates, cities, profile bodies, profile
markup formats, themes, UI settings, tags, remarks, and ignore-list state. All
50 accounts have deterministic 300×300 local PNG userpics generated by
`seed.py`; the HTTP suite validates every `/photos/{id}.png` response.

## Coverage

The data set includes:

- pending and committed news;
- ordinary, resolved, closed, sticky, and draft forum topics;
- an article with headings, a table, and code;
- single-image and multi-image/slider gallery topics;
- committed multiselect and pending polls, variants, and votes;
- flat and nested comments, edited and deleted comments;
- topic/comment reactions with matching JSON counters and reaction log rows;
- tags, memories, events, profile tags, moderation metadata, and role boundaries;
- Markdown, LORCODE (`BBCODE_TEX`), line-break (`BBCODE_ULB`), and plain profile/content paths.
- collapsed/expanded topic cuts and external-link registrable domains;
- per-letter tag thresholds/actions and all seven saved UI themes.
- at least one topic in every group from every live content section;
- 30-day rolling timestamps recalculated from `CURRENT_TIMESTAMP` on every run;
- populated notifications, tracked topics, deleted topics, and both reaction
  directions for `crane2000`.

`test_port.py` checks database consistency and externally observable behavior:
canonical routes, HTML content types, sanitizer behavior, media delivery,
gallery DOM variants, poll visibility/results, comment nesting, reactions,
closed-topic controls, canonical/OpenGraph metadata, original client scripts,
login return URLs, score thresholds, and corrector/moderator authorization.
The visual smoke matrix covers public pages at desktop and mobile widths; the
HTTP suite additionally verifies authenticated tracker and theme markup, the
four private activity pages, month-scale pagination, and local avatar bytes.

The SQL transaction contains hard assertions for the 50/1000/5000 totals and
for complete group coverage. A partial seed therefore fails and rolls back
instead of producing a misleading benchmark environment.

## Production-source provenance

`source_catalog.json` records the seven-day public-content reference window and
the original section pages used for structural comparison, including the
[`unclestephen` profile](https://www.linux.org.ru/people/unclestephen/profile)
and feed. Titles, timestamps, tags and links identify the compatibility
references; fixture bodies are short deterministic paraphrases and generated
images are synthetic. The test suite therefore exercises the same current
content shapes without copying a production archive into the repository.
The production `proprietary` news item is posted to the fresh Java catalog's
equivalent `commercial` group; `source_catalog.json` records this mapping
explicitly because the production-only group rename is absent from Liquibase.
