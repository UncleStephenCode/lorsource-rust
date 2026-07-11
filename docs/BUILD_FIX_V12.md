# Build fix v12

Исправления по логу `cargo build --release` из Docker Compose.

## Исправлено

- `CForumService` получил метод `vecListGroupsBySection`, который делегирует вызов в `TrForumRepository`.
- В `src/routes/topics.rs` исправлен порядок extractor'ов для POST handlers Axum: `CurrentUser` теперь стоит до body-consuming `Form(...)`.
- В `src/routes/rss.rs` убран `impl IntoResponse` из вспомогательной функции `render_rss`; функция возвращает конкретный тип `(HeaderMap, String)`, чтобы избежать lifetime capture проблем Rust 2024.
- В `help_page` больше не передаётся ссылка на временный результат `page.replace(...)` в `html_escape::encode_text`.
- `section_from_uri` в `topics` и `legacy` теперь возвращает настоящие `'static` string literals, а не borrowed path segments.

## Проверено в sandbox

```bash
python3 -m py_compile tools/*.py compat/*.py
bash -n scripts/*.sh .devcontainer/init-db.sh
```

`cargo`/`rustc` в sandbox отсутствуют, поэтому полноценную сборку нужно повторить через Docker Compose.
