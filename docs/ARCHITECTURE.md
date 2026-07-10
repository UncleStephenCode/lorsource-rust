# Архитектура Rust-порта

```text
src/main.rs          запуск Axum/Tokio, middleware, static files
src/config.rs        конфигурация из ENV
src/db.rs            PostgreSQL pool + sqlx migrations
src/models.rs        типы домена, совместимые с историческими таблицами LOR
src/markup.rs        безопасный рендеринг подмножества LOR BBCode/markup
src/auth.rs          extractor CurrentUser и dev cookie session
src/routes/*.rs      контроллеры, заменяющие Scala/Spring MVC controllers
templates/*.html     Askama templates вместо JSP
static/app.css       минимальная тема, близкая к старому форумному виду
db/migrations        dev-схема и seed data
```

## База данных

Порт не ломает историческую модель: `topics`, `comments`, `msgbase`, `users`, `groups`, `sections`, `tags` сохранены по именам и основным колонкам. Это позволяет постепенно переносить DAO/Service-логику из Scala в Rust без одномоментной смены схемы.

## Следующие шаги для полного production-переноса

1. Перенести password verifier под bcrypt-поле из миграции `2026-06-16-bcrypt-passwd.xml`.
2. Перенести права `GroupPermissionService`, `TopicPermissionService`, `UserPermissionService` в отдельный модуль `src/rights`.
3. Добавить event bus/notifications для `user_events`.
4. Перенести image upload/gallery storage.
5. Покрыть маршруты интеграционными тестами через `tower::ServiceExt`.
