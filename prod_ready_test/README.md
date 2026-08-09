# Production-readiness fixture

`prod_ready_test` loads an isolated, deterministic data set into the disposable
Docker Compose database and verifies the Rust port through its public HTTP and
HTML interfaces. It does not test registration: accounts are inserted directly
into the Java/Liquibase-compatible PostgreSQL schema.

## Safety

The seed mutates the Compose database named `lor`. It refuses to run unless:

- the explicit confirmation value is supplied;
- the `postgres` and `app` services are running;
- the target is `lor`, the PostgreSQL role is `postgres`, and Liquibase metadata exists;
- the expected five-section catalog is present.

Fixture IDs are confined to `9100001..9104099`. Repeated runs delete and
recreate only fixture-owned rows and then synchronize affected sequences. Do
not run this against a production database.

## Run

From the repository root:

```bash
docker compose build app
python3 prod_ready_test/run_all.py --start
```

Include browser screenshots (Chrome/Chromium required):

```bash
python3 prod_ready_test/run_all.py --start --visual
```

Screenshots are written to `/tmp/prod_ready_test_artifacts`. Individual stages:

```bash
python3 prod_ready_test/seed.py \
  --confirm seed-disposable-compose-lor
python3 prod_ready_test/test_port.py
python3 prod_ready_test/visual_smoke.py
```

Pillow (`python3-pillow`) is required for deterministic gallery and profile
images. Pass `--skip-media` only when intentionally testing SQL without the
gallery UI assertions.

## Accounts

Every fixture account uses the development-only password:

```text
Birds-ProdReady-2026
```

The ten ordinary users cover scores `45`, `50`, `70`, `201`, `300`, `500`,
`750`, `1000`, `2000`, and `3000`. The additional role accounts are:

| Login | Role | Relevant flags |
|---|---|---|
| `tern_corrector` | corrector | `corrector=true`, `canmod=false` |
| `ibis_corrector` | corrector | `corrector=true`, `canmod=false` |
| `hawk_moderator` | moderator | `canmod=true`, `candel=false` |
| `eagle_moderator` | senior moderator | `canmod=true`, `candel=true` |

The complete machine-readable account and route list is in `manifest.json`.
Accounts have distinct registration dates, cities, profile bodies, profile
markup formats, themes, UI settings, tags, photos, remarks, and ignore-list
state.

## Coverage

The data set includes:

- pending and committed news;
- ordinary, resolved, closed, sticky, and draft forum topics;
- an article with headings, a table, and code;
- single-image and multi-image/slider gallery topics;
- committed multiselect and pending polls, variants, and votes;
- flat and nested comments, edited and deleted comments;
- topic/comment reactions with matching JSON counters and reaction log rows;
- tags, memories, events, profile tags, moderation metadata, and role boundaries;
- Markdown, LORCODE (`BBCODE_TEX`), line-break (`BBCODE_ULB`), and plain profile/content paths.
- collapsed/expanded topic cuts and external-link registrable domains;
- per-letter tag thresholds/actions and all seven saved UI themes.

`test_port.py` checks database consistency and externally observable behavior:
canonical routes, HTML content types, sanitizer behavior, media delivery,
gallery DOM variants, poll visibility/results, comment nesting, reactions,
closed-topic controls, canonical/OpenGraph metadata, original client scripts,
login return URLs, score thresholds, and corrector/moderator authorization.
The visual smoke matrix covers 24 public pages at desktop and mobile widths;
the HTTP suite additionally verifies authenticated tracker and theme markup.

## Production-source provenance

`source_catalog.json` records public content examples found in the
[`unclestephen` profile](https://www.linux.org.ru/people/unclestephen/profile)
and feed. Titles and links identify the compatibility references; fixture
bodies and generated images are synthetic and deterministic. The test suite
therefore exercises the same content categories without copying an archive of
production user content into the repository.
