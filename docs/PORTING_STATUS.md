# Porting status

This archive is no longer only a hand-written MVP skeleton. It now contains a reproducible inventory of the original Scala/Spring surface and a Rust compatibility surface that can be checked automatically.

## What was done in this iteration

1. **Controller map built** from original Scala annotations with `tools/extract_original_routes.py`.
2. **URL and HTTP methods extracted** into `docs/generated/original_routes.{json,csv}`.
3. **Rust route declarations extracted** into `docs/generated/rust_routes.{json,csv}`.
4. **Route coverage generated** in `docs/ROUTE_COVERAGE.md`.
5. **Original demo DB schema inventory extracted** from `sql/demo.db` into `docs/DB_SCHEMA_ORIGINAL.md` and `docs/generated/original_demo_schema.{json,csv}`.
6. **Legacy DB compatibility migration added**: `db/migrations/0003_legacy_schema_compat.sql`.
7. **Auth/session/security porting scaffold added**: `src/security.rs`; login now uses password verification instead of accepting any non-empty password.
8. **Original DB model inventory added**: `src/models_compat.rs`.
9. **Compatibility smoke tests added**: `compat/endpoints.json`, `compat/test_http_compat.py`.
10. **Service/DAO migration map added**: `docs/SERVICE_PORTING_MAP.md`.

## Route status

Current generated report: `docs/ROUTE_COVERAGE.md`.

The Rust router declares all extracted original endpoint shapes. In v4 the remaining explicit `legacy::not_implemented` routes were replaced with concrete handlers; route coverage is still not the same as production-grade business-rule parity.

## DB status

Current generated report: `docs/SCHEMA_COVERAGE.md`.

The Rust migrations cover the active original demo tables plus additive compatibility tables for later Liquibase-era features: images, memories, persistent logins, reactions, warnings, user remarks, invites, tag synonyms, email-domain blocks and related indexes.

Old `jam_*` tables from the demo dump are marked as dropped upstream because the original Liquibase history removed JamWiki later.

## Not yet production-equivalent

The project still is **not** a full one-to-one functional port. The next work is to replace simplified handlers with exact service-level parity and tighten compatibility tests from coarse status/content-type checks to endpoint-specific assertions.

## v3 continuation

This iteration replaced a large share of `legacy::not_implemented` placeholders with working Rust handlers for low-risk read/redirect and lightweight write flows:

- legacy redirects: `group.jsp`, `group-lastmod.jsp`, `view-section.jsp`, `view-news.jsp`;
- archives: section archives, monthly archives and forum group monthly archives;
- compatibility utilities: `markup/preview`, `check-login`, `yandex-tableau`, `show-comments.jsp`, `show-replies.jsp`;
- topic/comment action forms for delete/undelete, commit/uncommit and moving topics;
- user-facing lists and settings: deleted topics/comments, drafts, favorites, tracked topics, profile edit, settings, remarks;
- social features skeletons: memories, user filters, reactions, votes, tag rename/delete;
- moderation utilities: deleted-comments view, post score update, image-delete flag and userpic removal.


## v4 continuation

This iteration compares the uploaded Java/Scala source with the Rust branch and removes the remaining explicit placeholders:

- `/activate` and `/activate.jsp` now implement activation form, HMAC token verification and session login;
- `/check-login` now matches the original nick-availability AJAX endpoint;
- `/addphoto.jsp` now supports multipart userpic upload with size/dimension checks and `/photos/*` serving;
- `/deregister.jsp` now implements password check, required confirmations, profile cleanup and account blocking;
- admin/moderator routes previously mapped to one broad stub now have concrete compatibility handlers;
- `monthly_stats` migration now matches the original demo table shape.

Remaining gaps are tracked in `docs/FUNCTIONAL_COVERAGE.md` and `docs/FUNCTIONAL_COMPARISON_JAVA_RUST.md`.
