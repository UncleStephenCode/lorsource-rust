# Functional coverage report

This report distinguishes route declaration coverage from functional handler coverage. `ROUTE_COVERAGE.md` answers whether old URLs exist; this file tracks routes still deliberately mapped to placeholders or broad stubs.

Total Rust route declarations: **159**
Routes still mapped to `legacy::not_implemented`: **0**
Routes still mapped to `stub_admin`: **0**

The current router is compiled against Axum 0.8.9 and uses `{parameter}` for
all dynamic paths. The 193-variant Java structural inventory remains at zero
missing declarations after that migration.

## v4 implementation notes

- Replaced the remaining explicit placeholders for `/activate`, `/activate.jsp`, `/addphoto.jsp` and `/deregister.jsp` with real handlers.
- Fixed `/check-login`: it now matches the original registration AJAX endpoint and checks `nick` availability instead of returning current-session state.
- Added HMAC-SHA256 activation code verification compatible with the original `SecretTokenService` formula when `SITE_SECRET` matches the Java secret.
- Added userpic upload with multipart parsing, size/dimension checks and `/photos/*` static serving.
- Added destructive deregistration flow: password check, required confirmations, profile cleanup and user blocking.
- Replaced admin stubs with moderation handlers for GeoIP lookup surface, asynchronous monthly search reindexing, IP bans, group editing, user moderation actions and warnings.
- Fixed `monthly_stats` compatibility migration to match the original demo schema columns.

## Remaining functional gaps

There are no routes left that intentionally return `legacy::not_implemented`, but full production parity still requires endpoint-specific compatibility work for the larger subsystems below:

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
  window/parent-chain are covered statefully. Exhaustive tracker/realtime
  presentation for every uncommon expired edge case remains;
- dual-runtime verification on a clone of the real Java database and media store.

The persistent ActiveMQ search queue is represented by an atomic filesystem
spool under `UPLOAD_DIR/search-queue`; failed jobs survive a process restart
and are retried by the background worker. Java scheduled maintenance jobs are
implemented in `src/bootstrap/background.rs` with PostgreSQL advisory locks so
multiple Rust replicas retain single-scheduler side-effect semantics.

## v5 additions

- Fixed current Java poll compatibility: `polls`, `polls_variants`, `vote_users.variant_id` and POST `/vote.jsp` semantics.
- Added current Java account settings/audit surfaces: `user_settings`, `user_log_action`, `user_log`, and a Rust audit helper.
- Compatibility scripts now run Python tools through `python3`, so the suite works even when archive extraction drops executable bits.
