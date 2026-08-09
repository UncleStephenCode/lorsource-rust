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

Current compatibility evidence additionally covers Spring remember-me cookie
generation/verification (including legacy three-part cookies), legacy Jasypt
and BCrypt password verification, login/topic/comment flood caches, hCaptcha,
IP blocks/slow mode, trusted proxy handling and the write-handler permission
checks exercised by the stateful posting/moderation flows. Production CAPTCHA
keys and outbound connectivity, plus exhaustive proof for every uncommon
permission branch on a current production clone, remain cutover evidence rather
than missing implementation.

Uploaded media is also inside the authorization boundary. `/images/{id}/*`
checks the owning topic before reading a file, `/gallery/preview/*` requires an
authenticated session, and `/photos/*` reproduces the original active versus
historical userpic policy. The stateful gallery regression proves that an
anonymous direct image URL changes from 200 to 403 after its topic is deleted,
while the author retains the history access granted by Java.
