use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::{
    domain::{
        image::{
            StImageDeleteActor, StImageDeleteTarget, StImageReference, TrImageDeleteRepository,
            stCheckImageDelete,
        },
        user::model::StUserSummary,
    },
    error::{AppError, Result},
};

#[derive(Debug, Clone)]
pub struct StPreparedImageAsset {
    pub iId: i32,
    pub sMediumUrl: String,
    pub sOriginalUrl: String,
    pub iWidth: i32,
    pub iHeight: i32,
    pub iMediumWidth: i32,
    pub iMediumHeight: i32,
    pub sSrcSet: String,
}

#[derive(Debug, Clone)]
pub struct StImageDeleteForm {
    pub stTarget: StImageDeleteTarget,
    pub stImage: StPreparedImageAsset,
}

#[derive(Debug, Clone)]
pub struct CImageDeleteService<R>
where
    R: TrImageDeleteRepository,
{
    oRepository: R,
    pathUploadRoot: PathBuf,
}

impl<R> CImageDeleteService<R>
where
    R: TrImageDeleteRepository,
{
    pub fn new(oRepository: R, pathUploadRoot: impl Into<PathBuf>) -> Self {
        Self {
            oRepository,
            pathUploadRoot: pathUploadRoot.into(),
        }
    }

    pub async fn stForm(
        &self,
        stUser: &StUserSummary,
        iImageId: i32,
        sRemoteIp: &str,
    ) -> Result<StImageDeleteForm> {
        let stTarget = self.stCheckedTarget(stUser, iImageId, sRemoteIp).await?;
        let stReference = StImageReference {
            iId: stTarget.iImageId,
            sExtension: stTarget.sImageExtension.clone(),
        };
        let pathUploadRoot = self.pathUploadRoot.clone();
        let optImage = tokio::task::spawn_blocking(move || {
            optPrepareImageAsset(&pathUploadRoot, &stReference)
        })
        .await
        .map_err(|stError| AppError::Anyhow(stError.into()))?;
        let stImage = optImage.ok_or_else(|| {
            AppError::Anyhow(anyhow::anyhow!(
                "image {} exists in the database but its files are unavailable",
                stTarget.iImageId
            ))
        })?;
        Ok(StImageDeleteForm { stTarget, stImage })
    }

    pub async fn sDelete(
        &self,
        stUser: &StUserSummary,
        iImageId: i32,
        sRemoteIp: &str,
    ) -> Result<String> {
        let stTarget = self.stCheckedTarget(stUser, iImageId, sRemoteIp).await?;
        self.oRepository
            .vDelete(stTarget.iImageId, stTarget.iTopicId, stUser.id)
            .await?;
        Ok(stTarget.sForceLastModUrl())
    }

    async fn stCheckedTarget(
        &self,
        stUser: &StUserSummary,
        iImageId: i32,
        sRemoteIp: &str,
    ) -> Result<StImageDeleteTarget> {
        let (optTarget, stRestrictions) = tokio::try_join!(
            self.oRepository.optTarget(iImageId),
            self.oRepository.stRestrictions(stUser.id, sRemoteIp),
        )?;
        let stTarget = optTarget.ok_or(AppError::NotFound)?;
        let vecReferences = stTarget.vecActiveImages.clone();
        let pathUploadRoot = self.pathUploadRoot.clone();
        let iPreparedImageCount = tokio::task::spawn_blocking(move || {
            vecReferences
                .iter()
                .filter(|stImage| optPrepareImageAsset(&pathUploadRoot, stImage).is_some())
                .count()
        })
        .await
        .map_err(|stError| AppError::Anyhow(stError.into()))?;
        let stPermission = stCheckImageDelete(
            &stTarget,
            stActor(stUser),
            stRestrictions,
            iPreparedImageCount,
            Utc::now(),
        );
        if !stPermission.bPermitted() {
            return Err(AppError::Forbidden);
        }
        Ok(stTarget)
    }
}

fn stActor(stUser: &StUserSummary) -> StImageDeleteActor {
    StImageDeleteActor {
        iUserId: stUser.id,
        iScore: stUser.score.unwrap_or(0),
        bModerator: stUser.canmod,
        bAdministrator: stUser.candel,
        bCorrector: stUser.corrector,
        bBlocked: stUser.blocked.unwrap_or(false),
    }
}

