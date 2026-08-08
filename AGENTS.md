# AGENTS.md

## Mission

This repository contains a Rust port of the original **lorsource** Java/Scala/Spring application used by LOR / linux.org.ru.

The goal is **behavioral and migration compatibility with the original application**, not merely a similar feature set.

Treat the original Java/Scala source tree as the **source of truth** for:

- HTTP paths and methods;
- query/form parameter names and defaults;
- redirects and status codes;
- HTML vs JSON behavior;
- authentication, sessions and authorization;
- database schema, constraints, sequences and semantics;
- legacy `*.jsp` endpoints;
- topic/comment/moderation workflows;
- user profile/settings;
- themes and static assets;
- external integrations and side effects.

A Rust implementation is correct only when its externally observable behavior matches the original closely enough for an in-place migration, unless a deliberate incompatibility is explicitly documented.

---

## Repository roles

The workspace may contain two trees:

- `lorsource-java*` — original Java/Scala/Spring implementation;
- `lorsource-rust*` — Rust port.

If both are available, always inspect the original before changing Rust behavior.

Do not trust old parity reports blindly. Re-run checks against the current source trees.

---

## Technology baseline

The Rust implementation should use:

- current stable Rust;
- Rust edition 2024;
- Axum;
- Tokio;
- SQLx;
- PostgreSQL;
- Askama;
- Docker / Docker Compose;
- devcontainer;
- OpenSearch where required by original behavior.

The site listens on port:

```text
8181
```

Default local URL:

```text
http://localhost:8181
```

Do not reintroduce `8080` for the web application.

---

## Architecture

Use a layered architecture:

```text
HTTP / routes
      |
      v
application / services
      |
      v
domain / repository traits
      |
      v
infra / postgres / external systems
```

Preferred source layout:

```text
src/
  application/
  domain/
  infra/
    postgres/
  routes/
  bootstrap/
```

### Routes

Routes should handle only:

- request extraction;
- HTTP-specific validation;
- current-user/session extraction;
- service invocation;
- redirects;
- status codes;
- template rendering.

Do not put substantial SQL or business logic directly in route handlers.

Bad:

```rust
pub async fn handler(...) {
    let stRow = sqlx::query!("SELECT ...")
        .fetch_one(&stState.stDb)
        .await?;

    // business rules here
}
```

Preferred:

```rust
pub async fn handler(
    State(stState): State<StAppState>,
) -> Result<impl IntoResponse> {
    let stTopic = stState
        .cTopicService
        .stGetTopic(...)
        .await?;

    Ok(...)
}
```

### Application services

Application services coordinate:

- repositories;
- authorization policies;
- transactions;
- domain operations;
- external systems.

### Repositories

Move non-trivial SQL behind repository traits.

Example:

```rust
#[async_trait]
pub trait TrTopicRepository: Send + Sync {
    async fn optFindById(
        &self,
        iTopicId: i32,
    ) -> Result<Option<StTopic>>;
}
```

PostgreSQL implementations belong under `src/infra/postgres/`.

---

## Naming convention

New architectural code follows the established Hungarian-style convention where practical.

### Types

```text
St*  struct
C*   concrete implementation / service / component
Tr*  trait
Ty*  type alias
En*  enum, when appropriate
```

Examples:

```rust
pub struct StTopic {}
pub struct CTopicService<R> {}
pub trait TrTopicRepository {}
pub type TyTopicId = i32;
```

### Variables

Use meaningful prefixes where they improve consistency:

```text
s*      String / &str
i*      integer
b*      bool
vec*    Vec
opt*    Option
st*     struct value
c*      service/component
map*    map
set*    set
dt*     date/time
```

Examples:

```rust
let sNick = "admin";
let iTopicId = 123;
let bCanModerate = true;
let vecTopics = ...;
let optUser = ...;
```

Do not perform destructive mass-renames merely to satisfy notation.

External protocol names, database column names, serde fields, form parameters and template bindings must preserve compatibility.

