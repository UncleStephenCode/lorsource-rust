\set ON_ERROR_STOP on
\if :{?accounts_only}
\else
\set accounts_only false
\endif

-- Deterministic production-readiness fixture for the disposable Compose DB.
-- Run through seed.py; it performs the external safety checks before psql.
BEGIN;

SET LOCAL lock_timeout = '10s';
SET LOCAL statement_timeout = '60s';

DO $$
DECLARE
    missing_groups integer[];
BEGIN
    IF current_database() <> 'lor' THEN
        RAISE EXCEPTION 'prod_ready_test only supports the disposable lor database';
    END IF;

    SELECT array_agg(required.id ORDER BY required.id)
      INTO missing_groups
      FROM (VALUES (2), (6), (126), (1340), (2121), (4068), (4962),
                   (7300), (8404), (10161), (19360), (19362), (19387),
                   (19393), (19399)) AS required(id)
      LEFT JOIN groups g ON g.id = required.id
     WHERE g.id IS NULL;

    IF missing_groups IS NOT NULL THEN
        RAISE EXCEPTION 'current Java demo schema is missing groups: %', missing_groups;
    END IF;
END
$$;

-- Fixture-owned IDs are deliberately isolated. Remove only this namespace so
-- repeated runs restore the same state without touching imported Java data.
-- Browser-created topics/comments use normal sequences, so ownership by the
-- fixture accounts is part of the namespace too.
CREATE TEMP TABLE prod_ready_owned_topics ON COMMIT DROP AS
SELECT id FROM topics
 WHERE id BETWEEN 9101001 AND 9101099
    OR userid BETWEEN 9100001 AND 9100050;
ALTER TABLE prod_ready_owned_topics ADD PRIMARY KEY (id);
CREATE TEMP TABLE prod_ready_owned_comments ON COMMIT DROP AS
SELECT id FROM comments
 WHERE id BETWEEN 9102001 AND 9102099
    OR topic IN (SELECT id FROM prod_ready_owned_topics)
    OR userid BETWEEN 9100001 AND 9100050;
ALTER TABLE prod_ready_owned_comments ADD PRIMARY KEY (id);

DELETE FROM user_events
 WHERE message_id IN (SELECT id FROM prod_ready_owned_topics)
    OR comment_id IN (SELECT id FROM prod_ready_owned_comments)
    OR userid BETWEEN 9100001 AND 9100050
    OR origin_user BETWEEN 9100001 AND 9100050;
DELETE FROM reactions_log
 WHERE topic_id IN (SELECT id FROM prod_ready_owned_topics)
    OR comment_id IN (SELECT id FROM prod_ready_owned_comments)
    OR origin_user BETWEEN 9100001 AND 9100050;
DELETE FROM message_warnings
 WHERE topic IN (SELECT id FROM prod_ready_owned_topics)
    OR comment IN (SELECT id FROM prod_ready_owned_comments)
    OR author BETWEEN 9100001 AND 9100050
    OR closed_by BETWEEN 9100001 AND 9100050;
DELETE FROM edit_info
 WHERE msgid IN (SELECT id FROM prod_ready_owned_topics)
    OR msgid IN (SELECT id FROM prod_ready_owned_comments)
    OR editor BETWEEN 9100001 AND 9100050;
DELETE FROM del_info
 WHERE msgid IN (SELECT id FROM prod_ready_owned_topics)
    OR msgid IN (SELECT id FROM prod_ready_owned_comments)
    OR delby BETWEEN 9100001 AND 9100050;
DELETE FROM telegram_posts
 WHERE topic_id IN (SELECT id FROM prod_ready_owned_topics);
DELETE FROM topic_users_notified
 WHERE topic IN (SELECT id FROM prod_ready_owned_topics)
    OR userid BETWEEN 9100001 AND 9100050;
DELETE FROM vote_users
 WHERE vote BETWEEN 9103001 AND 9103099
    OR vote IN (SELECT id FROM polls WHERE topic IN (SELECT id FROM prod_ready_owned_topics));
DELETE FROM polls_variants
 WHERE vote BETWEEN 9103001 AND 9103099
    OR vote IN (SELECT id FROM polls WHERE topic IN (SELECT id FROM prod_ready_owned_topics));
DELETE FROM polls
 WHERE id BETWEEN 9103001 AND 9103099
    OR topic IN (SELECT id FROM prod_ready_owned_topics);
DELETE FROM images
 WHERE id BETWEEN 9104001 AND 9104099
    OR topic IN (SELECT id FROM prod_ready_owned_topics);
DELETE FROM memories
 WHERE topic IN (SELECT id FROM prod_ready_owned_topics)
    OR userid BETWEEN 9100001 AND 9100050;
DELETE FROM tags WHERE msgid IN (SELECT id FROM prod_ready_owned_topics);
DELETE FROM comments WHERE id IN (SELECT id FROM prod_ready_owned_comments);
DELETE FROM topics WHERE id IN (SELECT id FROM prod_ready_owned_topics);
DELETE FROM msgbase
 WHERE id IN (SELECT id FROM prod_ready_owned_topics)
    OR id IN (SELECT id FROM prod_ready_owned_comments)
    OR id BETWEEN 9101001 AND 9102099;
DELETE FROM ignore_list WHERE userid BETWEEN 9100001 AND 9100050 OR ignored BETWEEN 9100001 AND 9100050;
DELETE FROM user_remarks WHERE user_id BETWEEN 9100001 AND 9100050 OR ref_user_id BETWEEN 9100001 AND 9100050;
DELETE FROM user_tags WHERE user_id BETWEEN 9100001 AND 9100050;
DELETE FROM user_settings WHERE id BETWEEN 9100001 AND 9100050;
DELETE FROM users WHERE id BETWEEN 9100001 AND 9100050;

