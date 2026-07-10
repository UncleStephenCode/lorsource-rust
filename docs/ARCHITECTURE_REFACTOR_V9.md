# Architecture refactor v9

Цель итерации — уйти от «одного слоя handlers + прямой SQL» к более явному layout в духе `quoi-dev/geoip`: тонкий bootstrap, отдельный config/state, доменная модель, application services, infrastructure adapters и web routes.

## Что изменено

- Rust edition обновлён с `2021` до `2024`.
- Зафиксирован актуальный toolchain через `rust-toolchain.toml`: `1.97.0`.
- Dockerfile и `.devcontainer/Dockerfile` переведены с `rust:1.82-*` на `rust:1.97-*`.
- Добавлены Cargo lints, разрешающие венгерские идентификаторы в новом слое.
- Доменные структуры вынесены из единого `models.rs` в модули:
  - `src/domain/forum/model.rs`
  - `src/domain/topic/model.rs`
  - `src/domain/comment/model.rs`
  - `src/domain/user/model.rs`
  - `src/domain/tag/model.rs`
  - `src/domain/event/model.rs`
  - `src/domain/common/model.rs`
  - `src/domain/compat/model.rs`
- `models.rs` и `models_compat.rs` теперь являются фасадами совместимости через type aliases.
- Добавлен слой repository contracts:
  - `TrForumRepository`
  - `TrTopicRepository`
  - `TrCommentRepository`
  - `TrUserRepository`
  - `TrTagRepository`
- Добавлен PostgreSQL infrastructure layer:
  - `src/infra/postgres/database.rs`
  - `src/infra/postgres/forum_repository.rs`
  - `src/infra/postgres/topic_repository.rs`
- Добавлен application layer:
  - `CForumService`
  - `CTopicService`
- Основные forum/topic read/write flows переведены с прямого SQL в route handlers на service/repository слой.
- `.devcontainer` оставлен адаптированным под Rust-порт, PostgreSQL, OpenSearch, sqlx migrations и актуальный Rust.

## Венгерская нотация

В новом архитектурном слое используются префиксы:

- `St*` — структуры данных (`StTopicSummary`, `StAppState`, `StConfig`);
- `C*` — concrete classes/implementations (`CTopicPgRepository`, `CForumService`);
- `Tr*` — traits/contracts (`TrTopicRepository`);
- `Ty*` — type aliases (`TyPgPool`);
- `s*`, `i*`, `b*`, `vec*`, `opt*`, `o*`, `st*`, `v*` — строки, числа, bool, коллекции, Option, объекты, структуры/результаты и void-like методы.

Старые публичные имена (`TopicSummary`, `AppState`, `Config` и т.п.) сохранены как aliases, чтобы не ломать шаблоны, handlers и compatibility-тесты за одну итерацию.

## SQL abstraction status

До рефакторинга основные list/view topic/group функции выполняли SQL прямо в `src/routes`. В v9 они перенесены в `infra/postgres/*_repository.rs`, а `routes/topics.rs` и `routes/groups.rs` работают через `C*Service`.

Полная миграция ещё не завершена: legacy/admin/auth/users/comments/tags/api handlers всё ещё содержат прямые SQL-вызовы. Это зафиксировано в `docs/generated/architecture_report_v9.json`, чтобы последующие итерации были измеримыми.

## Проверки

В окружении sandbox нет `cargo`, `rustc` и Docker, поэтому runtime-сборка не выполнялась. Выполнены доступные проверки:

```bash
python3 -m py_compile tools/*.py compat/*.py
bash -n scripts/*.sh .devcontainer/init-db.sh
python3 tools/architecture_report.py
python3 tools/extract_axum_routes.py --json docs/generated/rust_routes_v9.json
```

## Следующие шаги

1. Перенести SQL из `routes/auth.rs` в `infra/postgres/user_repository.rs` и `application/auth`.
2. Перенести SQL из `routes/comments.rs` в `comment_repository`.
3. Перенести `routes/api.rs`, `routes/tags.rs`, `routes/users.rs`, `routes/admin.rs`, `routes/legacy.rs`.
4. После каждого шага запускать `tools/architecture_report.py` и добиваться `direct_sql_in_routes = []`.
5. После появления Rust toolchain выполнить `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.
