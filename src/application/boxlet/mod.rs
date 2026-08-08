use std::{cmp::Ordering, path::PathBuf};

use crate::{
    domain::boxlet::{
        model::{StGalleryBoxletItem, StTagCloudItem},
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
                sAltTitle: sProcessTitle(&stRow.sTitle),
                sTitle: stRow.sTitle,
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

fn sProcessTitle(sTitle: &str) -> String {
    sTitle.trim().replace(" -- ", "&nbsp;&mdash; ")
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

    use super::CBoxletService;
    use crate::{
        domain::boxlet::{
            model::{StGalleryBoxletRow, StTagCloudRow},
            repository::TrBoxletRepository,
        },
        error::Result,
    };

    #[derive(Debug, Clone, Default)]
    struct CTestRepository {
        vecTags: Vec<StTagCloudRow>,
        vecGallery: Vec<StGalleryBoxletRow>,
        mapNicks: HashMap<i32, String>,
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
                    stGalleryRow(42, 7, "  Тест -- тире  "),
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
        assert_eq!(stItem.sAltTitle, "Тест&nbsp;&mdash; тире");

        std::fs::remove_dir_all(&pathRoot).expect("remove test directory");
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