---

## Compatibility is the primary requirement

Route-count equality does not prove compatibility.

For every original endpoint compare:

1. URL path;
2. HTTP method;
3. query parameters;
4. form parameters;
5. parameter names;
6. required/optional semantics;
7. default values;
8. validation;
9. authentication requirement;
10. authorization requirement;
11. response status;
12. redirect location;
13. response content type;
14. HTML model data;
15. cookies/session;
16. database changes;
17. side effects;
18. error behavior.

Example:

```text
GET /login.jsp?from=/forum/
```

Correct porting includes the `from` parameter, safe redirect handling, hidden `redirectUrl`, session creation and post-login redirect behavior.

---

## Legacy URL compatibility

The original application exposes legacy endpoints such as:

```text
/login.jsp
/add.jsp
/group.jsp
/view-news.jsp
/tracker.jsp
/commit.jsp
/uncommit.jsp
/resolve.jsp
/usermod.jsp
/groupmod.jsp
```

Do not remove these just because cleaner Rust routes exist.

When the original redirects a legacy URL to a canonical URL, reproduce that behavior.

Preserve query parameters and URL encoding.

---

## Axum routing rules

Axum is stricter than Spring MVC.

Pay special attention to:

- `/forum` vs `/forum/`;
- dynamic route conflicts;
- specific routes vs generic routes;
- POST bodies;
- fallback redirects.

Use Axum route syntax:

```text
/people/{nick}
/news/{group}/{id}
```

Do not use obsolete syntax:

```text
/people/:nick
```

Specific user routes must be explicitly registered:

```text
/people/{nick}/profile
/people/{nick}/settings
/people/{nick}/edit
/people/{nick}
```

A trailing-slash compatibility fallback may redirect safe GET/HEAD requests, but must not blindly redirect body-carrying POST requests.

---

## Critical paths

Always verify these after relevant changes:

```text
/
/forum
/forum/
/news
/articles
/gallery
/polls
/tracker
/tracker/
/tracker.jsp
/login.jsp
/add.jsp
/people/admin/profile
/people/admin/settings
```

Also verify real topic paths:

```text
/news/{group}/{topic_id}
/forum/{group}/{topic_id}
```

A 404 on these is a release blocker unless the original returns the same result for the same state.

---

## Authentication and sessions

Authentication must follow the original behavior.

Expected compatibility includes:

- `/login.jsp`;
- the original login-processing form contract;
- `from` / `redirectUrl`;
- safe local redirects;
- signed/authenticated session cookie;
- logout;
- profile/settings access after login;
- activation and blocking checks.

Do not implement:

```text
login -> always redirect to /people/{nick}/profile
```

unless the original does exactly that.

### Safe redirects

Reject destinations such as:

```text
//evil.example
https://evil.example
/\evil.example
```

### Development account

A dev database may contain:

```text
admin / admin
```

This is development-only behavior and must never become a production security assumption.

---

## Authorization

Do not reduce the original model to a generic `is_admin` flag without verifying the source.

Inspect original rules involving fields and roles such as:

```text
canmod
candel
score
max_score
activated
blocked
```

and Spring Security configuration.

Moderation actions must enforce actor/target restrictions compatible with the original.

---

## PostgreSQL compatibility

The Rust service must be able to run against data originating from the Java/Liquibase application.

Preserve:

- tables;
- meaningful columns;
- IDs;
- foreign keys;
- unique constraints;
- enum values;
- indexes where semantically relevant;
- PostgreSQL sequences.

Do not solve migration issues by dropping the database.

`docker compose down -v` is acceptable for disposable development testing only.

---

## Migration rules

Every migration must be considered against two scenarios.

### Scenario A

Fresh PostgreSQL database.

### Scenario B

Existing Java/Liquibase PostgreSQL database.

Use safe constructs where appropriate:

```sql
ADD COLUMN IF NOT EXISTS
CREATE INDEX IF NOT EXISTS
```

