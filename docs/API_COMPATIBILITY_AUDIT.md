# API and HTTP compatibility audit

This audit compares the current Rust tree with the current original tree in
`../lorsource-java`. The Java/Spring declarations, controller bodies, called
services and binders are the source of truth. Route-count equality is not used
as evidence of semantic parity.

## Inventory result

The source extractors report 193 Java mappings on 131 unique paths. Rust has a
path match for every extracted mapping. At the start of this audit, 119 had an
explicitly declared method match and 74 were reported as `partial-method`.
After the method-parity changes, the same current-source comparison reports
174 `method-declared` rows and 19 structural partials: 17 are Axum implicit
HEAD false positives and two are explicit unrestricted routers. The original
74 rows are classified completely below: 55 were corrected to source-exact
ANY declarations and 19 require no method change.

The corrected Rust extractor reports 172 production declarations (160 in the
main router and 12 in the admin router). It masks `#[cfg(test)]` items before
scanning, so a test-only router can no longer satisfy production coverage.

The Java source contains 18 `@ResponseBody` mappings:

| Contract | Java mapping variants | Audit result |
|---|---:|---|
| AJAX comment | `POST /add_comment_ajax` | Fixed nullable model binding, query-first CSRF, UTF-16/text validation distinctions, anonymous/authenticated ordering, canonical success URL and exact `application/json;charset=utf-8` wire header. Stateful no-mutation checks cover missing/malformed topic and conflicting CSRF values. |
| GeoIP | `GET /admin/geoip` | Source-reviewed; method and JSON family match. Moderator/security-chain behavior remains covered by the general authenticated matrix. |
| Login availability | `ANY /check-login` | Fixed required `nick`, empty-vs-missing distinction, query/POST-form merge, lazy named-parameter firewall access and all seven StrictHttpFirewall methods. |
| Markup preview | `POST /markup/preview` | Fixed profile-default markup, anonymous allowed formats, Java UTF-16 length, exact JSON charset, and removal of unsupported `msg`/`message` aliases. |
| Memories | two `POST /memories.jsp` handlers | Fixed `add`/`remove` mapping predicates, required `watch`, and ambiguous add+remove fail-closed behavior. Stateful test proves the ambiguity cannot delete a row. |
| Notification click/count/reset | three mappings | Source-reviewed. Count now sends Java's `Cache-Control: no-cache`; authentication and POST CSRF remain enforced. |
| Profile year statistics | `GET,HEAD /people/{nick}/profile?year-stats` | Source-reviewed as query-conditioned JSON on the shared profile path; Axum GET supplies HEAD. |
| Reactions | two `POST /reactions/ajax` handlers | Fixed required binding before authorization, removed the `msgid` alias, comment-vs-topic dispatch by parameter presence, target load before action split, first-hyphen semantics and MatchError-compatible 500 for an authenticated value without `-`. |
| Tag autocomplete | `ANY /tags?term` | Fixed empty-term presence dispatch and query/POST-form binding for all allowed methods. |
| User-filter JSON variants | eight header/parameter-conditioned mappings | Source-reviewed. HTML-vs-JSON selection, authentication and CSRF are retained. Normalization/escaping edge cases remain P2 below. |
| Yandex tableau | `GET /yandex-tableau` | Fixed anonymous `{}`, authenticated notification count and authenticated `no-cache`. |

Source-regression coverage is in
`tools/tests/test_api_source_contract.py`. Anonymous differential runtime cases
are in `compat/api_endpoints.json`; mutation and authenticated ordering cases
are in `compat/test_write_flows.py`.

## Classification of the 74 partial-method rows

### 1. GET/HEAD declaration parity: 17

These are extractor false positives for method parity: Spring explicitly lists
GET and HEAD, while an Axum `get(...)` route serves HEAD with the GET handler
and suppresses the response body.

