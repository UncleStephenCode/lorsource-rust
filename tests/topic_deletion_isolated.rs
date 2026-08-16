#![allow(dead_code)]

// Keep a repository/service proof independent from the HTTP route wiring.
// The harness compiles the production layers in their real crate hierarchy
// and exercises them in a disposable schema without fixed IDs or cleanup in
// the canonical public schema.

mod error {
    #[derive(Debug, thiserror::Error)]
    pub enum AppError {
        #[error("database error: {0}")]
        Sqlx(#[from] sqlx::Error),
        #[error("internal error: {0}")]
        Anyhow(#[from] anyhow::Error),
    }

    pub type Result<T> = std::result::Result<T, AppError>;
}

mod domain {
    pub mod topic {
        pub mod options {
            use async_trait::async_trait;

            use crate::error::Result;

            #[async_trait]
            pub trait TrTopicReindexQueue: Send + Sync {
                async fn vUpdateMessage(&self, iTopicId: i32, bWithComments: bool) -> Result<()>;
            }
        }

        pub mod deletion {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/domain/topic/deletion.rs"
            ));
        }
    }
}

mod application {
    pub mod topic {
        pub mod deletion {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/application/topic/deletion.rs"
            ));
        }
    }
}

mod infra {
    pub mod postgres {
        pub mod topic_deletion_repository {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/infra/postgres/topic_deletion_repository.rs"
            ));
        }
    }
}

#[cfg(test)]
mod database_delta_tests {
    use chrono::{DateTime, TimeZone, Utc};
    use sqlx::{PgPool, postgres::PgPoolOptions};

    use crate::{
        domain::topic::deletion::{
            StDeleteTopicCommand, StTopicDeletionActor, TrTopicDeletionRepository,
        },
        infra::postgres::topic_deletion_repository::CTopicDeletionPgRepository,
    };

    async fn vExecute(oPool: &PgPool, sSql: &str) -> anyhow::Result<()> {
        // Dynamic SQL is limited to a UUID-hex schema name generated below;
        // no request or fixture input can reach these isolated DDL strings.
        sqlx::query(sqlx::AssertSqlSafe(sSql.to_owned()))
            .execute(oPool)
            .await?;
        Ok(())
    }

