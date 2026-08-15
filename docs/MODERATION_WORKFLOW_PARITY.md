# Topic moderation workflow parity

Source of truth:

- `lorsource-java/src/main/scala/ru/org/linux/topic/TopicModificationController.scala:99-194`
- `lorsource-java/src/main/scala/ru/org/linux/topic/ResolveController.scala:29-46`
- `lorsource-java/src/main/webapp/WEB-INF/jsp/mtn.jsp:23-55`
- `lorsource-java/src/main/webapp/WEB-INF/jsp/uncommit.jsp:21-35`
- `lorsource-java/src/main/webapp/WEB-INF/tags/topic.tag:31-296`

## HTTP matrix

| Path | Methods | Required parameters | Binding/auth order | OPTIONS `Allow` |
|---|---|---|---|---|
| `/mt.jsp` | GET, HEAD, POST, OPTIONS | GET: `msgid`; POST: `msgid`, `moveto` | binding, then moderator check | `POST,GET,HEAD,OPTIONS` |
| `/mtn.jsp` | GET, HEAD, OPTIONS | `msgid` | binding, then moderator check | `GET,HEAD,OPTIONS` |
| `/uncommit.jsp` | POST, GET, HEAD, OPTIONS | `msgid` | binding, then moderator check | `POST,GET,HEAD,OPTIONS` |
| `/resolve.jsp` | GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS | `msgid`, `resolve` | binding, then authorized-user check | `GET,HEAD,POST,PUT,PATCH,DELETE,OPTIONS` |

Missing, empty and non-integer required values are HTTP 400. Unsupported
methods are HTTP 405; current production exposes the controller mapping only
in that response (`POST, GET` for `/mt.jsp`, `GET` for `/mtn.jsp`, and
`POST, GET` for `/uncommit.jsp`), which intentionally differs from the richer
automatic OPTIONS value. `TRACE /resolve.jsp` is HTTP 405. OPTIONS is HTTP 200
with an empty body and `Content-Length: 0`.

`/resolve.jsp` is an unrestricted Spring mapping, so PUT reaches required
parameter binding rather than a method rejection. A live empty PUT without
`msgid` returns HTTP 400 with no `Content-Type`, an empty body and
`Content-Length: 0`; the Rust compatibility branch reproduces this narrowly
observed container path while ordinary GET binding errors retain themed HTML.

The vendored Java comparator observed during the source audit returned its
container-generated 400 page as `text/html;charset=iso-8859-1`; current
production returns the Tomcat error page as `text/html;charset=utf-8` for the
equivalent legacy binding failure. Rust intentionally guarantees the status,
HTML media type and binding-before-auth ordering, but uses the application's
normal UTF-8 error template rather than attempting to reproduce a
container-specific error page byte-for-byte. This is an explicit known
presentation divergence, not an untested behavior.

## Database and side-effect matrix

| Operation | Transaction delta | Ordering after commit | Redirect/result |
|---|---|---|---|
| uncommit | `moderate=false`, `commitby=NULL`, `commitdate=NULL`; `lastmod` unchanged | enqueue full topic reindex | action-done page linking the stale canonical URL |
| move | row lock; update `groupid` and `lastmod`; clear URL/link text for a link-disabled target; append markup-specific move information | enqueue full topic reindex even for controller-level same-group no-op | 302 to stale canonical URL with stale `lastmod` |
| resolve | set exact boolean (`resolve == "yes"`) and advance stored `lastmod` by exactly one second | no search queue/event side effect | 302 to stale canonical URL with stale `lastmod` |

Unknown move targets are looked up before the same-group comparison and
therefore fail as an infrastructure error. Deleted topics can be displayed by
both move forms but cannot be submitted to `/mt.jsp`. Uncommit rejects, in
order, expired, deleted and already-uncommitted topics.

## Topic card

`templates/topic_card.html` is shared by the canonical topic page and the
uncommit confirmation. The canonical page uses `show_menu=true` and supplies
memories, edit summary and warnings. The uncommit page follows the JSP's
`messageMenu=null`, `showMenu=false` contract: it retains PreparedTopic data
(remark, committer, moderator IP/user-agent, images, poll, reactions and
postscore) while omitting userpic, edit summary, memories and all menu links.

The edit form now uses the same topic-card template for every rendered POST
preview, including validation and no-op responses. Its adapter deliberately
builds an unpersisted presentation model from the submitted title, body, URL,
link text, tags, draft/publish state and poll values rather than reloading and
displaying stale database values. It merges persisted and staged preview
images, renders Markdown/LORCODE cuts in the expanded topic form, and retains
the viewer-dependent userpic and reaction state. As in the original preview,
the card has no message menu or canonical-only edit summary, memories and
warnings. The explicit `title_plain` field prevents the storage-escaped title
from being decoded or escaped at the wrong layer.

## Deliberate tags-only hardening

The Java source's corrector tags-only branch leaves a crafted-request gap for
changes to existing URL/link and poll fields. Rust deliberately rejects every
protected-field delta while still accepting an actual tags-only edit. This is
a tested security hardening in `src/application/topic/edit.rs`, not a claim of
byte-for-byte compatibility with that Java authorization defect.

## Regression coverage

- `src/domain/topic/moderation.rs`: binding, restrictions and move markup.
- `src/application/topic/moderation.rs`: role/state/queue ordering.
- `src/infra/postgres/topic_moderation_repository.rs`: transactional SQL.
- `src/routes/topic_moderation.rs`: method and `Allow` contracts.
- `src/routes/topics.rs`: common-card edit-preview adapter, expanded-cut
  rendering and unpersisted image/poll/form state.
- `tests/topic_moderation_isolated.rs`: stateful PostgreSQL delta (ignored
  unless `LOR_MODERATION_DB_INTEGRATION_CONFIRM=isolated-schema` and
  `LOR_MODERATION_DB_INTEGRATION_DATABASE_URL` point at a disposable DB).
- `tests/topic_moderation_templates.rs`: form and error DOM.
- `compat/endpoints.json`: browser-level missing/bad/unsupported/OPTIONS
  cases for all four legacy paths.
