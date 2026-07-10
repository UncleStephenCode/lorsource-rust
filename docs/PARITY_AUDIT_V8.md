# Java/Rust parity audit v8

This iteration re-ran the static route inventory against the uploaded `lorsource-java-cleared` archive and then checked the Rust port for functional mismatches in high-risk flows.

## Static route coverage

- Java/Scala endpoint entries extracted from controllers: **184**
- Rust route declarations covering those endpoint shapes: **184/184**
- Missing route declarations: **0**
- Method mismatches: **0**
- Explicit `legacy::not_implemented`/HTTP 501 placeholders in `src`: **0**
- Old Axum `/:param` route syntax in `src`: **0**

See `docs/ROUTE_COVERAGE.md` and generated JSON under `docs/generated/`.

## Fixes made in v8

### Register permit compatibility

The current Java `SecretTokenService` no longer signs register permits as a plain HMAC payload. It encrypts `permit:<expiryMillis>` using PBKDF2-HMAC-SHA256 + AES-256-GCM and Base64 encodes `salt || iv || ciphertext`. The Rust port now implements the same token shape in `src/security.rs` and uses it in registration.

### Password reset flow

The Rust port previously treated `/reset-password` as an authenticated direct password-set endpoint. Java uses `/lostpwd.jsp` to issue a reset code and `/reset-password` to verify `nick + code` before generating a new password. The port now follows that flow:

- `/lostpwd.jsp` checks the email, user state, moderator restrictions and one-reset-per-day rule;
- reset code uses the Java-compatible HMAC payload `nick:email:resetMillis:reset`;
- `/reset-password` validates code expiry and generates a new password;
- user log writes `sent_password_reset` and `reset_password`.

SMTP delivery is still an adapter point; in development the reset code is displayed in the response so compatibility tests can complete without mail infrastructure.

### User filters

The original Java controller distinguishes `add`/`del` params and uses `tagName` for tag filters and `id` for ignored users. The Rust port now accepts those Java form shapes for:

- `/user-filter/favorite-tag`
- `/user-filter/ignore-tag`
- `/user-filter/ignore-user`

It also blocks moderator use of ignore filters, matching Java behaviour.

### Reactions

The Rust reaction endpoint now mirrors more of `ReactionController` / `ReactionService`:

- validates the current Java allowed reaction set;
- prevents reactions to own topic/comment;
- blocks reactions on deleted or expired topics/comments;
- enforces the Java limit: 5 set operations per 10 minutes;
- uses the Java `reactions_log(topic_id, comment_id, origin_user)` conflict target order.

### Poll voting

`/vote.jsp` now checks poll/topic expiry before accepting votes, matching the Java `msg.expired` check.

### Devcontainer

The original Java `.devcontainer` has been ported/adapted for Rust:

- Rust toolchain image;
- PostgreSQL 16 service;
- OpenSearch 3.6 service;
- VS Code Rust extensions;
- post-create DB initialization using SQLx migrations with a psql fallback;
- upload directories prepared for userpics/gallery files.

## Still not proven production-equivalent

This archive improves parity, but a full proof still requires runtime comparison with both applications running. The current execution environment does not contain `cargo`, `rustc` or Docker, so `cargo build`, `cargo test`, `docker compose up` and live endpoint diffing were not executed here.

Deep subsystems that still need runtime-backed parity tests:

- captcha and IP anti-abuse rules;
- SMTP integration and mail templates;
- Spring Security remember-me / persistent sessions / force-unlogin semantics;
- complete OpenSearch indexing and query parity;
- full gallery thumbnail/preview pipeline;
- realtime notification hub;
- exact JSP model attribute parity for every legacy view.
