# Functional coverage report

This report distinguishes route declaration coverage from functional handler coverage. `ROUTE_COVERAGE.md` answers whether old URLs exist; this file tracks routes still deliberately mapped to placeholders or broad stubs.

The route declaration total is generated from the current tree and is not
duplicated here. See `docs/generated/rust_routes.json` and the current CI
artifact instead of relying on a hand-maintained number.
Routes still mapped to `legacy::not_implemented`: **0**
Routes still mapped to `stub_admin`: **0**

The current router uses `{parameter}` for all dynamic paths. Structural
coverage is recalculated in CI; it does not establish behavioral parity.

## v4 implementation notes

- Replaced the remaining explicit placeholders for `/activate`, `/activate.jsp`, `/addphoto.jsp` and `/deregister.jsp` with real handlers.
- Fixed `/check-login`: it now matches the original registration AJAX endpoint and checks `nick` availability instead of returning current-session state.
- Added HMAC-SHA256 activation code verification compatible with the original `SecretTokenService` formula when `SITE_SECRET` matches the Java secret.
- Added userpic upload with multipart parsing, size/dimension checks and the
  original `<userid>:<signed-int>.<extension>` filename contract. `/photos/*`
  is served through the Java `UserpicPermissionInterceptor` policy rather than
  an unrestricted static directory: active files are public, historical files
  are owner/moderator-only, and other viewers receive the original 302 target.
- `/images/*` and `/gallery/preview/*` are permission-aware. Finalized images
  use `TopicPermissionService.checkView` plus edit-history permission for
  deleted images; previews require an authenticated session. Direct URLs can
  no longer bypass deleted/draft/topic visibility rules.
- `/adv/**` responses in the Java 2xx/3xx window are accumulated by path and
  transactionally flushed once per minute to canonical `adv_counts`; failed
  flushes are merged back into the live batch and graceful shutdown performs
  a final flush.
- Authenticated session resolution and `lastlogin` refresh now run at the
  application boundary like `LastLoginInterceptor`, including MVC/static
  handlers which do not otherwise extract `CurrentUser`; the resolved user is
  cached in request extensions for downstream handlers.
- Static `Cache-Control` follows the current `urlrewrite.xml`: one-hour
  CSS/JS/font caching, ten-year cachebusters/OpenSans/webjars/images, the
  original double jquery header and `no-cache` advertisements. Protected
  uploaded media retain Spring's separate `31556926`-second policy.
- Successful static paths excluded by the current Spring Security XML bypass
  session hydration and CSRF-cookie creation, while missing resources and
  non-excluded `manifest.json`, `robots.txt` and `qrerror` assets retain the
  Java security-chain behavior. The generated `lor.js`, `plugins.js` and
  `diff_match_patch.js` bundles are synchronized from the original build.
- The base page uses the original `$script` dependency chain for jQuery,
  `lor.js`, `plugins.js`, highlighting and realtime. Request timezone parsing
  rejects the same bad ZoneIds as `CommonContextFilter`, falls back to the
  runtime system timezone, renders the Java `date`/`dateinterval` contracts on
  the server and initializes the original `fixTimezone` browser correction.
  The neutral `<time>` contract covers topics/comments, profile dates and
  statistics, moderation and edit history, deleted content, reactions,
  notifications, same-IP results and OpenSearch result signatures; interval
  versus compact-interval modes follow the corresponding Java tags.
- Topic/comment edit history reconstructs the current-to-original sequence
  from type-scoped `edit_info` records, renders the original `.messages/.msg`
  DOM, loads the byte-identical diff controller and supports the original
  `/edit.jsp?msgid=...&fromHistory=...` text restore contract. Topic history
  uses `canViewHistory`; comment history retains Java's distinct anonymous
  visibility after the topic-level view check.
- Canonical-host filtering follows the remaining Tuckey rules: unknown hosts
  receive an absolute 302 to `https://www.linux.org.ru`, plain-HTTP `www` is
  upgraded, development/beta hosts remain accepted, and the historical
  `stoplinux.org.ru` redirect is preserved. Path rewrites still run first.
- Added destructive deregistration flow: password check, required confirmations, profile cleanup and user blocking.
- Replaced admin stubs with moderation handlers for GeoIP lookup surface, asynchronous monthly search reindexing, IP bans, group editing, user moderation actions and warnings.
- Fixed `monthly_stats` compatibility migration to match the original demo schema columns.

## Remaining production-evidence gaps

There are no routes left that intentionally return `legacy::not_implemented`
and the current Java-source audit found no whole user-facing subsystem that is
absent from the Rust port. Full production parity is still not proved: the
remaining work below requires production-clone, storage, network or load
evidence rather than another placeholder implementation.

- live verification of gallery preview/reuse/three-day cleanup against the
  production storage/CDN mount;
- production-egress verification of `ipwho.is` (its isolated success/API
  error/non-2xx/parse adapter tests pass), SMTP exception reporting,
  disposable-domain/TOR feeds (their isolated 2xx/non-2xx adapter tests pass)
  and Telegram publishing (direct→proxy and token-redaction tests pass; a real
  channel/token rehearsal remains);
- moderator profile/userpic cleanup, score50, corrector, freeze/defrost,
  password reset, block/unblock and destructive mass-delete now have a guarded
  HTTP+database transaction/audit regression, including reply-preserving
  deletion order, `del_info`, event cleanup and unread-counter recalculation;
- topic/comment warning creation and corrector clearing now use the original
  bean fields, roles, localized active-recipient events, five-per-hour limit,
  canonical redirects and an atomic warning/event/counter transaction; the
  stateful regression covers their canonical tables;
- notification presentation now carries the Java warning/delete payload,
  closed-warning strikeout, delete bonus, author, section and title tags;
  DEL click-through and the owner/moderator `/view-deleted?id=` permission
  window/parent-chain are covered statefully. Notification RSS/Atom now carries
  the rendered message body, comment author, stable event id and reaction note
  used by `UserEventFeedView`;
- tracker parity now uses the profile `trackerMode`, `topics`, `messages` and
  `oldTracker` values, reproduces `/tracker.jsp`'s exact 302/default-filter
  redirect, last-visible-comment author/date/link, hidden-comment count,
  closed/uncommitted markers and both original DOM modes. The realtime source
  audit confirmed that Java deliberately permits subscriptions to existing
  deleted/draft/expired topics, filters deleted missed comments, suppresses
  `POSTSCORE_HIDE_COMMENTS`, and applies the branch ignore list; Rust matches
  those semantics. Production-clone event timing/load evidence remains;
- dual-runtime verification on a clone of the real Java database and media store.

The persistent ActiveMQ search queue is represented by an atomic filesystem
spool under `UPLOAD_DIR/search-queue`; failed jobs survive a process restart
and are retried by the background worker. Java scheduled maintenance jobs use
a single FIFO execution gate inside `src/bootstrap/background.rs` plus per-job
PostgreSQL advisory locks. Exactly one replica must have scheduled jobs
enabled: the locks prevent overlapping executions, but cannot suppress a later
sequential execution by another replica.

## v5 additions

- Fixed current Java poll compatibility: `polls`, `polls_variants`, `vote_users.variant_id` and POST `/vote.jsp` semantics.
- Added current Java account settings/audit surfaces: `user_settings`, `user_log_action`, `user_log`, and a Rust audit helper.
- Compatibility scripts now run Python tools through `python3`, so the suite works even when archive extraction drops executable bits.
