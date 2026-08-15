use std::{cmp::Ordering, path::PathBuf};

use crate::{
    domain::boxlet::{
        model::{
            StGalleryBoxletItem, StPollBoxlet, StTagCloudItem, StTopicBoxletItem, StTopicBoxletRow,
        },
        repository::TrBoxletRepository,
    },
    error::Result,
};

pub const I_GALLERY_BOXLET_ITEMS: i32 = 3;
pub const I_TAG_CLOUD_ITEMS: i32 = 75;

#[derive(Debug, Clone)]
pub struct CBoxletService<R>
where
    R: TrBoxletRepository,
{
    oRepository: R,
    pathUploadRoot: PathBuf,
}

impl<R> CBoxletService<R>
where
    R: TrBoxletRepository,
{
    pub fn new(oRepository: R, pathUploadRoot: impl Into<PathBuf>) -> Self {
        Self {
            oRepository,
            pathUploadRoot: pathUploadRoot.into(),
        }
    }

    pub async fn vecTagCloud(&self) -> Result<Vec<StTagCloudItem>> {
        let vecRows = self.oRepository.vecTopTags(I_TAG_CLOUD_ITEMS).await?;
        if vecRows.is_empty() {
            return Ok(Vec::new());
        }

        let vecLogCounters: Vec<f64> = vecRows
            .iter()
            .map(|stRow| f64::from(stRow.iCounter).ln())
            .collect();
        let fMax = vecLogCounters
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let fRawMin = vecLogCounters.iter().copied().fold(f64::INFINITY, f64::min);
        let fMin = if fMax == fRawMin { fMax - 1.0 } else { fRawMin };

        let mut vecTags: Vec<StTagCloudItem> = vecRows
            .into_iter()
            .zip(vecLogCounters)
            .map(|(stRow, fCounter)| StTagCloudItem {
                sUrl: format!("/tag/{}", urlencoding::encode(&stRow.sValue)),
                sValue: stRow.sValue,
                iWeight: (10.0 * (fCounter - fMin) / (fMax - fMin)).round() as i32,
            })
            .collect();
        vecTags.sort_by(|stLeft, stRight| enJavaStringCompare(&stLeft.sValue, &stRight.sValue));
        Ok(vecTags)
    }

    pub async fn vecGallery(&self) -> Result<Vec<StGalleryBoxletItem>> {
        let vecRows = self
            .oRepository
            .vecGalleryItems(I_GALLERY_BOXLET_ITEMS)
            .await?;
        let mut vecItems = Vec::with_capacity(vecRows.len());

        for stRow in vecRows {
            let sUserNick = match self.oRepository.sUserNick(stRow.iUserId).await {
                Ok(sNick) => sNick,
                Err(stError) => {
                    tracing::warn!(
                        topic_id = stRow.iMsgId,
                        user_id = stRow.iUserId,
                        error = %stError,
                        "failed to prepare gallery boxlet user"
                    );
                    continue;
                }
            };
            let sImageMedium = format!("images/{}/1000px.jpg", stRow.iImageId);
            let pathMedium = self.pathUploadRoot.join(&sImageMedium);
            let (iImageWidth, iImageHeight) = match image::image_dimensions(&pathMedium) {
                Ok(stDimensions) => stDimensions,
                Err(stError) => {
                    tracing::warn!(
                        topic_id = stRow.iMsgId,
                        image_id = stRow.iImageId,
                        extension = %stRow.sExtension,
                        path = %pathMedium.display(),
                        error = %stError,
                        "failed to get gallery boxlet image info"
                    );
                    continue;
                }
            };

            vecItems.push(StGalleryBoxletItem {
                sAltTitle: crate::domain::title::sProcessTitlePlainForDisplay(&stRow.sTitle),
                sTitle: crate::domain::title::sPlainForDisplay(&stRow.sTitle),
                iStat: stRow.iStat,
                sUserNick,
                sLink: format!("/gallery/{}/{}", stRow.sGroupUrlName, stRow.iMsgId),
                sImageSrcset: sImageSrcset(stRow.iImageId),
                sImageMedium,
                iImageWidth,
                iImageHeight,
                sImagePaddingPercent: sJavaDouble(
                    100.0 * f64::from(iImageHeight) / f64::from(iImageWidth),
                ),
            });
        }

        Ok(vecItems)
    }

    pub async fn iMessagesPerPage(&self, optUserId: Option<i32>) -> Result<i32> {
        let optSettings = match optUserId {
            Some(iUserId) => self.oRepository.optUserSettings(iUserId).await?,
            None => None,
        };
        Ok(crate::profile::ProfileSettings::from_hstore_text(optSettings).messages)
    }

    pub async fn vecTop10(&self, iMessagesPerPage: i32) -> Result<Vec<StTopicBoxletItem>> {
        self.vecPrepareTopics(self.oRepository.vecTopTopics().await?, iMessagesPerPage)
    }

    pub async fn vecArticles(&self, iMessagesPerPage: i32) -> Result<Vec<StTopicBoxletItem>> {
        self.vecPrepareTopics(self.oRepository.vecArticles().await?, iMessagesPerPage)
    }

    pub async fn stPoll(&self, optUserId: Option<i32>) -> Result<StPollBoxlet> {
        let mut vecPolls = self.oRepository.vecMostRecentPolls().await?;
        if vecPolls.len() != 1 {
            return Err(anyhow::anyhow!(if vecPolls.is_empty() {
                "Голосование не существует"
            } else {
                "найдено несколько самых новых голосований"
            })
            .into());
        }
        let stPoll = vecPolls.remove(0);
        let vecVariants = self
            .oRepository
            .vecPollResults(stPoll.iPollId, optUserId.unwrap_or(0))
            .await?;
        let bUserVoted = vecVariants.iter().any(|stVariant| stVariant.bUserVoted);
        let iVotes = self.oRepository.iPollVotes(stPoll.iPollId).await?;
        let iUsers = self.oRepository.iPollUsers(stPoll.iPollId).await?;

        Ok(StPollBoxlet {
            iPollId: stPoll.iPollId,
            iTopicId: stPoll.iTopicId,
            bMultiSelect: stPoll.bMultiSelect,
            sTitle: crate::domain::title::sMakeTitlePlainForDisplay(&stPoll.sTitle),
            vecVariants,
            iVotes,
            iUsers,
            bUserVoted,
        })
    }

    fn vecPrepareTopics(
        &self,
        vecRows: Vec<StTopicBoxletRow>,
        iMessagesPerPage: i32,
    ) -> Result<Vec<StTopicBoxletItem>> {
        vecRows
            .into_iter()
            .map(|stRow| {
                let sSection = match stRow.iSectionId {
                    1 => "news",
                    2 => "forum",
                    3 => "gallery",
                    5 => "polls",
                    6 => "articles",
                    iSectionId => {
                        return Err(anyhow::anyhow!("Раздел #{iSectionId} не существует").into());
                    }
                };
                let sUrl = format!("/{}/{}/{}", sSection, stRow.sGroupUrlName, stRow.iMsgId);
                let iPages =
                    ((f64::from(stRow.iCommentCount) / f64::from(iMessagesPerPage)).ceil()) as i32;
                let iLastModified = stRow.dtLastModified.timestamp_millis();
                let sMessageUrl = if iPages == 1 {
                    format!("{sUrl}?lastmod={iLastModified}")
                } else {
                    sUrl.clone()
                };
                let optLastPageUrl = (iPages > 1)
                    .then(|| format!("{sUrl}/page{}?lastmod={iLastModified}", iPages - 1));

                Ok(StTopicBoxletItem {
                    sMessageUrl,
                    sTitle: crate::domain::title::sProcessTitlePlainForDisplay(&stRow.sTitle),
                    iCommentCount: stRow.iCommentCount,
                    iPages,
                    optLastPageUrl,
                })
            })
            .collect()
    }
}

