use async_trait::async_trait;

use crate::{
    domain::user::moderation::{
        EnUserModerationMutation, StModerationUser, StUserModerationMutationResult,
    },
    error::Result,
};

#[async_trait]
pub trait TrUserModerationRepository: Send + Sync {
    async fn optUser(&self, iUserId: i32) -> Result<Option<StModerationUser>>;

    async fn stApply(
        &self,
        enMutation: EnUserModerationMutation,
    ) -> Result<StUserModerationMutationResult>;
}
