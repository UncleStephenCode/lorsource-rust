use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StExactUserIdentity {
    pub iId: i32,
    pub sNick: String,
}

#[derive(Debug, Clone)]
pub struct StActivationIdentity {
    pub iId: i32,
    pub sNick: String,
    pub optEmail: Option<String>,
    pub optRegistrationDate: Option<DateTime<Utc>>,
    pub bActivated: bool,
}

#[derive(Debug, Clone)]
pub struct StPasswordResetIdentity {
    pub iId: i32,
    pub sNick: String,
    pub optEmail: Option<String>,
    pub dtReset: DateTime<Utc>,
    pub bBlocked: bool,
    pub bActivated: bool,
    pub bAdministrator: bool,
    pub bAnonymous: bool,
}

/// Account selected by Java `UserDao.getByEmail(email, searchBlocked=true)`
/// before issuing a password-reset code.
#[derive(Debug, Clone)]
pub struct StPasswordResetRequestIdentity {
    pub iId: i32,
    pub sNick: String,
    pub sEmail: String,
    pub bBlocked: bool,
    pub bActivated: bool,
    pub bModerator: bool,
    pub bAdministrator: bool,
    pub bAnonymous: bool,
}

#[async_trait]
pub trait TrUserIdentityRepository: Send + Sync {
    async fn optExactIdentity(&self, sNick: &str) -> Result<Option<StExactUserIdentity>>;

    async fn optActivationIdentity(&self, sNick: &str) -> Result<Option<StActivationIdentity>>;

    async fn optPasswordResetIdentity(
        &self,
        sNick: &str,
    ) -> Result<Option<StPasswordResetIdentity>>;

    async fn optPasswordResetRequestIdentity(
        &self,
        sEmail: &str,
    ) -> Result<Option<StPasswordResetRequestIdentity>>;

    async fn bExactNickExists(&self, sNick: &str) -> Result<bool>;

    async fn bSimilarNickExists(&self, sNick: &str) -> Result<bool>;

    async fn optProfileSettings(&self, iUserId: i32) -> Result<Option<String>>;

    async fn vecEventTypes(&self, iUserId: i32) -> Result<Vec<String>>;
}