-- Password for every account: Birds-ProdReady-2026
INSERT INTO users (
    id, nick, name, passwd, url, email, canmod, photo, town, candel,
    blocked, score, max_score, lastlogin, regdate, activated, corrector,
    userinfo, unread_events, token_generation, userinfo_markup
) VALUES
    (9100001, 'swift45',       'Стриж Стартовый',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     'https://example.test/swift', 'swift45@example.test', false, NULL, 'Казань', false,
     false, 45, 45, CURRENT_TIMESTAMP - interval '3 hours', '2026-06-01 09:00:00+03', true, false,
     E'# Стриж на старте\n\nНовый участник с минимальным score для проверки ограничений.\n\n- читает новости\n- отвечает в форуме\n- использует `Markdown`', 1, 0, 'MARKDOWN'),
    (9100002, 'finch50',       'Зяблик Пороговый',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     'https://example.test/finch', 'finch50@example.test', false, NULL, 'Томск', false,
     false, 50, 50, CURRENT_TIMESTAMP - interval '1 day', '2025-11-13 08:30:00+03', true, false,
     E'[b]Зяблик[/b] проверяет границу score=50.\n\n[url=https://www.linux.org.ru/]Оригинальный LOR[/url]. [user]crane2000[/user]', 0, 0, 'BBCODE_TEX'),
    (9100003, 'lark70',        'Жаворонок Автор',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     NULL, 'lark70@example.test', false, NULL, 'Омск', false,
     false, 70, 70, CURRENT_TIMESTAMP - interval '2 days', '2024-08-24 17:15:00+03', true, false,
     E'Профиль в legacy-режиме переноса строк.\nВторая строка должна быть отдельной.\nТретья строка содержит @swift45.', 0, 0, 'BBCODE_ULB'),
    (9100004, 'robin201',      'Малиновка Меточная',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     'https://example.test/robin', 'robin201@example.test', false, NULL, 'Москва', false,
     false, 201, 201, CURRENT_TIMESTAMP - interval '4 hours', '2023-03-18 11:20:00+03', true, false,
     E'## Порог создания меток\n\nScore **201** позволяет проверять создание новых меток вне премодерации.', 2, 0, 'MARKDOWN'),
    (9100005, 'oriole300',     'Иволга Игровая',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     NULL, 'oriole300@example.test', false, NULL, 'Пермь', false,
     false, 300, 320, CURRENT_TIMESTAMP - interval '6 hours', '2022-07-07 07:07:00+03', true, false,
     E'# Игровой профиль\n\nТестирует опросы, цитаты и списки.\n\n> Контент должен выглядеть одинаково в ленте и теме.', 0, 0, 'MARKDOWN'),
    (9100006, 'falcon500',     'Сокол Администраторский',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     'https://example.test/falcon', 'falcon500@example.test', false, NULL, 'Самара', false,
     false, 500, 500, CURRENT_TIMESTAMP - interval '30 minutes', '2021-01-10 14:00:00+03', true, false,
     E'[b]Сокол[/b]\n\n[quote]Проверка старых тем оформления и LORCODE.[/quote]\n\n[code]systemctl status test.service[/code]', 0, 0, 'BBCODE_TEX'),
    (9100007, 'heron750',      'Цапля Галерейная',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     NULL, 'heron750@example.test', false, NULL, 'Псков', false,
     false, 750, 750, CURRENT_TIMESTAMP - interval '8 hours', '2020-05-05 05:05:00+03', true, false,
     E'# Одиночное изображение\n\nПрофиль автора галереи проверяет responsive-контейнер.', 0, 0, 'MARKDOWN'),
    (9100008, 'raven1000',     'Ворон Слайдерный',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     'https://example.test/raven', 'raven1000@example.test', false, NULL, 'Санкт-Петербург', false,
     false, 1000, 1100, CURRENT_TIMESTAMP - interval '10 minutes', '2019-09-09 19:09:00+03', true, false,
     E'# Несколько изображений\n\nПроверяется slider, `srcset` и мобильная компоновка.', 3, 0, 'MARKDOWN'),
    (9100009, 'crane2000',     'Журавль Голосующий',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     NULL, 'crane2000@example.test', false, NULL, 'Уфа', false,
     false, 2000, 2000, CURRENT_TIMESTAMP - interval '12 hours', '2017-12-31 23:30:00+03', true, false,
     E'### Опросы\n\nПроверяет результаты, множественный выбор и уже выбранные варианты.', 0, 0, 'MARKDOWN'),
    (9100010, 'albatross3000', 'Альбатрос Ветеран',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     'https://example.test/albatross', 'albatross3000@example.test', false, NULL, 'Владивосток', false,
     false, 3000, 3200, CURRENT_TIMESTAMP - interval '5 minutes', '2011-04-12 06:00:00+04', true, false,
     E'<p>Legacy HTML сохраняется только как миграционная фикстура.</p><script>alert(1)</script><p><strong>Скрипт должен быть удалён sanitizer-ом.</strong></p>', 0, 0, 'PLAIN'),
    (9100011, 'tern_corrector', 'Крачка Корректор',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     'https://example.test/tern', 'tern.corrector@example.test', false, '9100011.png', 'Калининград', false,
     false, 850, 850, CURRENT_TIMESTAMP - interval '7 minutes', '2018-02-14 12:00:00+03', true, true,
     E'# Корректор\n\nПроверяет и подтверждает премодерируемые материалы, но не модерирует пользователей.', 4, 0, 'MARKDOWN'),
    (9100012, 'ibis_corrector', 'Ибис Корректор',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     NULL, 'ibis.corrector@example.test', false, '9100012.png', 'Воронеж', false,
     false, 650, 700, CURRENT_TIMESTAMP - interval '20 minutes', '2016-06-16 16:16:00+03', true, true,
     E'[b]Второй корректор[/b]\n\nПроверяет запрет подтверждения собственного материала.', 1, 0, 'BBCODE_TEX'),
    (9100013, 'hawk_moderator', 'Ястреб Модератор',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     'https://example.test/hawk', 'hawk.moderator@example.test', true, '9100013.png', 'Екатеринбург', false,
     false, 1200, 1200, CURRENT_TIMESTAMP - interval '2 minutes', '2015-10-21 10:21:00+03', true, false,
     E'# Модератор\n\nМожет подтверждать контент и выполнять обычные модераторские действия, но не управляет другим модератором.', 5, 0, 'MARKDOWN'),
    (9100014, 'eagle_moderator', 'Орёл Старший Модератор',
     '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
     'https://example.test/eagle', 'eagle.moderator@example.test', true, '9100014.png', 'Новосибирск', true,
     false, 1800, 1900, CURRENT_TIMESTAMP - interval '1 minute', '2012-12-12 12:12:00+04', true, false,
     E'# Старший модератор\n\n`candel=true` отделён от обычного `canmod` для проверки исходной модели доступа.', 2, 0, 'MARKDOWN');

