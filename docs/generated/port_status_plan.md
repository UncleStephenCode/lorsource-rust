# Статус порта Java → Rust: что сделано, что осталось

Собрано по результатам полного построчного сравнения ~/Документы/lorsource-java с текущим Rust-портом
(5 параллельных gap-анализов по подсистемам + security-аудит + реализация в этой сессии).
Обновлено: 2026-07-11.

## ЗАВЕРШЕНО в этой сессии

### Security (критические уязвимости, все закрыты и протестированы)
- delete/undelete/edit комментариев и тем — не было проверки авторизации вообще
- email hijack — смена email без пароля и подтверждения, теперь через `oldpass` + `new_email`/activation
- `/people/{nick}/profile/wipe` — деструктивный GET без подтверждения → модератор-only confirmation-страница
- `/delip.jsp` — делал противоположное действие (unban вместо mass-delete+ban)
- `/people/{nick}/reactions`, `/people/{nick}/remarks` — публичный доступ без авторизации; remarks вдобавок показывал не то направление
- История правок — читалась анонимусами
- `user_remarks` — схема не совпадала с Java (userid/who/remark vs id/user_id/ref_user_id/remark_text)
- 3 stored-XSS (профиль userinfo `|safe`, autolink regex не исключал кавычку, markdown пропускал сырой HTML) + 1 reflected-XSS (`filter` в notifications) — закрыты через `ammonia` (аналог jsoup Safelist.relaxed() из оригинала) как универсальный финальный фильтр + regex-фикс + allow-list на filter
- SQL injection — аудит не нашёл ни одного случая (везде параметризованные запросы)

### Функциональность
- Корневая причина ~90% первоначальных 404 — весь роутинг был в синтаксисе axum 0.8 (`{param}`) при закреплённой версии axum 0.7 (нужен `:param`) — все параметризованные маршруты не матчились вообще
- Система нотификаций — реализована с нуля (генерация REPLY/WATCH событий, счётчик, HTML-страница, mark-as-read, click-through, yandex-tableau), плюс починен enum `event_type` (был с придуманными значениями)
- Редиректы после действий с комментами/темами — `?cid=N` как в оригинале
- Premoderation + черновики + лимит длины сообщения при создании тем — раньше отсутствовало полностью
- Reactions — GET показывает HTML/редиректит, POST редиректит (не JSON), JSON только на `/aj`; починен краш (`groups.expire` → `sections.expire`)
- Тема оформления теперь рендерится на сервере (middleware), а не только через JS; подключена syntax highlighting для code-блоков
- Sequence для `tags_values` синхронизирован; `s_msgid`/`s_uid` уже были ок
- `/memories.jsp` — был сломан (отсутствовал unique constraint под ON CONFLICT)

## ОСТАЁТСЯ (по данным 5 отчётов, актуальность проверена частично)

