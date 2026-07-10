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

## Что уже перенесено

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
- старые совместимые URL `*.jsp`, где это уместно;
- boxlet endpoints;
- healthcheck и Docker-окружение.

## Важное ограничение

Исходный проект большой: в архиве было около 645 файлов основного кода и более 50 контроллеров. Этот архив — рабочий Rust-порт ядра и маршрутов, пригодный как основа для дальнейшего переноса, но не дословная механическая перепись каждого Scala/JSP класса. Production-функции вроде полноценной модерации, старого security layer, email, image pipeline, warning workflow и всех пользовательских настроек оставлены как явно выделенные точки расширения.

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
```

См. также:

- `docs/ROUTE_MAP.md`
- `docs/ARCHITECTURE.md`