INSERT INTO user_settings(id, settings)
SELECT id, hstore(
    ARRAY['style','format.mode','topics','messages','photos','hideAdsense','mainGallery','avatar','trackerMode','oldTracker','oldNotifications','reactionNotification'],
    ARRAY[style,format_mode,topics,messages,photos,hide_adsense,main_gallery,avatar,tracker_mode,old_tracker,old_notifications,reaction_notification]
)
FROM (VALUES
    (9100001,'tango-auto','markdown','30','25','true','true','true','identicon','main','false','false','true'),
    (9100002,'tango-light','lorcode','50','50','true','false','true','retro','all','false','false','true'),
    (9100003,'tango','ntobr','100','100','false','true','false','empty','main','true','true','false'),
    (9100004,'black','markdown','200','50','true','true','true','monsterid','all','false','false','true'),
    (9100005,'white2','markdown','30','200','true','false','true','wavatar','main','false','true','true'),
    (9100006,'waltz','lorcode','50','50','true','true','false','robohash','all','true','false','true'),
    (9100007,'zomg_ponies','markdown','100','25','true','true','true','retro','main','false','false','false'),
    (9100008,'tango-auto','markdown','300','300','true','false','true','identicon','all','false','false','true'),
    (9100009,'tango-light','markdown','500','500','false','true','true','empty','main','false','false','true'),
    (9100010,'tango','ntobr','200','200','true','true','false','robohash','all','true','true','false'),
    (9100011,'tango-auto','markdown','100','100','true','true','true','identicon','main','false','false','true'),
    (9100012,'black','lorcode','50','50','true','true','true','retro','all','false','false','true'),
    (9100013,'tango','markdown','300','200','true','true','true','monsterid','main','false','false','true'),
    (9100014,'tango-light','markdown','500','500','true','true','true','robohash','all','false','false','true')
) AS fixture(id,style,format_mode,topics,messages,photos,hide_adsense,main_gallery,avatar,tracker_mode,old_tracker,old_notifications,reaction_notification);

-- Browser-seed mode intentionally stops here: only accounts, roles and their
-- settings may be inserted through SQL. Topics, comments, polls, reactions
-- and uploaded images are then created through the real HTTP/UI workflows.
\if :accounts_only
SELECT setval('s_uid', GREATEST((SELECT max(id) FROM users),1), true);
-- The Java demo snapshot contains historical group.stat3 values for groups
-- whose topic rows are not part of the snapshot.  Run the same maintenance
-- functions as StatUpdater so the forum index cannot advertise activity that
-- no group page can display.
SELECT stat_update2();
SELECT update_monthly_stats();
COMMIT;
\quit
\endif

