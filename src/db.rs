//! Backward-compatible database facade.
//!
//! New code should use `crate::infra::postgres::database::oConnect`.
//! This facade keeps the old bootstrap path stable during incremental refactor.

use sqlx::PgPool;

pub async fn connect(sDatabaseUrl: &str) -> anyhow::Result<PgPool> {
    crate::infra::postgres::database::oConnect(sDatabaseUrl).await
}

pub async fn verify_schema(oPool: &PgPool) -> anyhow::Result<()> {
    crate::infra::postgres::database::vVerifySchema(oPool).await
}
