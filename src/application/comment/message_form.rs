use crate::{
    domain::comment::message_form::{
        EnCommentMessageBindingError, StCommentMessageParameters, TrCommentMessageRepository,
    },
    error::AppError,
};

#[derive(Debug, thiserror::Error)]
pub enum EnCommentMessageServiceError {
    #[error(transparent)]
    Binding(#[from] EnCommentMessageBindingError),
    #[error(transparent)]
    Application(#[from] AppError),
}

#[derive(Debug, Clone)]
pub struct CCommentMessageService<R>
where
    R: TrCommentMessageRepository,
{
    oRepository: R,
}

impl<R> CCommentMessageService<R>
where
    R: TrCommentMessageRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    /// Reproduces `@ModelAttribute("add") @Valid CommentRequest` before the
    /// controller enters `MaybeAuthorized`.  In particular, missing database
    /// objects are binding failures (HTTP 400), not the topic page's 404.
    pub async fn stValidate(
        &self,
        stParameters: StCommentMessageParameters,
    ) -> Result<StCommentMessageParameters, EnCommentMessageServiceError> {
        let stTopic = self
            .oRepository
            .optTopicValidation(stParameters.iTopicId)
            .await?
            .ok_or(EnCommentMessageBindingError::InvalidTopic)?;
        if stTopic.bDeleted {
            return Err(EnCommentMessageBindingError::TopicDeleted.into());
        }
        if stTopic.bExpired {
            return Err(EnCommentMessageBindingError::TopicExpired.into());
        }

        if let Some(iReplyToId) = stParameters.optReplyToId {
            let stReply = self
                .oRepository
                .optCommentValidation(iReplyToId)
                .await?
                .ok_or(EnCommentMessageBindingError::InvalidReplyTo)?;
            if stReply.bDeleted {
                return Err(EnCommentMessageBindingError::ReplyDeleted.into());
            }
            if stReply.iTopicId != stParameters.iTopicId {
                return Err(EnCommentMessageBindingError::ReplyTopicMismatch.into());
            }
        }

        if let Some(iOriginalId) = stParameters.optOriginalId
            && self
                .oRepository
                .optCommentValidation(iOriginalId)
                .await?
                .is_none()
        {
            return Err(EnCommentMessageBindingError::InvalidOriginal.into());
        }

        if let Some(sNick) = stParameters.optNick.as_deref()
            && !self.oRepository.bUserExists(sNick).await?
        {
            return Err(EnCommentMessageBindingError::InvalidNick.into());
        }

        Ok(stParameters)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::comment::message_form::{
            StCommentMessageCommentValidation, StCommentMessageTopicValidation,
        },
        error::Result as AppResult,
    };
    use async_trait::async_trait;
    use std::collections::{HashMap, HashSet};

    #[derive(Clone, Default)]
    struct CMemoryRepository {
        mapTopics: HashMap<i32, StCommentMessageTopicValidation>,
        mapComments: HashMap<i32, StCommentMessageCommentValidation>,
        setUsers: HashSet<String>,
    }

    #[async_trait]
    impl TrCommentMessageRepository for CMemoryRepository {
        async fn optTopicValidation(
            &self,
            iTopicId: i32,
        ) -> AppResult<Option<StCommentMessageTopicValidation>> {
            Ok(self.mapTopics.get(&iTopicId).copied())
        }

        async fn optCommentValidation(
            &self,
            iCommentId: i32,
        ) -> AppResult<Option<StCommentMessageCommentValidation>> {
            Ok(self.mapComments.get(&iCommentId).copied())
        }

        async fn bUserExists(&self, sNick: &str) -> AppResult<bool> {
            Ok(self.setUsers.contains(sNick))
        }
    }

    fn stParameters() -> StCommentMessageParameters {
        StCommentMessageParameters {
            iTopicId: 42,
            optReplyToId: None,
            optOriginalId: None,
            optNick: None,
            sMessage: String::new(),
        }
    }

    fn cRepository() -> CMemoryRepository {
        let mut cRepository = CMemoryRepository::default();
        cRepository.mapTopics.insert(
            42,
            StCommentMessageTopicValidation {
                bDeleted: false,
                bExpired: false,
            },
        );
        cRepository
    }

    #[tokio::test]
    async fn database_backed_binding_happens_before_controller_permissions() {
        let stError = CCommentMessageService::new(CMemoryRepository::default())
            .stValidate(stParameters())
            .await
            .unwrap_err();
        assert!(matches!(
            stError,
            EnCommentMessageServiceError::Binding(EnCommentMessageBindingError::InvalidTopic)
        ));
    }

    #[tokio::test]
    async fn validates_reply_original_and_nick_even_though_view_uses_only_topic() {
        let mut cRepository = cRepository();
        cRepository.mapComments.insert(
            7,
            StCommentMessageCommentValidation {
                iTopicId: 42,
                bDeleted: false,
            },
        );
        cRepository.setUsers.insert("crane".to_owned());
        let mut stParameters = stParameters();
        stParameters.optReplyToId = Some(7);
        stParameters.optOriginalId = Some(7);
        stParameters.optNick = Some("crane".to_owned());
        let stValidated = CCommentMessageService::new(cRepository)
            .stValidate(stParameters)
            .await
            .unwrap();
        assert_eq!(stValidated.iTopicId, 42);
    }

    #[tokio::test]
    async fn rejects_deleted_or_cross_topic_reply_like_comment_request_validator() {
        for stReply in [
            StCommentMessageCommentValidation {
                iTopicId: 42,
                bDeleted: true,
            },
            StCommentMessageCommentValidation {
                iTopicId: 43,
                bDeleted: false,
            },
        ] {
            let mut cRepository = cRepository();
            cRepository.mapComments.insert(7, stReply);
            let mut stParameters = stParameters();
            stParameters.optReplyToId = Some(7);
            let stError = CCommentMessageService::new(cRepository)
                .stValidate(stParameters)
                .await
                .unwrap_err();
            assert!(matches!(
                stError,
                EnCommentMessageServiceError::Binding(
                    EnCommentMessageBindingError::ReplyDeleted
                        | EnCommentMessageBindingError::ReplyTopicMismatch
                )
            ));
        }
    }
}
