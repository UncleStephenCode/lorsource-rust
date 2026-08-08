# Runtime fix v18

> Archived report. Do not follow its volume deletion or SQLx migration steps.
> Current startup is non-mutating and current database operations are in
> `docs/DATABASE_COMPATIBILITY.md`.

Исправления по фактической проверке dev-инстанса на `localhost:8181`.

## Исправлено

- `POST /add.jsp` больше не должен падать на `tags_values_pkey`: добавлена миграция `0008_runtime_sequence_and_dev_user_fix.sql`, которая поднимает sequence `tags_values.id` до `max(id)` после seed/import.
- В `0002_seed.sql` добавлен `setval` для `tags_values.id`, чтобы чистая dev-БД сразу создавалась корректно.
- `/tracker` и `/tracker/` теперь возвращают HTML-страницу трекера, а не JSON.
- `/tracker.jsp` теперь ведёт себя ближе к Java-оригиналу: редиректит на `/tracker/`, сохраняя `filter`, если он отличается от `all`.
- Добавлены trailing-slash aliases для `/people/{nick}/profile/` и `/people/{nick}/settings/`.
- Добавлена защитная dev-миграция для пользователя `admin`, чтобы после старых dev-volume он оставался активированным администратором.

## После применения в dev

Если ошибка уже воспроизводилась на старом volume, проще пересоздать dev-БД:

```bash
docker compose down -v
docker compose up --build
```

Если volume нужно сохранить, достаточно перезапустить приложение: новая SQLx-миграция `0008` должна выполниться и починить sequence.
