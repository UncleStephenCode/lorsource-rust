# AUTH_VISIBILITY_FIX_V16

Fixes a dev-port login UX bug where a successful login created the internal
`lor_session` cookie but the header still rendered as anonymous because the
base template had no current-user context.

Changes:

- `verify_login` now returns `LoginIdentity { id, nick, style }` instead of only
  a numeric user id.
- `POST /login_process` sets:
  - `lor_session` — HttpOnly signed session cookie used for authorization;
  - `lor_user` — non-HttpOnly display hint used only by the static header;
  - `lor_theme` — optional style cookie loaded from `user_settings`.
- successful login redirects to `/people/{nick}/profile`, so the result is
  immediately visible.
- `GET /login.jsp` redirects an already authenticated user to their profile.
- logout removes both `lor_session` and `lor_user` with `path=/`.
- `templates/base.html` updates `#topProfile` client-side when `lor_user` is
  present.

The real authorization source remains `lor_session`; `lor_user` is not trusted
by backend handlers and is only used for rendering the legacy-looking header.
