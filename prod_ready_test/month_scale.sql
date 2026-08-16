-- Deterministic high-volume continuation of seed.sql.
-- Text/relationships are stable; all activity dates are recalculated from
-- CURRENT_TIMESTAMP on every run and span the immediately preceding month.
BEGIN;
SET LOCAL lock_timeout = '10s';
SET LOCAL statement_timeout = '120s';

-- Complete the 50-account matrix. The first 14 named role/boundary accounts
-- live in seed.sql; these 36 ordinary bird accounts supply realistic volume.
INSERT INTO users (
    id,nick,name,passwd,url,email,canmod,photo,town,candel,blocked,score,max_score,
    lastlogin,regdate,activated,corrector,userinfo,unread_events,token_generation,userinfo_markup
)
SELECT 9100000+n,
       'bird'||lpad(n::text,2,'0'),
       'Тестовая птица '||lpad(n::text,2,'0'),
       '$2b$12$1vqIDkN68YKWtXm75uuRtunF9SE92eT7k1lQolfhzUIHTDPA9lD8q',
       CASE WHEN n%3=0 THEN 'https://example.test/bird/'||n ELSE NULL END,
       'bird'||lpad(n::text,2,'0')||'@example.test',
       false,(9100000+n)::text||'.png',
       (ARRAY['Москва','Казань','Томск','Пермь','Омск','Уфа'])[1+(n%6)],
       false,(n=50),100+n*37,120+n*39,
       CURRENT_TIMESTAMP-make_interval(hours => n),
       CURRENT_TIMESTAMP-make_interval(days => n*91),
       true,false,
       E'# Профиль птицы '||lpad(n::text,2,'0')||E'\n\nАвтоматическая месячная фикстура.\n\n- стабильный контент\n- динамические даты\n- локальный userpic',
       0,0,'MARKDOWN'
  FROM generate_series(15,50) n;

-- Local avatars intentionally override Gravatar for every fixture account.
UPDATE users
   SET photo=id::text||'.png'
 WHERE id BETWEEN 9100001 AND 9100050;

INSERT INTO user_settings(id,settings)
SELECT id,hstore(
    ARRAY['style','format.mode','topics','messages','photos','hideAdsense','mainGallery','avatar','trackerMode','oldTracker','oldNotifications','reactionNotification'],
    ARRAY['tango-auto','markdown','50','100','true','true','true','identicon','main','false','false','true']
)
FROM users WHERE id BETWEEN 9100015 AND 9100050;

-- A real ban record exercises the anonymous `errors/user-banned` profile
-- model (date and reason), not only the users.blocked authorization flag.
INSERT INTO ban_info(userid,bandate,reason,ban_by) VALUES
    (9100050,CURRENT_TIMESTAMP-interval '7 days',
     'Проверка страницы заблокированного профиля',9100013);

-- Moderator tracker renders recent photos through the same Userpic JSP tag.
-- Keep three fresh audit entries for the landscape, portrait, and small
-- fixtures so that surface is exercised on every current-date seed run.
INSERT INTO user_log(userid,action_userid,action_date,action,info) VALUES
    (9100001,9100001,CURRENT_TIMESTAMP-interval '3 hours','set_userpic',hstore('new_userpic','9100001.png')),
    (9100002,9100002,CURRENT_TIMESTAMP-interval '2 hours','set_userpic',hstore('new_userpic','9100002.png')),
    (9100003,9100003,CURRENT_TIMESTAMP-interval '1 hour','set_userpic',hstore('new_userpic','9100003.png'));

-- 982 generated topics plus the 18 hand-authored seed.sql topics = 1000.
-- Cycling over the live catalog guarantees coverage of every group in all
-- five content sections, including every forum subsection.
INSERT INTO msgbase(id,message,markup)
SELECT 9110000+n,
       E'Автоматическая тема месячного набора №'||n||E'.\n\n'
       ||CASE n%3
           WHEN 0 THEN E'Формат Markdown: **жирный текст**, @raven1000, список и `код`.\n\n- первый пункт\n- второй пункт'
           WHEN 1 THEN E'Формат LORCODE: [b]жирный текст[/b], [user]crane2000[/user] и [code]cargo test[/code]'
           ELSE E'Обычный текст без разметки.\nВторая строка проверяет перенос.'
         END,
       CASE n%3 WHEN 0 THEN 'MARKDOWN'::markup_type WHEN 1 THEN 'BBCODE_TEX'::markup_type ELSE 'PLAIN'::markup_type END
  FROM generate_series(19,1000) n;