- `/deregister.jsp` — `DeregisterController.show`
- `/jump-message.jsp` — `TopicController.jumpMessage`
- `/login.jsp` — `LoginController.loginForm`
- `/notifications` — `UserEventController.showNotifications`
- `/people/{nick}/profile` — reset-password, ordinary profile and `year-stats` handlers (3 rows)
- `/people/{nick}/profile/wipe` — `UserModificationController.wipe`
- `/search.jsp` — `SearchController.search`
- `/show-replies.jsp` — ordinary, moderator and RSS handlers (3 rows)
- `/tag/{tag}` — HTML page and feed handlers (2 rows)
- `/user-filter` — `UserFilterController.showList`
- `/view-all.jsp` — `UncommitedTopicsController.viewAll`
- `/view-news.jsp` — `TagTopicListController.tagFeedOld`

### 2. Explicit unrestricted servlet routers: 2

These are intentionally implemented as explicit GET/HEAD/POST/PUT/PATCH/
DELETE/OPTIONS routers rather than an Axum `any` fallback, so binding, CSRF,
OPTIONS `Allow` and unsupported-method behavior remain testable.

- `/comment-message.jsp` — `AddCommentController.showFormTopic`
- `/resolve.jsp` — `ResolveController.resolve`

### 3. API, feed and legacy redirect bare-mapping parity: 12

Spring selects these controllers for all seven methods admitted by its
StrictHttpFirewall, but selection is not the final wire contract. Automatic
OPTIONS is an empty 200 with `Allow: GET,HEAD,POST,PUT,PATCH,DELETE,OPTIONS`.
`@ResponseBody` JSON and `RedirectView` results bypass JSP and therefore work
on PUT/PATCH/DELETE; `ModelAndView` RSS/HTML/error results reach the JSP servlet,
whose unsafe-method gate returns an empty 405 with
`Allow: GET, HEAD, POST, OPTIONS`. Query parameters precede URL-encoded POST
form values; PUT/PATCH/DELETE bodies are deliberately not form-bound.

- `/check-login` — JSON availability check
- `/group-lastmod.jsp` — canonical group redirect
- `/group.jsp` — canonical group redirect
- `/people/{nick}` — retired `output=rss` 410 and ordinary user-feed handlers (2 rows)
- `/section-rss.jsp` — RSS feed
- `/tags` — HTML default and JSON `term` handlers (2 rows)
- `/tags.jsp` — canonical tags redirect
- `/tracker.jsp` — canonical tracker redirect
- `/view-message.jsp` — canonical topic redirect
- `/view-section.jsp` — canonical section redirect

The shared Rust dispatch adapter now runs controller binding first, preserves
JSON and redirects, and reproduces the later JSP boundary for view responses.
The runtime matrix exercises PUT, PATCH, DELETE and synthesized OPTIONS across
this group, including required-parameter binding. POST remains protected by
CSRF exactly as in `CSRFHandlerInterceptor`.

### 4. Safe read-only browser bare-mapping parity corrected: 43

These Java handlers have a bare `@RequestMapping` and render read-only HTML.
They use unrestricted controller declarations behind the global
StrictHttpFirewall, including POST's original CSRF interceptor behavior and
Servlet query-first URL-encoded form binding. GET/HEAD/POST render the JSP;
automatic OPTIONS returns an empty all-seven-method advertisement; and
PUT/PATCH/DELETE run the read controller but are rejected when its ModelAndView
is forwarded to the JSP servlet. This downstream 405 is a servlet-container
contract, not an explicit controller method restriction.

The adapter preserves direct binding 400 and pre-view security 403 outcomes,
passes JSON/redirect branches, and models the secondary error-JSP failure
observed for controller-thrown 404/500 responses. `/errors/404` itself is a
direct ModelAndView and therefore has the ordinary JSP 405 outcome. The mixed
forum `pageN` route retains its explicit GET-only mapping while the calendar
shape on the same Axum path retains bare-mapping behavior.

