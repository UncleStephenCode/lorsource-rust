use async_trait::async_trait;
use sqlx::PgPool;

use crate::{
    domain::user::identity::{
        StActivationIdentity, StExactUserIdentity, StPasswordResetIdentity,
        StPasswordResetRequestIdentity, TrUserIdentityRepository,
    },
    error::Result,
};

const S_EXACT_IDENTITY: &str = "SELECT id,nick FROM users WHERE nick=$1";
const S_ACTIVATION_IDENTITY: &str = r#"
SELECT id,nick,email,regdate,activated
  FROM users
 WHERE nick=$1"#;
const S_PASSWORD_RESET_IDENTITY: &str = r#"
SELECT id,nick,email,lostpwd,COALESCE(blocked,false),activated,candel,
       (passwd IS NULL OR passwd='')
  FROM users
 WHERE nick=$1"#;
const S_PASSWORD_RESET_REQUEST_IDENTITY: &str = r#"
SELECT id,nick,email,COALESCE(blocked,false),activated,canmod,candel,
       (passwd IS NULL OR passwd='')
  FROM users
 WHERE normalize_email(email)=normalize_email($1)
 ORDER BY blocked ASC,id DESC
 LIMIT 1"#;

#[derive(Debug, Clone)]
pub struct CUserIdentityPgRepository {
    oPool: PgPool,
}

impl CUserIdentityPgRepository {
    pub fn new(oPool: PgPool) -> Self {
        Self { oPool }
    }
}

#[async_trait]
impl TrUserIdentityRepository for CUserIdentityPgRepository {
    async fn optExactIdentity(&self, sNick: &str) -> Result<Option<StExactUserIdentity>> {
        let optRow: Option<(i32, String)> = sqlx::query_as(S_EXACT_IDENTITY)
            .bind(sNick)
            .fetch_optional(&self.oPool)
            .await?;
        Ok(optRow.map(|(iId, sNick)| StExactUserIdentity { iId, sNick }))
    }

    async fn optActivationIdentity(&self, sNick: &str) -> Result<Option<StActivationIdentity>> {
        let optRow = sqlx::query_as::<
            _,
            (
                i32,
                String,
                Option<String>,
                Option<chrono::DateTime<chrono::Utc>>,
                bool,
            ),
        >(S_ACTIVATION_IDENTITY)
        .bind(sNick)
        .fetch_optional(&self.oPool)
        .await?;
        Ok(
            optRow.map(|(iId, sNick, optEmail, optRegistrationDate, bActivated)| {
                StActivationIdentity {
                    iId,
                    sNick,
                    optEmail,
                    optRegistrationDate,
                    bActivated,
                }
            }),
        )
    }

    async fn optPasswordResetIdentity(
        &self,
        sNick: &str,
    ) -> Result<Option<StPasswordResetIdentity>> {
        let optRow = sqlx::query_as::<
            _,
            (
                i32,
                String,
                Option<String>,
                chrono::DateTime<chrono::Utc>,
                bool,
                bool,
                bool,
                bool,
            ),
        >(S_PASSWORD_RESET_IDENTITY)
        .bind(sNick)
        .fetch_optional(&self.oPool)
        .await?;
        Ok(optRow.map(
            |(iId, sNick, optEmail, dtReset, bBlocked, bActivated, bAdministrator, bAnonymous)| {
                StPasswordResetIdentity {
                    iId,
                    sNick,
                    optEmail,
                    dtReset,
                    bBlocked,
                    bActivated,
                    bAdministrator,
                    bAnonymous,
                }
            },
        ))
    }

    async fn optPasswordResetRequestIdentity(
        &self,
        sEmail: &str,
    ) -> Result<Option<StPasswordResetRequestIdentity>> {
        let optRow = sqlx::query_as::<_, (i32, String, String, bool, bool, bool, bool, bool)>(
            S_PASSWORD_RESET_REQUEST_IDENTITY,
        )
        .bind(sEmail)
        .fetch_optional(&self.oPool)
        .await?;
        Ok(optRow.map(
            |(iId, sNick, sEmail, bBlocked, bActivated, bModerator, bAdministrator, bAnonymous)| {
                StPasswordResetRequestIdentity {
                    iId,
                    sNick,
                    sEmail,
                    bBlocked,
                    bActivated,
                    bModerator,
                    bAdministrator,
                    bAnonymous,
                }
            },
        ))
    }

    async fn bExactNickExists(&self, sNick: &str) -> Result<bool> {
        Ok(
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE nick=$1)")
                .bind(sNick)
                .fetch_one(&self.oPool)
                .await?,
        )
    }

    async fn bSimilarNickExists(&self, sNick: &str) -> Result<bool> {
        Ok(sqlx::query_scalar(
            r#"SELECT EXISTS(
                 SELECT 1 FROM users
                  WHERE score>=200
                    AND lastlogin>CURRENT_TIMESTAMP-interval '3 years'
                    AND levenshtein_less_equal(lower(nick),lower($1),1)<=1
               )"#,
        )
        .bind(sNick)
        .fetch_one(&self.oPool)
        .await?)
    }

    async fn optProfileSettings(&self, iUserId: i32) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT settings::text FROM user_settings WHERE id=$1")
                .bind(iUserId)
                .fetch_optional(&self.oPool)
                .await?,
        )
    }

    async fn vecEventTypes(&self, iUserId: i32) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT DISTINCT type::text FROM user_events WHERE userid=$1")
                .bind(iUserId)
                .fetch_all(&self.oPool)
                .await?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        S_ACTIVATION_IDENTITY, S_EXACT_IDENTITY, S_PASSWORD_RESET_IDENTITY,
        S_PASSWORD_RESET_REQUEST_IDENTITY,
    };

    #[test]
    fn every_identity_lookup_uses_java_exact_nick_semantics() {
        for sSql in [
            S_EXACT_IDENTITY,
            S_ACTIVATION_IDENTITY,
            S_PASSWORD_RESET_IDENTITY,
        ] {
            assert!(sSql.contains("WHERE nick=$1"));
            assert!(!sSql.to_ascii_lowercase().contains("lower(nick)"));
            assert!(!sSql.to_ascii_lowercase().contains("limit 1"));
        }
    }

    #[test]
    fn password_reset_request_uses_java_normalization_and_candidate_order() {
        assert!(
            S_PASSWORD_RESET_REQUEST_IDENTITY
                .contains("normalize_email(email)=normalize_email($1)")
        );
        assert!(S_PASSWORD_RESET_REQUEST_IDENTITY.contains("ORDER BY blocked ASC,id DESC"));
        assert!(S_PASSWORD_RESET_REQUEST_IDENTITY.contains("LIMIT 1"));
        assert!(!S_PASSWORD_RESET_REQUEST_IDENTITY.contains("lower(email)"));
    }
}
