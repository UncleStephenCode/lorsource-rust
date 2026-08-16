use std::time::Duration;

use crate::{
    domain::tag::{
        model::{EnTagSectionOutcome, EnTagSectionTopics, StTagSectionPage},
        repository::{TrTagTopicCountRepository, TrTagTopicListRepository},
    },
    error::{AppError, Result},
};

pub const I_TAG_TOPIC_MAX_OFFSET: i32 = 300;
pub const I_TAG_FEED_PAGE_SIZE: i32 = 20;

#[derive(Debug, Clone)]
pub struct CTagTopicListService<R, C>
where
    R: TrTagTopicListRepository,
    C: TrTagTopicCountRepository,
{
    oRepository: R,
    oCountRepository: C,
}

impl<R, C> CTagTopicListService<R, C>
where
    R: TrTagTopicListRepository,
    C: TrTagTopicCountRepository,
{
    pub fn new(oRepository: R, oCountRepository: C) -> Self {
        Self {
            oRepository,
            oCountRepository,
        }
    }

    pub async fn enSectionPage(
        &self,
        sTag: &str,
        iSectionId: i32,
        iRawOffset: i32,
        optViewerId: Option<i32>,
    ) -> Result<EnTagSectionOutcome> {
        // The Scala controller resolves the section before looking up the tag.
        let stSection = self
            .oRepository
            .optSection(iSectionId)
            .await?
            .ok_or(AppError::NotFound)?;
        let Some(stTag) = self.oRepository.optTagInfo(sTag).await? else {
            return self
                .oRepository
                .optSynonymTarget(sTag)
                .await?
                .map(|sMainTag| EnTagSectionOutcome::Redirect {
                    sMainTag,
                    iSectionId,
                })
                .ok_or(AppError::NotFound);
        };

        let iOffset = iFixTagOffset(iRawOffset);
        let (vecSections, stProfile, stViewerState) = tokio::try_join!(
            self.oRepository.vecTagSections(stTag.iId),
            self.oRepository.stViewerProfile(optViewerId),
            self.oRepository.stViewerState(stTag.iId, optViewerId),
        )?;
        let iPageSize = if stSection.iId == 2 {
            stProfile.iTopics
        } else {
            I_TAG_FEED_PAGE_SIZE
        };
        let enTopics = if stSection.iId == 2 {
            EnTagSectionTopics::Forum(
                self.oRepository
                    .vecForumTopics(&stSection, stTag.iId, optViewerId, iOffset, iPageSize)
                    .await?,
            )
        } else {
            EnTagSectionTopics::Feed(
                self.oRepository
                    .vecFeedTopics(&stSection, stTag.iId, optViewerId, iOffset, iPageSize)
                    .await?,
            )
        };
        if enTopics.iLen() == 0 {
            return Err(AppError::NotFound);
        }

        // TagPageController.Timeout is shared by the OpenSearch count. A
        // timeout or search outage is non-fatal and renders no counter.
        let iCounter = match tokio::time::timeout(
            Duration::from_millis(500),
            self.oCountRepository
                .iCountTagTopics(sTag, &stSection.sUrlName),
        )
        .await
        {
            Ok(Ok(iCount)) => iCount,
            Ok(Err(stError)) => {
                tracing::warn!(error = %stError, tag = sTag, section = %stSection.sUrlName, "unable to count section tag topics");
                0
            }
            Err(_) => {
                tracing::warn!(tag = sTag, section = %stSection.sUrlName, "section tag topic count timed out");
                0
            }
        };

        Ok(EnTagSectionOutcome::Page(StTagSectionPage {
            stSection,
            vecSections,
            stProfile,
            stViewerState,
            enTopics,
            iOffset,
            iPageSize,
            iCounter,
        }))
    }
}

pub fn iFixTagOffset(iOffset: i32) -> i32 {
    iOffset.clamp(0, I_TAG_TOPIC_MAX_OFFSET)
}

pub fn bTagSectionHasNext(iOffset: i32, iItems: usize, iPageSize: i32) -> bool {
    iOffset < I_TAG_TOPIC_MAX_OFFSET && iItems == iPageSize.max(0) as usize
}

pub fn optTagSectionPreviousOffset(iOffset: i32, iPageSize: i32) -> Option<i32> {
    (iOffset > iPageSize).then_some(iOffset - iPageSize)
}

pub fn sTagSectionUrl(sTag: &str, iSectionId: i32, iOffset: i32) -> String {
    let mut sUrl = format!("/tag/{}", sEncodeSpringTagPath(sTag));
    if iSectionId != 0 {
        sUrl.push_str(&format!("?section={iSectionId}"));
    }
    if iOffset != 0 {
        sUrl.push(if iSectionId == 0 { '?' } else { '&' });
        sUrl.push_str(&format!("offset={iOffset}"));
    }
    sUrl
}

fn sEncodeSpringTagPath(sValue: &str) -> String {
    // UriTemplate expands a URI path value (RFC 3986 pchar), not a single
    // percent-encoded path segment. In particular, '+' stays literal.
    const ARR_HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut sEncoded = String::with_capacity(sValue.len());
    for iByte in sValue.bytes() {
        if iByte.is_ascii_alphanumeric()
            || matches!(
                iByte,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
                    | b':'
                    | b'@'
                    | b'/'
            )
        {
            sEncoded.push(char::from(iByte));
        } else {
            sEncoded.push('%');
            sEncoded.push(char::from(ARR_HEX[usize::from(iByte >> 4)]));
            sEncoded.push(char::from(ARR_HEX[usize::from(iByte & 0x0f)]));
        }
    }
    sEncoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_is_clamped_like_topic_list_service() {
        assert_eq!(iFixTagOffset(-1), 0);
        assert_eq!(iFixTagOffset(0), 0);
        assert_eq!(iFixTagOffset(299), 299);
        assert_eq!(iFixTagOffset(301), 300);
    }

    #[test]
    fn navigation_keeps_java_strict_previous_boundary() {
        assert!(bTagSectionHasNext(0, 20, 20));
        assert!(!bTagSectionHasNext(300, 20, 20));
        assert!(!bTagSectionHasNext(0, 19, 20));
        assert_eq!(optTagSectionPreviousOffset(20, 20), None);
        assert_eq!(optTagSectionPreviousOffset(21, 20), Some(1));
        assert_eq!(optTagSectionPreviousOffset(40, 20), Some(20));
    }

    #[test]
    fn url_omits_zero_offset_and_encodes_the_tag() {
        assert_eq!(sTagSectionUrl("c++", 2, 0), "/tag/c++?section=2");
        assert_eq!(
            sTagSectionUrl("язык rust", 1, 20),
            "/tag/%D1%8F%D0%B7%D1%8B%D0%BA%20rust?section=1&offset=20"
        );
    }
}