fn enJavaStringCompare(sLeft: &str, sRight: &str) -> Ordering {
    sLeft.encode_utf16().cmp(sRight.encode_utf16())
}

fn sImageSrcset(iImageId: i32) -> String {
    [500, 1000, 1500, 2000]
        .into_iter()
        .map(|iSize| format!("images/{iImageId}/{iSize}px.jpg {iSize}w"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sJavaDouble(fValue: f64) -> String {
    let mut sValue = fValue.to_string();
    if !sValue.contains('.') && !sValue.contains('e') && !sValue.contains('E') {
        sValue.push_str(".0");
    }
    sValue
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use async_trait::async_trait;
    use chrono::{TimeZone, Utc};

    use super::CBoxletService;
    use crate::{
        domain::boxlet::{
            model::{
                StGalleryBoxletRow, StPollBoxletRow, StPollVariantResult, StTagCloudRow,
                StTopicBoxletRow,
            },
            repository::TrBoxletRepository,
        },
        error::Result,
    };

    #[derive(Debug, Clone, Default)]
    struct CTestRepository {
        vecTags: Vec<StTagCloudRow>,
        vecGallery: Vec<StGalleryBoxletRow>,
        mapNicks: HashMap<i32, String>,
        vecTopTopics: Vec<StTopicBoxletRow>,
        vecArticles: Vec<StTopicBoxletRow>,
        optSettings: Option<String>,
        vecPolls: Vec<StPollBoxletRow>,
        vecPollResults: Vec<StPollVariantResult>,
        iPollVotes: i32,
        iPollUsers: i32,
    }

    #[async_trait]
    impl TrBoxletRepository for CTestRepository {
        async fn vecTopTags(&self, _iLimit: i32) -> Result<Vec<StTagCloudRow>> {
            Ok(self.vecTags.clone())
        }

        async fn vecGalleryItems(&self, _iLimit: i32) -> Result<Vec<StGalleryBoxletRow>> {
            Ok(self.vecGallery.clone())
        }

        async fn sUserNick(&self, iUserId: i32) -> Result<String> {
            Ok(self.mapNicks[&iUserId].clone())
        }

        async fn vecTopTopics(&self) -> Result<Vec<StTopicBoxletRow>> {
            Ok(self.vecTopTopics.clone())
        }

        async fn vecArticles(&self) -> Result<Vec<StTopicBoxletRow>> {
            Ok(self.vecArticles.clone())
        }

        async fn optUserSettings(&self, _iUserId: i32) -> Result<Option<String>> {
            Ok(self.optSettings.clone())
        }

        async fn vecMostRecentPolls(&self) -> Result<Vec<StPollBoxletRow>> {
            Ok(self.vecPolls.clone())
        }

        async fn vecPollResults(
            &self,
            _iPollId: i32,
            _iUserId: i32,
        ) -> Result<Vec<StPollVariantResult>> {
            Ok(self.vecPollResults.clone())
        }

        async fn iPollVotes(&self, _iPollId: i32) -> Result<i32> {
            Ok(self.iPollVotes)
        }

        async fn iPollUsers(&self, _iPollId: i32) -> Result<i32> {
            Ok(self.iPollUsers)
        }
    }

    #[tokio::test]
    async fn tag_cloud_matches_java_log_weights_sorting_and_urls() {
        let cService = CBoxletService::new(
            CTestRepository {
                vecTags: vec![
                    StTagCloudRow {
                        sValue: "rust".to_string(),
                        iCounter: 100,
                    },
                    StTagCloudRow {
                        sValue: "c++".to_string(),
                        iCounter: 10,
                    },
                    StTagCloudRow {
                        sValue: "линукс ядро".to_string(),
                        iCounter: 32,
                    },
                ],
                ..CTestRepository::default()
            },
            "unused",
        );

        let vecTags = cService.vecTagCloud().await.expect("tag cloud");
        assert_eq!(
            vecTags
                .iter()
                .map(|stTag| (stTag.sValue.as_str(), stTag.iWeight, stTag.sUrl.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("c++", 0, "/tag/c%2B%2B"),
                ("rust", 10, "/tag/rust"),
                (
                    "линукс ядро",
                    5,
                    "/tag/%D0%BB%D0%B8%D0%BD%D1%83%D0%BA%D1%81%20%D1%8F%D0%B4%D1%80%D0%BE"
                ),
            ]
        );
    }

    #[tokio::test]
    async fn equal_tag_counters_get_java_weight_ten() {
        let cService = CBoxletService::new(
            CTestRepository {
                vecTags: vec![
                    StTagCloudRow {
                        sValue: "a".to_string(),
                        iCounter: 10,
                    },
                    StTagCloudRow {
                        sValue: "b".to_string(),
                        iCounter: 10,
                    },
                ],
                ..CTestRepository::default()
            },
            "unused",
        );

        let vecTags = cService.vecTagCloud().await.expect("tag cloud");
        assert!(vecTags.iter().all(|stTag| stTag.iWeight == 10));
    }

    #[tokio::test]
    async fn gallery_uses_java_paths_dimensions_and_skips_missing_media() {
        let pathRoot = stTestRoot();
        let pathImage = pathRoot.join("images/7/1000px.jpg");
        std::fs::create_dir_all(pathImage.parent().expect("image parent"))
            .expect("create image directory");
        image::RgbImage::from_pixel(640, 480, image::Rgb([1, 2, 3]))
            .save(&pathImage)
            .expect("save test image");

        let cService = CBoxletService::new(
            CTestRepository {
                vecGallery: vec![
                    stGalleryRow(
                        42,
                        7,
                        "  A &amp; B &lt; C &quot;Q&quot; &#39;X&#39; &#x41; &#128512; -- tail  ",
                    ),
                    stGalleryRow(43, 8, "missing"),
                ],
                mapNicks: HashMap::from([(11, "tester".to_string())]),
                ..CTestRepository::default()
            },
            &pathRoot,
        );

        let vecItems = cService.vecGallery().await.expect("gallery");
        assert_eq!(vecItems.len(), 1);
        let stItem = &vecItems[0];
        assert_eq!(stItem.sLink, "/gallery/screenshots/42");
        assert_eq!(stItem.sImageMedium, "images/7/1000px.jpg");
        assert_eq!(
            stItem.sImageSrcset,
            "images/7/500px.jpg 500w, images/7/1000px.jpg 1000w, images/7/1500px.jpg 1500w, images/7/2000px.jpg 2000w"
        );
        assert_eq!((stItem.iImageWidth, stItem.iImageHeight), (640, 480));
        assert_eq!(stItem.sImagePaddingPercent, "75.0");
        // gallery.jsp prints the visible title verbatim, while its alt text
        // passes through TitleTag/processTitle before the browser decodes it.
        assert_eq!(stItem.sTitle, "  A & B < C \"Q\" 'X' A 😀 -- tail  ");
        assert_eq!(stItem.sAltTitle, "A & B < C \"Q\" 'X' A 😀\u{a0}— tail");

        std::fs::remove_dir_all(&pathRoot).expect("remove test directory");
    }

    #[tokio::test]
    async fn topic_boxlet_uses_profile_page_size_and_java_lastmod_links() {
        let stRow = StTopicBoxletRow {
            iMsgId: 42,
            sGroupUrlName: "linux-org-ru".to_owned(),
            iSectionId: 2,
            sTitle: "  A &amp; B &lt; C &quot;Q&quot; &#39;X&#39; &#x41; &#128512; -- LOR  "
                .to_owned(),
            dtLastModified: Utc.timestamp_millis_opt(1_725_000_000_123).unwrap(),
            iCommentCount: 26,
        };
        let cService = CBoxletService::new(
            CTestRepository {
                vecTopTopics: vec![stRow.clone()],
                vecArticles: vec![StTopicBoxletRow {
                    iSectionId: 6,
                    ..stRow
                }],
                optSettings: Some("\"messages\"=>\"25\"".to_owned()),
                ..CTestRepository::default()
            },
            "unused",
        );

        let iMessages = cService
            .iMessagesPerPage(Some(7))
            .await
            .expect("profile messages");
        assert_eq!(iMessages, 25);
        let vecTop = cService.vecTop10(iMessages).await.expect("top ten");
        assert_eq!(vecTop[0].iPages, 2);
        assert_eq!(vecTop[0].sMessageUrl, "/forum/linux-org-ru/42");
        assert_eq!(
            vecTop[0].optLastPageUrl.as_deref(),
            Some("/forum/linux-org-ru/42/page1?lastmod=1725000000123")
        );
        assert_eq!(vecTop[0].sTitle, "A & B < C \"Q\" 'X' A 😀\u{a0}— LOR");

        let vecArticles = cService.vecArticles(50).await.expect("articles");
        assert_eq!(
            vecArticles[0].sMessageUrl,
            "/articles/linux-org-ru/42?lastmod=1725000000123"
        );
        assert_eq!(vecArticles[0].iPages, 1);
        assert!(vecArticles[0].optLastPageUrl.is_none());
    }

    #[tokio::test]
    async fn poll_boxlet_aggregates_original_vote_state_and_counts() {
        let cService = CBoxletService::new(
            CTestRepository {
                vecPolls: vec![StPollBoxletRow {
                    iPollId: 8,
                    iTopicId: 88,
                    bMultiSelect: true,
                    sTitle: "A &amp; B &lt; C &quot;Q&quot; &#39;X&#39; &#x41; &#128512;"
                        .to_owned(),
                }],
                vecPollResults: vec![
                    StPollVariantResult {
                        iId: 1,
                        sLabel: "A".to_owned(),
                        iVotes: 4,
                        bUserVoted: false,
                    },
                    StPollVariantResult {
                        iId: 2,
                        sLabel: "B".to_owned(),
                        iVotes: 3,
                        bUserVoted: true,
                    },
                ],
                iPollVotes: 7,
                iPollUsers: 5,
                ..CTestRepository::default()
            },
            "unused",
        );

        let stPoll = cService.stPoll(Some(17)).await.expect("poll");
        assert_eq!(stPoll.iPollId, 8);
        assert!(stPoll.bUserVoted);
        assert_eq!((stPoll.iVotes, stPoll.iUsers), (7, 5));
        assert_eq!(stPoll.sTitle, "A & B < C «Q» 'X' A 😀");
    }

    #[tokio::test]
    async fn missing_recent_poll_matches_java_error_path() {
        let cService = CBoxletService::new(CTestRepository::default(), "unused");
        assert!(cService.stPoll(None).await.is_err());
    }

    fn stGalleryRow(iMsgId: i32, iImageId: i32, sTitle: &str) -> StGalleryBoxletRow {
        StGalleryBoxletRow {
            iMsgId,
            iUserId: 11,
            sTitle: sTitle.to_string(),
            iStat: 9,
            sGroupUrlName: "screenshots".to_string(),
            iImageId,
            sExtension: "jpg".to_string(),
        }
    }

    fn stTestRoot() -> PathBuf {
        std::env::temp_dir().join(format!("lor-boxlet-test-{}", uuid::Uuid::new_v4()))
    }
}
