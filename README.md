# lorsource-rust

Экспериментальный перенос ядра сайта LOR/lorsource со Scala/Spring MVC на Rust.

Стек проекта:

- Rust 2021
- Axum + Tokio для HTTP
- SQLx + PostgreSQL для доступа к БД
- Askama вместо JSP
- tower-http для static files, gzip и trace middleware
- Docker Compose для локального запуска

## Быстрый запуск через Docker Compose

```bash
docker compose up --build
```

Открыть:

```text
http://localhost:8080
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
curl -fsS http://localhost:8080/healthz
curl -fsS http://localhost:8080/rss | head
```

Проверка карты маршрутов и схемы:

```bash
./scripts/run-compatibility-suite.sh
```

HTTP smoke-тесты по Rust-порту:

```bash
NEW_BASE_URL=http://localhost:8080 python3 compat/test_http_compat.py
```

Сравнение старого и нового приложения, если старый Scala-сайт запущен рядом:

```bash
OLD_BASE_URL=http://localhost:8081 \
NEW_BASE_URL=http://localhost:8080 \
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
- legacy-route stubs для ещё не перенесённой бизнес-логики;
- модельный слой совместимости `src/models_compat.rs`;
- auth/security scaffold с BCrypt и signed session cookies;
- миграция совместимости `db/migrations/0003_legacy_schema_compat.sql`;
- HTTP smoke compatibility tests.

## Важное ограничение

Исходный проект большой. Этот архив — рабочий Rust-порт ядра, маршрутов и схемы совместимости, пригодный как основа для дальнейшего переноса, но **не production-ready замена исходного Scala/Spring приложения**.

Сейчас все извлечённые URL-формы объявлены в Rust-router, но часть legacy endpoint’ов возвращает `501 Not Implemented`. Это сделано намеренно: endpoint больше не теряется как случайный `404`, а попадает в проверяемый backlog.

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
src/routes/admin.rs    admin/moderator route stubs
src/routes/legacy.rs   known but not yet ported legacy endpoints
src/security.rs        password/session/permission compatibility layer
src/models_compat.rs   original schema model inventory
```

См. также:

- `docs/PORTING_STATUS.md`
- `docs/CONTROLLER_MAP.md`
- `docs/ROUTE_MAP.md`
- `docs/ROUTE_COVERAGE.md`
- `docs/SCHEMA_COVERAGE.md`
- `docs/AUTH_SECURITY_PORT.md`
- `docs/SERVICE_PORTING_MAP.md`
- `docs/COMPATIBILITY_TESTS.md`
- `docs/DEMO_DB_COMPARISON.md`
- `docs/ARCHITECTURE.md`
