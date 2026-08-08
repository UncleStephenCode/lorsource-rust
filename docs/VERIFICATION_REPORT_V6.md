# Verification report v6: current Java source vs Rust port

> Archived verification report. Mentioned Rust migrations are no longer
> active and are retained only under `compat/legacy-rust-db/offline-sql/`.

This report was produced from the uploaded archives `lorsource-java(2).zip` and `lorsource-rust(3).zip`.
The check was repeated from clean unpacked trees, not from the previous generated reports.

## Automated checks performed

- Extracted Spring MVC controller endpoints from the current Java/Scala source.
- Extracted Axum route declarations from the Rust port.
- Compared endpoint shapes and methods.
- Searched the Rust tree for explicit `legacy::not_implemented`, `todo!`, `unimplemented!`, `panic!` and HTTP 501 placeholders.
- Compared recent Liquibase schema changes with the Rust migrations for tables used by active DAOs.
- Checked Python tools and shell scripts with `py_compile` and `bash -n`.
- Checked Axum route declarations for obsolete `/:param` syntax and duplicate dynamic route shapes.

## Route result

- Java/Scala controller endpoint entries extracted: **184**.
- Rust endpoint shapes covered after normalization: **184/184**.
- Missing Rust route declarations: **0**.
- Explicit `legacy::not_implemented` handlers: **0**.

Important v6 correction: the previous Rust archive used Axum 0.6-style `/:param` routes while `Cargo.toml` depends on Axum 0.7. v6 converts routes to Axum 0.7 `{param}` syntax.

## Bugs found and fixed in v6

1. **Axum route syntax**
   - Problem: `/:group`, `/:nick`, `/:id` route parameters are invalid for Axum 0.7 and can panic at router construction.
   - Fix: all route declarations now use `{group}`, `{nick}`, `{id}`, etc.

2. **Legacy page URLs**
   - Problem: the Java route is `/.../{id}/page{page}` (`page2`), while the Rust port used `/.../{id}/page/{page}`.
   - Fix: v6 routes the last path segment through a `pageN` parser. For forum URLs this is unified with `/forum/{group}/{year}/{month}` to avoid duplicate dynamic route shapes.

3. **`/resolve.jsp` method and semantics**
   - Problem: Java accepts the route through plain `@RequestMapping`, and uses query parameters `msgid` and `resolve=yes/no`; Rust only exposed POST and toggled blindly.
   - Fix: v6 exposes GET and POST, checks that the group is resolvable, checks author/moderator permission, and honors `resolve=yes` / non-yes.

4. **`message_warnings` schema**
   - Problem: the Rust migration used draft columns `userid`, `moderator`, `topic_id`, `comment_id`, `reason`, `resolved`, `resolved_at`; current Java uses `topic`, `comment`, `author`, `message`, `warning_type`, `closed_by`, `closed_when`.
   - Fix: migration `0005_verify_current_java_alignment.sql` adds/backfills the Java columns and the Rust warning handlers now write/read the Java column names.

5. **Warning counter**
   - Problem: Rust used `topics.warning_counter`, but current Java uses `topics.open_warnings`.
   - Fix: v6 adds/backfills `open_warnings` and updates warning handlers to recalculate it using the same open-rule-warning logic.

6. **`reactions_log` schema and behavior**
   - Problem: Rust used `userid`, `msgid`, `set_value`, `action_date`; current Java uses `origin_user`, `topic_id`, `comment_id`, `set_date`, `reaction` and updates `topics.reactions` / `comments.reactions` JSONB.
   - Fix: v6 adds/backfills Java columns and rewrites reaction handlers to use the Java model and `reaction-action` values such as `+1-true` / `+1-false`.

7. **`user_invites` schema**
   - Problem: Rust used `id`, `uuid invite_code`, `created_at`, `used_by`, `used_at`; current Java uses `invite_code text`, `owner`, `issue_date`, `invited_user`, `email`, `valid_until`.
   - Fix: v6 adds/backfills the Java columns and converts `invite_code` to text.

8. **`user_settings` usage**
   - Problem: Rust profile settings still wrote to `users.settings` JSONB, while current Java moved style/settings into `user_settings.settings` hstore.
   - Fix: v6 reads/writes `user_settings.settings` through hstore-compatible SQL.

## Remaining non-parity areas

The port is closer, but it is still not a fully exact production replacement for the Java application. These areas still require endpoint-specific compatibility tests and service ports:

- registration permit encryption, captcha, domain-block policy and anti-flood checks;
- SMTP email templates and real delivery for registration, activation and password reset;
- full Spring Security remember-me semantics and force-unlogin generation handling;
- exact JSP model attributes and legacy view templates;
- OpenSearch search backend and real reindex queue;
- full notification/tracker/realtime event generation;
- full image processing pipeline and thumbnail parity;
- detailed moderation/audit side effects beyond the corrected schema writes.

## Files generated/updated by this verification

- `db/migrations/0005_verify_current_java_alignment.sql`
- `docs/VERIFICATION_REPORT_V6.md`
- `docs/ROUTE_COVERAGE.md`
- `docs/generated/current_java_routes.json`
- `docs/generated/rust_routes.json`
- `docs/generated/route_coverage.json`
