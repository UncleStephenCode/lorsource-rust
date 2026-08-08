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

## Следующие шаги для полного production-переноса

1. Перенести password verifier под bcrypt-поле из миграции `2026-06-16-bcrypt-passwd.xml`.
2. Перенести права `GroupPermissionService`, `TopicPermissionService`, `UserPermissionService` в отдельный модуль `src/rights`.
3. Добавить event bus/notifications для `user_events`.
4. Перенести image upload/gallery storage.
5. Покрыть маршруты интеграционными тестами через `tower::ServiceExt`.
