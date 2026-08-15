use std::io::Cursor;

use image::AnimationDecoder;

use crate::{
    domain::user::userpic::TrUserpicRepository,
    error::{AppError, Result},
};

pub const I_MAX_USERPIC_FILE_SIZE: usize = 100 * 1024;
pub const I_MIN_USERPIC_SIZE: u32 = 50;
pub const I_MAX_USERPIC_SIZE: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnUserpicFormat {
    Jpeg,
    Gif,
    Png,
}

impl EnUserpicFormat {
    pub fn sExtension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::Png => "png",
        }
    }

    fn enImageFormat(self) -> image::ImageFormat {
        match self {
            Self::Jpeg => image::ImageFormat::Jpeg,
            Self::Gif => image::ImageFormat::Gif,
            Self::Png => image::ImageFormat::Png,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CUserpicService<R>
where
    R: TrUserpicRepository,
{
    oRepository: R,
    sUploadRoot: String,
}

impl<R> CUserpicService<R>
where
    R: TrUserpicRepository,
{
    pub fn new(oRepository: R, sUploadRoot: impl Into<String>) -> Self {
        Self {
            oRepository,
            sUploadRoot: sUploadRoot.into(),
        }
    }

    pub async fn bCanUpload(&self, iUserId: i32) -> Result<bool> {
        Ok(self
            .oRepository
            .optUploadPolicy(iUserId)
            .await?
            .ok_or(AppError::NotFound)?
            .bPermitted())
    }

    pub async fn vCheckUpload(&self, iUserId: i32) -> Result<()> {
        if !self.bCanUpload(iUserId).await? {
            return Err(AppError::Forbidden);
        }
        Ok(())
    }

    /// Validates exactly the image family accepted by `ImageUtil.imageCheck`:
    /// JPEG, non-animated GIF, and non-animated PNG/APNG.
    pub fn enValidate(arrData: &[u8]) -> Result<EnUserpicFormat> {
        if arrData.is_empty() {
            return Err(AppError::BadRequest("изображение не задано".to_owned()));
        }
        let enDetected = image::guess_format(arrData)
            .map_err(|_| AppError::BadRequest("Invalid image".to_owned()))?;
        let enFormat = match enDetected {
            image::ImageFormat::Jpeg => EnUserpicFormat::Jpeg,
            image::ImageFormat::Gif => EnUserpicFormat::Gif,
            image::ImageFormat::Png => EnUserpicFormat::Png,
            _ => {
                return Err(AppError::BadRequest(format!(
                    "Does unsupported format {enDetected:?}"
                )));
            }
        };

        if bAnimated(arrData, enFormat)? {
            return Err(AppError::BadRequest(
                "Сбой загрузки изображения: анимация не допустима".to_owned(),
            ));
        }

        // `UserService.checkUserPic` first asks ImageIO for the format and
        // animation metadata and only then checks the byte-size limit.
        if arrData.len() > I_MAX_USERPIC_FILE_SIZE {
            return Err(AppError::BadRequest(
                "Сбой загрузки изображения: слишком большой файл".to_owned(),
            ));
        }

        let (iWidth, iHeight) =
            image::ImageReader::with_format(Cursor::new(arrData), enFormat.enImageFormat())
                .into_dimensions()
                .map_err(|_| AppError::BadRequest("Invalid image".to_owned()))?;
        if !(I_MIN_USERPIC_SIZE..=I_MAX_USERPIC_SIZE).contains(&iWidth)
            || !(I_MIN_USERPIC_SIZE..=I_MAX_USERPIC_SIZE).contains(&iHeight)
        {
            return Err(AppError::BadRequest(
                "Сбой загрузки изображения: недопустимые размеры фотографии".to_owned(),
            ));
        }

        // Force a complete decode after the cheap header/dimension checks so
        // truncated or corrupt payloads never reach the public photos tree.
        let _stImage = crate::image_upload::stDecodeWithLimits(
            arrData,
            enFormat.enImageFormat(),
            I_MAX_USERPIC_SIZE,
            I_MAX_USERPIC_SIZE,
            8 * 1024 * 1024,
        )
        .map_err(|_| AppError::BadRequest("Invalid image".to_owned()))?;

        Ok(enFormat)
    }

    pub async fn sInstall(&self, iUserId: i32, arrData: &[u8]) -> Result<String> {
        self.vCheckUpload(iUserId).await?;
        let enFormat = Self::enValidate(arrData)?;
        let stPhotosDirectory = std::path::Path::new(&self.sUploadRoot).join("photos");
        tokio::fs::create_dir_all(&stPhotosDirectory).await?;

        let (sFilename, stPath) = loop {
            let sFilename = format!(
                "{iUserId}:{}.{}",
                rand::random::<i32>(),
                enFormat.sExtension()
            );
            let stPath = stPhotosDirectory.join(&sFilename);
            if !tokio::fs::try_exists(&stPath).await? {
                break (sFilename, stPath);
            }
        };

        tokio::fs::write(&stPath, arrData).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&stPath, std::fs::Permissions::from_mode(0o644)).await?;
        }

        if let Err(stError) = self.oRepository.vSetUserpic(iUserId, &sFilename).await {
            let _ = tokio::fs::remove_file(&stPath).await;
            return Err(stError);
        }

        Ok(sFilename)
    }
}

