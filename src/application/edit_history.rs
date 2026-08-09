use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::{
    domain::edit_history::{StHistoryPoll, TrEditHistoryRepository},
    error::{AppError, Result},
    markup,
};

#[derive(Debug, Clone)]
pub struct StPreparedEditHistory {
    pub bOriginal: bool,
    pub bCurrent: bool,
    pub sEditor: String,
    pub dtEdit: DateTime<Utc>,
    pub sTitle: String,
    pub optMessageHtml: Option<String>,
    pub optTags: Option<Vec<String>>,
    pub optUrl: Option<String>,
    pub optLinkText: Option<String>,
    pub optMinor: Option<bool>,
    pub optPoll: Option<StHistoryPoll>,
    pub optRestoreFrom: Option<i32>,
    pub vecAddedImages: Vec<i32>,
    pub vecRemovedImages: Vec<i32>,
    pub vecAddedMainImages: Vec<i32>,
    pub vecRemovedMainImages: Vec<i32>,
}

impl StPreparedEditHistory {
    pub fn bHasLink(&self) -> bool {
        self.optUrl.is_some() || self.optLinkText.is_some()
    }

    pub fn sLinkUrl(&self) -> &str {
        self.optUrl.as_deref().unwrap_or("#")
    }

    pub fn sLinkText(&self) -> &str {
        self.optLinkText
            .as_deref()
            .unwrap_or("(текст ссылки не изменен)")
    }
}

pub struct CEditHistoryService<R> {
    oRepository: R,
}

impl<R: TrEditHistoryRepository> CEditHistoryService<R> {
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    pub async fn vecTopicHistory(&self, iTopicId: i32) -> Result<Vec<StPreparedEditHistory>> {
        let stSource = self.oRepository.stTopicSource(iTopicId).await?;
        let vecRows = self.oRepository.vecRows(iTopicId, "TOPIC").await?;
        if vecRows.is_empty() {
            return Ok(Vec::new());
        }
        let mut sMessage = stSource.sMessage;
        let sMarkup = stSource.sMarkup;
        let mut sTitle = stSource.sTitle;
        let mut optUrl = stSource.optUrl;
        let mut optLinkText = stSource.optLinkText;
        let mut bMinor = stSource.bMinor;
        let mut vecTags = stSource.vecTags;
        let mut vecImages = stSource.vecImageIds;
        let mut optPoll = stSource.optPoll;
        let mut optLastMessageId = None;
        let mut vecPrepared = Vec::with_capacity(vecRows.len() + 1);

        for (iIndex, stRow) in vecRows.into_iter().enumerate() {
            let mut vecAddedImages = Vec::new();
            let mut vecRemovedImages = Vec::new();
            let mut vecAddedMainImages = Vec::new();
            let mut vecRemovedMainImages = Vec::new();
            if let Some(vecOldImages) = stRow.optOldAdditionalImages.as_ref() {
                let setCurrent = vecImages.iter().copied().collect::<HashSet<_>>();
                let setOld = vecOldImages.iter().copied().collect::<HashSet<_>>();
                vecAddedImages = vecImages
                    .iter()
                    .copied()
                    .filter(|iId| !setOld.contains(iId))
                    .collect();
                vecRemovedImages = vecOldImages
                    .iter()
                    .copied()
                    .filter(|iId| !setCurrent.contains(iId))
                    .collect();
            } else if stRow.optLegacyMainImage == Some(0) {
                vecAddedMainImages.extend(vecImages.first().copied());
            } else if let Some(iOldImage) = stRow.optLegacyMainImage
                && iOldImage > 0
                && !vecImages.contains(&iOldImage)
            {
                vecRemovedMainImages.push(iOldImage);
            }

            let optMessageHtml = stRow
                .optOldMessage
                .as_ref()
                .map(|_| markup::render_message_with_markup(&sMessage, Some(&sMarkup), None));
            let optRestoreFrom = stRow.optOldMessage.as_ref().and(optLastMessageId);
            vecPrepared.push(StPreparedEditHistory {
                bOriginal: false,
                bCurrent: iIndex == 0,
                sEditor: stRow.sEditor.clone(),
                dtEdit: stRow.dtEdit,
                sTitle: stRow
                    .optOldTitle
                    .as_ref()
                    .map(|_| sTitle.clone())
                    .unwrap_or_default(),
                optMessageHtml,
                optTags: stRow.optOldTags.as_ref().map(|_| vecTags.clone()),
                optUrl: stRow.optOldUrl.as_ref().and(optUrl.clone()),
                optLinkText: stRow.optOldLinkText.as_ref().and(optLinkText.clone()),
                optMinor: stRow.optOldMinor.map(|_| bMinor),
                optPoll: stRow.optOldPoll.as_ref().and(optPoll.clone()),
                optRestoreFrom,
                vecAddedImages,
                vecRemovedImages,
                vecAddedMainImages,
                vecRemovedMainImages,
            });

            if let Some(sOldMessage) = stRow.optOldMessage {
                sMessage = sOldMessage;
                optLastMessageId = Some(stRow.iId);
            }
            if let Some(sOldTitle) = stRow.optOldTitle {
                sTitle = sOldTitle;
            }
            if let Some(sOldTags) = stRow.optOldTags {
                vecTags = vecParseTags(&sOldTags);
            }
            if let Some(sOldUrl) = stRow.optOldUrl {
                optUrl = Some(sOldUrl);
            }
            if let Some(sOldLinkText) = stRow.optOldLinkText {
                optLinkText = Some(sOldLinkText);
            }
            if let Some(bOldMinor) = stRow.optOldMinor {
                bMinor = bOldMinor;
            }
            if let Some(stOldPoll) = stRow.optOldPoll {
                optPoll = optPollFromJson(&stOldPoll);
            }
            if let Some(vecOldImages) = stRow.optOldAdditionalImages {
                vecImages = vecOldImages;
            }
        }

        vecPrepared.push(StPreparedEditHistory {
            bOriginal: true,
            bCurrent: false,
            sEditor: stSource.sAuthor,
            dtEdit: stSource.dtPost,
            sTitle,
            optMessageHtml: Some(markup::render_message_with_markup(
                &sMessage,
                Some(&sMarkup),
                None,
            )),
            optTags: (!vecTags.is_empty()).then_some(vecTags),
            optUrl,
            optLinkText,
            optMinor: None,
            optPoll,
            optRestoreFrom: optLastMessageId,
            vecAddedImages: vecImages,
            vecRemovedImages: Vec::new(),
            vecAddedMainImages: Vec::new(),
            vecRemovedMainImages: Vec::new(),
        });
        Ok(vecPrepared)
    }

