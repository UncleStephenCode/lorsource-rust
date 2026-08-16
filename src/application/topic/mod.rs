pub mod deletion;
pub mod edit;
pub mod moderation;
pub mod options;
pub mod posting;

use crate::domain::comment::model::{StCommentItem, StCommentPageMeta};
use crate::domain::markup::model::StMarkupUserDirectory;
use crate::domain::topic::{
    model::{
        StLegacyTopicRedirect, StMainTopicSummary, StRssImage, StRssPoll, StRssTag, StRssTopic,
        StTopicDetail, StTopicScrollers, StTopicSummary,
    },
    repository::{StNewTopic, TrTopicRepository},
};
use crate::error::{AppError, Result};
use chrono::{DateTime, Months, Utc};
use sqlx::{Postgres, Transaction};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct CTopicService<R>
where
    R: TrTopicRepository,
{
    oRepository: R,
}

#[derive(Debug, Clone)]
pub struct StRssFeed {
    pub sTitle: String,
    pub vecItems: Vec<StPreparedRssTopic>,
    pub optLastModified: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct StRssSource {
    pub sTitle: String,
    pub vecTopics: Vec<StRssTopic>,
    pub optLastModified: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct StPreparedRssTopic {
    pub sStoredTitle: String,
    pub dtPublished: DateTime<Utc>,
    pub sAuthorNick: String,
    pub sTopicUrl: String,
    pub sDescriptionElement: String,
}

impl<R> CTopicService<R>
where
    R: TrTopicRepository,
{
    pub fn new(oRepository: R) -> Self {
        Self { oRepository }
    }

    pub async fn vecListTopics(
        &self,
        optSection: Option<&str>,
        optGroup: Option<&str>,
        iOffset: i64,
        iLimit: i64,
    ) -> Result<Vec<StTopicSummary>> {
        self.oRepository
            .vecListTopics(optSection, optGroup, iOffset, iLimit, false, false)
            .await
    }

    pub async fn vecListTopicsFiltered(
        &self,
        optSection: Option<&str>,
        optGroup: Option<&str>,
        iOffset: i64,
        iLimit: i64,
        bNoTalks: bool,
        bTech: bool,
    ) -> Result<Vec<StTopicSummary>> {
        self.oRepository
            .vecListTopics(optSection, optGroup, iOffset, iLimit, bNoTalks, bTech)
            .await
    }

    pub async fn vecListMainTopics(
        &self,
        bShowGalleryOnMain: bool,
        optViewerId: Option<i32>,
        iLimit: i64,
    ) -> Result<Vec<StMainTopicSummary>> {
        self.oRepository
            .vecListMainTopics(bShowGalleryOnMain, optViewerId, iLimit)
            .await
    }

    pub async fn stGetTopic(&self, iTopicId: i32) -> Result<StTopicDetail> {
        self.oRepository.stGetTopic(iTopicId).await
    }

    pub async fn stLegacyTopicRedirect(&self, iTopicId: i32) -> Result<StLegacyTopicRedirect> {
        self.oRepository.stLegacyTopicRedirect(iTopicId).await
    }

    pub async fn stTopicScrollers(
        &self,
        iTopicId: i32,
        optViewerIdForIgnoreList: Option<i32>,
    ) -> Result<StTopicScrollers> {
        self.oRepository
            .stTopicScrollers(iTopicId, optViewerIdForIgnoreList)
            .await
    }

    pub async fn stRssSource(
        &self,
        iSectionId: i32,
        iGroupId: i32,
        optFilter: Option<&str>,
        optViewerId: Option<i32>,
    ) -> Result<StRssSource> {
        let (bNoTalks, bTech, optFilterTitle) = stParseRssFilter(optFilter)?;
        let stContext = self.oRepository.stRssContext(iSectionId, iGroupId).await?;
        let mut sTitle = stContext.sSectionName.clone();
        if let Some(sGroupTitle) = stContext.optGroupTitle.as_deref() {
            sTitle.push_str(" - ");
            sTitle.push_str(sGroupTitle);
        }
        if let Some(sFilterTitle) = optFilterTitle {
            sTitle.push_str(" (");
            sTitle.push_str(sFilterTitle);
            sTitle.push(')');
        }
        let dtFrom = Utc::now()
            .checked_sub_months(Months::new(3))
            .expect("three months before a valid current date");
        let vecTopics = self
            .oRepository
            .vecListRssTopics(iSectionId, iGroupId, bNoTalks, bTech, optViewerId, dtFrom)
            .await?;
        let optLastModified = vecTopics.iter().map(|stTopic| stTopic.dtLastModified).max();
        Ok(StRssSource {
            sTitle,
            vecTopics,
            optLastModified,
        })
    }

    pub async fn stPrepareRssFeedWithUsers(
        &self,
        stSource: StRssSource,
        sPublicUrl: &str,
        sUploadDir: &str,
        optMarkupUsers: Option<&StMarkupUserDirectory>,
    ) -> Result<StRssFeed> {
        let mut vecItems = Vec::with_capacity(stSource.vecTopics.len());
        for stTopic in stSource.vecTopics {
            let sDescriptionElement =
                sRenderRssDescription(&stTopic, sPublicUrl, sUploadDir, optMarkupUsers).await?;
            let sTopicUrl = stTopic.sTopicUrl();
            vecItems.push(StPreparedRssTopic {
                sStoredTitle: stTopic.sStoredTitle,
                dtPublished: stTopic.dtPublished,
                sAuthorNick: stTopic.sAuthorNick,
                sTopicUrl,
                sDescriptionElement,
            });
        }
        Ok(StRssFeed {
            sTitle: stSource.sTitle,
            vecItems,
            optLastModified: stSource.optLastModified,
        })
    }

    pub async fn vecListComments(&self, iTopicId: i32) -> Result<Vec<StCommentItem>> {
        self.oRepository.vecListComments(iTopicId).await
    }

    pub async fn vecCommentPageMeta(
        &self,
        vecCommentIds: &[i32],
        optViewerId: Option<i32>,
        bModeratorSession: bool,
        bLoadWarnings: bool,
    ) -> Result<Vec<StCommentPageMeta>> {
        self.oRepository
            .vecCommentPageMeta(vecCommentIds, optViewerId, bModeratorSession, bLoadWarnings)
            .await
    }

    pub async fn iNextMessageId(&self, txPg: &mut Transaction<'_, Postgres>) -> Result<i32> {
        self.oRepository.iNextMessageId(txPg).await
    }

    pub async fn vInsertTopicMessage(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        iMsgId: i32,
        sMessage: &str,
        sMarkup: &str,
    ) -> Result<()> {
        self.oRepository
            .vInsertTopicMessage(txPg, iMsgId, sMessage, sMarkup)
            .await
    }

    pub async fn vInsertTopic(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        stNewTopic: StNewTopic<'_>,
    ) -> Result<()> {
        self.oRepository.vInsertTopic(txPg, stNewTopic).await
    }

    pub async fn vReplaceTags(
        &self,
        txPg: &mut Transaction<'_, Postgres>,
        iMsgId: i32,
        optTags: Option<&str>,
    ) -> Result<()> {
        self.oRepository.vReplaceTags(txPg, iMsgId, optTags).await
    }
}

fn stParseRssFilter(optFilter: Option<&str>) -> Result<(bool, bool, Option<&'static str>)> {
    match optFilter {
        None => Ok((false, false, None)),
        Some("notalks") => Ok((true, false, Some("без talks"))),
        Some("tech") => Ok((false, true, Some("тех. форум"))),
        Some(_) => Err(AppError::BadRequest(
            "Некорректное значение filter".to_owned(),
        )),
    }
}

async fn sRenderRssDescription(
    stTopic: &StRssTopic,
    sPublicUrl: &str,
    sUploadDir: &str,
    optMarkupUsers: Option<&StMarkupUserDirectory>,
) -> Result<String> {
    let sOrigin = sPublicUrl.trim_end_matches('/');
    let sCanonicalUrl = format!("{sOrigin}{}", stTopic.sTopicUrl());
    let mut sDescription = String::from("<description><![CDATA[\n");

    if (stTopic.bImagePost || stTopic.bImagesAllowed)
        && let Some(sImage) = optRenderFirstRssImage(
            &stTopic.vecImages,
            &stTopic.sStoredTitle,
            stTopic.bImagePost,
            sOrigin,
            sUploadDir,
        )
        .await
    {
        sDescription.push_str(&sImage);
        sDescription.push('\n');
    }

    sDescription.push_str(
        &crate::markup::render_topic_with_minimized_cut_policy_and_users(
            &stTopic.sMessage,
            &stTopic.sMarkup,
            &sCanonicalUrl,
            stTopic.bNofollow,
            Some(sPublicUrl),
            optMarkupUsers,
        ),
    );
    sDescription.push('\n');

    if stTopic.bPollPostAllowed {
        let stPoll = stTopic.optPoll.as_ref().ok_or_else(|| {
            AppError::Anyhow(anyhow::anyhow!(
                "poll is missing for poll topic #{}",
                stTopic.iId
            ))
        })?;
        sDescription.push_str(&sRenderRssPoll(stPoll));
        sDescription.push('\n');
    }

    if !stTopic.vecTags.is_empty() {
        sDescription.push_str(&sRenderRssTags(&stTopic.vecTags));
        sDescription.push('\n');
    }
    sDescription.push_str("]]></description>");
    Ok(sDescription)
}

fn sRenderRssPoll(stPoll: &StRssPoll) -> String {
    let mut sHtml = String::from("<table>");
    let mut iTotal = 0_i64;
    for stVariant in &stPoll.vecVariants {
        sHtml.push_str("<tr><td>");
        sHtml.push_str(&sEscapeHtmlLikeGuava(&stVariant.sLabel));
        sHtml.push_str("</td><td>");
        sHtml.push_str(&stVariant.iVotes.to_string());
        sHtml.push_str("</td></tr>");
        iTotal += i64::from(stVariant.iVotes);
    }
    sHtml.push_str("<tr><td colspan=2>Всего голосов: ");
    sHtml.push_str(&iTotal.to_string());
    sHtml.push_str("</td></tr>");
    if stPoll.bMultiSelect {
        sHtml.push_str("<tr><td colspan=2>Всего проголосовавших: ");
        sHtml.push_str(&stPoll.iVoterCount.to_string());
        sHtml.push_str("</td></tr>");
    }
    sHtml.push_str("</table>");
    sHtml
}

fn sRenderRssTags(vecTags: &[StRssTag]) -> String {
    let mut sHtml = String::from("<p class=\"tags\"><i class=\"icon-tag\"></i>&nbsp;");
    for (iIndex, stTag) in vecTags.iter().enumerate() {
        if iIndex != 0 {
            sHtml.push_str(", ");
        }
        let sName = sEscapeHtmlLikeGuava(&stTag.sName);
        if stTag.iCounter >= 2 && bIsGoodTag(&stTag.sName) {
            let sUrl = format!("/tag/{}", urlencoding::encode(&stTag.sName));
            sHtml.push_str("<a class=tag rel=tag href=\"");
            sHtml.push_str(&sUrl);
            sHtml.push_str("\">");
            sHtml.push_str(&sName);
            sHtml.push_str("</a>");
        } else {
            sHtml.push_str("<span class=tag>");
            sHtml.push_str(&sName);
            sHtml.push_str("</span>");
        }
    }
    sHtml.push_str("</p>");
    sHtml
}

fn bIsGoodTag(sTag: &str) -> bool {
    let iLength = sTag.encode_utf16().count();
    if !(1..=32).contains(&iLength) {
        return false;
    }
    let mut iterChars = sTag.chars();
    let Some(cFirst) = iterChars.next() else {
        return false;
    };
    let vecRest = iterChars.collect::<Vec<_>>();
    let cLast = vecRest.last().copied().unwrap_or(cFirst);
    let bEdge = |cValue: char| cValue.is_alphanumeric() || matches!(cValue, '+' | '-');
    let bMiddle =
        |cValue: char| cValue.is_alphanumeric() || matches!(cValue, '.' | ' ' | '+' | '-');
    (cFirst.is_alphanumeric() || cFirst == '-')
        && bEdge(cLast)
        && vecRest.iter().copied().all(bMiddle)
}

fn sEscapeHtmlLikeGuava(sValue: &str) -> String {
    let mut sEscaped = String::with_capacity(sValue.len());
    for cValue in sValue.chars() {
        match cValue {
            '&' => sEscaped.push_str("&amp;"),
            '<' => sEscaped.push_str("&lt;"),
            '>' => sEscaped.push_str("&gt;"),
            '\"' => sEscaped.push_str("&quot;"),
            '\'' => sEscaped.push_str("&#39;"),
            _ => sEscaped.push(cValue),
        }
    }
    sEscaped
}

async fn optRenderFirstRssImage(
    vecImages: &[StRssImage],
    sStoredTitle: &str,
    bImagePost: bool,
    sOrigin: &str,
    sUploadDir: &str,
) -> Option<String> {
    for stImage in vecImages {
        if !matches!(
            stImage.sExtension.to_ascii_lowercase().as_str(),
            "gif" | "jpg" | "jpeg" | "png"
        ) {
            // Java ImageInfo supports exactly these formats. A file with any
            // other extension is skipped by ImageService.prepareImage.
            continue;
        }
        let iImageId = stImage.iId;
        let sOriginalRelative = format!("images/{iImageId}/original.{}", stImage.sExtension);
        let sMediumRelative = format!("images/{iImageId}/1000px.jpg");
        let pathOriginal = Path::new(sUploadDir).join(&sOriginalRelative);
        let pathMedium = Path::new(sUploadDir).join(&sMediumRelative);
        let optDimensions = tokio::task::spawn_blocking(move || {
            let (iFullWidth, iFullHeight) = image::image_dimensions(pathOriginal).ok()?;
            let (iMediumWidth, iMediumHeight) = image::image_dimensions(pathMedium).ok()?;
            Some((iFullWidth, iFullHeight, iMediumWidth, iMediumHeight))
        })
        .await
        .ok()
        .flatten();
        let Some((iFullWidth, iFullHeight, iMediumWidth, iMediumHeight)) = optDimensions else {
            continue;
        };
        return Some(sRenderRssImage(
            iImageId,
            &stImage.sExtension,
            sStoredTitle,
            bImagePost,
            sOrigin,
            (iFullWidth, iFullHeight),
            (iMediumWidth, iMediumHeight),
        ));
    }
    None
}

fn sRenderRssImage(
    iImageId: i32,
    sExtension: &str,
    sStoredTitle: &str,
    bImagePost: bool,
    sOrigin: &str,
    (iFullWidth, iFullHeight): (u32, u32),
    (iMediumWidth, iMediumHeight): (u32, u32),
) -> String {
    let sOriginal = format!("{sOrigin}/images/{iImageId}/original.{sExtension}");
    let sMedium = format!("{sOrigin}/images/{iImageId}/1000px.jpg");
    let sSrcset = if iFullWidth <= 2000 {
        let sScaled = [500_u32, 1000, 1500, 2000]
            .into_iter()
            .filter(|iSize| *iSize < iFullWidth)
            .map(|iSize| format!("images/{iImageId}/{iSize}px.jpg {iSize}w"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{sScaled}, {sOriginal} {iFullWidth}w")
    } else {
        [500_u32, 1000, 1500, 2000]
            .into_iter()
            .map(|iSize| format!("images/{iImageId}/{iSize}px.jpg {iSize}w"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let fPadding = 100.0 * f64::from(iMediumHeight) / f64::from(iMediumWidth);
    let iMaxWidth = iFullWidth.min(2000);
    // PreparedTopic.message.title has already passed StringUtil.makeTitle.
    // image.tag then applies TitleTag/processTitle only to `alt`, while the
    // schema caption receives the makeTitle result directly.
    let sMadeTitle = crate::domain::title::sMakeTitleForLegacyView(sStoredTitle);
    let sProcessedTitle = crate::domain::title::sProcessTitleForLegacyView(&sMadeTitle);
    let (sOpenLink, sCloseLink) = if bImagePost || iFullWidth >= 1920 || iFullHeight >= 1080 {
        (
            format!("<a href=\"{sOriginal}\" itemprop=\"contentURL\">"),
            "</a>",
        )
    } else {
        (String::new(), "")
    };

    format!(
        "<div class=\"medium-image-container\" style=\"max-width: {iMaxWidth}px; max-height: 90vh; width: min(var(--image-width), calc(90vh * {iMediumWidth} / {iMediumHeight}))\">\n<figure class=\"medium-image\" style=\"position: relative; padding-bottom: {fPadding}%; padding-bottom: min({fPadding}%, 90vh); margin: 0\">\n{sOpenLink}<img itemprop=\"thumbnail\" class=\"medium-image\" src=\"{sMedium}\" alt=\"{sProcessedTitle}\" srcset=\"{sSrcset}\" sizes=\"100vw\" style=\"position: absolute; max-height: 90vh\" width={iMediumWidth} height={iMediumHeight}>{sCloseLink}\n<meta itemprop=\"caption\" content=\"{sMadeTitle}\">\n</figure>\n</div>"
    )
}

#[cfg(test)]
mod rss_tests {
    use super::*;
    use crate::domain::topic::model::StRssPollVariant;

    #[test]
    fn rss_filter_contract_matches_the_java_enum() {
        assert_eq!(stParseRssFilter(None).unwrap(), (false, false, None));
        assert_eq!(
            stParseRssFilter(Some("notalks")).unwrap(),
            (true, false, Some("без talks"))
        );
        assert_eq!(
            stParseRssFilter(Some("tech")).unwrap(),
            (false, true, Some("тех. форум"))
        );
        assert!(stParseRssFilter(Some("TECH")).is_err());
    }

    #[test]
    fn rss_poll_matches_prepared_poll_rendering() {
        let sHtml = sRenderRssPoll(&StRssPoll {
            bMultiSelect: true,
            iVoterCount: 2,
            vecVariants: vec![
                StRssPollVariant {
                    sLabel: "A & <B> \"Q\" 'X'".to_owned(),
                    iVotes: 2,
                },
                StRssPollVariant {
                    sLabel: "C".to_owned(),
                    iVotes: 1,
                },
            ],
        });
        assert_eq!(
            sHtml,
            "<table><tr><td>A &amp; &lt;B&gt; &quot;Q&quot; &#39;X&#39;</td><td>2</td></tr><tr><td>C</td><td>1</td></tr><tr><td colspan=2>Всего голосов: 3</td></tr><tr><td colspan=2>Всего проголосовавших: 2</td></tr></table>"
        );
    }

    #[test]
    fn rss_tags_link_only_public_tags_with_java_threshold() {
        let sHtml = sRenderRssTags(&[
            StRssTag {
                sName: "rust lang".to_owned(),
                iCounter: 2,
            },
            StRssTag {
                sName: "new-tag".to_owned(),
                iCounter: 1,
            },
        ]);
        assert!(sHtml.contains("href=\"/tag/rust%20lang\">rust lang</a>"));
        assert!(sHtml.contains("<span class=tag>new-tag</span>"));
    }

    #[test]
    fn rss_image_uses_make_title_then_process_title_only_for_alt() {
        let sHtml = sRenderRssImage(
            7,
            "png",
            "  A -- &quot;Q&quot;  ",
            true,
            "https://www.linux.org.ru",
            (1920, 1080),
            (1000, 563),
        );
        assert!(sHtml.contains("alt=\"A&nbsp;&mdash; &#171;Q&#187;\""));
        assert!(
            sHtml.contains("content=\"  A -- &#171;Q&#187;  \""),
            "{sHtml}"
        );
        assert!(!sHtml.contains("&quot;Q&quot;"));
    }

    #[tokio::test]
    async fn rss_description_contains_minimized_body_poll_and_tags() {
        let dtNow = Utc::now();
        let stTopic = StRssTopic {
            iId: 42,
            sStoredTitle: "Title".to_owned(),
            dtPublished: dtNow,
            dtLastModified: dtNow,
            sAuthorNick: "author".to_owned(),
            sGroupUrlName: "group".to_owned(),
            sSectionPrefix: "polls".to_owned(),
            sMessage: "[outside](https://outside.example/) before\n\n>>>\nhidden\n<<<\nafter"
                .to_owned(),
            sMarkup: "MARKDOWN".to_owned(),
            bImagePost: false,
            bImagesAllowed: false,
            bPollPostAllowed: true,
            bNofollow: true,
            vecTags: vec![StRssTag {
                sName: "rust".to_owned(),
                iCounter: 2,
            }],
            vecImages: Vec::new(),
            optPoll: Some(StRssPoll {
                bMultiSelect: false,
                iVoterCount: 1,
                vecVariants: vec![StRssPollVariant {
                    sLabel: "yes".to_owned(),
                    iVotes: 1,
                }],
            }),
        };
        let sHtml = sRenderRssDescription(&stTopic, "https://www.linux.org.ru", "/tmp", None)
            .await
            .unwrap();
        assert!(sHtml.starts_with("<description><![CDATA["));
        assert!(sHtml.contains("https://www.linux.org.ru/polls/group/42#cut"));
        assert!(!sHtml.contains("hidden"));
        assert!(sHtml.contains("Всего голосов: 1"));
        assert!(sHtml.contains("href=\"/tag/rust\""));
        assert!(
            !sHtml.contains("rel=\"nofollow\""),
            "Flexmark's minimized-cut renderer intentionally ignores nofollow: {sHtml}"
        );
        assert!(sHtml.ends_with("]]></description>"));
    }

    #[tokio::test]
    async fn lorcode_rss_uses_author_policy_unless_topic_is_committed() {
        let dtNow = Utc::now();
        let stRestrictedAuthor = crate::domain::topic::link_policy::StAuthorLinkState {
            iScore: 45,
            bBlocked: false,
            bAnonymous: false,
            bFrozen: false,
        };
        let stBase = StRssTopic {
            iId: 7,
            sStoredTitle: "Title".to_owned(),
            dtPublished: dtNow,
            dtLastModified: dtNow,
            sAuthorNick: "new-user".to_owned(),
            sGroupUrlName: "group".to_owned(),
            sSectionPrefix: "forum".to_owned(),
            sMessage: "https://outside.example/path".to_owned(),
            sMarkup: "BBCODE_TEX".to_owned(),
            bImagePost: false,
            bImagesAllowed: false,
            bPollPostAllowed: false,
            bNofollow: !stRestrictedAuthor.bFollowInTopic(false),
            vecTags: Vec::new(),
            vecImages: Vec::new(),
            optPoll: None,
        };
        let sRestricted = sRenderRssDescription(&stBase, "https://www.linux.org.ru", "/tmp", None)
            .await
            .unwrap();
        assert!(sRestricted.contains("rel=\"nofollow\""), "{sRestricted}");

        let mut stCommitted = stBase;
        stCommitted.bNofollow = !stRestrictedAuthor.bFollowInTopic(true);
        let sCommitted =
            sRenderRssDescription(&stCommitted, "https://www.linux.org.ru", "/tmp", None)
                .await
                .unwrap();
        assert!(!sCommitted.contains("rel=\"nofollow\""), "{sCommitted}");
    }
}
