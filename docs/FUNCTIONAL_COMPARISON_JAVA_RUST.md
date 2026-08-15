# Functional comparison: original Java/Scala lorsource vs Rust port

## Scope

The original archive is the current lorsource Scala/Spring MVC codebase with JSP views, PostgreSQL DAOs, Spring Security, mail, captcha, image processing, search and moderation services. The Rust archive is the Axum/SQLx/Askama porting branch.

## Automated inventory

This document spans several historical port iterations, so fixed route and
test totals are intentionally not duplicated here. The current normalized
path/method inventory is [`ROUTE_COVERAGE.md`](ROUTE_COVERAGE.md), generated
from both source trees by `scripts/run-compatibility-suite.sh`. It reports
declared-method and partial/unrestricted-method coverage separately; neither a
zero-missing result nor equal route counts prove parameter, authorization,
HTML, database or side-effect parity.

The v4 database notes below refer to the then-active demo-schema comparison.
Current Java/Liquibase database evidence is tracked separately in
[`DATABASE_COMPATIBILITY.md`](DATABASE_COMPATIBILITY.md) and must be validated
against a named production clone before cutover.

Generated files:

- `docs/generated/original_routes.json`
- `docs/generated/rust_routes.json`
- `docs/generated/route_coverage.json`
- `docs/generated/schema_coverage.json`

## v4 parity improvements

### Registration activation

Original: `RegisterController` exposes `GET/POST /activate` and `/activate.jsp`, verifies activation codes with `SecretTokenService`, activates the user and logs them in.

Rust v4: `/activate` and `/activate.jsp` now render an activation form, verify HMAC-SHA256 activation codes using the same payload shape `nick:email:regdateMillis:activate`, activate the user and set a signed session cookie. For dev bootstrap there is also `dev-activate`.

### Login availability AJAX

Original: `/check-login?nick=...` validates nick syntax, maximum length and duplicate names.

Rust v4: `/check-login` now implements the same semantic check and returns a JSON string result. The previous implementation incorrectly returned current-session state.

### Userpic upload

Original: `UserpicController` accepts multipart `/addphoto.jsp`, checks file size and dimensions, stores a generated photo name and updates `users.photo`.

Rust: `/addphoto.jsp` accepts multipart upload, validates PNG/JPEG/WEBP size
and 50–300 px dimensions, enforces the complete `checkLoadUserpic` policy
(freeze, score, hourly limit, moderator reset and recent score loss), and
atomically updates `users.photo` with the canonical `set_userpic` audit row.
New names match Java's `<userid>:<signed-int>.<extension>` contract. The
permission-aware `/photos/*` handler implements active/historical userpic
visibility and the original 302 fallback behavior.

Finalized `/images/*` and temporary `/gallery/preview/*` files are no longer
mounted as unrestricted static directories. They implement
`GalleryPermissionInterceptor`: preview access requires authentication, while
finalized files inherit topic visibility and deleted-image history rules.

Static delivery reproduces the current Tuckey/Spring cache contract rather
than inheriting the dynamic-page `private` default: theme and ordinary assets
use the original one-hour/ten-year split, `/webjars` and OpenSans use ten
years, advertisements use `no-cache`, and uploaded media retain their separate
`31556926`-second resource-handler value. The Java/Rust differential matrix
asserts the exact headers, including the historical queried-PNG regex edge.
Successful resources excluded by Spring Security do not hydrate a user or
create a CSRF cookie; secured top-level and `qrerror` resources retain the
dynamic chain. Generated browser bundles, manifest/robots/verification files
and reverse-proxy error assets are served on their original paths.

The HTML head loads the synchronized `lor.js` and `plugins.js` bundles after
jQuery, as the original JSP does. The shared request-timezone implementation
validates `tz`, formats `default`, `date`, `interval` and `compact-interval`
time elements using the Java rules (including historical offsets), and passes
the resolved server ZoneId to `fixTimezone`; this also removed four divergent
route-local timezone parsers. All currently identified browser-facing raw date
surfaces now use this contract, including profiles/statistics, edit and user
logs, deleted content, reactions, notifications, same-IP moderation results and
OpenSearch results.

Topic and comment edit-history pages now reconstruct changes from
`edit_info` by object type, retain the original topic/comment access-policy
difference, render the JSP-compatible `.messages/.msg` structure, and load the
original diff controller. Authenticated dual-runtime probes also verify the
same number of current/original versions and identical `fromHistory` form
prefill behavior.

### Deregistration

Original: `DeregisterController` allows only non-moderator users with `max_score >= 100`, checks password, requires both confirmations, clears profile fields and blocks the user.

Rust v4: `/deregister.jsp` implements the same high-level policy: permission gate, password check, confirmation checks, profile cleanup, `blocked=true` and session removal.

### Admin/moderation surface

Original: admin and moderation controllers cover GeoIP, search reindex, IP bans, group editing, user moderation and warnings.

