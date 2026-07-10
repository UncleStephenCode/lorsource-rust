use anyhow::Context;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Duration;

pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await
        .with_context(|| format!("failed to connect to PostgreSQL: {database_url}"))
}

pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("./db/migrations").run(pool).await?;
    Ok(())
}
