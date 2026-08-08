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

The canonical-host and HTTP-to-HTTPS rules are intentionally not enabled yet.
They depend on the deployment's exact public-host and proxy/TLS configuration;
enabling the Java production host list unchanged would break local and migrated
installations. Static-asset cache headers and outbound `;jsessionid` removal are
separate compatibility work and are not claimed by this path-redirect layer.