    async fn vCreateFixture(oPool: &PgPool) -> anyhow::Result<()> {
        for sSql in [
            r#"CREATE TABLE sections(
                   id integer PRIMARY KEY,
                   name text NOT NULL,
                   moderate boolean NOT NULL,
                   vote boolean,
                   imagepost boolean NOT NULL,
                   imageallowed boolean NOT NULL,
                   havelink boolean NOT NULL,
                   expire interval NOT NULL
               )"#,
            r#"CREATE TABLE groups(
                   id integer PRIMARY KEY,
                   title text NOT NULL,
                   urlname text NOT NULL,
                   section integer NOT NULL REFERENCES sections(id),
                   stat3 integer NOT NULL DEFAULT 0
               )"#,
            r#"CREATE TABLE users(
                   id integer PRIMARY KEY,
                   nick text NOT NULL,
                   score integer,
                   max_score integer,
                   blocked boolean,
                   passwd text,
                   frozen_until timestamptz,
                   unread_events integer NOT NULL DEFAULT 0
               )"#,
            r#"CREATE TABLE msgbase(
                   id integer PRIMARY KEY,
                   message text NOT NULL,
                   markup text NOT NULL
               )"#,
            r#"CREATE TABLE topics(
                   id integer PRIMARY KEY REFERENCES msgbase(id),
                   userid integer NOT NULL REFERENCES users(id),
                   title text NOT NULL,
                   url text,
                   linktext text,
                   groupid integer NOT NULL REFERENCES groups(id),
                   deleted boolean NOT NULL,
                   draft boolean NOT NULL,
                   moderate boolean NOT NULL,
                   sticky boolean NOT NULL,
                   resolved boolean NOT NULL,
                   stat1 integer NOT NULL,
                   postdate timestamptz NOT NULL,
                   commitdate timestamptz,
                   lastmod timestamptz NOT NULL,
                   postscore integer,
                   minor boolean NOT NULL,
                   postip inet,
                   ua_id integer
               )"#,
            r#"CREATE TABLE memories(
                   userid integer NOT NULL REFERENCES users(id),
                   topic integer NOT NULL REFERENCES topics(id),
                   PRIMARY KEY(userid,topic)
               )"#,
            r#"CREATE TABLE del_info(
                   msgid integer PRIMARY KEY,
                   delby integer NOT NULL REFERENCES users(id),
                   reason text,
                   deldate timestamptz,
                   bonus integer
               )"#,
            r#"CREATE TABLE user_events(
                   id bigserial PRIMARY KEY,
                   userid integer NOT NULL REFERENCES users(id),
                   type text NOT NULL,
                   private boolean NOT NULL,
                   message_id integer REFERENCES topics(id),
                   comment_id integer,
                   message text,
                   unread boolean NOT NULL DEFAULT true
               )"#,
            r#"CREATE FUNCTION topins() RETURNS trigger LANGUAGE plpgsql AS $$
               BEGIN
                 UPDATE groups SET stat3=stat3+1 WHERE id=NEW.groupid;
                 UPDATE topics SET lastmod=CURRENT_TIMESTAMP WHERE id=NEW.id;
                 IF NEW.userid != 2 THEN
                   INSERT INTO memories(userid,topic) VALUES(NEW.userid,NEW.id);
                 END IF;
                 RETURN NULL;
               END $$"#,
            "CREATE TRIGGER topins_t AFTER INSERT ON topics FOR EACH ROW EXECUTE FUNCTION topins()",
            r#"CREATE FUNCTION msgdel() RETURNS trigger LANGUAGE plpgsql AS $$
               BEGIN
                 UPDATE topics SET lastmod=CURRENT_TIMESTAMP WHERE id=NEW.msgid;
                 RETURN NULL;
               END $$"#,
            "CREATE TRIGGER msgdel_t AFTER INSERT ON del_info FOR EACH ROW EXECUTE FUNCTION msgdel()",
            r#"CREATE FUNCTION msgundel() RETURNS trigger LANGUAGE plpgsql AS $$
               BEGIN
                 UPDATE topics SET lastmod=CURRENT_TIMESTAMP WHERE id=OLD.msgid;
                 RETURN NULL;
               END $$"#,
            "CREATE TRIGGER msgundel_t AFTER DELETE ON del_info FOR EACH ROW EXECUTE FUNCTION msgundel()",
            r#"CREATE FUNCTION new_event() RETURNS trigger LANGUAGE plpgsql AS $$
               BEGIN
                 UPDATE users SET unread_events=unread_events+1 WHERE id=NEW.userid;
                 RETURN NULL;
               END $$"#,
            "CREATE TRIGGER new_event_t AFTER INSERT ON user_events FOR EACH ROW EXECUTE FUNCTION new_event()",
            r#"INSERT INTO sections(id,name,moderate,vote,imagepost,imageallowed,havelink,expire)
               VALUES(2,'Форум',false,false,false,false,true,interval '30 days')"#,
            "INSERT INTO groups(id,title,urlname,section,stat3) VALUES(10,'General','general',2,0)",
            r#"INSERT INTO users(id,nick,score,max_score,blocked,passwd,frozen_until,unread_events) VALUES
                 (2,'anonymous',0,0,false,'',NULL,0),
                 (7,'author',5,100,false,'hash',NULL,0),
                 (8,'moderator',1000,1000,false,'hash',NULL,0),
                 (9,'watcher',100,100,false,'hash',NULL,0),
                 (10,'race-author',100,100,false,'hash',NULL,0),
                 (11,'conflict-author',50,50,false,'hash',NULL,0)"#,
            r#"INSERT INTO msgbase(id,message,markup) VALUES
                 (42,'body 42','MARKDOWN'),
                 (43,'body 43','PLAIN'),
                 (44,'body 44','BBCODE_TEX'),
                 (45,'body 45','MARKDOWN'),
                 (46,'body 46','MARKDOWN')"#,
            r#"INSERT INTO topics(
                   id,userid,title,url,linktext,groupid,deleted,draft,moderate,sticky,
                   resolved,stat1,postdate,commitdate,lastmod,postscore,minor,postip,ua_id
               ) VALUES
                 (42,7,'topic 42','https://example.test/42','details',10,false,false,true,true,
                  false,0,CURRENT_TIMESTAMP-interval '1 hour',CURRENT_TIMESTAMP-interval '1 hour',
                  '2001-01-01 00:00:00+00',-9999,false,'127.0.0.1',1),
                 (43,10,'topic 43',NULL,NULL,10,false,false,true,true,
                  false,0,CURRENT_TIMESTAMP-interval '1 hour',CURRENT_TIMESTAMP-interval '1 hour',
                  '2001-01-01 00:00:00+00',-9999,false,'127.0.0.2',2),
                 (44,11,'topic 44',NULL,NULL,10,false,false,true,true,
                  false,0,CURRENT_TIMESTAMP-interval '1 hour',CURRENT_TIMESTAMP-interval '1 hour',
                  '2001-01-01 00:00:00+00',-9999,false,'127.0.0.3',3),
                 (45,11,'topic 45',NULL,NULL,10,true,false,true,false,
                  false,0,CURRENT_TIMESTAMP-interval '1 hour',CURRENT_TIMESTAMP-interval '1 hour',
                  '2001-01-01 00:00:00+00',-9999,false,'127.0.0.4',4),
                 (46,2,'topic 46',NULL,NULL,10,false,false,true,true,
                  false,0,CURRENT_TIMESTAMP-interval '1 hour',CURRENT_TIMESTAMP-interval '1 hour',
                  '2001-01-01 00:00:00+00',-9999,false,'127.0.0.5',5)"#,
            // topins_t intentionally changes lastmod and creates memories;
            // reset only the fixture timestamps after proving those canonical
            // insert effects below.
            "UPDATE topics SET lastmod='2001-01-01 00:00:00+00'",
            "INSERT INTO del_info(msgid,delby,reason,deldate,bonus) VALUES(44,8,'existing','2026-08-01 00:00:00+00',-3)",
            "UPDATE topics SET lastmod='2001-01-01 00:00:00+00' WHERE id=44",
            r#"INSERT INTO user_events(userid,type,private,message_id,message) VALUES
                 (7,'TAG',false,42,'tag'),
                 (9,'WATCH',false,42,'watch'),
                 (9,'DEL',true,42,'old notification'),
                 (9,'WATCH',false,43,'race sentinel')"#,
        ] {
            vExecute(oPool, sSql).await?;
        }
        Ok(())
    }

    async fn stExerciseDatabaseDeltas(oPool: &PgPool) -> anyhow::Result<()> {
        let oRepository = CTopicDeletionPgRepository::new(oPool.clone());
        let stModerator = StTopicDeletionActor {
            iUserId: 8,
            sNick: "moderator",
            bModerator: true,
            bAdministrator: false,
        };
        let dtFixtureLastMod = Utc.with_ymd_and_hms(2001, 1, 1, 0, 0, 0).unwrap();

        // The disposable fixture includes the canonical topins_t behavior so
        // test creation/cleanup cannot silently leak group counters or
        // auto-created memories as fixed-ID tests previously did.
        let stInsertEffects: (i32, i64, i64) = sqlx::query_as(
            "SELECT (SELECT stat3 FROM groups WHERE id=10), \
                    (SELECT count(*) FROM memories), \
                    (SELECT count(*) FROM memories WHERE userid=2)",
        )
        .fetch_one(oPool)
        .await?;
        anyhow::ensure!(
            stInsertEffects == (5, 4, 0),
            "topins_t group/memory fixture contract changed: {stInsertEffects:?}"
        );

        let stTopic = oRepository
            .optSnapshot(42)
            .await?
            .ok_or_else(|| anyhow::anyhow!("topic 42 snapshot"))?;
        anyhow::ensure!(
            stTopic.sCanonicalUrl() == "/forum/general/42"
                && stTopic.sStoredTitle == "topic 42"
                && stTopic.sMessage == "body 42"
                && stTopic.sMarkup == "MARKDOWN"
                && stTopic.optUrl.as_deref() == Some("https://example.test/42")
                && stTopic.optLinkText.as_deref() == Some("details")
                && stTopic.sPostIp == "127.0.0.1"
                && stTopic.iUserAgentId == 1,
            "full undelete form snapshot was not loaded"
        );
        let stMutation = oRepository
            .stDelete(
                stModerator,
                &stTopic,
                &StDeleteTopicCommand {
                    iTopicId: 42,
                    sReason: "4.6 Спам".into(),
                    iPenalty: 20,
                },
            )
            .await?;
        anyhow::ensure!(
            stMutation.bDeleted && stMutation.iAppliedScoreDelta == -20,
            "delete mutation result differs"
        );
        let stDeleted: (bool, bool, DateTime<Utc>, i32, i32, String, i32) = sqlx::query_as(
            "SELECT t.deleted,t.sticky,t.lastmod,u.score,di.delby,di.reason,di.bonus \
               FROM topics t JOIN users u ON u.id=t.userid \
               JOIN del_info di ON di.msgid=t.id WHERE t.id=42",
        )
        .fetch_one(oPool)
        .await?;
        anyhow::ensure!(stDeleted.0 && !stDeleted.1, "delete/sticky delta");
        anyhow::ensure!(stDeleted.2 > dtFixtureLastMod, "msgdel_t lastmod delta");
        anyhow::ensure!(
            (stDeleted.3, stDeleted.4, stDeleted.5.as_str(), stDeleted.6)
                == (-15, 8, "4.6 Спам", -20),
            "additive score/plain del_info delta: {stDeleted:?}"
        );
        let stEvents: (i64, i64, i32, i32) = sqlx::query_as(
            "SELECT \
               (SELECT count(*) FROM user_events WHERE message_id=42 AND type IN ('TAG','REF','REPLY','WATCH','REACTION','WARNING')), \
               (SELECT count(*) FROM user_events WHERE message_id=42 AND type='DEL'), \
               (SELECT unread_events FROM users WHERE id=7), \
               (SELECT unread_events FROM users WHERE id=9)",
        )
        .fetch_one(oPool)
        .await?;
        anyhow::ensure!(
            stEvents == (0, 2, 1, 2),
            "event cleanup/DEL notification/new_event_t delta: {stEvents:?}"
        );

        // Stale controller snapshot: once another request has deleted the
        // row, absolutely every database side effect is skipped.
        let stRaceTopic = oRepository
            .optSnapshot(43)
            .await?
            .ok_or_else(|| anyhow::anyhow!("topic 43 snapshot"))?;
        vExecute(oPool, "UPDATE topics SET deleted=true WHERE id=43").await?;
        let stRaceMutation = oRepository
            .stDelete(
                stModerator,
                &stRaceTopic,
                &StDeleteTopicCommand {
                    iTopicId: 43,
                    sReason: "race".into(),
                    iPenalty: 10,
                },
            )
            .await?;
        let stRaceDelta: (i32, i64, i64, DateTime<Utc>) = sqlx::query_as(
            "SELECT (SELECT score FROM users WHERE id=10), \
                    (SELECT count(*) FROM del_info WHERE msgid=43), \
                    (SELECT count(*) FROM user_events WHERE message_id=43), \
                    (SELECT lastmod FROM topics WHERE id=43)",
        )
        .fetch_one(oPool)
        .await?;
        anyhow::ensure!(
            !stRaceMutation.bDeleted && stRaceDelta == (100, 0, 1, dtFixtureLastMod),
            "lost-race side effects were not conditional: {stRaceDelta:?}"
        );

        // A pre-existing del_info row must fail the plain INSERT and roll the
        // preceding topic/score writes back atomically.
        let stConflictTopic = oRepository
            .optSnapshot(44)
            .await?
            .ok_or_else(|| anyhow::anyhow!("topic 44 snapshot"))?;
        anyhow::ensure!(
            oRepository
                .stDelete(
                    stModerator,
                    &stConflictTopic,
                    &StDeleteTopicCommand {
                        iTopicId: 44,
                        sReason: "replacement".into(),
                        iPenalty: 9,
                    },
                )
                .await
                .is_err(),
            "plain del_info conflict unexpectedly succeeded"
        );
        let stConflictDelta: (bool, bool, i32, String, i32) = sqlx::query_as(
            "SELECT t.deleted,t.sticky,u.score,di.reason,di.bonus \
               FROM topics t JOIN users u ON u.id=t.userid \
               JOIN del_info di ON di.msgid=t.id WHERE t.id=44",
        )
        .fetch_one(oPool)
        .await?;
        anyhow::ensure!(
            stConflictDelta == (false, true, 50, "existing".into(), -3),
            "del_info conflict did not roll back: {stConflictDelta:?}"
        );

        // Java deliberately does not apply the frozen-user exemption to the
        // anonymous ID; preserve even the resulting negative score.
        let stAnonymousTopic = oRepository
            .optSnapshot(46)
            .await?
            .ok_or_else(|| anyhow::anyhow!("topic 46 snapshot"))?;
        let stAnonymousMutation = oRepository
            .stDelete(
                stModerator,
                &stAnonymousTopic,
                &StDeleteTopicCommand {
                    iTopicId: 46,
                    sReason: "anonymous".into(),
                    iPenalty: 7,
                },
            )
            .await?;
        let stAnonymousDelta: (i32, i32, i64) = sqlx::query_as(
            "SELECT (SELECT score FROM users WHERE id=2), \
                    (SELECT bonus FROM del_info WHERE msgid=46), \
                    (SELECT count(*) FROM user_events WHERE message_id=46 AND type='DEL')",
        )
        .fetch_one(oPool)
        .await?;
        anyhow::ensure!(
            stAnonymousMutation.iAppliedScoreDelta == -7 && stAnonymousDelta == (-7, -7, 0),
            "anonymous score/notification contract: {stAnonymousDelta:?}"
        );

        // Undelete reverses the stored negative bonus additively, leaves
        // sticky=false, keeps DEL events, and relies on msgundel_t for lastmod.
        vExecute(
            oPool,
            "UPDATE topics SET lastmod='2001-01-01 00:00:00+00' WHERE id=42",
        )
        .await?;
        let stDeletedTopic = oRepository
            .optSnapshot(42)
            .await?
            .ok_or_else(|| anyhow::anyhow!("deleted topic 42 snapshot"))?;
        oRepository.vUndelete(&stDeletedTopic).await?;
        let stUndeleted: (bool, bool, DateTime<Utc>, i32, i64, i64) = sqlx::query_as(
            "SELECT t.deleted,t.sticky,t.lastmod,u.score, \
                    (SELECT count(*) FROM del_info WHERE msgid=42), \
                    (SELECT count(*) FROM user_events WHERE message_id=42 AND type='DEL') \
               FROM topics t JOIN users u ON u.id=t.userid WHERE t.id=42",
        )
        .fetch_one(oPool)
        .await?;
        anyhow::ensure!(
            !stUndeleted.0
                && !stUndeleted.1
                && stUndeleted.2 > dtFixtureLastMod
                && stUndeleted.3 == 5
                && stUndeleted.4 == 0
                && stUndeleted.5 == 2,
            "undelete delta: {stUndeleted:?}"
        );

        // TopicDao.undelete still updates a stale deleted snapshot without a
        // del_info row, but no msgundel_t means no lastmod side effect.
        let stNoInfoTopic = oRepository
            .optSnapshot(45)
            .await?
            .ok_or_else(|| anyhow::anyhow!("topic 45 snapshot"))?;
        oRepository.vUndelete(&stNoInfoTopic).await?;
        let stNoInfoDelta: (bool, DateTime<Utc>) =
            sqlx::query_as("SELECT deleted,lastmod FROM topics WHERE id=45")
                .fetch_one(oPool)
                .await?;
        anyhow::ensure!(
            stNoInfoDelta == (false, dtFixtureLastMod),
            "no-del_info undelete must not synthesize lastmod: {stNoInfoDelta:?}"
        );

        let stUnrelated: (i32, i64) = sqlx::query_as(
            "SELECT (SELECT stat3 FROM groups WHERE id=10),(SELECT count(*) FROM memories)",
        )
        .fetch_one(oPool)
        .await?;
        anyhow::ensure!(
            stUnrelated == (5, 4),
            "delete/undelete changed topins_t-owned state: {stUnrelated:?}"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires an explicitly selected disposable PostgreSQL database"]
    async fn topic_delete_transactions_match_java_in_an_isolated_uuid_schema() {
        assert_eq!(
            std::env::var("LOR_TOPIC_DELETION_DB_INTEGRATION_CONFIRM").as_deref(),
            Ok("isolated-schema"),
            "set LOR_TOPIC_DELETION_DB_INTEGRATION_CONFIRM=isolated-schema"
        );
        let sDatabaseUrl = std::env::var("LOR_TOPIC_DELETION_DB_INTEGRATION_DATABASE_URL")
            .expect("set LOR_TOPIC_DELETION_DB_INTEGRATION_DATABASE_URL to a disposable database");
        let oPool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&sDatabaseUrl)
            .await
            .expect("disposable PostgreSQL database must be reachable");
        let sSchema = format!("topic_deletion_{}", uuid::Uuid::new_v4().simple());
        vExecute(&oPool, &format!("CREATE SCHEMA {sSchema}"))
            .await
            .expect("temporary schema must be creatable");
        vExecute(&oPool, &format!("SET search_path TO {sSchema}"))
            .await
            .expect("temporary schema must be selectable");

        let stResult = async {
            vCreateFixture(&oPool).await?;
            stExerciseDatabaseDeltas(&oPool).await
        }
        .await;

        let stResetResult = vExecute(&oPool, "SET search_path TO public").await;
        let stDropResult = vExecute(&oPool, &format!("DROP SCHEMA {sSchema} CASCADE")).await;
        stResetResult.expect("search_path cleanup must succeed");
        stDropResult.expect("temporary schema cleanup must succeed");
        stResult.expect("topic deletion database delta contract");
    }
}