WITH catalog AS (
    SELECT array_agg(g.id ORDER BY g.section,g.id) AS ids
      FROM groups g
      JOIN sections s ON s.id=g.section
     WHERE g.section IN (1,2,3,5,6)
), source AS (
    SELECT n,
           c.ids[1+mod(n-19,cardinality(c.ids))] AS group_id
      FROM generate_series(19,1000) n CROSS JOIN catalog c
)
INSERT INTO topics (
    id,groupid,userid,title,url,moderate,postdate,linktext,deleted,stat1,stat3,lastmod,
    commitby,notop,commitdate,postscore,postip,sticky,resolved,minor,draft,
    allow_anonymous,reactions,open_warnings
)
SELECT 9110000+s.n,s.group_id,9100001+mod(s.n-1,50),
       'Месячная тестовая тема №'||lpad(s.n::text,4,'0'),
       CASE WHEN g.section=1 AND s.n%4=0 THEN 'https://example.test/source/'||s.n ELSE NULL END,
       g.section<>2,
       CURRENT_TIMESTAMP-make_interval(secs => ((s.n-1)*2592000.0/999.0)),
       CASE WHEN g.section=1 AND s.n%4=0 THEN 'Источник' ELSE NULL END,
       (s.n IN (909,959) OR s.n%173=0),
       0,0,
       CURRENT_TIMESTAMP-make_interval(secs => ((s.n-1)*2592000.0/999.0)),
       CASE WHEN g.section<>2 THEN 9100011 ELSE NULL END,
       false,
       CASE WHEN g.section<>2 THEN CURRENT_TIMESTAMP-make_interval(secs => ((s.n-1)*2592000.0/999.0))+interval '5 minutes' ELSE NULL END,
       -9999,('198.51.100.'||(1+mod(s.n,250)))::inet,false,(s.n%41=0),false,false,true,'{}',0
  FROM source s JOIN groups g ON g.id=s.group_id;

INSERT INTO del_info(msgid,delby,reason,deldate,bonus)
SELECT t.id,9100014,'Тестовая причина удаления',t.postdate+interval '30 minutes',-5
  FROM topics t
 WHERE t.id BETWEEN 9110019 AND 9111000 AND t.deleted;

-- 4,982 generated comments plus 18 hand-authored comments = exactly 5,000.
INSERT INTO msgbase(id,message,markup)
SELECT 9120000+n,
       'Комментарий месячной фикстуры №'||n||E'.\nПроверяются ветвление, пагинация, аватары и история пользователя.'
       ||CASE n%3
           WHEN 0 THEN E' @raven1000'
           WHEN 1 THEN E' [user]crane2000[/user]'
           ELSE ''
         END,
       CASE n%3 WHEN 0 THEN 'MARKDOWN'::markup_type WHEN 1 THEN 'BBCODE_ULB'::markup_type ELSE 'PLAIN'::markup_type END
  FROM generate_series(19,5000) n;

WITH fixture_topics AS (
    SELECT array_agg(id ORDER BY id) AS ids
      FROM topics WHERE userid BETWEEN 9100001 AND 9100050
), source AS (
    SELECT n, f.ids[1+mod(n-19,cardinality(f.ids))] AS topic_id
      FROM generate_series(19,5000) n CROSS JOIN fixture_topics f
)
INSERT INTO comments(
    id,topic,userid,title,postdate,replyto,deleted,postip,editor_id,edit_date,edit_count,reactions
)
SELECT 9120000+s.n,s.topic_id,9100001+mod(s.n+7,50),
       'Re: месячная тестовая тема',
       LEAST(CURRENT_TIMESTAMP-interval '1 minute',
             GREATEST(t.postdate+interval '1 minute',
                      CURRENT_TIMESTAMP-make_interval(secs => ((s.n-1)*2592000.0/4999.0)))),
       CASE WHEN s.n>1018 THEN 9120000+s.n-1000 ELSE NULL END,
       s.n%401=0,('203.0.113.'||(1+mod(s.n,250)))::inet,NULL,NULL,0,'{}'
  FROM source s JOIN topics t ON t.id=s.topic_id;

