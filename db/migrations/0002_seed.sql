INSERT INTO sections(id,name,moderate,imagepost,preformat,havelink,vote,add_info) VALUES
  (1,'Новости',false,false,false,true,false,'Новости свободного ПО, Linux и IT'),
  (2,'Форум',false,false,false,false,false,'Форумные разделы'),
  (3,'Галерея',false,true,false,false,false,'Изображения и скриншоты'),
  (4,'Статьи',true,false,false,true,false,'Статьи и длинные материалы'),
  (5,'Опросы',false,false,false,false,true,'Опросы сообщества')
ON CONFLICT(id) DO NOTHING;

INSERT INTO groups(id,title,section,urlname,info,resolvable) VALUES
  (1,'Linux',2,'linux','Обсуждение Linux и дистрибутивов',true),
  (2,'General',2,'general','Общие темы',false),
  (3,'Desktop',2,'desktop','Рабочие окружения и графика',true),
  (4,'Open Source',1,'opensource','Новости свободного ПО',false),
  (5,'Security',1,'security','Безопасность',false),
  (6,'Articles',4,'articles','Статьи пользователей',false),
  (7,'Screenshots',3,'screenshots','Скриншоты и изображения',false),
  (8,'Polls',5,'polls','Опросы',false)
ON CONFLICT(id) DO NOTHING;

INSERT INTO users(id,nick,name,email,canmod,candel,score,max_score,regdate,activated,corrector,userinfo) VALUES
  (1,'admin','Rust Admin','admin@example.test',true,true,100,100,now(),true,true,'<p>Системный пользователь dev-сборки.</p>'),
  (2,'unclestephen','Demo User','demo@example.test',false,false,42,42,now(),true,false,'<p>Демо-профиль для проверки портированного движка.</p>')
ON CONFLICT(id) DO NOTHING;

INSERT INTO msgbase(id,message,bbcode) VALUES
  (100,'Добро пожаловать в экспериментальный порт LOR-движка на Rust.\n\nПоддержаны разделы, группы, темы, комментарии, метки, RSS и часть старых JSP-маршрутов.',true),
  (101,'[b]Axum + SQLx + Askama[/b] дают простой асинхронный каркас. Старые Scala/Spring-контроллеры разложены по Rust-модулям routes/*.rs.',true),
  (102,'Это комментарий к первой теме. Можно добавить новые через форму ниже.',true)
ON CONFLICT(id) DO NOTHING;

INSERT INTO topics(id,groupid,userid,title,url,postdate,linktext,stat1,stat2,lastmod,sticky,resolved) VALUES
  (100,4,1,'Rust-порт LOR-движка: первый запуск','https://www.rust-lang.org/',now()-interval '2 hours','Rust',1,120,now()-interval '1 hour',true,false),
  (101,1,2,'Как устроен новый каркас',NULL,now()-interval '1 hour',NULL,0,57,now()-interval '1 hour',false,true)
ON CONFLICT(id) DO NOTHING;

INSERT INTO comments(id,topic,userid,title,postdate,replyto,deleted) VALUES
  (102,100,2,'Комментарий',now()-interval '50 minutes',NULL,false)
ON CONFLICT(id) DO NOTHING;

INSERT INTO tags_values(id,value,counter) VALUES
  (1,'rust',2), (2,'lor',2), (3,'axum',1), (4,'linux',1)
ON CONFLICT(id) DO NOTHING;

INSERT INTO tags(msgid,tagid) VALUES
  (100,1),(100,2),(100,3),(101,1),(101,2),(101,4)
ON CONFLICT DO NOTHING;

SELECT setval('s_uid', GREATEST((SELECT max(id) FROM users), 10), true);
SELECT setval('s_msgid', GREATEST((SELECT max(id) FROM msgbase), 100), true);
