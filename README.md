# lorsource-rust

Перенос движка LOR/lorsource со Scala/Spring MVC на Rust с
приоритетом поведенческой и миграционной совместимости.

Стек проекта:

- Rust 2024
- Axum + Tokio для HTTP
- SQLx + PostgreSQL для доступа к БД
- Askama вместо JSP
- tower-http для static files, `/photos/*`, gzip и trace middleware
- Docker Compose для локального запуска

## Быстрый запуск через Docker Compose

```bash
docker compose up --build
```

Открыть:

```text
http://localhost:8181
```

PostgreSQL будет поднят рядом. Для нового пустого volume одноразовый
`db-bootstrap` загрузит канонический Java demo dump и все Liquibase changesets.
На уже существующей Java БД этот шаг только проверяет Liquibase history и
ничего не накатывает. Mixed, legacy-Rust и неизвестные схемы отклоняются.

## Локальный запуск без сборки контейнера приложения

```bash
cp .env.example .env
make dev-db
export DATABASE_URL=postgres://linuxweb:linuxweb@localhost:5432/lor
cargo run
```

Rust-приложение не выполняет миграций при старте. Оно подключается как
Java-role `linuxweb` и проверяет 33 таблицы, 214 колонок, enum, sequence,
function, extension и оставшиеся Java triggers только через PostgreSQL catalog.

## Проверка

```bash
curl -fsS http://localhost:8181/healthz
curl -fsS http://localhost:8181/readyz
curl -fsS http://localhost:8181/rss | head
```

Проверка карты маршрутов и схемы:

```bash
./scripts/run-compatibility-suite.sh
```

На хосте без Rust toolchain полный fmt/check/test/clippy-набор
запускается как отдельная, не входящая в release-образ стадия:

```bash
docker build --target quality .
```

HTTP smoke-тесты по Rust-порту:

```bash
NEW_BASE_URL=http://localhost:8181 python3 compat/test_http_compat.py
```

Сравнение старого и нового приложения, если старый Scala-сайт запущен рядом:

```bash
OLD_BASE_URL=http://localhost:8081 \
NEW_BASE_URL=http://localhost:8181 \
python3 compat/test_http_compat.py
```

Воспроизводимый Java comparator и release/cutover gate описаны в
[`docs/COMPATIBILITY_TESTS.md`](docs/COMPATIBILITY_TESTS.md) и
[`docs/PRODUCTION_CUTOVER.md`](docs/PRODUCTION_CUTOVER.md).
Requirement-by-requirement границы локальных доказательств и обязательных
operator evidence собраны в
[`docs/PRODUCTION_READINESS_EVIDENCE.md`](docs/PRODUCTION_READINESS_EVIDENCE.md).
Готовый hardened runtime-манифест находится в
[`deploy/compose.production.yml`](deploy/compose.production.yml); его preflight
выполняет `scripts/check-production-runtime.sh`, а локальную
форму runtime без подключения к production-сервисам проверяет
`scripts/test-production-runtime-shape.sh`.

## Что уже перенесено / подготовлено

- главная лента;
- разделы `news`, `forum`, `articles`, `gallery`, `polls`;
- группы и списки тем;
- страница темы с комментариями;
- создание/редактирование тем с тегами, markup, polls и media;
- защищённая выдача `/gallery/preview`, `/images` и `/photos` с Java-совместимыми
  проверками доступа к preview, удалённым темам и историческим userpic;
- Java-совместимый минутный batch-счётчик успешных запросов `/adv/**` в
  канонической таблице `adv_counts`;
- глобальное обновление `users.lastlogin` для авторизованной сессии с
  оригинальным часовым throttle, включая handlers без `CurrentUser`;
- Java/Tuckey-совместимые browser/CDN cache headers для CSS, JS, fonts,
  webjars, изображений и `/adv/**`;
- Spring Security-совместимый bypass для публичных CSS/JS/font/image
  resources: успешные прямые ответы не создают CSRF-cookie и не
  получают dynamic `private` cache policy;
- Java/Tuckey-совместимая canonical-host/HTTPS нормализация с сохранением
  path и query;
- добавление/редактирование/удаление комментариев;
- Java-совместимая история правок тем и комментариев с
  визуальным diff, типизацией `TOPIC`/`COMMENT` и `fromHistory`
  восстановлением текста;
- реакции, скрытие и игнорирование веток;
- теги и страницы тегов;
- профили пользователей;
- поиск через OpenSearch с персистентной очередью индексации;
- RSS;
- boxlet endpoints;
- healthcheck и Docker-окружение;
- воспроизводимая карта текущих Java+Scala Spring mapping’ов, включая условия запросов и form/model metadata;
- отдельная инвентаризация WebSocket, urlrewrite, servlet/resource mapping’ов и статических корней;
- структурное сравнение Rust route declarations (оно не считается доказательством поведенческой совместимости);
- v4 functional coverage: explicit `legacy::not_implemented` routes removed;
- перенесены activation, userpic upload, deregistration, `/check-login` и базовая admin/moderation surface;
- v5: схема и обработчик `/vote.jsp` приведены к текущему Java-коду (`polls`, `polls_variants`, `vote_users.variant_id`, `voteid` + repeated `vote`);
- v5: добавлены `user_settings`, `user_log`, `user_log_action` и базовое логирование account/moderation действий;
- модельный слой совместимости `src/models_compat.rs`;
- auth/security scaffold с BCrypt и signed session cookies;
- канонический Java/Liquibase bootstrap и fail-closed проверка схемы;
- HTTP smoke и dual-runtime compatibility tests с JSON evidence-отчётом.
- guarded stateful posting/reaction/gallery и usermod/warning HTTP+DB
  regressions, которые CI и cutover gate запускают на disposable-БД.