-- Recompute the visible counters exactly as the application does.
UPDATE topics t
   SET stat1=x.visible,stat3=x.visible,lastmod=GREATEST(t.postdate,x.last_visible)
  FROM (
      SELECT t0.id,
             count(c.id) FILTER (WHERE NOT c.deleted)::integer AS visible,
             COALESCE(max(c.postdate) FILTER (WHERE NOT c.deleted),t0.postdate) AS last_visible
        FROM topics t0 LEFT JOIN comments c ON c.topic=t0.id
       WHERE t0.userid BETWEEN 9100001 AND 9100050
       GROUP BY t0.id,t0.postdate
  ) x WHERE t.id=x.id;

-- Polls on thirty generated poll topics, three variants and deterministic
-- votes. Percentages are derived by the application from these rows.
WITH selected AS (
    SELECT t.id,row_number() OVER (ORDER BY t.id)::integer AS rn
      FROM topics t JOIN groups g ON g.id=t.groupid
     WHERE t.id BETWEEN 9110019 AND 9111000 AND g.section=5
     ORDER BY t.id LIMIT 30
)
INSERT INTO polls(id,topic,multiselect)
SELECT 9130000+rn,id,rn%2=0 FROM selected;

INSERT INTO polls_variants(id,vote,label,votes)
SELECT 9131000+(p.id-9130001)*3+v.n,p.id,
       (ARRAY['Да, регулярно','Иногда','Нет'])[v.n],0
  FROM polls p CROSS JOIN generate_series(1,3) v(n)
 WHERE p.id BETWEEN 9130001 AND 9130030;

INSERT INTO vote_users(vote,userid,variant_id)
SELECT p.id,9100000+u.n,9131000+(p.id-9130001)*3+1+mod(u.n+p.id,3)
  FROM polls p CROSS JOIN generate_series(1,20) u(n)
 WHERE p.id BETWEEN 9130001 AND 9130030;
UPDATE polls_variants pv SET votes=(SELECT count(*)::integer FROM vote_users vu WHERE vu.variant_id=pv.id)
 WHERE pv.vote BETWEEN 9130001 AND 9130030;

-- Representative media for both gallery groups (screenshots/workplaces are
-- already explicitly covered by seed.sql; this adds month-scale pagination).
WITH selected AS (
    SELECT t.id,row_number() OVER (ORDER BY t.id)::integer AS rn
      FROM topics t JOIN groups g ON g.id=t.groupid
     WHERE t.id BETWEEN 9110019 AND 9111000 AND g.section=3
     ORDER BY t.id LIMIT 60
)
INSERT INTO images(id,topic,deleted,extension,main)
SELECT 9140000+rn,id,false,'png',true FROM selected;

-- Tags are stable and reused instead of creating one value per bulk topic.
INSERT INTO tags_values(value,counter) VALUES
    ('месячная фикстура',0),('нагрузочный тест',0),('аватар',0)
ON CONFLICT(value) DO NOTHING;
INSERT INTO tags(msgid,tagid)
SELECT t.id,tv.id
  FROM topics t
  JOIN tags_values tv ON tv.value=(ARRAY['месячная фикстура','нагрузочный тест','аватар'])[1+mod(t.id,3)]
 WHERE t.id BETWEEN 9110019 AND 9111000
ON CONFLICT DO NOTHING;
UPDATE tags_values tv SET counter=(SELECT count(*)::integer FROM tags t WHERE t.tagid=tv.id)
 WHERE tv.value IN ('месячная фикстура','нагрузочный тест','аватар');

-- Crane receives enough watched topics to exercise both pagination directions.
INSERT INTO memories(userid,topic,add_date,watch)
SELECT 9100009,t.id,CURRENT_TIMESTAMP-make_interval(hours => (row_number() OVER (ORDER BY t.id))::integer),true
  FROM topics t
 WHERE t.id BETWEEN 9110019 AND 9111000 AND NOT t.deleted AND t.userid<>9100009
 ORDER BY t.id LIMIT 45;