Rust: previous broad admin stubs were replaced by concrete handlers that operate on the canonical tables (`b_ips`, `ban_info`, `groups`, `users`, `message_warnings`). Search reindex runs the Java-compatible current-three-month or full month sequence, while normal writes use an atomic persistent spool with retry after restart. Real GeoIP remains an adapter point requiring live verification.

The browser search now mirrors `SearchController`/`SearchService`: legacy enum
ids, user and topic-author filters, timezone-aware selected dates, exact
interval bounds, recency function-score, fast-vector highlighting, significant
tag terms, section/group post-filters and original facet-selection behavior.
Indexed bodies are rendered and sanitized HTML selected from `msgbase.markup`,
rather than a plain-text approximation. Topic pages also execute the original
two-field `MoreLikeThisService` query (title, indexed body and optional tags),
render the two-column `related-topics` block and retain Java's one-hour cache
and 500 ms page deadline. Tag pages use the original topic-only
`significant_terms` aggregation for «См. также» with the same 500 ms limit,
plus effective-date grouping, spacer-aware two-column partitioning and the
original news/forum freshness ordering. Forum group pages select the last
visible non-ignored branch comment and honor ignored tags with favorite-tag
override. Moderator tracker pages include the original recent user/IP and
userpic operational lists.
Production startup rejects an incompatible existing `messages` mapping instead
of deferring the failure to the first search request.


## v5 current Java-source parity fixes

The uploaded Java/Scala archive is newer than the historic demo dump in the poll/settings/audit areas. The Rust port now carries an additional migration and handler updates for those differences:

- current poll tables `polls` and `polls_variants` are created and populated from old `votenames/votes` when an old dump is imported;
- `vote_users.variant_id` is added and old rows are rewritten from variant-id semantics to current poll-id + variant-id semantics;
- POST `/vote.jsp` now accepts the same form shape as `VoteController`: `voteid=<poll id>` plus one or more `vote=<variant id>` values;
- `user_settings` and PostgreSQL `hstore` are added for the current settings model;
- `user_log_action` and `user_log` are added for `UserLogDao` compatibility;
- basic account/moderation actions now write audit records through `src/audit.rs`;
- the compatibility shell script no longer depends on executable bits on Python tools.

See `docs/CURRENT_JAVA_COMPATIBILITY.md` for the detailed finding list.

## Still not production-equivalent

The port now has URL coverage and no explicit 501 placeholders. Captcha, topic
and comment flood checks, IP blocks/slow mode, Java remember-me cookies, the
three SMTP account flows, OpenSearch indexing, WebSocket delivery, reaction
visibility and the principal notification writes are implemented and tested.
The browser notification view now also resolves reactions against the current
topic/comment JSONB state, omits removed reactions, groups reaction/WATCH
events like `UserEventPrepareService`, honors the `oldNotifications` profile
key and submits grouped rows through `/notifications-click`.

Production equivalence is still not proven for:

- gallery preview/reuse/cleanup is implemented; production storage/CDN
  deployment still requires rehearsal;
- production-egress verification of the Java-compatible `ipwho.is` GeoIP
  adapter (isolated success/API-error/non-2xx/parse tests pass), administrator
  exception-mail and scheduled Telegram adapter; TOR/disposable feeds have
  isolated 2xx/non-2xx coverage but still require production-egress rehearsal;
- user-moderation audit/score transactions and destructive mass-delete are
  covered by a guarded HTTP+database regression, including reply-preserving
  order, `del_info`, event cleanup and unread-counter recalculation; warning
  creation/clearing, active moderator/corrector events, counters and rate limit
  are covered in the same flow. Warning/delete payload rendering, closed
  strikeout, delete bonus, DEL click-through and the original recent-author
  `/view-deleted?id=` path are also covered. Tracker now renders activity from
  the same last visible comment as `GroupListDao`, uses profile pagination and
  default filters, emits Java's legacy redirect, hides counts for
  `POSTSCORE_HIDE_COMMENTS`, and supports both `oldTracker` DOM variants.
  `RealtimeEventHub`, `TopicDao` and `CommentReadService` were audited for
  deleted/draft/expired/hidden topics and missed-comment/ignore-list behavior;
  the Rust repository/service contract matches them. Production-clone
  timing/load evidence is still required;
- every JSP model attribute and pixel-level theme/page combination remains an
  exhaustive diagnostic target; all seven theme IDs, their server-selected
  stylesheet/header DOM and core layout hooks are covered statefully;
- the isolated demo dual-runtime HTTP matrix passes, including canonical
  paths, legacy redirects, comment jumps, RSS and an initialized OpenSearch;
  a migration rehearsal on a clone of the real production Java database and
  uploaded-media store is still mandatory.

Use this file together with `docs/SERVICE_PORTING_MAP.md` to continue service-by-service replacement of simplified Rust handlers with full parity implementations.
