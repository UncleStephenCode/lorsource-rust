use async_trait::async_trait;

use crate::{domain::email::model::StEmailMessage, error::Result};

#[async_trait]
pub trait TrEmailSender: Send + Sync {
    async fn vSend(&self, stMessage: &StEmailMessage) -> Result<()>;
}