-- Synthetic bodies are based on public unclestephen topics. Each body keeps a
-- stable link to the production example while exercising a markup/content path.
INSERT INTO msgbase(id, message, markup) VALUES
    (9101001, E'Краткая тестовая новость о том, как альтернативная прошивка сохраняет независимый путь установки приложений.\n\n* проверяется Markdown;\n* ссылка-источник;\n* премодерация новости.\n\n[Оригинальный материал](https://www.linux.org.ru/news/android/18335149)', 'MARKDOWN'),
    (9101002, E'Тестовая редакционная заметка о споре вокруг имени открытого сетевого проекта.\n\n> Материал подтверждён корректором и содержит внешнюю ссылку.\n\n>>>\nЭта часть должна быть скрыта под cut в ленте и раскрыта в теме.\n<<<\n\n[Оригинальный материал](https://www.linux.org.ru/news/russia/18335616)', 'MARKDOWN'),
    (9101003, E'Привет, $username.\n\nКакая доля приобретённых игр действительно запускается и проходится?\n\n[b]Этот текст проверяет LORCODE[/b], [quote]цитату[/quote] и обсуждение в форуме.\n\nУпоминания: [user]crane2000[/user], [user]bird50[/user], [user]missing_fixture_user[/user].\n\nОригинал: https://www.linux.org.ru/polls/polls/18327393', 'BBCODE_TEX'),
    (9101004, E'Есть локальное зеркало провайдеров и несколько платформенных архивов.\n\n```hcl\nprovider_installation {\n  network_mirror { url = "https://mirror.example.test/providers/" }\n}\n```\n\nНужно проверить отображение кода, длинных строк и ответа с решением.\n\n[Оригинальная тема](https://www.linux.org.ru/forum/admin/18253960)', 'MARKDOWN'),
    (9101005, E'Одиночная иллюстрация тестового Linux-десктопа. Проверяются размеры, подпись, srcset и ссылка на оригинал.\n\n[Образец галереи](https://www.linux.org.ru/gallery/screenshots/18317899)', 'MARKDOWN'),
    (9101006, E'Галерея из трёх изображений: общий вид, системный монитор и игровое окно.\n\n1. Первый кадр.\n2. Второй кадр.\n3. Третий кадр.\n\n[Образец галереи](https://www.linux.org.ru/gallery/screenshots/18317899)', 'MARKDOWN'),
    (9101007, E'Тестовый опрос по мотивам обсуждения игровой библиотеки. Он проверяет множественный выбор и согласованные счётчики голосов.\n\nОригинальный опрос: https://www.linux.org.ru/polls/polls/18327393', 'BBCODE_TEX'),
    (9101008, E'Неподтверждённый опрос должен быть виден автору, корректорам и модераторам, но голосование до подтверждения недоступно.', 'MARKDOWN'),
    (9101009, E'# Проверка длинного материала\n\nЭта синтетическая статья использует несколько блоков Markdown и проверяет структуру длинного текста.\n\n## Сценарии\n\n- внутренние ссылки и `inline code`;\n- fenced code;\n- таблица;\n- типографика на мобильном экране.\n\n| Компонент | Статус |\n|---|---|\n| Axum | проверяется |\n| SQLx | проверяется |\n| Askama | проверяется |\n\n```rust\nfn main() { println!("prod-ready fixture"); }\n```\n\nИсточник тематики: https://www.linux.org.ru/people/unclestephen/', 'MARKDOWN'),
    (9101010, E'Да, слово «вайбкодинг» спорное, но здесь интересна проверка реакций в интерфейсе.\n\nПросьба оставить одну из реакций: 👍, 😊, ☕☕, 🎉 или 🔥.\n\n@raven1000 должен получить ссылку-mention, @bird50 — зачёркнутую ссылку, а @missing_fixture_user — только зачёркнутое имя.\n\nИсточник тематики: https://www.linux.org.ru/people/unclestephen/', 'MARKDOWN'),
    (9101011, E'Тема на точной границе score=50. Она нужна для проверки доступа к Talks и комментариям пользователей с разным score.', 'MARKDOWN'),
    (9101012, E'[b]Материал корректора[/b]\n\nСобственную новость должен подтверждать другой корректор или модератор.\n\n[code]commitby != userid[/code]', 'BBCODE_TEX'),
    (9101013, E'Модераторская тема для проверки меню управления, предупреждений, изменения postscore и служебной информации.', 'MARKDOWN'),
    (9101014, E'Одиночное изображение рабочего места, созданное корректором и подтверждённое модератором.', 'MARKDOWN'),
    (9101015, E'Тема старшего модератора о безопасной конфигурации. Обычный модератор не должен получать управление старшим модератором.', 'MARKDOWN'),
    (9101016, E'Черновик нового пользователя. Он не должен попадать в публичные ленты, но должен отображаться владельцу в списке черновиков.', 'MARKDOWN'),
    (9101017, E'Неподтверждённая галерея должна отображаться в очереди полноценной карточкой: с этим текстом, изображением, тегами и подписью автора.', 'MARKDOWN'),
    (9101018, E'# Неподтверждённая статья\n\nОчередь премодерации должна сохранять заголовки, абзацы и **Markdown**, а не превращать материал в короткую строку.', 'MARKDOWN'),

    (9102001, E'Проверяю новость как обычный пользователь: ссылка открывается, теги не склеены. @raven1000 видит mention.', 'MARKDOWN'),
    (9102002, E'[quote]теги не склеены[/quote]\nПодтверждаю: отдельные метки отображаются корректно. [user]crane2000[/user]', 'BBCODE_TEX'),
    (9102003, E'Ответ второго уровня нужен для проверки ветки комментариев и ссылки «Показать ответы».', 'MARKDOWN'),
    (9102004, E'В игровом обсуждении удобнее видеть уже выбранный вариант опроса.', 'MARKDOWN'),
    (9102005, E'Переделал структуру тестового зеркала — решение отображается в ответе.', 'MARKDOWN'),
    (9102006, E'Одиночное изображение не должно превращаться в slider.', 'MARKDOWN'),
    (9102007, E'На мобильном экране кнопки slider должны оставаться доступными.', 'MARKDOWN'),
    (9102008, E'Выбираю несколько вариантов, потому что опрос multiselect.', 'MARKDOWN'),
    (9102009, E'Корректор видит неподтверждённый опрос, но не голосует до commit.', 'MARKDOWN'),
    (9102010, E'Статья корректно разбивается на заголовки, таблицу и код.', 'MARKDOWN'),
    (9102011, E'👍 Реакция также должна работать на комментарии.', 'MARKDOWN'),
    (9102012, E'Комментарий модератора: действия управления должны быть доступны только подходящей роли.', 'MARKDOWN'),
    (9102013, E'Комментарий старшего модератора с проверкой candel.', 'MARKDOWN'),
    (9102014, E'Этот комментарий содержит сведения о редактировании.', 'MARKDOWN'),
    (9102015, E'Удалённый комментарий нужен для проверки видимости автора и модератора.', 'MARKDOWN'),
    (9102016, E'Пользователь score=45 может отвечать в обычном форуме, но не создавать тему в Talks.', 'MARKDOWN'),
    (9102017, E'Пользователь score=50 находится ровно на разрешённой границе Talks.', 'MARKDOWN'),
    (9102018, E'Комментарий с legacy line break.\nВторая строка должна быть отдельной.', 'BBCODE_ULB');