### Крупное (архитектурные пробелы, не косметика)
1. ~~**Tracker**~~ — ЗАВЕРШЕНО: реализован реальный `TrackerFilterEnum` (all/main/notalks/tech), читает сохранённый `trackerMode`, сортировка по активности за 7 дней.
2. ~~**Поиск**~~ — ЗАВЕРШЕНО: подключён OpenSearch (индекс `messages`, поля точно как в `MessageIndexDocument`), реальные facets (section/group), sort (relevance/date/date-reverse), interval/range фильтры, пагинация. Индексация на всех write-путях (создание/правка/удаление/commit темы и коммента), `/admin/search-reindex` делает настоящий bulk-reindex. Упрощения относительно оригинала: без highlighting (excerpt вместо `<em>`-подсветки), без significant_terms по тегам, без function_score recency-буста.
3. ~~**`/tag/{tag}`**~~ — ЗАВЕРШЕНО: агрегация по секциям с лимитами как в оригинале (21/6/20/20/20), резолвинг synonym через redirect, список синонимов, кнопки избранное/игнор, rename/delete для модератора. Без related tags и full/brief news date-partition — сознательное упрощение.
4. ~~**`/forum/{group}`**~~ — ЗАВЕРШЕНО (кроме showDeleted/ignore-list): фильтр по тегу (404 на несуществующий), sticky-темы вперёд, lastmod-режим, redirect на архив при offset>300. showDeleted/showignored не реализованы — ignore_list вообще нигде не подключён к листингам, это отдельная более крупная задача.
5. ~~**Rename/delete тега**~~ — ЗАВЕРШЕНО: поля формы приведены к оригиналу (oldTagName/tagName/firstLetter/createSynonym), реализован merge с созданием synonym, починена схема `tags_synonyms`.
6. ~~**`/people/{nick}`**~~ — ЗАВЕРШЕНО: теперь реальная лента сообщений (с фильтром по секции, 404 на пустую ленту), не алиас профиля.
7. **Топик-вью** (`TopicController`) — не хватает пагинации, canonical-редиректа при несовпадении group/section, thread-режима (сейчас просто якорь), скрытия удалённых комментариев по флагу.
8. ~~**Профиль: бан/freeze/userlog/other-accounts**~~ — ЗАВЕРШЕНО. Осталось: invited users (нет системы приглашений в Rust вообще), mystery-man аватар-заглушка.
9. ~~**`usermod.jsp` freeze/unfreeze**~~ — ЗАВЕРШЕНО: реальные длительности заморозки + разморозка, isFreezable-проверка. isBlockable-проверки на остальных действиях (block/unblock) пока не сделаны — это отдельный, меньший пункт.
10. ~~**EditSettings**~~ — ПРОВЕРЕНО/ЗАВЕРШЕНО: валидация уже была (allow-list на все поля), реально не хватало: удаления HTML-режима разметки (Java явно его отключил), score-gating устаревших тем, self-only прав доступа (было self-or-moderator).
11. **`/groupmod.jsp`** — не редактируется `urlName`, нет admin-vs-moderator разграничения.
12. ~~**`/help/{page}`**~~ — ЗАВЕРШЕНО: подключён реальный markdown-контент (lorcode/markdown/rules), 404 на остальные.
13. ~~**`/sameip.jsp`**~~ — ЗАВЕРШЕНО: CIDR-маска, UA-фильтр, score-фильтр, список пользователей, block-info для точного IP.
14. **`GeoLocationController`** — заглушка, нет реального geo-бэкенда (возможно, ок как no-op).

### Среднее
- `/show-replies.jsp` — режим модератора (просмотр чужих notifications) и RSS/Atom-фид не реализованы.
- ~~`/about`~~ — ЗАВЕРШЕНО: список модераторов/корректоров.
- ~~`/markup/preview`~~ — ЗАВЕРШЕНО: валидация формата разметки.
- ~~`/delete_image`~~ — ЗАВЕРШЕНО: guard на удаление главного изображения темы, проверка автор-или-модератор.
- HSTS / security-заголовки — не проверено, вероятно отсутствуют.
- Comment/topic notifications по MENTION (упоминание @nick в тексте) — не реализовано (есть только REPLY/WATCH).
- del_info/counters — нет триггеров `topins`/`comins`, счётчики `stat1-4` у groups не обновляются вообще; счётчики topics частично вручную.

### Мелкое
- ~~`/logout` разрешает GET~~ — ЗАВЕРШЕНО: теперь только POST (форма в base.html), закрыт CSRF-через-ссылку вектор.
- ~~`DeregisterController` frozen-статус~~ — ЗАВЕРШЕНО.
- `password` legacy Jasypt-хеши (до миграции на bcrypt) не верифицируются — нужно проверить реальный прод-дамп.
- `users.style` — мёртвая колонка (осталась от старой схемы, в реальной Java-БД её нет).

## Как читать этот список
Пункты в «Крупное» — самостоятельные фичи по объёму сравнимые с тем, что уже сделано в этой сессии (notifications, premoderation). Разумный порядок: tracker (компактно, часто используется) → тег/группа страницы → поиск (требует архитектурного решения про OpenSearch) → профиль/usermod доводка.
