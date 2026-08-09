# Legacy URL rewrites

The Rust HTTP stack implements the path redirects from the current Java
`WEB-INF/urlrewrite.xml` before normal Axum routing.

Compatibility details:

- `type="redirect"` is HTTP 302 (`sendRedirect`), not Axum's usual 303 or 307;
- matching is case-insensitive because the original `<from>` rules do not set
  `casesensitive="true"`;
- Tuckey first UTF-8 percent-decodes `getRequestURI()`, then the original
  global `use-query-string="true"` makes rules match that decoded path plus a
  non-empty raw query. It does not implicitly append a query to the destination;
- therefore anchored rules such as `^/rss.jsp$` match `/rss.jsp` but do not
  match `/rss.jsp?section=1`;
- the `topic-rss.jsp` and `/profile/{nick}/...` patterns preserve raw query
  escapes and parameter order exactly as the Java replacement does, while
  percent escapes in their path components are decoded before matching.

The canonical-host and HTTP-to-HTTPS rules are implemented as a separate outer
middleware, after the path-rewrite middleware in request order. It reproduces
the current Tuckey host prefixes: unknown hosts redirect to the absolute
`https://www.linux.org.ru` URL, plain HTTP on `www.linux.org.ru` upgrades to
HTTPS, beta/test/local hosts pass through, and `stoplinux.org.ru` keeps its
historical absolute redirect. Trusted proxy handling is shared with the rest of
the security boundary, so an untrusted `X-Forwarded-Proto` cannot suppress the
HTTPS upgrade.

Static-asset cache headers are implemented by another response middleware and
preserve the original rule ordering and regular-expression edge cases. Rust
does not create servlet URL-rewritten `;jsessionid` links, so the Java outbound
cleanup rule has no Rust output on which to operate.
