use crate::error::Result;
use sqlx::PgPool;

/// Insert an item into the original `user_log` audit table.
///
/// The Scala application records moderation/account operations in
/// `UserLogDao`.  The Rust port keeps the helper deliberately small and uses
/// dynamic SQL because the `user_log_action` enum lives in PostgreSQL.
pub async fn log_user_action(
    pool: &PgPool,
    target_user_id: i32,
    actor_user_id: i32,
    action: &str,
    info: &[(&str, &str)],
) -> Result<()> {
    let keys: Vec<String> = info.iter().map(|(key, _)| (*key).to_string()).collect();
    let values: Vec<String> = info.iter().map(|(_, value)| (*value).to_string()).collect();
    sqlx::query(
        r#"INSERT INTO user_log(userid, action_userid, action_date, action, info)
           VALUES($1, $2, now(), $3::user_log_action, hstore($4::text[], $5::text[]))"#,
    )
    .bind(target_user_id)
    .bind(actor_user_id)
    .bind(action)
    .bind(keys)
    .bind(values)
    .execute(pool)
    .await?;
    Ok(())
}
