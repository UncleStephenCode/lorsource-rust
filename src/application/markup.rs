use std::collections::HashSet;

use crate::{
    domain::markup::{model::StMarkupUserDirectory, repository::TrMarkupUserRepository},
    error::Result,
    markup,
};

#[derive(Debug, Clone)]
pub struct CMarkupService<R>
where
    R: TrMarkupUserRepository,
{
    oRepository: R,
}

impl<R> CMarkupService<R>
where
    R: TrMarkupUserRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    /// Resolve every Markdown LorUser and LORCODE MemberTag in a
    /// page/feed/indexing batch with one PostgreSQL request. Exact case is
    /// retained because Java's `UserDao.findUserId` uses `WHERE nick = ?`,
    /// not a case-insensitive lookup.
    pub async fn stResolveBatch<'a, I>(&self, iterMessages: I) -> Result<StMarkupUserDirectory>
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let mut setSeen = HashSet::new();
        let mut vecNicks = Vec::new();
        let mut bHasMarkdownMention = false;
        for (sMessage, sMarkup) in iterMessages {
            if !bUsesUserReference(sMarkup) {
                continue;
            }
            let vecMentions = markup::extract_mentions(sMessage, sMarkup);
            if sMarkup == "MARKDOWN" && !vecMentions.is_empty() {
                bHasMarkdownMention = true;
            }
            for sNick in vecMentions {
                if setSeen.insert(sNick.clone()) {
                    vecNicks.push(sNick);
                }
            }
        }
        if vecNicks.is_empty() {
            return Ok(StMarkupUserDirectory::default());
        }
        let vecUsers = match self.oRepository.vecFindByNicks(&vecNicks).await {
            Ok(vecUsers) => vecUsers,
            Err(stError) if bHasMarkdownMention => {
                // LorUserRenderer calls `findUserCached` directly. Unlike
                // MemberTag it does not isolate a failed lookup, so a batch
                // containing Markdown references must retain that failure.
                return Err(stError);
            }
            Err(stError) => {
                // MemberTag catches every UserService lookup exception and
                // renders that name as missing.  A batched PostgreSQL lookup
                // must retain the same page-level failure isolation.
                tracing::warn!(
                    error = %stError,
                    count = vecNicks.len(),
                    "failed to resolve LORCODE MemberTag users"
                );
                Vec::new()
            }
        };
        Ok(StMarkupUserDirectory::stFromUsers(vecUsers))
    }

    pub async fn stResolveMessageIds(
        &self,
        vecMessageIds: &[i32],
    ) -> Result<StMarkupUserDirectory> {
        if vecMessageIds.is_empty() {
            return Ok(StMarkupUserDirectory::default());
        }
        let vecSources = self
            .oRepository
            .vecSourcesByMessageIds(vecMessageIds)
            .await?;
        self.stResolveBatch(
            vecSources
                .iter()
                .map(|stSource| (&*stSource.sMessage, &*stSource.sMarkup)),
        )
        .await
    }
}

fn bUsesUserReference(sMarkup: &str) -> bool {
    matches!(
        sMarkup,
        "MARKDOWN" | "BBCODE_TEX" | "BBCODE_ULB" | "LORCODE"
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::domain::markup::model::StMarkupUser;

    #[derive(Clone, Default)]
    struct CTestRepository {
        vecCalls: Arc<Mutex<Vec<Vec<String>>>>,
    }

    #[async_trait]
    impl TrMarkupUserRepository for CTestRepository {
        async fn vecFindByNicks(&self, vecNicks: &[String]) -> Result<Vec<StMarkupUser>> {
            self.vecCalls
                .lock()
                .expect("calls lock")
                .push(vecNicks.to_vec());
            Ok(vecNicks
                .iter()
                .filter(|sNick| sNick.as_str() != "missing")
                .map(|sNick| StMarkupUser {
                    sInputNick: sNick.clone(),
                    sCanonicalNick: sNick.clone(),
                    bBlocked: sNick == "blocked",
                })
                .collect())
        }

        async fn vecSourcesByMessageIds(
            &self,
            _vecMessageIds: &[i32],
        ) -> Result<Vec<crate::domain::markup::model::StMarkupSource>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn resolves_one_deduplicated_batch_and_ignores_other_markup_modes() {
        let cRepository = CTestRepository::default();
        let vecCalls = Arc::clone(&cRepository.vecCalls);
        let cService = CMarkupService::new(cRepository);
        let stDirectory = cService
            .stResolveBatch([
                ("[user]alice[/user] [user]blocked[/user]", "BBCODE_TEX"),
                ("[USER]alice[/USER] [user]missing[/user]", "BBCODE_ULB"),
                ("markdown @markdown", "MARKDOWN"),
                ("[user]not-lorcode[/user]", "MARKDOWN"),
            ])
            .await
            .expect("resolve batch");

        assert_eq!(
            *vecCalls.lock().expect("calls lock"),
            vec![vec![
                "alice".to_owned(),
                "blocked".to_owned(),
                "missing".to_owned(),
                "markdown".to_owned(),
            ]]
        );
        assert_eq!(stDirectory.iLen(), 3);
        assert!(!stDirectory.optFind("alice").expect("alice").bBlocked);
        assert!(stDirectory.optFind("blocked").expect("blocked").bBlocked);
        assert!(stDirectory.optFind("missing").is_none());
    }

    #[tokio::test]
    async fn empty_batch_does_not_hit_repository() {
        let cRepository = CTestRepository::default();
        let vecCalls = Arc::clone(&cRepository.vecCalls);
        let cService = CMarkupService::new(cRepository);
        let stDirectory = cService
            .stResolveBatch([("plain without references", "MARKDOWN")])
            .await
            .expect("resolve batch");

        assert!(stDirectory.bIsEmpty());
        assert!(vecCalls.lock().expect("calls lock").is_empty());
    }

    #[derive(Clone)]
    struct CFailRepository;

    #[async_trait]
    impl TrMarkupUserRepository for CFailRepository {
        async fn vecFindByNicks(&self, _vecNicks: &[String]) -> Result<Vec<StMarkupUser>> {
            Err(crate::error::AppError::Anyhow(anyhow::anyhow!(
                "lookup unavailable"
            )))
        }

        async fn vecSourcesByMessageIds(
            &self,
            _vecMessageIds: &[i32],
        ) -> Result<Vec<crate::domain::markup::model::StMarkupSource>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn lookup_failure_degrades_to_java_missing_user_rendering() {
        let cService = CMarkupService::new(CFailRepository);
        let stDirectory = cService
            .stResolveBatch([("[user]alice[/user]", "BBCODE_TEX")])
            .await
            .expect("MemberTag lookup is isolated");

        assert!(stDirectory.bIsEmpty());
    }

    #[tokio::test]
    async fn markdown_lookup_failure_is_not_hidden() {
        let cService = CMarkupService::new(CFailRepository);
        let stResult = cService
            .stResolveBatch([("hello @alice", "MARKDOWN")])
            .await;

        assert!(stResult.is_err());
    }
}