-- Reactions made by crane and reactions received by crane are separate stable
-- sets, matching the two original profile modes.
INSERT INTO reactions_log(origin_user,topic_id,comment_id,set_date,reaction)
SELECT 9100009,t.id,NULL,CURRENT_TIMESTAMP-make_interval(mins => (row_number() OVER (ORDER BY t.id))::integer),'👍'
  FROM topics t
 WHERE t.id BETWEEN 9110019 AND 9111000 AND NOT t.deleted AND t.userid<>9100009
 ORDER BY t.id LIMIT 100;

WITH targets AS (
    SELECT t.id,row_number() OVER (ORDER BY t.id) AS trn
      FROM topics t
     WHERE t.id BETWEEN 9110019 AND 9111000 AND NOT t.deleted AND t.userid=9100009
), candidates AS (
    SELECT t.id,u.id AS origin_id,row_number() OVER (ORDER BY t.id,u.id) AS rn
      FROM targets t CROSS JOIN users u
     WHERE u.id BETWEEN 9100001 AND 9100050 AND u.id<>9100009
     ORDER BY t.id,u.id LIMIT 100
)
INSERT INTO reactions_log(origin_user,topic_id,comment_id,set_date,reaction)
SELECT origin_id,id,NULL,CURRENT_TIMESTAMP-make_interval(mins => (120+rn)::integer),'🔥' FROM candidates;

UPDATE topics t SET reactions=x.payload
  FROM (
    SELECT rl.topic_id,jsonb_object_agg(rl.origin_user::text,rl.reaction) AS payload
      FROM reactions_log rl
     WHERE rl.topic_id BETWEEN 9110019 AND 9111000 AND rl.comment_id IS NULL
     GROUP BY rl.topic_id
  ) x WHERE t.id=x.topic_id;

-- Notification events from reaction rows plus one sample of every other event
-- type exposed by UserEventFilterEnum.
INSERT INTO user_events(userid,type,private,event_date,message_id,comment_id,message,unread,origin_user)
SELECT target.userid,'REACTION',true,rl.set_date,rl.topic_id,NULL,rl.reaction,true,rl.origin_user
  FROM reactions_log rl JOIN topics target ON target.id=rl.topic_id
 WHERE rl.topic_id BETWEEN 9110019 AND 9111000 AND target.userid<>rl.origin_user;

WITH sample AS (
    SELECT min(t.id) FILTER (WHERE t.userid<>9100009 AND NOT t.deleted) AS ordinary_topic,
           min(t.id) FILTER (WHERE t.userid=9100009 AND NOT t.deleted) AS crane_topic,
           min(t.id) FILTER (WHERE t.deleted) AS deleted_topic
      FROM topics t WHERE t.id BETWEEN 9110019 AND 9111000
)
INSERT INTO user_events(userid,type,private,event_date,message_id,comment_id,message,unread,origin_user)
SELECT 9100009,e.type::event_type,true,CURRENT_TIMESTAMP-make_interval(mins => e.age),
       CASE WHEN e.type='DEL' THEN s.deleted_topic ELSE s.ordinary_topic END,NULL,e.message,true,9100013
  FROM sample s CROSS JOIN (VALUES
    ('REPLY',201,'Ответ на сообщение'),('WATCH',202,'Новое в отслеживаемой теме'),
    ('WATCH',203,'Ещё один ответ в отслеживаемой теме'),('DEL',204,'4.1 Тестовое удаление'),
    ('REF',205,'Упоминание @crane2000'),('TAG',206,'месячная фикстура'),
    ('WARNING',207,'Тестовое предупреждение модератора')
  ) e(type,age,message);

UPDATE users u SET unread_events=(SELECT count(*)::integer FROM user_events e WHERE e.userid=u.id AND e.unread)
 WHERE u.id BETWEEN 9100001 AND 9100050;

-- Contract assertions: fail the seed transaction instead of silently
-- producing a partial benchmark.
DO $$
DECLARE
    iUsers integer;
    iTopics integer;
    iComments integer;
    iMissingGroups integer;
    bDatesInWindow boolean;
    bDatesSpanMonth boolean;