If an old column may no longer exist, guard the operation with PostgreSQL catalog checks or a `DO $$ ... $$` block.

Never write an unconditional backfill from a column that may have been removed in the current Java schema.

---

## Sequence safety

Explicit-ID seed/import operations can leave sequences behind.

This already caused:

```text
duplicate key value violates unique constraint "tags_values_pkey"
```

After explicit-ID inserts, synchronize the sequence.

Example:

```sql
SELECT setval(
    pg_get_serial_sequence('tags_values', 'id'),
    GREATEST(
        (SELECT COALESCE(MAX(id), 0) FROM tags_values),
        1
    ),
    true
);
```

Audit all seeded/imported serial tables, especially:

- tags;
- users;
- topics/messages;
- comments;
- polls;
- legacy imported data.

---

## SQL correctness

Always qualify ambiguous columns in `UPDATE ... FROM`, joins and CTEs.

Bad:

```sql
UPDATE reactions_log rl
SET origin_user = COALESCE(origin_user, userid)
FROM comments c
WHERE rl.msgid = c.id;
```

Correct:

```sql
UPDATE reactions_log rl
SET origin_user = COALESCE(rl.origin_user, rl.userid)
FROM comments c
WHERE rl.msgid = c.id;
```

---

## Known schema-sensitive areas

Pay particular attention to:

```text
users
user_settings
user_log
user_log_action
topics
comments
msgbase
sections
groups
tags
tags_values
tags_synonyms
user_tags
user_remarks
message_warnings
user_events
reactions_log
user_invites
images
polls
polls_variants
vote_users
telegram_posts
email_domains_block
adv_counts
```

Previously discovered current-schema details include:

```text
msgbase.markup
topics.open_warnings
comments.edit_date
comments.editor_id
sections.imageallowed
sections.restrict_topics
user_tags.user_id
```

Do not assume an older Rust compatibility schema still matches the current original.

---

## Transactions

Operations that modify related tables must be transactional where the original operation is atomic.

Topic creation can affect:

- topic/message data;
- group/section metadata;
- tags;
- tag counters;
- images;
- events;
- notifications.

A tag failure must not silently leave a partially created topic unless that matches original semantics.

---

## Topic creation

The `/add.jsp` flow is critical.

Verify:

1. GET renders the correct form.
2. POST accepts original parameter names.
3. authenticated user becomes author.
4. section/group rules are enforced.
5. topic/message is inserted.
6. tags are processed.
7. counters are updated.
8. transaction commits.
9. redirect points to canonical URL.
10. canonical URL resolves.

Examples:

```text
/news/opensource/123
/forum/linux-org-ru/456
```

Never redirect a successful creation to a route that does not resolve.

---

## Topic/comment URL compatibility

Canonical URLs depend on section and group.

Expected section families include:

```text
forum
news
articles
gallery
polls
```

Do not hardcode every topic under `/forum/...`.

---

## Tracker

The original `/tracker` is a browser-facing HTML page.

Expected behavior:

```text
/tracker     -> HTML
/tracker/    -> HTML or original-compatible canonical redirect
/tracker.jsp -> original-compatible legacy behavior
```

Do not return raw JSON from the browser route.

---

## User profile and settings

Important user-facing paths:

```text
/people/{nick}/profile
/people/{nick}/settings
/people/{nick}/edit
```

Profile functionality should be ported from the original, including meaningful fields where applicable:

- nick;
- ID;
- real name;
- URL;
- city;
- registration date;
- last activity;
- status;
- score;
- avatar;
- user information;
- tags;
- topic/comment statistics;
- moderation-visible information.

Settings must persist to the same logical data used by the original application.

If access requires login, redirect to `/login.jsp?from=<current path>` rather than returning an unexplained 404.

---

## Themes

Theme fidelity requires both the original CSS and compatible HTML structure.

Known theme identifiers include:

```text
tango
tango-light
tango-auto
black
white2
waltz
zomg_ponies
```

