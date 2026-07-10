# Auth/session/security port

Original security stack:

- Spring Security configured in `WEB-INF/springapp-security.xml`.
- Stateless remember-me cookie named `remember_me`.
- All URLs are permitted at the Spring layer; real permissions are mostly enforced inside controller/service code.
- Global Spring CSRF is disabled.
- Roles are derived from user flags: activated, corrector, moderator, admin-like moderator/delete rights.
- `PasswordEncoderImpl` supports BCrypt and legacy Jasypt hashes; BCrypt input is truncated to a 72-byte UTF-8 boundary.

Rust port state:

- `src/security.rs` implements:
  - BCrypt verification/hash helpers with the same 72-byte truncation rule;
  - role/permission enums;
  - signed timed session token helpers;
  - CSRF token helpers for future form hardening.
- `src/auth.rs` now validates login by nick or email against `users.passwd` and rejects blocked/inactive users.
- Development seed users use `{noop}` passwords only for local fixtures:
  - `admin / admin`
  - `unclestephen / demo`
- Registration stores BCrypt hashes.

Still pending:

- exact Spring remember-me token generation compatibility;
- legacy Jasypt password verification or an offline rehash migration;
- rate limiting/flood protection from `FloodProtector`;
- captcha flow;
- IP block enforcement;
- all permission checks from `rights/*Checker.scala` and `*PermissionService.scala` wired into write handlers;
- request-wide context equivalent of `CommonContextFilter`.