INSERT INTO topics (
    id, groupid, userid, title, url, moderate, postdate, linktext, deleted,
    stat1, stat3, lastmod, commitby, notop, commitdate, postscore, postip,
    sticky, resolved, minor, draft, allow_anonymous, reactions, open_warnings
) VALUES
    (9101001,19399,9100001,'LineageOS и проверка разработчиков: тестовая новость','https://www.linux.org.ru/news/android/18335149',false,CURRENT_TIMESTAMP-interval '6 days 23 hours','Оригинальный материал',false,0,0,CURRENT_TIMESTAMP-interval '6 days 22 hours',NULL,false,NULL,-9999,'192.0.2.11',false,false,false,false,true,'{}',0),
    (9101002,2121,9100004,'Спор вокруг бренда открытого сетевого проекта','https://www.linux.org.ru/news/russia/18335616',true,CURRENT_TIMESTAMP-interval '6 days 18 hours','Оригинальный материал',false,0,0,CURRENT_TIMESTAMP-interval '6 days 15 hours',9100011,false,CURRENT_TIMESTAMP-interval '6 days 17 hours',-9999,'192.0.2.12',false,false,false,false,true,jsonb_build_object('9100008','🔥','9100013','🎉'),0),
    (9101003,10161,9100005,'Проходите ли вы игры, которые покупаете?',NULL,false,CURRENT_TIMESTAMP-interval '6 days',NULL,false,0,0,CURRENT_TIMESTAMP-interval '4 hours',NULL,false,NULL,-9999,'192.0.2.13',false,false,false,false,true,jsonb_build_object('9100001','👍','9100008','😊','9100013','☕☕'),0),
    (9101004,1340,9100006,'Вожусь с Terraform: тест локального mirror',NULL,false,CURRENT_TIMESTAMP-interval '5 days 18 hours',NULL,false,0,0,CURRENT_TIMESTAMP-interval '1 day',NULL,false,NULL,-9999,'192.0.2.14',false,true,false,false,false,'{}',0),
    (9101005,4962,9100007,'Linux-десктоп: одиночное изображение',NULL,true,CURRENT_TIMESTAMP-interval '5 days',NULL,false,0,0,CURRENT_TIMESTAMP-interval '4 days 23 hours',9100011,false,CURRENT_TIMESTAMP-interval '4 days 23 hours',-9999,'192.0.2.15',false,false,false,false,true,jsonb_build_object('9100010','👍'),0),
    (9101006,4962,9100008,'Linux-десктоп: галерея из трёх изображений',NULL,true,CURRENT_TIMESTAMP-interval '4 days 18 hours',NULL,false,0,0,CURRENT_TIMESTAMP-interval '4 days 17 hours',9100013,false,CURRENT_TIMESTAMP-interval '4 days 17 hours',-9999,'192.0.2.16',false,false,false,false,true,jsonb_build_object('9100002','🎉','9100014','🔥'),0),
    (9101007,19387,9100009,'Как много игр из библиотеки вы действительно запускаете?',NULL,true,CURRENT_TIMESTAMP-interval '4 days',NULL,false,0,0,CURRENT_TIMESTAMP-interval '3 days 23 hours',9100012,false,CURRENT_TIMESTAMP-interval '3 days 23 hours',-9999,'192.0.2.17',false,false,false,false,true,jsonb_build_object('9100005','👍'),0),
    (9101008,19387,9100010,'Как вы относитесь к маркировке использования ИИ в играх?',NULL,false,CURRENT_TIMESTAMP-interval '3 days 18 hours',NULL,false,0,0,CURRENT_TIMESTAMP-interval '3 days 17 hours',NULL,false,NULL,-9999,'192.0.2.18',false,false,false,false,true,'{}',0),
    (9101009,19362,9100003,'Проверка длинной статьи при портировании LOR',NULL,true,CURRENT_TIMESTAMP-interval '3 days',NULL,false,0,0,CURRENT_TIMESTAMP-interval '2 days 23 hours',9100011,false,CURRENT_TIMESTAMP-interval '2 days 23 hours',-9999,'192.0.2.19',false,false,false,false,true,jsonb_build_object('9100014','🎉'),0),
    (9101010,4068,9100004,'Вайбкодю реакции для тестового профиля',NULL,false,CURRENT_TIMESTAMP-interval '2 days 18 hours',NULL,false,0,0,CURRENT_TIMESTAMP-interval '2 days 17 hours',NULL,false,NULL,-9999,'192.0.2.20',false,false,false,false,true,jsonb_build_object('9100006','🤡','9100009','👍','9100013','🔥'),0),
    (9101011,8404,9100002,'Тема в Talks на границе score=50',NULL,false,CURRENT_TIMESTAMP-interval '2 days',NULL,false,0,0,CURRENT_TIMESTAMP-interval '1 day 23 hours',NULL,false,NULL,50,'192.0.2.21',false,false,false,false,false,'{}',0),
    (9101012,6,9100011,'Новость корректора, подтверждённая коллегой','https://example.test/corrector-news',true,CURRENT_TIMESTAMP-interval '42 hours','Тестовый источник',false,0,0,CURRENT_TIMESTAMP-interval '41 hours',9100012,false,CURRENT_TIMESTAMP-interval '41 hours',-9999,'192.0.2.22',false,false,false,false,false,jsonb_build_object('9100013','👍'),0),
    (9101013,4068,9100013,'Модераторская проверка интерфейса темы',NULL,false,CURRENT_TIMESTAMP-interval '36 hours',NULL,false,0,0,CURRENT_TIMESTAMP-interval '35 hours',NULL,false,NULL,10000,'192.0.2.23',true,false,false,false,false,'{}',1),
    (9101014,19393,9100012,'Рабочее место корректора',NULL,true,CURRENT_TIMESTAMP-interval '30 hours',NULL,false,0,0,CURRENT_TIMESTAMP-interval '29 hours',9100013,false,CURRENT_TIMESTAMP-interval '29 hours',-9999,'192.0.2.24',false,false,false,false,true,jsonb_build_object('9100007','😊'),0),
    (9101015,7300,9100014,'Безопасная конфигурация тестового инстанса',NULL,false,CURRENT_TIMESTAMP-interval '24 hours',NULL,false,0,0,CURRENT_TIMESTAMP-interval '21 hours',NULL,false,NULL,-9999,'192.0.2.25',false,true,false,false,false,'{}',0),
    (9101016,19399,9100001,'Черновик тестовой новости',NULL,false,CURRENT_TIMESTAMP-interval '18 hours',NULL,false,0,0,CURRENT_TIMESTAMP-interval '18 hours',NULL,false,NULL,-9999,'192.0.2.26',false,false,false,true,true,'{}',0),
    (9101017,19393,9100012,'Неподтверждённое рабочее место',NULL,false,CURRENT_TIMESTAMP-interval '12 hours',NULL,false,0,0,CURRENT_TIMESTAMP-interval '12 hours',NULL,false,NULL,-9999,'192.0.2.27',false,false,false,false,true,'{}',0),
    (9101018,19362,9100002,'Неподтверждённая статья о совместимости',NULL,false,CURRENT_TIMESTAMP-interval '6 hours',NULL,false,0,0,CURRENT_TIMESTAMP-interval '6 hours',NULL,false,NULL,-9999,'192.0.2.28',false,false,false,false,true,'{}',0);

