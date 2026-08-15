# Parity audit v7: current Java/Scala source vs Rust port

> Historical snapshot only. Counts in this report describe the named v7 input
> archives and must not be copied into current readiness claims. The live
> structural inventory is [`ROUTE_COVERAGE.md`](ROUTE_COVERAGE.md); it does not
> prove request, authorization, HTML, database or side-effect parity.

Input archives used for this audit:

- `lorsource-java-cleared.zip` — current Java/Scala source of the original engine.
- `lorsource-rust(4).zip` — Rust port before v7 corrections.

## Static inventory

| Area | Java/Scala original | Rust port v7 |
|---|---:|---:|
| Java/Scala source files | 351 | — |
| Rust source files | — | 24 |
| Spring controller classes | 62 | mapped into Axum route modules |
| Extracted Spring endpoint entries | 184 | 184 covered by route declarations |
| Rust route declarations | — | 149 Axum declarations, covering 184 Java entries after method/path normalization |
| Explicit `legacy::not_implemented` / `501` placeholders | — | 0 |
| Old Axum `/:param` route syntax | — | 0 |

## Corrections applied in v7

### Registration and activation

The original `RegisterController` does not create immediately activated users. It creates an inactive user with score/maxScore 45, validates password confirmation/rules/email/nick, checks similar nicknames, and waits for an activation code.

The Rust port now follows that shape more closely:

- `register.jsp` requires `password`, `password2`, `rules=okay`, `email`, `permit`.
- password minimum length is 10.
- password must not case-insensitively match the nick.
- nick validation follows Java `StringUtil.checkLoginName`: lowercase `[a-z][a-z0-9_-]*`, with registration max length 19.
- created users are `activated=false`, `score=45`, `max_score=45`.
- `/check-login` now checks both exact and similar users via `levenshtein_less_equal`, matching `UserDao.hasSimilarUsers` semantics.
- migration enables PostgreSQL `fuzzystrmatch`, required by that check.

The Rust `permit` implementation is HMAC-based rather than Java AES-GCM/PBKDF2-compatible. It preserves the expiry semantics for the Rust port, but exact Java permit token interoperability remains a follow-up if shared tokens are required.

### Write attribution

The earlier Rust port inserted new topics/comments as user id `1`. That is not compatible with the original engine: write actions are bound to the authenticated user.

v7 changes:

- `POST /add.jsp` requires `CurrentUser` and writes `topics.userid = current_user.id`.
- `POST /add_comment.jsp` and `/add_comment_ajax` require `CurrentUser` and write `comments.userid = current_user.id`.

### Legacy redirects

`/jump-message.jsp` previously always appended `#comment-{msgid}`, even when `msgid` was a topic id. v7 now distinguishes topic and comment ids:

- topic id -> `/{section}/{group}/{topic}`;
- comment id -> `/{section}/{group}/{topic}#comment-{comment}`.

### Compile-level correction

A malformed Rust format string in `users::reactions` was fixed.

## Still not proven as full production parity

Route coverage and schema compatibility are not the same as exact business-rule equivalence. The following subsystems still need runtime compatibility tests against a running Java instance and a running Rust instance:

- Java AES-GCM/PBKDF2 register permit token interoperability.
- Captcha implementation and anti-flood checks.
- Full Spring Security remember-me and `persistent_logins` behavior.
- Full SMTP mail templates and delivery side effects.
- Exact OpenSearch indexing/reindexing behavior.
- Full image upload pipeline, thumbnails, gallery previews and media storage lifecycle.
- Full notification/realtime event fanout.
- Exact JSP model attributes and rendered HTML parity.
- Full permission matrix for moderators/admins/correctors and score-based restrictions.

## Conclusion

v7 is substantially closer than v6: it fixes real functional mismatches found by re-reading the current original source. It should be treated as a statically aligned Rust port, not as a mathematically proven drop-in replacement until the runtime compatibility suite is executed with both applications and expanded to cover the subsystems above.