fn optPrepareImageAsset(
    pathUploadRoot: &Path,
    stImage: &StImageReference,
) -> Option<StPreparedImageAsset> {
    let pathOriginal = pathUploadRoot.join(format!(
        "images/{}/original.{}",
        stImage.iId, stImage.sExtension
    ));
    let pathMedium = pathUploadRoot.join(format!("images/{}/1000px.jpg", stImage.iId));
    let (iWidth, iHeight) = image::image_dimensions(pathOriginal).ok()?;
    let (iMediumWidth, iMediumHeight) = image::image_dimensions(pathMedium).ok()?;
    let sSrcSet = [500, 1000, 1500, 2000]
        .into_iter()
        .map(|iSize| format!("/images/{}/{iSize}px.jpg {iSize}w", stImage.iId))
        .collect::<Vec<_>>()
        .join(", ");
    Some(StPreparedImageAsset {
        iId: stImage.iId,
        sMediumUrl: format!("/images/{}/1000px.jpg", stImage.iId),
        sOriginalUrl: format!("/images/{}/original.{}", stImage.iId, stImage.sExtension),
        iWidth: iWidth as i32,
        iHeight: iHeight as i32,
        iMediumWidth: iMediumWidth as i32,
        iMediumHeight: iMediumHeight as i32,
        sSrcSet,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::{Duration, TimeZone};

    use crate::domain::{
        image::{StImageDeleteRestrictions, StImageDeleteTarget},
        topic::posting::StIpBlockInfo,
    };

    use super::*;

    #[derive(Clone)]
    struct CRepository {
        stTarget: StImageDeleteTarget,
        vecDeletes: Arc<Mutex<Vec<(i32, i32, i32)>>>,
    }

    #[async_trait]
    impl TrImageDeleteRepository for CRepository {
        async fn optTarget(&self, iImageId: i32) -> Result<Option<StImageDeleteTarget>> {
            Ok((iImageId == self.stTarget.iImageId).then(|| self.stTarget.clone()))
        }

        async fn stRestrictions(
            &self,
            _iUserId: i32,
            _sRemoteIp: &str,
        ) -> Result<StImageDeleteRestrictions> {
            Ok(StImageDeleteRestrictions {
                bFrozen: false,
                stIpBlock: StIpBlockInfo::default(),
            })
        }

        async fn vDelete(&self, iImageId: i32, iTopicId: i32, iEditorId: i32) -> Result<()> {
            self.vecDeletes
                .lock()
                .unwrap()
                .push((iImageId, iTopicId, iEditorId));
            Ok(())
        }
    }

    fn stUser() -> StUserSummary {
        StUserSummary {
            id: 7,
            nick: "author".into(),
            name: None,
            score: Some(100),
            max_score: Some(100),
            photo: None,
            town: None,
            regdate: None,
            canmod: false,
            candel: false,
            corrector: false,
            blocked: Some(false),
            userinfo: None,
        }
    }

    fn stTarget(vecActiveImages: Vec<StImageReference>) -> StImageDeleteTarget {
        let dtNow = Utc.with_ymd_and_hms(2026, 8, 15, 12, 0, 0).unwrap();
        StImageDeleteTarget {
            iImageId: 101,
            iTopicId: 202,
            sImageExtension: "png".into(),
            iAuthorId: 7,
            sTopicTitle: "test".into(),
            bTopicDeleted: false,
            bDraft: false,
            bCommitted: false,
            bSticky: false,
            bExpired: false,
            iPostScore: -9999,
            dtPostDate: dtNow - Duration::days(1),
            optCommitDate: None,
            dtLastMod: dtNow,
            iSectionId: 2,
            bSectionPremoderated: false,
            bSectionImagePost: false,
            sSectionPrefix: "forum".into(),
            sGroupUrlName: "general".into(),
            sMarkup: "MARKDOWN".into(),
            vecActiveImages,
        }
    }

    fn vWriteImage(pathRoot: &Path, stReference: &StImageReference) {
        let pathDirectory = pathRoot.join(format!("images/{}", stReference.iId));
        std::fs::create_dir_all(&pathDirectory).unwrap();
        let stImage = image::RgbImage::from_pixel(4, 3, image::Rgb([20, 30, 40]));
        stImage
            .save(pathDirectory.join(format!("original.{}", stReference.sExtension)))
            .unwrap();
        stImage.save(pathDirectory.join("1000px.jpg")).unwrap();
    }

    #[tokio::test]
    async fn service_checks_prepared_images_then_runs_repository_mutation() {
        let pathRoot =
            std::env::temp_dir().join(format!("lor-image-delete-{}", uuid::Uuid::new_v4()));
        let stReference = StImageReference {
            iId: 101,
            sExtension: "png".into(),
        };
        vWriteImage(&pathRoot, &stReference);
        let vecDeletes = Arc::new(Mutex::new(Vec::new()));
        let cService = CImageDeleteService::new(
            CRepository {
                stTarget: stTarget(vec![stReference]),
                vecDeletes: Arc::clone(&vecDeletes),
            },
            &pathRoot,
        );

        let sRedirect = cService.sDelete(&stUser(), 101, "127.0.0.1").await.unwrap();
        assert_eq!(sRedirect, "/forum/general/202?lastmod=1786795200000");
        assert_eq!(*vecDeletes.lock().unwrap(), vec![(101, 202, 7)]);
        std::fs::remove_dir_all(pathRoot).unwrap();
    }
}
