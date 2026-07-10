# Auth redirect parity v17

The previous Rust port redirected every successful login to
`/people/{nick}/profile`.  The current Java/Scala `LoginController` does not do
that for a normal successful login: it redirects to a sanitized `redirectUrl`
from the hidden login form field `redirectUrl`.  `/login.jsp?from=<local-path>` fills this
field, and an empty or unsafe value falls back to `/`.

This version mirrors that behavior:

- `GET /login.jsp?from=/some/path` stores `/some/path` in the form;
- already authenticated users opening `/login.jsp` are redirected to the same
  sanitized target;
- `POST /login_process` redirects to sanitized `redirectUrl`;
- only same-site relative URLs are accepted: `/...`, but not `//host`, `/\\host`
  or absolute URLs;
- the header login link now passes the current path via `from=...` just like the
  Java `login-link.tag`.

`/people/{nick}/profile` remains implemented by the whois/profile handler, but
login no longer forces users there unless the incoming `from` explicitly asks
for it.