BEGIN
    SELECT count(*) INTO iUsers FROM users WHERE id BETWEEN 9100001 AND 9100050;
    SELECT count(*) INTO iTopics FROM topics WHERE userid BETWEEN 9100001 AND 9100050;
    SELECT count(*) INTO iComments FROM comments WHERE userid BETWEEN 9100001 AND 9100050;
    SELECT count(*) INTO iMissingGroups
      FROM groups g JOIN sections s ON s.id=g.section
     WHERE s.id IN (1,2,3,5,6)
       AND NOT EXISTS (SELECT 1 FROM topics t WHERE t.groupid=g.id AND t.userid BETWEEN 9100001 AND 9100050);
    SELECT max(postdate)<=CURRENT_TIMESTAMP
           AND min(postdate)>=CURRENT_TIMESTAMP-interval '31 days'
      INTO bDatesInWindow
      FROM topics WHERE userid BETWEEN 9100001 AND 9100050;
    SELECT max(postdate)-min(postdate)>=interval '29 days'
      INTO bDatesSpanMonth
      FROM topics WHERE userid BETWEEN 9100001 AND 9100050;
    IF iUsers<>50 OR iTopics<>1000 OR iComments<>5000 OR iMissingGroups<>0
       OR NOT bDatesInWindow OR NOT bDatesSpanMonth THEN
        RAISE EXCEPTION 'month fixture contract failed: users %, topics %, comments %, missing groups %, date window %, month span %',
            iUsers,iTopics,iComments,iMissingGroups,bDatesInWindow,bDatesSpanMonth;
    END IF;
END
$$;

SELECT setval('s_uid',GREATEST((SELECT max(id) FROM users),1),true);
SELECT setval('s_msgid',GREATEST((SELECT max(id)::bigint FROM msgbase),1),true);
SELECT setval('vote_id',GREATEST((SELECT max(id) FROM polls),1),true);
SELECT setval('votes_id',GREATEST((SELECT max(id) FROM polls_variants),1),true);
-- `seed.py` materializes the entire deterministic 9140001..9140060 media
-- range even when the current group distribution yields fewer than 60
-- gallery rows.  Reserve that filesystem-owned range in the sequence too;
-- otherwise the next browser upload can select an unused database id whose
-- directory already belongs to the fixture and fail in Files.createDirectory.
SELECT setval(
    pg_get_serial_sequence('images','id'),
    GREATEST((SELECT max(id) FROM images),9140060,1),
    true
);
SELECT setval(pg_get_serial_sequence('memories','id'),GREATEST((SELECT max(id) FROM memories),1),true);
SELECT setval(pg_get_serial_sequence('user_events','id'),GREATEST((SELECT max(id) FROM user_events),1),true);
SELECT stat_update2();

-- The forum JSP displays groups.stat3 verbatim.  This is the exact invariant
-- established by Java StatUpdater/stat_update2: recent topic comment counts
-- cover two days, while newly-created topics are added for one day.  Keep the
-- fixture transaction from committing a misleading forum index.
DO $$
DECLARE
    iMismatchedGroups integer;
BEGIN
    WITH expected AS (
        SELECT g.id,
               COALESCE(sum(t.stat3) FILTER (
                   WHERE NOT t.deleted
                     AND t.lastmod>CURRENT_TIMESTAMP-'2 days'::interval
               ),0)::bigint
               + count(t.id) FILTER (
                   WHERE NOT t.deleted
                     AND t.postdate>CURRENT_TIMESTAMP-'1 day'::interval
               )::bigint AS stat3
          FROM groups g
          LEFT JOIN topics t ON t.groupid=g.id
         GROUP BY g.id
    )
    SELECT count(*)
      INTO iMismatchedGroups
      FROM groups g
      JOIN expected e USING(id)
     WHERE g.stat3::bigint<>e.stat3;

    IF iMismatchedGroups<>0 THEN
        RAISE EXCEPTION 'stat_update2 fixture contract failed for % groups',
            iMismatchedGroups;
    END IF;
END
$$;

SELECT update_monthly_stats();
COMMIT;