The Tango family should map to a shared CSS bundle with a theme variant, approximately:

```text
tango       -> /tango/combined.css + data-theme="dark"
tango-light -> /tango/combined.css + data-theme="light"
tango-auto  -> /tango/combined.css + data-theme="auto"
black       -> /black/combined.css
white2      -> /white2/combined.css
waltz       -> /waltz/combined.css
zomg_ponies -> /zomg_ponies/combined.css
```

Always verify exact behavior against the current original source.

Do not allow a stale theme cookie to unintentionally override the authenticated user's `user_settings` value.

The base HTML must contain the hooks expected by original CSS, for example:

```text
#hd
#sitetitle
#topProfile
#bd
#mainpage
#news
#boxlets
#ft
```

When a theme looks wrong, inspect both CSS and HTML structure. Do not hide markup incompatibility behind large CSS hacks.

---

## Static assets

Keep original-compatible asset paths where practical:

```text
/img
/font
/js
/tango
/black
/white2
/waltz
/zomg_ponies
/adv
```

Theme CSS generation must be reproducible.

---

## Askama templates

Templates are part of the compatibility layer.

Compare original JSP/tag files for:

- HTML IDs;
- classes;
- form names;
- form field names;
- hidden fields;
- action URLs;
- navigation links;
- conditional blocks;
- data attributes.

Do not compare screenshots only. Original CSS and JavaScript depend on structural HTML.

---

## Axum extractor rules

Body-consuming extractors such as `Form` must appear in an order supported by Axum.

Prefer:

```rust
pub async fn handler(
    State(stState): State<StAppState>,
    stCurrentUser: StCurrentUser,
    Form(stForm): Form<StForm>,
) -> Result<impl IntoResponse>
```

When Axum emits a generic `Handler` trait error, check extractor ordering first.

---

## Rust 2024 lifetime rules

Rust 2024 can capture more lifetimes in `impl Trait`.

Prefer concrete helper return types when borrowing is unnecessary.

For example:

```rust
async fn render_rss(...) -> Result<(HeaderMap, String)>
```

may be safer than a helper returning borrowed `impl IntoResponse`.

Never return request-derived data as `&'static str`.

---

## Error handling

Match original HTTP behavior where practical:

- missing resource -> 404;
- unauthenticated -> login redirect or 401 depending on endpoint;
- forbidden -> 403;
- validation error -> original-compatible form error or 4xx;
- safe legacy redirect -> appropriate 3xx.

Do not turn every failure into 500.

Do not expose sensitive database errors to clients.

---

## UI fidelity

Check more than the homepage:

- homepage;
- forum section;
- group page;
- topic page;
- comments;
- add topic;
- login;
- profile;
- settings;
- tracker;
- moderation pages;
- search;
- tag pages.

The provided source tree is the implementation source of truth unless the task explicitly targets current production behavior on linux.org.ru.

---

## OpenSearch and external systems

Do not replace original OpenSearch behavior with placeholder SQL and call it complete parity.

Likewise, do not silently no-op:

- SMTP;
- activation mail;
- password reset;
- notifications;
- Telegram integration;
- GeoIP;
- file/image storage.

Temporary dev implementations must be clearly marked as incomplete.

---

## Security-sensitive behavior

Always inspect the original implementation before changing:

- password hashing;
- session signing;
- activation tokens;
- reset tokens;
- CSRF behavior;
- safe redirects;
- moderation permissions;
- account blocking;
- upload validation.

Token formats may require binary compatibility with the original implementation.

---

## File uploads and userpics

Compare:

- accepted content types;
- maximum size;
- dimensions;
- image transformations;
- storage paths;
- thumbnails;
- cleanup;
- database references;
- original error behavior.

Successful multipart parsing alone is not full parity.

---

## Devcontainer

Keep `.devcontainer` usable with:

- current stable Rust;
- PostgreSQL;
- OpenSearch if required;
- rust-analyzer;
- rustfmt;
- clippy.