    pub async fn vecCommentHistory(
        &self,
        iTopicId: i32,
        iCommentId: i32,
    ) -> Result<Vec<StPreparedEditHistory>> {
        let stSource = self.oRepository.stCommentSource(iCommentId).await?;
        if stSource.iTopicId != iTopicId {
            return Err(AppError::NotFound);
        }
        let vecRows = self.oRepository.vecRows(iCommentId, "COMMENT").await?;
        if vecRows.is_empty() {
            return Ok(Vec::new());
        }
        let mut sMessage = stSource.sMessage;
        let mut sTitle = stSource.sTitle;
        let mut vecPrepared = Vec::with_capacity(vecRows.len() + 1);
        for (iIndex, stRow) in vecRows.into_iter().enumerate() {
            vecPrepared.push(StPreparedEditHistory {
                bOriginal: false,
                bCurrent: iIndex == 0,
                sEditor: stRow.sEditor,
                dtEdit: stRow.dtEdit,
                sTitle: stRow
                    .optOldTitle
                    .as_ref()
                    .map(|_| sTitle.clone())
                    .unwrap_or_default(),
                optMessageHtml: stRow.optOldMessage.as_ref().map(|_| {
                    markup::render_message_with_markup(&sMessage, Some(&stSource.sMarkup), None)
                }),
                optTags: None,
                optUrl: None,
                optLinkText: None,
                optMinor: None,
                optPoll: None,
                optRestoreFrom: None,
                vecAddedImages: Vec::new(),
                vecRemovedImages: Vec::new(),
                vecAddedMainImages: Vec::new(),
                vecRemovedMainImages: Vec::new(),
            });
            if let Some(sOldMessage) = stRow.optOldMessage {
                sMessage = sOldMessage;
            }
            if let Some(sOldTitle) = stRow.optOldTitle {
                sTitle = sOldTitle;
            }
        }
        vecPrepared.push(StPreparedEditHistory {
            bOriginal: true,
            bCurrent: false,
            sEditor: stSource.sAuthor,
            dtEdit: stSource.dtPost,
            sTitle,
            optMessageHtml: Some(markup::render_message_with_markup(
                &sMessage,
                Some(&stSource.sMarkup),
                None,
            )),
            optTags: None,
            optUrl: None,
            optLinkText: None,
            optMinor: None,
            optPoll: None,
            optRestoreFrom: None,
            vecAddedImages: Vec::new(),
            vecRemovedImages: Vec::new(),
            vecAddedMainImages: Vec::new(),
            vecRemovedMainImages: Vec::new(),
        });
        Ok(vecPrepared)
    }

    pub async fn sRestorableTopicMessage(&self, iTopicId: i32, iRecordId: i32) -> Result<String> {
        self.oRepository
            .sRestorableTopicMessage(iTopicId, iRecordId)
            .await
    }
}

fn vecParseTags(sRaw: &str) -> Vec<String> {
    let mut setSeen = HashSet::new();
    sRaw.replace('|', ",")
        .split(',')
        .map(str::trim)
        .filter(|sTag| !sTag.is_empty())
        .map(str::to_lowercase)
        .filter(|sTag| setSeen.insert(sTag.clone()))
        .collect()
}

fn optPollFromJson(stValue: &serde_json::Value) -> Option<StHistoryPoll> {
    let bMultiSelect = stValue
        .get("multiSelect")
        .or_else(|| stValue.get("multiselect"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let vecVariants = stValue
        .get("variants")?
        .as_array()?
        .iter()
        .filter_map(|stVariant| {
            stVariant
                .get("label")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect();
    Some(StHistoryPoll {
        bMultiSelect,
        vecVariants,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_tags_follow_java_split_lowercase_and_deduplication() {
        assert_eq!(
            vecParseTags("Rust, Linux | rust"),
            vec!["rust".to_string(), "linux".to_string()]
        );
    }

    #[test]
    fn current_poll_json_shape_is_read_back_for_history() {
        let stPoll = optPollFromJson(&serde_json::json!({
            "multiSelect": true,
            "variants": [{"id": 1, "label": "A"}, {"id": 2, "label": "B"}]
        }))
        .expect("poll");
        assert!(stPoll.bMultiSelect);
        assert_eq!(stPoll.vecVariants, ["A", "B"]);
    }
}
