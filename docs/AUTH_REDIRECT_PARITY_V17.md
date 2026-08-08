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

## Deliberate protected-page entrypoint difference

The repository compatibility contract in `AGENTS.md` requires an anonymous
browser request for `/people/{nick}/settings` or `/people/{nick}/edit` to be
redirected to `/login.jsp?from=<current path>`. The Rust handlers therefore
return `303 See Other`, retain the complete path/query as one percent-encoded
`from` value, and still return `403 Forbidden` when an authenticated user asks
for another user's form. `/people/{nick}/profile` remains public.

This is a deliberate difference from the current Java tree, not a claim of
exact current-source parity. `springapp-security.xml` permits all MVC paths;
`EditSettingsController` and `EditProfileController` call
`AuthUtil.AuthorizedOnly`, which raises `AccessViolationException` for an
anonymous session; `springapp-servlet.xml` resolves that exception to
`errors/code403`. If migration policy later chooses byte-for-byte behavior of
that Java revision over the repository contract, these two Rust redirects
must be changed back to a 403 response.
