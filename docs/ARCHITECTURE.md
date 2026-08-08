# Архитектура Rust-порта

```text
src/main.rs          запуск Axum/Tokio, middleware, static files
src/config.rs        конфигурация из ENV
src/db.rs            compatibility facade for PostgreSQL connect/validation
src/infra/postgres/  repositories and validate-only Java schema contract
src/models.rs        типы домена, совместимые с историческими таблицами LOR
src/markup.rs        безопасный рендеринг подмножества LOR BBCode/markup
src/auth.rs          extractor CurrentUser и dev cookie session
src/routes/*.rs      контроллеры, заменяющие Scala/Spring MVC controllers
templates/*.html     Askama templates вместо JSP
static/app.css       минимальная тема, близкая к старому форумному виду
compat/java-db       canonical Java demo + Liquibase bootstrap and validation
```

## База данных

Порт не владеет параллельной схемой: он запускается только на текущей Java/Liquibase схеме и проверяет её без изменений. Подробнее: `docs/DATABASE_COMPATIBILITY.md`.

## Оставшиеся production-gates

Bcrypt/сессии, права, `user_events`, image/gallery storage и
миграционно-критичные HTTP/DB сценарии уже реализованы.
Текущий блокер релиза — не отсутствующий код, а повторяемая
репетиция на клоне актуальной Java-БД и media storage с реальной
production-топологией SMTP, OpenSearch, GeoIP, blacklist feeds и Telegram.
Исполняемый порядок и fail-closed gate описаны в
`docs/PRODUCTION_CUTOVER.md` и `scripts/run-cutover-gate.sh`.