INSERT INTO comments (
    id, topic, userid, title, postdate, replyto, deleted, postip,
    editor_id, edit_date, edit_count, reactions
) VALUES
    (9102001,9101002,9100002,'Re: Спор вокруг бренда',CURRENT_TIMESTAMP-interval '6 days 17 hours',NULL,false,'198.51.100.1',NULL,NULL,0,jsonb_build_object('9100008','👍','9100013','🎉')),
    (9102002,9101002,9100003,'Re: Спор вокруг бренда',CURRENT_TIMESTAMP-interval '6 days 16 hours',9102001,false,'198.51.100.2',NULL,NULL,0,'{}'),
    (9102003,9101002,9100004,'Re: Спор вокруг бренда',CURRENT_TIMESTAMP-interval '6 days 15 hours',9102002,false,'198.51.100.3',NULL,NULL,0,jsonb_build_object('9100014','😊')),
    (9102004,9101003,9100009,'Re: Проходите ли вы игры',CURRENT_TIMESTAMP-interval '5 days 20 hours',NULL,false,'198.51.100.4',NULL,NULL,0,'{}'),
    (9102005,9101004,9100006,'Re: Terraform mirror',CURRENT_TIMESTAMP-interval '5 days 17 hours',NULL,false,'198.51.100.5',NULL,NULL,0,jsonb_build_object('9100011','👍')),
    (9102006,9101005,9100008,'Re: одиночное изображение',CURRENT_TIMESTAMP-interval '4 days 23 hours',NULL,false,'198.51.100.6',NULL,NULL,0,'{}'),
    (9102007,9101006,9100007,'Re: несколько изображений',CURRENT_TIMESTAMP-interval '4 days 17 hours',NULL,false,'198.51.100.7',NULL,NULL,0,jsonb_build_object('9100001','🎉')),
    (9102008,9101007,9100005,'Re: игровой опрос',CURRENT_TIMESTAMP-interval '3 days 23 hours',NULL,false,'198.51.100.8',NULL,NULL,0,'{}'),
    (9102009,9101008,9100011,'Re: неподтверждённый опрос',CURRENT_TIMESTAMP-interval '3 days 17 hours',NULL,false,'198.51.100.9',NULL,NULL,0,'{}'),
    (9102010,9101009,9100012,'Re: длинная статья',CURRENT_TIMESTAMP-interval '2 days 23 hours',NULL,false,'198.51.100.10',NULL,NULL,0,jsonb_build_object('9100014','🔥')),
    (9102011,9101010,9100010,'Re: реакции',CURRENT_TIMESTAMP-interval '2 days 17 hours',NULL,false,'198.51.100.11',NULL,NULL,0,jsonb_build_object('9100001','👍','9100002','😊','9100013','☕☕')),
    (9102012,9101013,9100013,'Re: модераторская тема',CURRENT_TIMESTAMP-interval '35 hours',NULL,false,'198.51.100.12',NULL,NULL,0,'{}'),
    (9102013,9101015,9100014,'Re: безопасная конфигурация',CURRENT_TIMESTAMP-interval '23 hours',NULL,false,'198.51.100.13',NULL,NULL,0,'{}'),
    (9102014,9101004,9100014,'Re: Terraform mirror',CURRENT_TIMESTAMP-interval '90 minutes',9102005,false,'198.51.100.14',9100014,(CURRENT_TIMESTAMP-interval '80 minutes')::timestamp,1,jsonb_build_object('9100006','👍')),
    (9102015,9101003,9100001,'Re: удалённый комментарий',CURRENT_TIMESTAMP-interval '70 minutes',NULL,true,'198.51.100.15',NULL,NULL,0,'{}'),
    (9102016,9101003,9100001,'Re: score 45',CURRENT_TIMESTAMP-interval '55 minutes',9102004,false,'198.51.100.16',NULL,NULL,0,'{}'),
    (9102017,9101011,9100002,'Re: score 50',CURRENT_TIMESTAMP-interval '40 minutes',NULL,false,'198.51.100.17',NULL,NULL,0,jsonb_build_object('9100004','🎉')),
    (9102018,9101015,9100003,'Re: legacy line break',CURRENT_TIMESTAMP-interval '25 minutes',9102013,false,'198.51.100.18',NULL,NULL,0,'{}');

