use async_trait::async_trait;

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StDeregisterUserState {
    pub iUserId: i32,
    pub iMaxScore: i32,
    pub bModerator: bool,
    pub bAdministrator: bool,
    pub bFrozen: bool,
}

#[async_trait]
pub trait TrUserAccountRepository: Send + Sync {
    async fn optDeregisterState(&self, iUserId: i32) -> Result<Option<StDeregisterUserState>>;

    async fn bPasswordMatches(&self, iUserId: i32, sPassword: &str) -> Result<bool>;

    async fn vDeregister(&self, iUserId: i32, sReason: &str) -> Result<()>;
}
