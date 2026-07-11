use anyhow::Context;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Duration;

pub type TyPgPool = PgPool;

pub async fn oConnect(sDatabaseUrl: &str) -> anyhow::Result<TyPgPool> {
    PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(5))
        .connect(sDatabaseUrl)
        .await
        .with_context(|| format!("failed to connect to PostgreSQL: {sDatabaseUrl}"))
}

pub async fn vMigrate(oPool: &TyPgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./db/migrations").run(oPool).await?;
    Ok(())
}
