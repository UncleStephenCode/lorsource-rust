# Private activity pages and rolling-month fixture

Audit date: 2026-08-15. Runtime: Rust 1.97.1, PostgreSQL 16, port 8181.

## Original route provenance

All requested paths exist in the current Java/Scala source; they are not
Rust-only aliases:

| Path | Original controller | Original view |
|---|---|---|
| `/notifications` | `user/UserEventController.scala` | `show-replies-new.jsp` or `show-replies.jsp` |
| `/people/{nick}/tracked` | `topic/UserTopicListController.scala` | `user-topics.jsp` + `news.tag` |
| `/people/{nick}/deleted-topics` | `topic/UserTopicListController.scala` | `deleted-topics.jsp` + `deleted-topics.tag` |
| `/people/{nick}/reactions[/to]` | `reaction/UserReactionsController.scala` | `user-reactions.jsp` |

Supporting behavior was compared with `UserEventPrepareService`,
`UserEventDao`, `ReactionService`, `ReactionDao`, `TopicListService`, and
`TopicListDao`. The source trees, not screenshots alone, define authorization,
pagination, grouping, query ordering, and link construction.

## Implemented behavior

- All four page families render through `base.html`; no endpoint returns a bare
  body fragment anymore.
- Every private user page allows only the owner or a moderator. Anonymous and
  unrelated ordinary users receive 403.
- `tracked` uses the original fixed 20-item paging, clamped offset, memory-ID
  ordering, profile link, complete `news_card.html` entries, and original
  previous/next rules.
- `deleted-topics` uses the viewer's `topics` setting as its limit and renders
  title, deleter, reason, penalty, creation time, and deletion time in the
  original four-column table.
- reactions preserve the 50+1 look-ahead query, `/to` mode, deleted-content
  visibility for moderators, target author, 250-character preview, canonical
  topic/comment link, and original two-button DOM.
- notifications expose only event filters present for the user, group watched
  and reaction events, preserve click/reset behavior, emit `Cache-Control:
  no-store`, and include RSS/Atom discovery and the visible RSS link.

## Fixture layout

`prod_ready_test/seed.sql` contains the small named compatibility scenarios.
`prod_ready_test/month_scale.sql` adds the rolling load in a separate
transaction:

- users: `9100001..9100050` (50);
- topics: 18 named plus 982 generated (1000 total);
- comments: 18 named plus 4982 generated (5000 total);
- current-to-minus-30-days topic/comment dates on every seed run;
- every live group in sections News, Forum, Gallery, Polls, and Articles;
- polls/variants/votes, screenshots, workplaces, tags, deletions, memories,
  reactions, and every notification filter type;
- sequence synchronization after all explicit IDs.

`seed.py` generates 50 deterministic PNG userpics, including landscape,
portrait, already-small, and square dimension cases, plus representative
1600×900 gallery originals and 500/1000/1500/2000 px derivatives. The files
are placed in the normal uploads volume and are served by the same protected
media handlers as uploaded production content. The regression suite verifies
viewer-level topic/comment suppression, unconditional profile rendering,
public active photos for blocked owners, and historical-photo redirects.

### Avatar display source map

| Java source | Rust surface | `misteryMan` | `showPhotos` gate |
|---|---|---:|---:|
| `TopicPrepareService` | root topic and edit preview | `true` | viewer |
| `CommentPrepareService` | comments, reaction and deletion previews | `false` | viewer |
| `WhoisController` | `/people/{nick}/profile` | `true` | none |
| `UserService.getRecentUserpics` | moderator tracker | `false` | none; drops `DisabledUserpic` |

All four paths share the same filesystem-aware resolver. It validates a local
PNG/JPEG/GIF using the filename-selected decoder, preserves `ImageInfo.scale`
aspect ratios, and falls back to the viewer-selected Gravatar mode or the 1×1
`/img/p.gif`. Gravatar URLs are never network-probed server-side.
Compose stores these files in the `lor_uploads` volume mounted at
`/app/uploads`; `seed.py` writes the normal `/app/uploads/photos/{id}.png`
paths used by the production media handler.

