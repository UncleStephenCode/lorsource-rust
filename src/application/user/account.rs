use crate::{
    domain::user::account::TrUserAccountRepository,
    error::{AppError, Result},
};

pub const S_DEREGISTER_REASON: &str = "самостоятельная блокировка аккаунта";

#[derive(Debug, Clone)]
pub struct CUserAccountService<R>
where
    R: TrUserAccountRepository,
{
    oRepository: R,
}

impl<R> CUserAccountService<R>
where
    R: TrUserAccountRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    pub async fn vCheckDeregister(&self, iUserId: i32) -> Result<()> {
        let stUser = self
            .oRepository
            .optDeregisterState(iUserId)
            .await?
            .ok_or(AppError::NotFound)?;

        if stUser.iMaxScore < 100 || stUser.bModerator || stUser.bAdministrator || stUser.bFrozen {
            return Err(AppError::Forbidden);
        }

        Ok(())
    }

    pub async fn vDeregister(&self, iUserId: i32) -> Result<()> {
        self.vCheckDeregister(iUserId).await?;
        self.oRepository
            .vDeregister(iUserId, S_DEREGISTER_REASON)
            .await
    }

    pub async fn bPasswordMatches(&self, iUserId: i32, sPassword: &str) -> Result<bool> {
        self.oRepository.bPasswordMatches(iUserId, sPassword).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::domain::user::account::StDeregisterUserState;

    type TyDeregisterCalls = Arc<Mutex<Vec<(i32, String)>>>;

    #[derive(Clone)]
    struct CTestRepository {
        optState: Option<StDeregisterUserState>,
        vecCalls: TyDeregisterCalls,
    }

    #[async_trait]
    impl TrUserAccountRepository for CTestRepository {
        async fn optDeregisterState(&self, _iUserId: i32) -> Result<Option<StDeregisterUserState>> {
            Ok(self.optState)
        }

        async fn vDeregister(&self, iUserId: i32, sReason: &str) -> Result<()> {
            self.vecCalls
                .lock()
                .expect("calls lock")
                .push((iUserId, sReason.to_owned()));
            Ok(())
        }

        async fn bPasswordMatches(&self, _iUserId: i32, sPassword: &str) -> Result<bool> {
            Ok(sPassword == "correct password")
        }
    }

    fn stState() -> StDeregisterUserState {
        StDeregisterUserState {
            iUserId: 42,
            iMaxScore: 100,
            bModerator: false,
            bAdministrator: false,
            bFrozen: false,
        }
    }

    fn cService(
        stState: StDeregisterUserState,
    ) -> (CUserAccountService<CTestRepository>, TyDeregisterCalls) {
        let vecCalls = Arc::new(Mutex::new(Vec::new()));
        (
            CUserAccountService::new(CTestRepository {
                optState: Some(stState),
                vecCalls: vecCalls.clone(),
            }),
            vecCalls,
        )
    }

    #[tokio::test]
    async fn deregistration_uses_the_java_self_block_reason() {
        let (cService, vecCalls) = cService(stState());
        cService.vDeregister(42).await.expect("deregister");

        assert_eq!(
            *vecCalls.lock().expect("calls lock"),
            vec![(42, S_DEREGISTER_REASON.to_owned())]
        );
    }

    #[tokio::test]
    async fn password_check_is_a_read_only_repository_operation() {
        let (cService, vecCalls) = cService(stState());

        assert!(
            cService
                .bPasswordMatches(42, "correct password")
                .await
                .expect("password check")
        );
        assert!(
            !cService
                .bPasswordMatches(42, "wrong password")
                .await
                .expect("password check")
        );
        assert!(vecCalls.lock().expect("calls lock").is_empty());
    }

    #[tokio::test]
    async fn rejects_every_java_permission_boundary() {
        let mut vecRejected = Vec::new();
        for (sCase, stState) in [
            (
                "score",
                StDeregisterUserState {
                    iMaxScore: 99,
                    ..stState()
                },
            ),
            (
                "moderator",
                StDeregisterUserState {
                    bModerator: true,
                    ..stState()
                },
            ),
            (
                "administrator",
                StDeregisterUserState {
                    bAdministrator: true,
                    ..stState()
                },
            ),
            (
                "frozen",
                StDeregisterUserState {
                    bFrozen: true,
                    ..stState()
                },
            ),
        ] {
            let (cService, vecCalls) = cService(stState);
            assert!(matches!(
                cService.vDeregister(42).await,
                Err(AppError::Forbidden)
            ));
            assert!(vecCalls.lock().expect("calls lock").is_empty());
            vecRejected.push(sCase);
        }
        assert_eq!(vecRejected.len(), 4);
    }
}
