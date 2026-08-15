use async_trait::async_trait;

use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StCommentMessageParameters {
    pub iTopicId: i32,
    pub optReplyToId: Option<i32>,
    pub optOriginalId: Option<i32>,
    pub optNick: Option<String>,
    pub sMessage: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StCommentMessageTopicValidation {
    pub bDeleted: bool,
    pub bExpired: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StCommentMessageCommentValidation {
    pub iTopicId: i32,
    pub bDeleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnCommentMessageBindingError {
    #[error("Required model attribute 'topic' is missing")]
    MissingTopic,
    #[error("Failed to convert model attribute 'topic'")]
    InvalidTopic,
    #[error("Failed to convert model attribute 'replyto'")]
    InvalidReplyTo,
    #[error("Failed to convert model attribute 'original'")]
    InvalidOriginal,
    #[error("Failed to convert model attribute 'nick'")]
    InvalidNick,
    #[error("Validation failed for model attribute 'msg'")]
    InvalidMessage,
    #[error("Validation failed: нельзя добавлять в удаленные темы")]
    TopicDeleted,
    #[error("Validation failed: нельзя добавлять в устаревшие темы")]
    TopicExpired,
    #[error("Validation failed: нельзя комментировать удаленные комментарии")]
    ReplyDeleted,
    #[error("Validation failed: некорректная тема")]
    ReplyTopicMismatch,
}

#[async_trait]
pub trait TrCommentMessageRepository: Send + Sync {
    async fn optTopicValidation(
        &self,
        iTopicId: i32,
    ) -> Result<Option<StCommentMessageTopicValidation>>;

    async fn optCommentValidation(
        &self,
        iCommentId: i32,
    ) -> Result<Option<StCommentMessageCommentValidation>>;

    async fn bUserExists(&self, sNick: &str) -> Result<bool>;
}

fn optFirst<'a>(vecParameters: &'a [(String, String)], sName: &str) -> Option<&'a str> {
    vecParameters
        .iter()
        .find_map(|(sKey, sValue)| (sKey == sName).then_some(sValue.as_str()))
}

fn iTopicId(sValue: &str) -> std::result::Result<i32, EnCommentMessageBindingError> {
    // CommentCreateService's Topic PropertyEditor deliberately accepts the
    // legacy `id,suffix` shape and uses the first comma-separated element.
    sValue
        .split(',')
        .next()
        .unwrap_or_default()
        .parse::<i32>()
        .map_err(|_| EnCommentMessageBindingError::InvalidTopic)
}

fn optCommentId(
    optValue: Option<&str>,
    stInvalid: EnCommentMessageBindingError,
) -> std::result::Result<Option<i32>, EnCommentMessageBindingError> {
    match optValue {
        None | Some("") | Some("0") => Ok(None),
        Some(sValue) => sValue.parse::<i32>().map(Some).map_err(|_| stInvalid),
    }
}

fn bValidXmlCharacter(cCharacter: char) -> bool {
    matches!(cCharacter, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&cCharacter)
        || ('\u{E000}'..='\u{FFFD}').contains(&cCharacter)
        || ('\u{10000}'..='\u{10FFFF}').contains(&cCharacter)
}

pub fn stBindCommentMessageParameters(
    vecParameters: &[(String, String)],
) -> std::result::Result<StCommentMessageParameters, EnCommentMessageBindingError> {
    let sTopic =
        optFirst(vecParameters, "topic").ok_or(EnCommentMessageBindingError::MissingTopic)?;
    let iTopicId = iTopicId(sTopic)?;
    let optReplyToId = optCommentId(
        optFirst(vecParameters, "replyto"),
        EnCommentMessageBindingError::InvalidReplyTo,
    )?;
    let optOriginalId = optCommentId(
        optFirst(vecParameters, "original"),
        EnCommentMessageBindingError::InvalidOriginal,
    )?;
    let optNick = optFirst(vecParameters, "nick")
        .filter(|sNick| !sNick.is_empty())
        .map(ToOwned::to_owned);
    let sMessage = optFirst(vecParameters, "msg")
        .unwrap_or_default()
        .to_owned();
    if !sMessage.chars().all(bValidXmlCharacter) {
        return Err(EnCommentMessageBindingError::InvalidMessage);
    }
    Ok(StCommentMessageParameters {
        iTopicId,
        optReplyToId,
        optOriginalId,
        optNick,
        sMessage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vecPairs(arrValues: &[(&str, &str)]) -> Vec<(String, String)> {
        arrValues
            .iter()
            .map(|(sKey, sValue)| ((*sKey).to_owned(), (*sValue).to_owned()))
            .collect()
    }

    #[test]
    fn query_first_binding_and_legacy_topic_suffix_match_spring() {
        let stBound = stBindCommentMessageParameters(&vecPairs(&[
            ("topic", "42,ignored"),
            ("topic", "99"),
            ("msg", "query"),
            ("msg", "body"),
            ("replyto", "0"),
        ]))
        .unwrap();
        assert_eq!(stBound.iTopicId, 42);
        assert_eq!(stBound.sMessage, "query");
        assert_eq!(stBound.optReplyToId, None);
    }

    #[test]
    fn malformed_model_attributes_fail_before_authorization() {
        assert_eq!(
            stBindCommentMessageParameters(&[]).unwrap_err(),
            EnCommentMessageBindingError::MissingTopic
        );
        assert_eq!(
            stBindCommentMessageParameters(&vecPairs(&[("topic", " 42")])).unwrap_err(),
            EnCommentMessageBindingError::InvalidTopic
        );
        assert_eq!(
            stBindCommentMessageParameters(&vecPairs(&[("topic", "42"), ("replyto", "bad")]))
                .unwrap_err(),
            EnCommentMessageBindingError::InvalidReplyTo
        );
    }

    #[test]
    fn xml_10_character_validation_matches_comment_request_validator() {
        assert!(
            stBindCommentMessageParameters(&vecPairs(&[
                ("topic", "42"),
                ("msg", "line one\nline two")
            ]))
            .is_ok()
        );
        assert_eq!(
            stBindCommentMessageParameters(&vecPairs(&[
                ("topic", "42"),
                ("msg", "bad\u{1}character")
            ]))
            .unwrap_err(),
            EnCommentMessageBindingError::InvalidMessage
        );
    }
}
