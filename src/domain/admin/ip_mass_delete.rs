use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StIpMassDeleteActor {
    pub iUserId: i32,
    pub bModerator: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StIpBanCommand {
    pub sIp: String,
    pub sReason: String,
    pub optBanUntil: Option<DateTime<Utc>>,
    pub bAllowPosting: bool,
    pub bCaptchaRequired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StIpMassDeleteCommand {
    pub sIp: String,
    pub dtCutoff: DateTime<Utc>,
    pub sReason: String,
    pub optBan: Option<StIpBanCommand>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StIpMassDeleteResult {
    pub vecDeletedTopicIds: Vec<i32>,
    pub vecDeletedCommentIds: Vec<i32>,
    pub vecSkippedCommentIds: Vec<i32>,
}

#[async_trait]
pub trait TrIpMassDeleteRepository: Send + Sync {
    /// `IpBlockDao.blockIP` is deliberately a separate, auto-committed
    /// operation. If the following mass-delete transaction fails, Java keeps
    /// the successfully written block.
    async fn vBlockIp(&self, iModeratorId: i32, stCommand: &StIpBanCommand) -> Result<()>;

    /// Selects and mutates every candidate inside the one `localTx` owned by
    /// `DeleteService.deleteByIPAddress`.
    async fn stDeleteByIp(
        &self,
        iModeratorId: i32,
        stCommand: &StIpMassDeleteCommand,
    ) -> Result<StIpMassDeleteResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_ban_is_distinct_from_an_absent_ban_operation() {
        let stCommand = StIpMassDeleteCommand {
            sIp: "203.0.113.5".to_owned(),
            dtCutoff: Utc::now(),
            sReason: String::new(),
            optBan: Some(StIpBanCommand {
                sIp: "203.0.113.5".to_owned(),
                sReason: String::new(),
                optBanUntil: None,
                bAllowPosting: false,
                bCaptchaRequired: false,
            }),
        };

        assert!(stCommand.optBan.is_some());
        assert!(stCommand.optBan.unwrap().optBanUntil.is_none());
    }
}