- `/`, `/index.jsp`, `/about`
- `/add-section.jsp` (2 parameter-conditioned rows)
- `/admin/email-domains`
- `/articles/archive`
- `/articles/{group}/{id}/history`
- `/articles/{group}/{id}/{commentid}/history`
- `/forum`, `/forum/lenta`, `/forum/{group}`, `/forum/{group}/archive`
- `/forum/{group}/{id}/history`
- `/forum/{group}/{id}/{commentid}/history`
- `/forum/{group}/{year}/{month}`
- `/gallery/archive`
- `/gallery/{group}/{id}/history`
- `/gallery/{group}/{id}/{commentid}/history`
- `/help/{page}`
- `/news/archive`
- `/news/{group}/{id}/history`
- `/news/{group}/{id}/{commentid}/history`
- `/people/{nick}/deleted-comments`, `/people/{nick}/drafts`, `/people/{nick}/favs`
- `/people/{nick}/reactions`, `/people/{nick}/reactions/{mode}`
- `/people/{nick}/remarks`, `/people/{nick}/tracked`
- `/polls/archive`
- `/polls/{group}/{id}/history`
- `/polls/{group}/{id}/{commentid}/history`
- `/sameip.jsp`, `/show-comments.jsp`, `/tags/{firstLetter}`
- `/tracker`, `/view-deleted`, `/whois.jsp`
- `/{section}/`, `/{section}/archive/{year}/{month}`
- `/{section}/{group}`, `/{section}/{group}/{id}`

`compat/api_endpoints.json` compares representative `/about` PUT as an exact
empty 405 and OPTIONS as an exact empty 200/all-seven `Allow` response, plus
POST CSRF precedence and firewall rejection. It also proves that a mixed
`/tags?term` JSON branch still accepts PATCH while the HTML branch reaches the
JSP gate.

## Binding, status and side-effect findings fixed in this audit

- Spring request parameters are query-first and add URL-encoded form values on
  POST only. A shared Rust helper now preserves duplicates and that ordering.
- Missing AJAX comment `topic` is handler JSON 200, not Axum extractor 422.
  Malformed numeric topic is also JSON 200 and cannot insert a comment.
- A valid query CSRF token wins over a conflicting form token; a bad query
  token wins over a valid form token. Every validation failure remains
  non-mutating.
- Automatic CSRF reads a POST body only for the Servlet form media types
  `application/x-www-form-urlencoded` and `multipart/form-data` (media type
  matching is case-insensitive). A matching token in `text/plain`, JSON or a
  body without a form media type is ignored; the stateful write-flow test
  proves `/logout_all_sessions` returns 403 without changing
  `users.token_generation` or invalidating the authenticated session.
- Automatic CSRF is attached after Spring-equivalent path/method selection,
  rather than to every Axum request globally. Thus a tokenless POST to a
  GET-only mapping such as `/mtn.jsp` or a topic `pageN` shape is 405 before
  the interceptor, while the DispatcherServlet's non-`HandlerMethod`
  not-found fallback remains intercepted and returns 403. The mixed forum
  calendar shape and ordinary mapped mutations remain 403. Full-router and
  pinned-runtime matrix cases cover all four outcomes.
- Missing reaction parameters and malformed numeric targets bind to 400 before
  authentication. A present action without a hyphen is parsed only after
  authentication and target lookup, reproducing the original error ordering.
- `reaction=emoji-`, `reaction=-true` and extra-hyphen values use the first
  hyphen and exact lowercase `true` semantics from Scala.
- Both memories action predicates present are ambiguous in Spring; Rust now
  returns a sanitized 500 before authorization or mutation instead of choosing
  remove.
- JSON endpoints with explicit UTF-8 `produces` use Spring 6.2's actual wire
  serialization, `application/json;charset=utf-8` (no space, lower-case charset).
- Authenticated notification counters and Yandex tableau responses use
  `Cache-Control: no-cache` rather than the port's generic private default.
- The global firewall now matches the pinned Spring Security 6.5.11 method
  allow-list, printable-ASCII request URI check, encoded/decoded URL
  blocklists and dot-segment normalization. Named parameter access is lazy:
  an unrelated NUL/unassigned key is ignored, while complete parameter-map
  enumeration still applies Spring's assigned/non-control predicate. Values
  remain unrestricted, matching the project's firewall customization.