## Важное ограничение

Локальный behavioral-parity scope значительно закрыт, но **порт ещё не
production-ready замена исходного Scala/Spring приложения**. Для
go/no-go нужна репетиция на клоне production-БД и media storage с
реальными snapshot/WAL identifiers, проверка внешних адаптеров и
отработанный rollback. `scripts/run-cutover-gate.sh` отказывается
давать зелёный статус без этих доказательств.

Сейчас все извлечённые URL-формы объявлены в Rust-router, и явных `legacy::not_implemented` маршрутов больше нет. SMTP-доставка activation/change-email/password-reset писем и асинхронных administrator exception reports совместима с локальным MTA Java-приложения; детали описаны в `docs/EMAIL_COMPATIBILITY.md`. OpenSearch reindex выполняет оригинальное помесячное разбиение, а write-события проходят через персистентный filesystem spool с retry после рестарта. Перенесены Java-планировщики статистики, тегов, событий, рейтинга, чёрных списков, Telegram и очистки старых файлов. Gallery поддерживает preview/reuse и трёхдневную очистку временных файлов. Это всё ещё не означает production parity: остаются live-проверка production storage/CDN и внешних адаптеров, а также обязательная репетиция на клоне реальной Java-БД и хранилища медиа.

## База данных

Единственный активный bootstrap лежит в `compat/java-db/`: там сохранены
исходные logical filenames, checksum и provenance Java commit. Для пустой
тестовой БД:

```bash
LOR_DB_BOOTSTRAP_CONFIRM=bootstrap-empty-java-db \
  compat/java-db/manage.sh bootstrap
```

Для existing Java DB разрешена только admin-проверка:

```bash
compat/java-db/manage.sh validate
```

Прежние Rust SQL перенесены в `compat/legacy-rust-db/offline-sql/` и оставлены
только для аудита. Их нельзя запускать. Полный runbook: `docs/DATABASE_COMPATIBILITY.md`.

## Структура

```text
src/routes/            HTTP extraction, redirects and rendering
src/application/       application services and transaction coordination
src/domain/            domain types and repository traits
src/infra/postgres/    PostgreSQL repositories and schema validation
compat/java-db/        canonical Java database bootstrap/validation
compat/legacy-rust-db/ offline historical Rust SQL (never executed)
```

См. также:

- `docs/PORTING_STATUS.md`
- `docs/CONTROLLER_MAP.md`
- `docs/ROUTE_MAP.md`
- `docs/ROUTE_COVERAGE.md`
- `docs/FUNCTIONAL_COVERAGE.md`
- `docs/SCHEMA_COVERAGE.md`
- `docs/AUTH_SECURITY_PORT.md`
- `docs/SERVICE_PORTING_MAP.md`
- `docs/COMPATIBILITY_TESTS.md`
- `docs/DEMO_DB_COMPARISON.md`
- `docs/DATABASE_COMPATIBILITY.md`
- `docs/PRODUCTION_CUTOVER.md`
- `docs/FUNCTIONAL_COMPARISON_JAVA_RUST.md`
- `docs/CURRENT_JAVA_COMPATIBILITY.md`
- `docs/CURRENT_SOURCE_TABLE_COVERAGE.md`
- `docs/ARCHITECTURE.md`

## v7 parity audit

This archive contains the v7 Rust port iteration. It was re-checked against the uploaded current Java/Scala source and includes additional fixes for registration validation, check-login similarity checks, write attribution for topic/comment creation, and legacy jump redirects. See `docs/PARITY_AUDIT_V7.md` and `docs/VERIFICATION_REPORT_V7.md`.


## v8 parity update

This archive includes an additional Java/Rust parity pass:

- Java-compatible encrypted register permits (`SecretTokenService` AES-GCM/PBKDF2 shape).
- Java-style lost-password and reset-code flow.
- `user-filter` add/delete form compatibility (`tagName`, `id`, `add`, `del`).
- Reaction validation/rate-limit/own-post restrictions aligned with the Java controller.
- Poll voting expiry check.
- Adapted `.devcontainer` with Rust tooling, PostgreSQL and OpenSearch.

See `docs/PARITY_AUDIT_V8.md` and `docs/DEVCONTAINER_PORT.md`.

## Architecture refactor v9

This archive contains the v9 architectural refactor: Rust 2024 / Rust 1.97.1 toolchain, Axum 0.8, domain/application/infra split, Hungarian-style identifiers in the new domain/service/repository layer, and PostgreSQL repositories for the forum/topic core flows. Dynamic routes use Axum 0.8's `{parameter}` syntax. See `docs/ARCHITECTURE_REFACTOR_V9.md` and `docs/generated/architecture_report_v9.json`.

### Profile and theme parity

The Rust port includes a whois-like `/people/{nick}/profile` page and Java-compatible profile settings in `/people/{nick}/settings`. The settings are stored in `user_settings.settings` using the same keys as the Java `DefaultProfile`: `style`, `format.mode`, `topics`, `messages`, `photos`, `hideAdsense`, `mainGallery`, `avatar`, `trackerMode`, `oldTracker`, `oldNotifications`, `reactionNotification`.

Original webapp assets from the Java project are served under `/img`, `/font`, `/js`, `/black`, `/tango`, `/white2`, `/waltz`, `/zomg_ponies` and `/adv`. Supported theme IDs are `tango`, `tango-light`, `tango-auto`, `black`, `white2`, `waltz`, and `zomg_ponies`.

Generated Java browser bundles plus `manifest.json`, `robots.txt` and the
`qrerror` assets are checked into `static/`. To reproduce them after changing
the original webapp, build Java first and run:

```bash
ORIGINAL_ROOT=../lorsource-java make static-sync
```