The web service must use port `8181`.

Initialize required PostgreSQL extensions such as:

```text
hstore
fuzzystrmatch
```

when the current port still depends on them.

---

## Docker

The project must build and run with:

```bash
docker compose build
docker compose up
```

The application must bind to port `8181`.

A successful `cargo check` does not prove Docker correctness. Docker also verifies templates, static files, migrations, environment and runtime libraries.

---

## Required validation workflow

For non-trivial changes run as much of the following as applicable.

### Formatting

```bash
cargo fmt --all -- --check
```

or:

```bash
cargo fmt --all
```

### Compile

```bash
cargo check
cargo build
```

### Lints

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Do not silence new warnings without a reason.

### Tests

```bash
cargo test
```

### Docker

```bash
docker compose build
docker compose up
```

### Python support tools

```bash
python3 -m py_compile tools/*.py compat/*.py
```

### Shell

```bash
bash -n scripts/*.sh
```

Also validate `.devcontainer/*.sh` when present.

---

## Runtime smoke tests

After startup:

```bash
curl -i http://localhost:8181/
curl -i http://localhost:8181/forum
curl -i http://localhost:8181/forum/
curl -i http://localhost:8181/news
curl -i http://localhost:8181/tracker
curl -i http://localhost:8181/tracker/
curl -i http://localhost:8181/tracker.jsp
curl -i http://localhost:8181/login.jsp
```

Authentication smoke test:

```bash
rm -f /tmp/lor.cookies

curl -i   -c /tmp/lor.cookies   'http://localhost:8181/login.jsp?from=/forum/'

curl -i   -b /tmp/lor.cookies   -c /tmp/lor.cookies   -X POST   'http://localhost:8181/login_process'   -H 'Content-Type: application/x-www-form-urlencoded'   --data 'nick=admin&passwd=admin&redirectUrl=/forum/'

curl -i   -b /tmp/lor.cookies   'http://localhost:8181/people/admin/profile'

curl -i   -b /tmp/lor.cookies   'http://localhost:8181/people/admin/settings'

curl -i   -b /tmp/lor.cookies   'http://localhost:8181/tracker'
```

Expected:

- login succeeds;
- session persists;
- profile is not 404;
- settings is not 404;
- tracker is HTML;
- authenticated navigation is visible.

---

## Topic creation regression test

Maintain a runtime regression test that:

1. authenticates;
2. loads `/add.jsp`;
3. submits a valid topic;
4. records the redirect;
5. verifies transaction completion;
6. GETs the canonical URL;
7. asserts it is not 404/500;
8. verifies title/body.

This catches regressions in:

- PostgreSQL sequences;
- tags;
- transactions;
- canonical route generation;
- group/section mapping.

---

## Compatibility tooling

Use tools under:

```text
tools/
compat/
docs/generated/
```

but never equate route count with full parity.

For example:

```text
184 / 184 routes
```

does not prove:

- correct form/query parameters;
- redirects;
- authorization;
- HTML;
- SQL;
- migrations;
- side effects.

---

## How to port a feature

For each feature:

1. locate the original controller;
2. locate called services;
3. locate DAOs/repositories;
4. locate model classes;
5. locate JSP/tag templates;
6. locate JavaScript/CSS;
7. locate Liquibase migrations/schema dependencies;
8. implement/correct the Rust behavior;
9. add regression tests;
10. run compile + runtime checks.

Do not port only the controller body while ignoring service and persistence semantics.

---

## Avoid fake parity

Do not use placeholders such as:

```rust
Ok(StatusCode::OK)
```

for endpoints that perform real work in the original.

Do not substitute:

- dummy JSON;
- placeholder HTML;
- unconditional redirects;
- hardcoded user IDs;
- hardcoded moderator decisions;
- no-op integrations.

If something remains incomplete, say so explicitly.

---

## No hardcoded user IDs

Never use:

