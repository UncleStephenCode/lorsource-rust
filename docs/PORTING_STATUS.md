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

The Rust router now declares all extracted original endpoint shapes. Important nuance: many legacy endpoints intentionally return **501 Not Implemented**. That is still useful because accidental 404 is gone and the porting backlog is explicit.

## DB status

Current generated report: `docs/SCHEMA_COVERAGE.md`.

The Rust migrations cover the active original demo tables plus additive compatibility tables for later Liquibase-era features: images, memories, persistent logins, reactions, warnings, user remarks, invites, tag synonyms, email-domain blocks and related indexes.

Old `jam_*` tables from the demo dump are marked as dropped upstream because the original Liquibase history removed JamWiki later.

## Not yet production-equivalent

The project still is **not** a full one-to-one functional port. The next work is to replace each `legacy::not_implemented` route with real service code and then tighten compatibility tests from coarse status/content-type checks to endpoint-specific assertions.

## v3 continuation

This iteration replaced a large share of `legacy::not_implemented` placeholders with working Rust handlers for low-risk read/redirect and lightweight write flows:

- legacy redirects: `group.jsp`, `group-lastmod.jsp`, `view-section.jsp`, `view-news.jsp`;
- archives: section archives, monthly archives and forum group monthly archives;
- compatibility utilities: `markup/preview`, `check-login`, `yandex-tableau`, `show-comments.jsp`, `show-replies.jsp`;
- topic/comment action forms for delete/undelete, commit/uncommit and moving topics;
- user-facing lists and settings: deleted topics/comments, drafts, favorites, tracked topics, profile edit, settings, remarks;
- social features skeletons: memories, user filters, reactions, votes, tag rename/delete;
- moderation utilities: deleted-comments view, post score update, image-delete flag and userpic removal.

Remaining placeholders are documented in `docs/FUNCTIONAL_COVERAGE.md`. They are left explicit because they require larger subsystems: activation tokens, photo upload/storage and account deregistration policy.