- The pinned runtime's Jetty 12 boundary rejects TRACE with an empty 403
  before `UrlRewriteFilter`, including `TRACE /rss.xml`. CONNECT and ambiguous
  encoded slash/period targets are parser-level 400 responses. The matrix
  compares the portable status contract for parser errors, not Jetty's
  branded ISO-8859-1 HTML page, cache header or version-dependent body.
- Search reindex and usermod parameter-conditioned dispatch runs before CSRF,
  binding and authorization and retains its source-specific missing mapping
  status. The pinned runtime returns 400 for missing memories/user-filter
  action predicates; ambiguous action pairs still fail closed without
  mutation.
- Comment/topic creation and comment editing now use the fallible durable
  search queue. Creation queues after commit and before realtime; comment edit
  queues after commit and emits no controller realtime event, matching source.
- `/tag/{tag}` now dispatches on the actual presence of `section`: the absent
  branch keeps the aggregate tag page and ignores `offset`, while the present
  branch renders the section-specific news/tracker model, pagination and
  synonym redirect. Empty `section` keeps Spring's default zero/404 behavior,
  malformed `section` or `offset` binds to 400, and a negative offset clamps
  to the first page. The 14-case conditioned UI matrix covers this flow plus
  add-section and legacy view-news binding.
- `/view-message.jsp` is ANY behind the firewall, binds the query-first Servlet
  parameter view, uses a layered repository read model, omits expired
  `lastmod`, and preserves RedirectView's raw filter/output query semantics,
  including Jetty's ISO-8859-1 Location serialization and space replacement
  per unrepresentable UTF-16 code unit.
- Dynamic responses now retain Spring Security's `X-Frame-Options: DENY`,
  `X-XSS-Protection: 0` and epoch `Expires`, while security-excluded static
  resources retain the MVC interceptor's `SAMEORIGIN` and omit the latter two
  headers. Differential cases lock both branches.

## Search and realtime

Search controllers, query defaults, output variants and the websocket mapping
were traced through their Java services and the Rust implementations. No new
P0/P1 HTTP-contract difference was proved in this pass. This does not turn an
unavailable OpenSearch, durable queue or websocket runtime into parity; the
runtime gates still require those dependencies and are reported separately.

## Remaining differences

- User-filter tag normalization and SQL-LIKE escaping have source-level edge
  differences that need data fixtures before changing persisted behavior.
- `comments.edit_date` is `timestamp without time zone`. Rust explicitly
  interprets it as UTC, but parity with historical JVM/default-timezone
  interpretation still needs representative production-clone evidence.
- External SMTP, Telegram and other integration side effects are outside this
  controller/API patch and remain subject to their dedicated readiness gates.
- Runtime differential results are authoritative only when both pinned Java
  and Rust stacks are started against controlled compatible databases. Source
  tests alone do not establish response-body equality.

## Validation commands

```bash
python3 tools/extract_original_routes.py ../lorsource-java --json docs/generated/current_java_routes.json --csv docs/generated/current_java_routes.csv
python3 tools/extract_axum_routes.py . --json docs/generated/rust_routes.json --csv docs/generated/rust_routes.csv
python3 tools/route_coverage.py --original docs/generated/current_java_routes.json --rust docs/generated/rust_routes.json --json docs/generated/route_coverage.json --csv docs/generated/route_coverage.csv --md docs/ROUTE_COVERAGE.md
python3 -m unittest tools.tests.test_api_source_contract tools.tests.test_extract_axum_routes tools.tests.test_http_compat
python3 compat/test_http_compat.py --matrix compat/endpoints.json --old "$OLD_BASE_URL" --new "$NEW_BASE_URL"
python3 compat/test_http_compat.py --matrix compat/api_endpoints.json --old "$OLD_BASE_URL" --new "$NEW_BASE_URL"
python3 compat/test_http_compat.py --matrix compat/ui_endpoints.json --old "$OLD_BASE_URL" --new "$NEW_BASE_URL"
WRITE_FLOW_ALLOW_MUTATION=yes python3 compat/test_write_flows.py
```
