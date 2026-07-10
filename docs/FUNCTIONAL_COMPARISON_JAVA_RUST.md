# Functional comparison: original Java/Scala lorsource vs Rust port

## Scope

The original archive is the current lorsource Scala/Spring MVC codebase with JSP views, PostgreSQL DAOs, Spring Security, mail, captcha, image processing, search and moderation services. The Rust archive is the Axum/SQLx/Askama porting branch.

## Automated inventory

| Area | Original | Rust port v4 | Status |
|---|---:|---:|---|
| Extracted Spring endpoint shapes | 184 | — | Source of truth |
| Axum route declarations | — | 146 | Some Axum routes cover several Spring handlers via method/parameter aggregation |
| Route coverage | 184/184 | 184/184 | Covered by declaration |
| `legacy::not_implemented` routes | — | 0 | Removed in v4 |
| Active original demo tables | 20 | 20 | Covered by migrations |
| Original `monthly_stats` columns | 5 | 5 | Fixed in v4 |

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

Rust v4: `/addphoto.jsp` now accepts multipart upload, validates PNG/JPEG/WEBP size and 50–300 px dimensions, stores files under `UPLOAD_DIR/photos`, serves them as `/photos/*` and updates `users.photo`.

### Deregistration

Original: `DeregisterController` allows only non-moderator users with `max_score >= 100`, checks password, requires both confirmations, clears profile fields and blocks the user.

Rust v4: `/deregister.jsp` implements the same high-level policy: permission gate, password check, confirmation checks, profile cleanup, `blocked=true` and session removal.

### Admin/moderation surface

Original: admin and moderation controllers cover GeoIP, search reindex, IP bans, group editing, user moderation and warnings.

Rust v4: previous broad admin stubs were replaced by concrete handlers that operate on the compatibility tables (`b_ips`, `ban_info`, `groups`, `users`, `message_warnings`). External integrations such as real GeoIP and search reindex workers remain adapter points.


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

The port now has URL coverage and no explicit 501 placeholders, but exact functional parity is still incomplete for subsystems that depend on external services, historical side effects or deep business rules:

- captcha, rate limits, flood protection and IP block policy;
- exact Spring Security roles/remember-me sessions;
- SMTP templates, resend flows and password reset token emails;
- full image pipeline including animation detection, resizing and CDN/storage layout;
- search reindex backend and ranking behavior;
- complete notification/tracker/realtime event generation;
- exact moderator audit log semantics and score penalties;
- all JSP-level presentation details.

Use this file together with `docs/SERVICE_PORTING_MAP.md` to continue service-by-service replacement of simplified Rust handlers with full parity implementations.