The scale transaction raises an exception unless it ends with exactly 50
users, 1000 topics, 5000 comments, and zero uncovered groups.

## Reproduction

```bash
docker compose build app
docker compose up -d
python3 prod_ready_test/seed.py \
  --confirm seed-disposable-compose-lor
python3 prod_ready_test/test_port.py --base http://127.0.0.1:8181
```

All fixture passwords are `Birds-ProdReady-2026`. Use `crane2000` for the four
private pages, `hawk_moderator` for cross-user access, and `raven1000` to verify
that an ordinary user cannot inspect another user's private activity.

## Recorded validation

- Docker release and quality targets: passed, including repository-wide
  rustfmt, all-target/all-feature check, 457 passing tests, 2 explicitly
  ignored tests and Clippy with warnings denied.
- The rebuilt application passed `/healthz`; edit-topic, comment-deletion and
  topic-deletion lifecycle scripts completed with scoped cleanup.
- `prod_ready_test/test_port.py`: 26 groups passed, 0 failed; destructive
  lifecycle scenarios restored their owned state in `finally`.
- Browser seed passed from a clean checkpoint and on immediate resume. The
  before/after database tuple stayed exactly 10 topics, 5 comments, 2
  reactions and 9 votes, with a peak of 2 authenticated contexts. Result:
  `/tmp/prod_ready_browser_seed_result.json`.
- The 168-hour diagnostic passed with topic day buckets
  `13,18,2,16,16,2,17` and section counts
  `1:30, 2:41, 3:6, 5:4, 6:3`. Its single-choice poll had 4 voters and
  percentages `[50,25,25]`; its multi-choice poll had 3 voters and
  percentages `[33,100,33]`.
- Browser activity timing covered 56 operations (p50 43.98 ms, p95 2896.99
  ms, max 3137.98 ms). The separate public-read sample covered 57 requests at
  concurrency 8 (889.15 requests/s, p50 3.88 ms, p95 33.33 ms, max 39.62 ms).
  The report is `/tmp/prod_ready_7d_benchmark_final.json` and records
  `registration_tested=false`.
- Java↔Rust HTTP comparison: 126/126 declared scenarios passed against pinned
  Java SHA `2ddf930005adac28077cb6ad74d1481485f44096`, including exact legacy
  status, `Allow`, content-length and `Location` cases. The report SHA-256 is
  `add67c20ebdf56d12145b13668f1adc8a55e7191d12b8707e27dfeb0589547b1`.
- Runtime database startup: all 605 required schema-object contract records
  matched the canonical Java/Liquibase catalog.
- Headless Chromium HTTP preflight and all 52 desktop/mobile captures passed.
  They remain useful for manual avatar/layout inspection, but are not claimed
  as a Java↔Rust perceptual diff.
- Fixture result: 50 users, 1000 topics, 5000 comments, 42 images, 20 polls,
  226 reaction-log rows.
- All 50 avatar URLs returned HTTP 200, `image/png`, and a valid PNG signature.
- Both pages of `tracked`, non-empty deleted-topic table, both reaction modes,
  all notification filters, anonymous denial, owner access, moderator access,
  and ordinary cross-user denial were exercised over HTTP.
- Current runtime checks additionally cover database-aware LORCODE/Markdown
  user references, Java-compatible main-page ordering, archive counts derived
  from the original query, and transactional image deletion with history.

The browser and read timings above are diagnostics of the local disposable
Compose stack, not production capacity or SLO evidence. They do not replace a
production-clone/load rehearsal with the real proxy, storage, search and
network topology.

## Continuation notes

When adding another compatibility scenario, keep the first 18 named records
stable and extend the generated layer without changing the exact 1000/5000
contract. If counts change deliberately, update the SQL assertions,
`manifest.json`, `test_port.py`, and this document together. Never remove the
ownership-based cleanup or sequence synchronization: browser-created rows may
use normal sequences while still belonging to fixture accounts.
