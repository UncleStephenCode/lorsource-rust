#![allow(dead_code)]

// Database proof for the PostgreSQL half of UserStatisticsService.  The
// OpenSearch count/topic aggregations are covered by production-layer unit
// tests; this harness proves the deliberately different local semantics in a
// disposable schema.

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
    pub mod user {
        pub mod statistics {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/domain/user/statistics.rs"
            ));
        }
    }
}

mod infra {
    pub mod postgres {
        pub mod user_statistics_repository {
            include!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/infra/postgres/user_statistics_repository.rs"
            ));
        }
    }
}

#[cfg(test)]
mod database_delta_tests {
    use chrono::{TimeZone, Utc};
    use sqlx::{PgPool, postgres::PgPoolOptions};

    use crate::{
        domain::user::statistics::TrUserStatisticsLocalRepository,
        infra::postgres::user_statistics_repository::CUserStatisticsPgRepository,
    };

    async fn vExecute(oPool: &PgPool, sSql: &str) -> anyhow::Result<()> {
        sqlx::query(sqlx::AssertSqlSafe(sSql.to_owned()))
            .execute(oPool)
            .await?;
        Ok(())
    }

    async fn vCreateFixture(oPool: &PgPool) -> anyhow::Result<()> {
        for sSql in [
            r#"CREATE TABLE users(
                   id integer PRIMARY KEY,
                   nick text NOT NULL,
                   blocked boolean NOT NULL
               )"#,
            r#"CREATE TABLE ignore_list(
                   userid integer NOT NULL REFERENCES users(id),
                   ignored integer NOT NULL REFERENCES users(id),
                   PRIMARY KEY(userid,ignored)
               )"#,
            r#"CREATE TABLE comments(
                   id integer PRIMARY KEY,
                   userid integer NOT NULL REFERENCES users(id),
                   postdate timestamptz NOT NULL,
                   deleted boolean NOT NULL
               )"#,
            r#"CREATE TABLE sections(
                   id integer PRIMARY KEY,
                   name text NOT NULL
               )"#,
            r#"INSERT INTO users(id,nick,blocked) VALUES
                   (10,'target',false),
                   (20,'active-viewer',false),
                   (21,'blocked-viewer',true)"#,
            "INSERT INTO ignore_list(userid,ignored) VALUES(20,10),(21,10)",
            r#"INSERT INTO comments(id,userid,postdate,deleted) VALUES
                   (100,10,'2020-01-02 03:04:05+00',true),
                   (101,10,'2022-02-03 04:05:06+00',false),
                   (102,10,'2024-03-04 05:06:07+00',true)"#,
            r#"INSERT INTO sections(id,name) VALUES
                   (1,'Новости'),(2,'Форум'),(3,'Галерея'),
                   (5,'Опросы'),(6,'Статьи')"#,
        ] {
            vExecute(oPool, sSql).await?;
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires an explicitly selected disposable PostgreSQL database"]
    async fn local_statistics_match_java_in_an_isolated_schema() {
        assert_eq!(
            std::env::var("LOR_USER_STATISTICS_DB_INTEGRATION_CONFIRM").as_deref(),
            Ok("isolated-schema"),
            "set LOR_USER_STATISTICS_DB_INTEGRATION_CONFIRM=isolated-schema"
        );
        let sDatabaseUrl = std::env::var("LOR_USER_STATISTICS_DB_INTEGRATION_DATABASE_URL")
            .expect("set LOR_USER_STATISTICS_DB_INTEGRATION_DATABASE_URL");
        let oPool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&sDatabaseUrl)
            .await
            .expect("disposable PostgreSQL database must be reachable");
        let sSchema = format!("user_statistics_{}", uuid::Uuid::new_v4().simple());
        vExecute(&oPool, &format!("CREATE SCHEMA {sSchema}"))
            .await
            .expect("temporary schema must be creatable");
        vExecute(&oPool, &format!("SET search_path TO {sSchema}"))
            .await
            .expect("temporary schema must be selectable");

        let stResult = async {
            vCreateFixture(&oPool).await?;
            let stActual = CUserStatisticsPgRepository::new(oPool.clone())
                .stLocalData(10)
                .await?;
            anyhow::ensure!(stActual.iIgnoreCount == 1);
            anyhow::ensure!(
                stActual.optFirstComment
                    == Some(Utc.with_ymd_and_hms(2020, 1, 2, 3, 4, 5).unwrap())
            );
            anyhow::ensure!(
                stActual.optLastComment == Some(Utc.with_ymd_and_hms(2024, 3, 4, 5, 6, 7).unwrap())
            );
            anyhow::ensure!(
                stActual
                    .vecSections
                    .iter()
                    .map(|stSection| (stSection.iId, stSection.sUrlName.as_str()))
                    .collect::<Vec<_>>()
                    == vec![
                        (1, "news"),
                        (2, "forum"),
                        (3, "gallery"),
                        (5, "polls"),
                        (6, "articles"),
                    ]
            );
            Ok::<_, anyhow::Error>(())
        }
        .await;

        let stResetResult = vExecute(&oPool, "SET search_path TO public").await;
        let stDropResult = vExecute(&oPool, &format!("DROP SCHEMA {sSchema} CASCADE")).await;
        stResetResult.expect("search_path cleanup must succeed");
        stDropResult.expect("temporary schema cleanup must succeed");
        stResult.expect("user statistics database contract");
    }
}