```rust
let iUserId = 1;
```

as the current user.

Always use authenticated session/current-user state for:

- topics;
- comments;
- reactions;
- moderation;
- tracker;
- settings;
- uploads.

---

## API vs HTML

Browser pages must return HTML when the original returns HTML.

Do not expose internal JSON at a legacy browser path.

Keep JSON APIs separate from user-facing pages.

---

## Redirect correctness

Every mutation redirect must be tested.

For example:

```text
Location: /news/opensource/123
```

must actually resolve.

A successful POST followed by a 404 target is a broken workflow.

---

## Theme regression checks

When changing layout/profile/settings, manually test all supported themes:

```text
tango
tango-light
tango-auto
black
white2
waltz
zomg_ponies
```

Check at least:

- header;
- navigation;
- homepage;
- topic;
- profile;
- settings;
- tracker;
- forms.

---

## Git hygiene

Do not commit:

```text
.env
.env.*
target/
.vscode/
.idea/
__pycache__/
*.pyc
node_modules/
local DB volumes
logs
temporary archives
temporary patches
```

Keep:

```text
.env.example
Cargo.lock
```

for this application unless repository policy explicitly says otherwise.

---

## Patch hygiene

Before sharing/applying patches:

```bash
git diff --check
git apply --check --whitespace=error-all patch-file.patch
```

Do not generate patches with trailing whitespace.

---

## Definition of done for a feature

A feature is ported only when:

- Rust compiles;
- relevant tests pass;
- Docker starts;
- migrations apply;
- endpoint exists;
- method matches;
- parameters match;
- authentication/authorization matches;
- DB side effects match;
- redirects resolve;
- response type matches;
- no unexpected 404/500 is introduced;
- important edge cases are covered.

---

## Definition of done for migration readiness

The Rust implementation is not ready to replace Java until at least:

- critical user flows work;
- legacy URLs resolve;
- authentication works;
- profile/settings work;
- themes work;
- topic/comment creation works;
- moderation works;
- tracker works;
- search behavior is acceptable;
- migrations work on a clone of a real Java DB;
- sequences remain valid;
- static resources resolve;
- browser pages do not unexpectedly return JSON;
- compatibility smoke tests pass;
- rollback/cutover is documented.

---

## Agent working style

When fixing a bug:

1. reproduce it;
2. inspect the relevant Java/Scala implementation;
3. inspect the Rust implementation;
4. identify the semantic difference;
5. fix the underlying class of problem, not just the reported URL;
6. add a regression test;
7. run build/tests;
8. report exactly what was verified.

Do not speculate when the source or runtime test can establish behavior.

Do not claim full compatibility without evidence.

---

## Reporting format

After substantial changes report:

```text
Found
- ...

Changed
- ...

Compatibility impact
- ...

Validation
- cargo fmt: ...
- cargo check: ...
- cargo build: ...
- cargo test: ...
- cargo clippy: ...
- docker compose build: ...
- runtime smoke tests: ...

Remaining differences
- ...
```

If something could not be run, state why.

---

## Historically fragile areas

Give extra regression attention to:

- login/session;
- safe login redirects;
- `/people/{nick}/profile`;
- `/people/{nick}/settings`;
- `/forum/`;
- canonical topic paths;
- `/tracker` HTML;
- topic creation;
- tag sequences;
- Java/Liquibase DB migration;
- reactions;
- warnings;
- user invites;
- theme selection;
- original CSS/HTML compatibility;
- static asset routing;
- Axum trailing slash behavior;
- Axum extractor ordering;
- Rust 2024 lifetime capture.

---

## Final principle

The goal is not:

> Rewrite a similar forum in Rust.

The goal is:

> Replace the existing lorsource Java/Scala application with a Rust implementation while preserving user-visible behavior, URLs, data, database compatibility and operational workflows closely enough that the transition can occur without breaking the site.

When maintainability and compatibility conflict, preserve compatibility first, then refactor behind stable external interfaces.
