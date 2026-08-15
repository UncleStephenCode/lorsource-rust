#![allow(dead_code)]

// This harness keeps exercising the moderation layers without bootstrapping
// the full web application. It complements (rather than substitutes for) the
// registered route and Docker gates.

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

        pub mod moderation {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/domain/topic/moderation.rs"
            ));
        }
    }
}

mod application {
    pub mod topic {
        pub mod moderation {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/application/topic/moderation.rs"
            ));
        }
    }
}

mod infra {
    pub mod postgres {
        pub mod topic_moderation_repository {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/infra/postgres/topic_moderation_repository.rs"
            ));
        }
    }
}

#[cfg(test)]
mod database_delta_tests {
    use chrono::{DateTime, Duration, Utc};
    use sqlx::{PgPool, postgres::PgPoolOptions};

    use crate::{
        domain::topic::moderation::{
            EnTopicMarkup, StMoveTopicCommand, TrTopicModerationRepository, sMoveInfo,
        },
        infra::postgres::topic_moderation_repository::CTopicModerationPgRepository,
    };

    async fn vExecute(oPool: &PgPool, sSql: &str) -> anyhow::Result<()> {
        // This test helper receives only literals above or a schema name
        // generated from UUID hexadecimal digits; no request/user input can
        // enter these deliberately isolated DDL statements.
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
                   havelink boolean NOT NULL,
                   expire interval NOT NULL
               )"#,
            r#"CREATE TABLE groups(
                   id integer PRIMARY KEY,
                   title text NOT NULL,
                   urlname text NOT NULL,
                   section integer NOT NULL REFERENCES sections(id),
                   resolvable boolean NOT NULL
               )"#,
            r#"CREATE TABLE users(
                   id integer PRIMARY KEY,
                   nick text NOT NULL,
                   score integer,
                   blocked boolean
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
                   moderate boolean NOT NULL,
                   sticky boolean NOT NULL,
                   commitby integer,
                   commitdate timestamptz,
                   postdate timestamptz NOT NULL,
                   lastmod timestamptz NOT NULL,
                   resolved boolean NOT NULL
               )"#,
            "CREATE TABLE edit_info(marker text NOT NULL)",
            "CREATE TABLE user_events(marker text NOT NULL)",
            r#"INSERT INTO sections(id,name,moderate,vote,havelink,expire) VALUES
                   (2,'Форум',false,false,true,interval '30 days'),
                   (6,'Статьи',false,false,false,interval '30 days')"#,
            r#"INSERT INTO groups(id,title,urlname,section,resolvable) VALUES
                   (10,'Old','old-group',2,true),
                   (20,'Target','target-group',6,true)"#,
            "INSERT INTO users(id,nick,score,blocked) VALUES(7,'author',300,false)",
            r#"INSERT INTO msgbase(id,message,markup) VALUES
                   (42,'original','MARKDOWN'),
                   (43,'racing','PLAIN')"#,
            r#"INSERT INTO topics(
                   id,userid,title,url,linktext,groupid,deleted,moderate,sticky,
                   commitby,commitdate,postdate,lastmod,resolved
               ) VALUES
                   (42,7,'title','https://example.test/a','details',10,false,true,false,
                    7,'2026-08-01 09:00:00+00','2026-08-01 08:00:00+00',
                    '2026-08-01 10:00:00+00',false),
                   (43,7,'race','https://example.test/race','race details',20,false,true,false,
                    7,'2026-08-01 09:00:00+00','2026-08-01 08:00:00+00',
                    '2026-08-01 10:00:00+00',false)"#,
            "INSERT INTO edit_info(marker) VALUES('untouched')",
            "INSERT INTO user_events(marker) VALUES('untouched')",
        ] {
            vExecute(oPool, sSql).await?;
        }
        Ok(())
    }

    async fn stExerciseDatabaseDeltas(oPool: &PgPool) -> anyhow::Result<()> {
        let oRepository = CTopicModerationPgRepository::new(oPool.clone());
        let dtOriginalLastMod =
            DateTime::parse_from_rfc3339("2026-08-01T10:00:00Z")?.with_timezone(&Utc);

        oRepository.vUncommit(42).await?;
        let stUncommitted: (
            bool,
            Option<i32>,
            Option<DateTime<Utc>>,
            DateTime<Utc>,
            i32,
            Option<String>,
            Option<String>,
            bool,
            String,
        ) = sqlx::query_as(
            "SELECT moderate,commitby,commitdate,lastmod,groupid,url,linktext,resolved,\
                    (SELECT message FROM msgbase WHERE id=topics.id) FROM topics WHERE id=42",
        )
        .fetch_one(oPool)
        .await?;
        anyhow::ensure!(!stUncommitted.0, "uncommit must clear moderate");
        anyhow::ensure!(
            stUncommitted.1.is_none() && stUncommitted.2.is_none(),
            "uncommit must clear commitby/commitdate"
        );
        anyhow::ensure!(
            stUncommitted.3 == dtOriginalLastMod,
            "uncommit must not touch lastmod"
        );
        anyhow::ensure!(
            stUncommitted.4 == 10
                && stUncommitted.5.as_deref() == Some("https://example.test/a")
                && stUncommitted.6.as_deref() == Some("details")
                && !stUncommitted.7
                && stUncommitted.8 == "original",
            "uncommit changed an unrelated topic/message field"
        );

        // Java obtains the msgbase markup after acquiring the move lock, but
        // keeps URL/link text/group/nick from the stale controller objects.
        vExecute(oPool, "UPDATE msgbase SET markup='PLAIN' WHERE id=42").await?;
        oRepository
            .vMove(StMoveTopicCommand {
                iTopicId: 42,
                iTargetGroupId: 20,
                bTargetLinksAllowed: false,
                optOriginalUrl: Some("https://example.test/a".to_owned()),
                optOriginalLinkText: Some("details".to_owned()),
                sOriginalGroupUrlName: "old-group".to_owned(),
                sModeratorNick: "moderator".to_owned(),
            })
            .await?;
        let stMoved: (i32, DateTime<Utc>, Option<String>, Option<String>, String) = sqlx::query_as(
            "SELECT groupid,lastmod,url,linktext,\
                    (SELECT message FROM msgbase WHERE id=topics.id) FROM topics WHERE id=42",
        )
        .fetch_one(oPool)
        .await?;
        anyhow::ensure!(stMoved.0 == 20, "move must change groupid");
        anyhow::ensure!(
            stMoved.1 > dtOriginalLastMod,
            "move must set lastmod to current time"
        );
        anyhow::ensure!(
            stMoved.2.is_none() && stMoved.3.is_none(),
            "a link-disabled target must clear URL/linktext"
        );
        let sExpectedMoveInfo = sMoveInfo(
            EnTopicMarkup::Html,
            Some("https://example.test/a"),
            Some("details"),
            "moderator",
            "old-group",
        );
        anyhow::ensure!(
            stMoved.4 == format!("original{sExpectedMoveInfo}"),
            "move info must use current markup and stale controller values"
        );

        // If a concurrent request already moved the row after the controller
        // snapshot, TopicDao skips its update/clear but TopicService still
        // appends move info for a link-disabled target.
        oRepository
            .vMove(StMoveTopicCommand {
                iTopicId: 43,
                iTargetGroupId: 20,
                bTargetLinksAllowed: false,
                optOriginalUrl: Some("https://stale.test/old".to_owned()),
                optOriginalLinkText: Some("stale".to_owned()),
                sOriginalGroupUrlName: "old-group".to_owned(),
                sModeratorNick: "moderator".to_owned(),
            })
            .await?;
        let stRacingMove: (DateTime<Utc>, Option<String>, String) = sqlx::query_as(
            "SELECT lastmod,url,(SELECT message FROM msgbase WHERE id=topics.id) \
             FROM topics WHERE id=43",
        )
        .fetch_one(oPool)
        .await?;
        anyhow::ensure!(
            stRacingMove.0 == dtOriginalLastMod,
            "the row-lock already-at-target branch must not touch lastmod"
        );
        anyhow::ensure!(
            stRacingMove.1.as_deref() == Some("https://example.test/race"),
            "the row-lock already-at-target branch must not clear the link"
        );
        anyhow::ensure!(
            stRacingMove.2
                == format!(
                    "racing{}",
                    sMoveInfo(
                        EnTopicMarkup::Html,
                        Some("https://stale.test/old"),
                        Some("stale"),
                        "moderator",
                        "old-group",
                    )
                ),
            "the move-info append must survive the row-lock race"
        );

        let dtBeforeResolve = stMoved.1;
        oRepository.vResolve(42, false).await?;
        let stFirstResolve: (bool, DateTime<Utc>) =
            sqlx::query_as("SELECT resolved,lastmod FROM topics WHERE id=42")
                .fetch_one(oPool)
                .await?;
        anyhow::ensure!(
            !stFirstResolve.0 && stFirstResolve.1 == dtBeforeResolve + Duration::seconds(1),
            "a same-value resolve must still add exactly one second"
        );
        oRepository.vResolve(42, true).await?;
        let stSecondResolve: (bool, DateTime<Utc>) =
            sqlx::query_as("SELECT resolved,lastmod FROM topics WHERE id=42")
                .fetch_one(oPool)
                .await?;
        anyhow::ensure!(
            stSecondResolve.0 && stSecondResolve.1 == dtBeforeResolve + Duration::seconds(2),
            "resolve must be unconditional and cumulative"
        );

        let stUnrelated: (i32, i64, i64) = sqlx::query_as(
            "SELECT (SELECT score FROM users WHERE id=7),\
                    (SELECT count(*) FROM edit_info),\
                    (SELECT count(*) FROM user_events)",
        )
        .fetch_one(oPool)
        .await?;
        anyhow::ensure!(
            stUnrelated == (300, 1, 1),
            "moderation operations must not change score/history/events"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires an explicitly selected disposable PostgreSQL database"]
    async fn moderation_transactions_match_java_in_an_isolated_schema() {
        assert_eq!(
            std::env::var("LOR_MODERATION_DB_INTEGRATION_CONFIRM").as_deref(),
            Ok("isolated-schema"),
            "set LOR_MODERATION_DB_INTEGRATION_CONFIRM=isolated-schema"
        );
        let sDatabaseUrl = std::env::var("LOR_MODERATION_DB_INTEGRATION_DATABASE_URL")
            .expect("set LOR_MODERATION_DB_INTEGRATION_DATABASE_URL to a disposable database");
        let oPool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&sDatabaseUrl)
            .await
            .expect("disposable PostgreSQL database must be reachable");
        let sSchema = format!("topic_moderation_{}", uuid::Uuid::new_v4().simple());
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
        stResult.expect("moderation database delta contract");
    }
}