-- Correct topic counters after trigger-driven inserts and the intentionally
-- deleted comment. stat1/stat3 have the same visible-comment meaning here.
UPDATE topics t
   SET stat1 = counts.visible,
       stat3 = counts.visible,
       lastmod = GREATEST(t.lastmod, counts.last_visible)
  FROM (
      SELECT t0.id,
             count(c.id) FILTER (WHERE NOT c.deleted)::integer AS visible,
             COALESCE(max(c.postdate) FILTER (WHERE NOT c.deleted), t0.postdate) AS last_visible
        FROM topics t0
        LEFT JOIN comments c ON c.topic=t0.id
       WHERE t0.id BETWEEN 9101001 AND 9101099
       GROUP BY t0.id,t0.postdate
  ) counts
 WHERE t.id=counts.id;

INSERT INTO tags_values(value,counter) VALUES
    ('prod-ready',0),('android',0),('lineageos',0),('linux foundation',0),
    ('игры',0),('steam',0),('terraform',0),('автоматизация',0),('галерея',0),
    ('слайдер',0),('опрос',0),('markdown',0),('реакции',0),('lor',0),
    ('безопасность',0),('корректор',0),('модерация',0)
ON CONFLICT(value) DO NOTHING;

INSERT INTO tags(msgid,tagid)
SELECT mapping.msgid,tv.id
FROM (VALUES
    (9101001,'prod-ready'),(9101001,'android'),(9101001,'lineageos'),
    (9101002,'prod-ready'),(9101002,'linux foundation'),
    (9101003,'игры'),(9101003,'steam'),(9101003,'prod-ready'),
    (9101004,'terraform'),(9101004,'автоматизация'),
    (9101005,'галерея'),(9101005,'prod-ready'),
    (9101006,'галерея'),(9101006,'слайдер'),(9101006,'prod-ready'),
    (9101007,'опрос'),(9101007,'игры'),(9101007,'steam'),
    (9101008,'опрос'),(9101008,'игры'),
    (9101009,'markdown'),(9101009,'prod-ready'),
    (9101010,'реакции'),(9101010,'lor'),
    (9101011,'lor'),(9101011,'prod-ready'),
    (9101012,'корректор'),(9101012,'prod-ready'),
    (9101013,'модерация'),(9101013,'lor'),
    (9101014,'галерея'),(9101014,'корректор'),
    (9101015,'безопасность'),(9101015,'prod-ready'),
    (9101016,'android'),(9101016,'prod-ready'),
    (9101017,'галерея'),(9101017,'prod-ready'),
    (9101018,'markdown'),(9101018,'prod-ready')
) AS mapping(msgid,value)
JOIN tags_values tv USING(value)
ON CONFLICT DO NOTHING;

UPDATE tags_values tv
   SET counter=(SELECT count(*)::integer FROM tags t WHERE t.tagid=tv.id)
 WHERE tv.value IN ('prod-ready','android','lineageos','linux foundation','игры','steam',
                    'terraform','автоматизация','галерея','слайдер','опрос','markdown',
                    'реакции','lor','безопасность','корректор','модерация');

INSERT INTO images(id,topic,deleted,extension,main) VALUES
    (9104001,9101005,false,'png',true),
    (9104002,9101006,false,'png',true),
    (9104003,9101006,false,'png',false),
    (9104004,9101006,false,'png',false),
    (9104005,9101014,false,'png',true),
    (9104006,9101017,false,'png',true);

INSERT INTO polls(id,topic,multiselect) VALUES
    (9103001,9101007,true),
    (9103002,9101008,false);
INSERT INTO polls_variants(id,vote,label,votes) VALUES
    (9103101,9103001,'Прошёл всё',0),
    (9103102,9103001,'Прошёл большую часть',0),
    (9103103,9103001,'Прошёл примерно половину',0),
    (9103104,9103001,'Прошёл меньшую часть',0),
    (9103105,9103001,'Почти ничего не запускал',0),
    (9103106,9103002,'Отношусь спокойно',0),
    (9103107,9103002,'Отношусь нейтрально',0),
    (9103108,9103002,'Отношусь настороженно',0);
INSERT INTO vote_users(vote,userid,variant_id) VALUES
    (9103001,9100001,9103102),
    (9103001,9100001,9103103),
    (9103001,9100002,9103103),
    (9103001,9100004,9103104),
    (9103001,9100005,9103104),
    (9103001,9100008,9103105),
    (9103001,9100013,9103102),
    (9103001,9100013,9103104);
UPDATE polls_variants pv
   SET votes=(SELECT count(*)::integer FROM vote_users vu WHERE vu.variant_id=pv.id)
 WHERE pv.vote BETWEEN 9103001 AND 9103099;

