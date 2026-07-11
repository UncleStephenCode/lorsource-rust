# lorsource-rust

Экспериментальный перенос ядра сайта LOR/lorsource со Scala/Spring MVC на Rust.

Стек проекта:

- Rust 2021
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

PostgreSQL будет поднят рядом, миграции и демо-данные применятся автоматически.

## Локальный запуск без сборки контейнера приложения

```bash
cp .env.example .env
docker compose -f docker-compose.dev.yml up -d postgres
export DATABASE_URL=postgres://lor:lor@localhost:5432/lor
export RUN_MIGRATIONS=true
cargo run
```

## Проверка

```bash
curl -fsS http://localhost:8181/healthz
curl -fsS http://localhost:8181/rss | head
```

Проверка карты маршрутов и схемы:

```bash
./scripts/run-compatibility-suite.sh
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

## Что уже перенесено / подготовлено

- главная лента;
- разделы `news`, `forum`, `articles`, `gallery`, `polls`;
- группы и списки тем;
- страница темы с комментариями;
- создание/редактирование тем в dev-режиме;
- добавление/редактирование/удаление комментариев в dev-режиме;
- теги и страницы тегов;
- профили пользователей;
- поиск по PostgreSQL FTS;
- RSS;
- boxlet endpoints;
- healthcheck и Docker-окружение;
- карта 184 исходных Spring endpoint’ов;
- Rust route declaration coverage для всех извлечённых endpoint shapes;
- v4 functional coverage: explicit `legacy::not_implemented` routes removed;
- перенесены activation, userpic upload, deregistration, `/check-login` и базовая admin/moderation surface;
- v5: схема и обработчик `/vote.jsp` приведены к текущему Java-коду (`polls`, `polls_variants`, `vote_users.variant_id`, `voteid` + repeated `vote`);
- v5: добавлены `user_settings`, `user_log`, `user_log_action` и базовое логирование account/moderation действий;
- модельный слой совместимости `src/models_compat.rs`;
- auth/security scaffold с BCrypt и signed session cookies;
- миграция совместимости `db/migrations/0003_legacy_schema_compat.sql`;
- HTTP smoke compatibility tests.

## Важное ограничение

Исходный проект большой. Этот архив — рабочий Rust-порт ядра, маршрутов и схемы совместимости, пригодный как основа для дальнейшего переноса, но **не production-ready замена исходного Scala/Spring приложения**.

Сейчас все извлечённые URL-формы объявлены в Rust-router, и явных `legacy::not_implemented` маршрутов больше нет. Это всё ещё не означает production parity: сложные подсистемы вроде captcha, SMTP, точной модераторской истории, поискового reindex backend и полного image pipeline пока реализованы упрощённо.

## Импорт старого demo dump

В исходнике есть `sql/demo.db` — это PostgreSQL dump. Он уже приложен как `sql/demo.db.gz`; импортировать можно так:

```bash
./scripts/import-original-demo.sh sql/demo.db.gz
```

Для обычного dev-запуска это не нужно: минимальная схема и seed уже лежат в `db/migrations`.

## Структура

```text
src/routes/topics.rs   темы, ленты, совместимые legacy URL
src/routes/comments.rs комментарии и jump-message
src/routes/groups.rs   разделы/группы
src/routes/users.rs    профили
src/routes/tags.rs     метки
src/routes/search.rs   поиск
src/routes/rss.rs      RSS
src/routes/api.rs      tracker/notifications/boxlets
src/routes/admin.rs    базовые admin/moderator handlers
src/routes/legacy.rs   legacy compatibility handlers
src/security.rs        password/session/permission compatibility layer
src/models_compat.rs   original schema model inventory
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

This archive contains the v9 architectural refactor: Rust 2024 / Rust 1.97 toolchain, domain/application/infra split, Hungarian-style identifiers in the new domain/service/repository layer, and PostgreSQL repositories for the forum/topic core flows. See `docs/ARCHITECTURE_REFACTOR_V9.md` and `docs/generated/architecture_report_v9.json`.

### Profile and theme parity

The Rust port includes a whois-like `/people/{nick}/profile` page and Java-compatible profile settings in `/people/{nick}/settings`. The settings are stored in `user_settings.settings` using the same keys as the Java `DefaultProfile`: `style`, `format.mode`, `topics`, `messages`, `photos`, `hideAdsense`, `mainGallery`, `avatar`, `trackerMode`, `oldTracker`, `reactionNotification`.

Original webapp assets from the Java project are served under `/img`, `/font`, `/js`, `/black`, `/tango`, `/white2`, `/waltz`, `/zomg_ponies` and `/adv`. Supported theme IDs are `tango`, `tango-light`, `tango-auto`, `black`, `white2`, `waltz`, and `zomg_ponies`.
