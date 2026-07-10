# Карта переноса маршрутов

Этот порт сохраняет публичные URL там, где это возможно без Spring MVC regexp-path.

## Реализовано как HTML

- `/`, `/index.jsp`
- `/forum`, `/forum/lenta`, `/forum/{group}`
- `/news/`, `/news/{group}`, `/news/{group}/{id}`
- `/articles/`, `/articles/{group}`, `/articles/{group}/{id}`
- `/gallery/`, `/gallery/{group}`, `/gallery/{group}/{id}`
- `/polls/`, `/polls/{group}`, `/polls/{group}/{id}`
- `/show-topics.jsp`, `/view-message.jsp`, `/jump-message.jsp`
- `/add.jsp`, `/edit.jsp`, `/delete.jsp`, `/undelete`, `/resolve.jsp`
- `/add_comment.jsp`, `/add_comment_ajax`, `/edit_comment`, `/delete_comment.jsp`, `/undelete_comment`
- `/tags`, `/tags.jsp`, `/tags/{first_letter}`, `/tag/{tag}`
- `/people/{nick}`, `/people/{nick}/profile`, `/whois.jsp`
- `/search.jsp`, `/rss`, `/section-rss.jsp`, `/about`

## Реализовано как JSON/совместимые заглушки

- `/tracker`, `/tracker.jsp`
- `/notifications`, `/notifications-count`, `/notifications-reset`
- `/top10.boxlet`, `/articles.boxlet`, `/poll.boxlet`
- admin/moderator endpoints: `/admin/*`, `/banip.jsp`, `/sameip.jsp`, `/groupmod.jsp`, `/usermod.jsp`, `/post-warning`, `/clear-warning`

## Отличия от Scala/Spring

- Старые JSP заменены на Askama-шаблоны.
- Старые regexp routes вида `/forum/{group}/{id}/page{page}` упрощены до `/forum/{group}/{id}/page/{page}`.
- Аутентификация реализована dev-совместимым cookie-механизмом. Для production нужно подключить проверку bcrypt-хеша из новых миграций исходного проекта.
- Полнотекстовый поиск использует PostgreSQL `to_tsvector('russian', ...)`, без внешнего Sphinx/Elastic слоя.
