use async_trait::async_trait;

use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StUserpicUploadPolicy {
    pub iScore: i32,
    pub bFrozen: bool,
    pub iRecentSetCount: i64,
    pub bRecentlyResetByModerator: bool,
    pub iRecentScoreLoss: i32,
}

impl StUserpicUploadPolicy {
    /// `EditProfileChecker.checkLoadUserpic` in the Java application.
    pub fn bPermitted(self) -> bool {
        !self.bFrozen
            && self.iScore >= 45
            && self.iRecentSetCount < 3
            && !self.bRecentlyResetByModerator
            && self.iRecentScoreLoss < 20
    }
}

#[async_trait]
pub trait TrUserpicRepository: Send + Sync {
    async fn optUploadPolicy(&self, iUserId: i32) -> Result<Option<StUserpicUploadPolicy>>;

    /// Atomically updates `users.photo` and writes Java-compatible
    /// `set_userpic` audit metadata, including the previous filename.
    async fn vSetUserpic(&self, iUserId: i32, sFilename: &str) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::StUserpicUploadPolicy;

    fn stPolicy() -> StUserpicUploadPolicy {
        StUserpicUploadPolicy {
            iScore: 45,
            bFrozen: false,
            iRecentSetCount: 2,
            bRecentlyResetByModerator: false,
            iRecentScoreLoss: 19,
        }
    }

    #[test]
    fn java_permission_boundaries_are_exact() {
        assert!(stPolicy().bPermitted());
        assert!(
            !StUserpicUploadPolicy {
                iScore: 44,
                ..stPolicy()
            }
            .bPermitted()
        );
        assert!(
            !StUserpicUploadPolicy {
                bFrozen: true,
                ..stPolicy()
            }
            .bPermitted()
        );
        assert!(
            !StUserpicUploadPolicy {
                iRecentSetCount: 3,
                ..stPolicy()
            }
            .bPermitted()
        );
        assert!(
            !StUserpicUploadPolicy {
                bRecentlyResetByModerator: true,
                ..stPolicy()
            }
            .bPermitted()
        );
        assert!(
            !StUserpicUploadPolicy {
                iRecentScoreLoss: 20,
                ..stPolicy()
            }
            .bPermitted()
        );
    }
}