INSERT INTO reactions_log(origin_user,topic_id,comment_id,set_date,reaction) VALUES
    (9100008,9101002,NULL,CURRENT_TIMESTAMP-interval '17 hours','🔥'),
    (9100013,9101002,NULL,CURRENT_TIMESTAMP-interval '16 hours','🎉'),
    (9100001,9101003,NULL,CURRENT_TIMESTAMP-interval '15 hours','👍'),
    (9100008,9101003,NULL,CURRENT_TIMESTAMP-interval '14 hours','😊'),
    (9100013,9101003,NULL,CURRENT_TIMESTAMP-interval '13 hours','☕☕'),
    (9100010,9101005,NULL,CURRENT_TIMESTAMP-interval '12 hours','👍'),
    (9100002,9101006,NULL,CURRENT_TIMESTAMP-interval '11 hours','🎉'),
    (9100014,9101006,NULL,CURRENT_TIMESTAMP-interval '10 hours','🔥'),
    (9100005,9101007,NULL,CURRENT_TIMESTAMP-interval '9 hours 50 minutes','👍'),
    (9100014,9101009,NULL,CURRENT_TIMESTAMP-interval '9 hours 40 minutes','🎉'),
    (9100006,9101010,NULL,CURRENT_TIMESTAMP-interval '9 hours 30 minutes','🤡'),
    (9100009,9101010,NULL,CURRENT_TIMESTAMP-interval '9 hours 20 minutes','👍'),
    (9100013,9101010,NULL,CURRENT_TIMESTAMP-interval '9 hours 10 minutes','🔥'),
    (9100013,9101012,NULL,CURRENT_TIMESTAMP-interval '9 hours 5 minutes','👍'),
    (9100007,9101014,NULL,CURRENT_TIMESTAMP-interval '9 hours 2 minutes','😊'),
    (9100008,9101002,9102001,CURRENT_TIMESTAMP-interval '9 hours','👍'),
    (9100013,9101002,9102001,CURRENT_TIMESTAMP-interval '8 hours','🎉'),
    (9100014,9101002,9102003,CURRENT_TIMESTAMP-interval '7 hours 50 minutes','😊'),
    (9100011,9101004,9102005,CURRENT_TIMESTAMP-interval '7 hours 40 minutes','👍'),
    (9100001,9101006,9102007,CURRENT_TIMESTAMP-interval '7 hours 30 minutes','🎉'),
    (9100014,9101009,9102010,CURRENT_TIMESTAMP-interval '7 hours 20 minutes','🔥'),
    (9100001,9101010,9102011,CURRENT_TIMESTAMP-interval '7 hours','👍'),
    (9100002,9101010,9102011,CURRENT_TIMESTAMP-interval '6 hours','😊'),
    (9100013,9101010,9102011,CURRENT_TIMESTAMP-interval '5 hours','☕☕'),
    (9100006,9101004,9102014,CURRENT_TIMESTAMP-interval '4 hours 50 minutes','👍'),
    (9100004,9101011,9102017,CURRENT_TIMESTAMP-interval '4 hours 40 minutes','🎉');

INSERT INTO user_events(userid,type,private,event_date,message_id,comment_id,message,unread,origin_user)
SELECT target.userid,'REACTION',true,rl.set_date,rl.topic_id,rl.comment_id,rl.reaction,true,rl.origin_user
FROM reactions_log rl
JOIN (
    SELECT t.id AS topic_id,NULL::integer AS comment_id,t.userid FROM topics t
    UNION ALL
    SELECT c.topic,c.id,c.userid FROM comments c
) target ON target.topic_id=rl.topic_id AND target.comment_id IS NOT DISTINCT FROM rl.comment_id
WHERE rl.topic_id BETWEEN 9101001 AND 9101099
  AND target.userid<>rl.origin_user;

INSERT INTO memories(userid,topic,add_date,watch) VALUES
    (9100001,9101002,CURRENT_TIMESTAMP-interval '10 hours',true),
    (9100002,9101003,CURRENT_TIMESTAMP-interval '9 hours',false),
    (9100008,9101010,CURRENT_TIMESTAMP-interval '8 hours',true),
    (9100011,9101008,CURRENT_TIMESTAMP-interval '7 hours',true),
    (9100013,9101013,CURRENT_TIMESTAMP-interval '6 hours',false);

INSERT INTO user_tags(user_id,tag_id,is_favorite)
SELECT fixture.user_id,tv.id,fixture.favorite
FROM (VALUES
    (9100001,'android',true),(9100001,'модерация',false),
    (9100004,'linux foundation',true),(9100004,'игры',false),
    (9100008,'галерея',true),(9100008,'опрос',true),
    (9100011,'корректор',true),(9100013,'модерация',true)
) fixture(user_id,value,favorite)
JOIN tags_values tv USING(value);

INSERT INTO user_remarks(user_id,ref_user_id,remark_text) VALUES
    (9100013,9100001,'Новый пользователь: проверить ограничения score=45'),
    (9100013,9100011,'Корректор новостей'),
    (9100008,9100007,'Автор одиночной галереи');
INSERT INTO ignore_list(userid,ignored) VALUES
    (9100006,9100005),
    (9100008,9100001);

-- Keep every explicit-ID sequence ahead of the imported fixture range.
SELECT setval('s_uid', GREATEST((SELECT max(id) FROM users),1), true);
SELECT setval('s_msgid', GREATEST((SELECT max(id)::bigint FROM msgbase),1), true);
SELECT setval('vote_id', GREATEST((SELECT max(id) FROM polls),1), true);
SELECT setval('votes_id', GREATEST((SELECT max(id) FROM polls_variants),1), true);
SELECT setval(pg_get_serial_sequence('images','id'), GREATEST((SELECT max(id) FROM images),1), true);
SELECT setval(pg_get_serial_sequence('memories','id'), GREATEST((SELECT max(id) FROM memories),1), true);
SELECT setval(pg_get_serial_sequence('user_remarks','id'), GREATEST((SELECT max(id) FROM user_remarks),1), true);
SELECT setval(pg_get_serial_sequence('user_events','id'), GREATEST((SELECT max(id) FROM user_events),1), true);

-- Keep the denormalized forum/archive statistics in the same state that the
-- original scheduled StatUpdater would produce after loading this fixture.
SELECT stat_update2();
SELECT update_monthly_stats();

COMMIT;