fn bAnimated(arrData: &[u8], enFormat: EnUserpicFormat) -> Result<bool> {
    match enFormat {
        EnUserpicFormat::Gif => {
            let stDecoder = image::codecs::gif::GifDecoder::new(Cursor::new(arrData))
                .map_err(|_| AppError::BadRequest("Invalid image".to_owned()))?;
            let vecFrames = stDecoder
                .into_frames()
                .take(2)
                .collect::<image::ImageResult<Vec<_>>>()
                .map_err(|_| AppError::BadRequest("Invalid image".to_owned()))?;
            Ok(vecFrames.len() > 1)
        }
        EnUserpicFormat::Png => {
            let stDecoder = image::codecs::png::PngDecoder::new(Cursor::new(arrData))
                .map_err(|_| AppError::BadRequest("Invalid image".to_owned()))?;
            stDecoder
                .is_apng()
                .map_err(|_| AppError::BadRequest("Invalid image".to_owned()))
        }
        EnUserpicFormat::Jpeg => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use image::ImageEncoder;

    use super::*;
    use crate::domain::user::userpic::StUserpicUploadPolicy;

    #[derive(Clone)]
    struct CTestRepository {
        stPolicy: StUserpicUploadPolicy,
        vecSets: Arc<Mutex<Vec<(i32, String)>>>,
        bFailSet: bool,
    }

    #[async_trait]
    impl TrUserpicRepository for CTestRepository {
        async fn optUploadPolicy(&self, _iUserId: i32) -> Result<Option<StUserpicUploadPolicy>> {
            Ok(Some(self.stPolicy))
        }

        async fn vSetUserpic(&self, iUserId: i32, sFilename: &str) -> Result<()> {
            if self.bFailSet {
                return Err(AppError::Anyhow(anyhow::anyhow!(
                    "simulated repository failure"
                )));
            }
            self.vecSets
                .lock()
                .expect("sets lock")
                .push((iUserId, sFilename.to_owned()));
            Ok(())
        }
    }

    fn stAllowedPolicy() -> StUserpicUploadPolicy {
        StUserpicUploadPolicy {
            iScore: 45,
            bFrozen: false,
            iRecentSetCount: 0,
            bRecentlyResetByModerator: false,
            iRecentScoreLoss: 0,
        }
    }

    fn vecPng(iWidth: u32, iHeight: u32) -> Vec<u8> {
        let vecPixels = vec![0_u8; (iWidth * iHeight * 4) as usize];
        let mut vecData = Vec::new();
        image::codecs::png::PngEncoder::new(&mut vecData)
            .write_image(&vecPixels, iWidth, iHeight, image::ExtendedColorType::Rgba8)
            .expect("encode PNG");
        vecData
    }

    fn vecJpeg(iWidth: u32, iHeight: u32) -> Vec<u8> {
        let vecPixels = vec![0_u8; (iWidth * iHeight * 3) as usize];
        let mut vecData = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut vecData)
            .write_image(&vecPixels, iWidth, iHeight, image::ExtendedColorType::Rgb8)
            .expect("encode JPEG");
        vecData
    }

    fn vecStaticGif() -> Vec<u8> {
        let mut vecData = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut vecData)
            .encode_frame(image::Frame::new(image::RgbaImage::new(50, 50)))
            .expect("encode GIF");
        vecData
    }

    fn vecAnimatedGif() -> Vec<u8> {
        let mut vecData = Vec::new();
        {
            let mut stEncoder = image::codecs::gif::GifEncoder::new(&mut vecData);
            stEncoder
                .encode_frame(image::Frame::new(image::RgbaImage::new(50, 50)))
                .expect("encode first GIF frame");
            stEncoder
                .encode_frame(image::Frame::new(image::RgbaImage::new(50, 50)))
                .expect("encode second GIF frame");
        }
        vecData
    }

    #[test]
    fn accepts_java_formats_and_rejects_webp() {
        assert_eq!(
            CUserpicService::<CTestRepository>::enValidate(&vecPng(50, 50)).expect("PNG accepted"),
            EnUserpicFormat::Png
        );
        assert_eq!(
            CUserpicService::<CTestRepository>::enValidate(&vecStaticGif())
                .expect("static GIF accepted"),
            EnUserpicFormat::Gif
        );
        assert_eq!(
            CUserpicService::<CTestRepository>::enValidate(&vecJpeg(50, 50))
                .expect("JPEG accepted"),
            EnUserpicFormat::Jpeg
        );

        let mut vecWebp = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut vecWebp)
            .write_image(
                &[0_u8; 50 * 50 * 4],
                50,
                50,
                image::ExtendedColorType::Rgba8,
            )
            .expect("encode WebP");
        assert!(matches!(
            CUserpicService::<CTestRepository>::enValidate(&vecWebp),
            Err(AppError::BadRequest(sMessage)) if sMessage.contains("unsupported format")
        ));
    }

    #[test]
    fn rejects_animation_and_java_dimension_boundaries() {
        assert!(matches!(
            CUserpicService::<CTestRepository>::enValidate(&vecAnimatedGif()),
            Err(AppError::BadRequest(sMessage)) if sMessage == "Сбой загрузки изображения: анимация не допустима"
        ));
        assert!(CUserpicService::<CTestRepository>::enValidate(&vecPng(50, 300)).is_ok());
        assert!(matches!(
            CUserpicService::<CTestRepository>::enValidate(&vecPng(49, 50)),
            Err(AppError::BadRequest(sMessage)) if sMessage.contains("недопустимые размеры")
        ));
    }

    #[test]
    fn java_validation_order_distinguishes_invalid_from_oversized_images() {
        let vecInvalid = vec![b'x'; I_MAX_USERPIC_FILE_SIZE + 1];
        assert!(matches!(
            CUserpicService::<CTestRepository>::enValidate(&vecInvalid),
            Err(AppError::BadRequest(sMessage)) if sMessage == "Invalid image"
        ));

        let mut vecOversizedPng = vecPng(50, 50);
        vecOversizedPng.resize(I_MAX_USERPIC_FILE_SIZE + 1, 0);
        assert!(matches!(
            CUserpicService::<CTestRepository>::enValidate(&vecOversizedPng),
            Err(AppError::BadRequest(sMessage)) if sMessage == "Сбой загрузки изображения: слишком большой файл"
        ));
    }

    #[tokio::test]
    async fn installation_writes_java_filename_and_calls_repository() {
        let stRoot = std::env::temp_dir().join(format!(
            "lorsource-userpic-service-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let vecSets = Arc::new(Mutex::new(Vec::new()));
        let cService = CUserpicService::new(
            CTestRepository {
                stPolicy: stAllowedPolicy(),
                vecSets: vecSets.clone(),
                bFailSet: false,
            },
            stRoot.to_string_lossy(),
        );
        let sFilename = cService
            .sInstall(42, &vecPng(50, 50))
            .await
            .expect("install userpic");
        assert!(sFilename.starts_with("42:"));
        assert!(sFilename.ends_with(".png"));
        assert!(stRoot.join("photos").join(&sFilename).is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let iMode = std::fs::metadata(stRoot.join("photos").join(&sFilename))
                .expect("installed userpic metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(iMode, 0o644);
        }
        assert_eq!(*vecSets.lock().expect("sets lock"), vec![(42, sFilename)]);
        std::fs::remove_dir_all(&stRoot).expect("remove userpic test tree");
    }

    #[tokio::test]
    async fn failed_database_update_rolls_back_the_installed_file() {
        let stRoot = std::env::temp_dir().join(format!(
            "lorsource-userpic-rollback-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let cService = CUserpicService::new(
            CTestRepository {
                stPolicy: stAllowedPolicy(),
                vecSets: Arc::new(Mutex::new(Vec::new())),
                bFailSet: true,
            },
            stRoot.to_string_lossy(),
        );

        assert!(cService.sInstall(42, &vecPng(50, 50)).await.is_err());
        let vecEntries = std::fs::read_dir(stRoot.join("photos"))
            .expect("photos directory")
            .collect::<std::io::Result<Vec<_>>>()
            .expect("photos entries");
        assert!(vecEntries.is_empty());
        std::fs::remove_dir_all(&stRoot).expect("remove rollback test tree");
    }
}
