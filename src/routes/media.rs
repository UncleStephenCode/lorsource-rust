use std::path::{Path as FsPath, PathBuf};

use axum::{
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use once_cell::sync::Lazy;
use regex::Regex;

use crate::{
    auth::CurrentUser,
    error::{AppError, Result},
    models::UserSummary,
    state::AppState,
};

const IMAGE_CACHE_SECONDS: u32 = 31_556_926;

static PHOTO_NAME_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(\d+)(?:(?::-?\d+)|(?:-[0-9a-fA-F-]{32,36}))?\.[\w]+$").expect("photo name regex")
});

fn bAllowedUploadFilename(sFilename: &str) -> bool {
    !sFilename.is_empty()
        && !sFilename.contains(['/', '\\', '\0'])
        && matches!(
            FsPath::new(sFilename)
                .extension()
                .and_then(|stExtension| stExtension.to_str()),
            Some("jpg" | "jpeg" | "png" | "gif" | "webp")
        )
}

fn bAllowedTopicImageFilename(sFilename: &str) -> bool {
    bAllowedUploadFilename(sFilename)
        && !matches!(
            FsPath::new(sFilename)
                .extension()
                .and_then(|stExtension| stExtension.to_str()),
            Some("jpeg" | "webp")
        )
}

fn optPhotoOwnerId(sFilename: &str) -> Option<i32> {
    PHOTO_NAME_RE
        .captures(sFilename)
        .and_then(|stCaptures| stCaptures.get(1))
        .and_then(|stId| stId.as_str().parse().ok())
}

async fn stServeUpload(pathFile: PathBuf) -> Result<Response> {
    let Some(sFilename) = pathFile.file_name().and_then(|stName| stName.to_str()) else {
        return Err(AppError::NotFound);
    };
    if !bAllowedUploadFilename(sFilename) {
        return Err(AppError::NotFound);
    }
    let sContentType = match pathFile
        .extension()
        .and_then(|stExtension| stExtension.to_str())
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => return Err(AppError::NotFound),
    };
    let vecBody = match tokio::fs::read(&pathFile).await {
        Ok(vecBody) => vecBody,
        Err(stError) if stError.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::NotFound);
        }
        Err(stError) => return Err(stError.into()),
    };
    let sCacheControl = format!("max-age={IMAGE_CACHE_SECONDS}");
    let sContentLength = vecBody.len().to_string();
    Ok((
        [
            (header::CONTENT_TYPE, sContentType),
            (header::CACHE_CONTROL, sCacheControl.as_str()),
            (header::CONTENT_LENGTH, sContentLength.as_str()),
        ],
        vecBody,
    )
        .into_response())
}

/// `GalleryPermissionInterceptor` protects finalized images with the same
/// topic visibility rules as the page containing them. A plain `ServeDir`
/// here would expose deleted topics and drafts to anyone who knew an image
/// identifier.
pub async fn finalized_image(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Path((iImageId, sFilename)): Path<(i32, String)>,
) -> Result<Response> {
    if iImageId <= 0 || !bAllowedTopicImageFilename(&sFilename) {
        return Err(AppError::NotFound);
    }
    let optImage: Option<(i32, bool, i32, bool)> = sqlx::query_as(
        r#"SELECT i.topic, i.deleted, t.userid,
                  NOT t.sticky AND COALESCE(t.commitdate,t.postdate) < now() - s.expire AS expired
           FROM images i
           JOIN topics t ON t.id=i.topic
           JOIN groups g ON g.id=t.groupid
           JOIN sections s ON s.id=g.section
           WHERE i.id=$1"#,
    )
    .bind(iImageId)
    .fetch_optional(&stState.pool)
    .await?;
    let Some((iTopicId, bImageDeleted, iAuthorId, bTopicExpired)) = optImage else {
        return Err(AppError::NotFound);
    };

    if let Err(stError) =
        crate::routes::topics::check_topic_viewable(&stState, iTopicId, &optUser).await
    {
        return match stError {
            AppError::NotFound | AppError::Forbidden => Err(AppError::Forbidden),
            stOther => Err(stOther),
        };
    }

    // Deleted files are edit-history material. Java exposes them only to a
    // moderator, the topic author, or another authenticated user while the
    // topic is not expired.
    if bImageDeleted
        && !optUser
            .as_ref()
            .is_some_and(|stUser| stUser.canmod || stUser.id == iAuthorId || !bTopicExpired)
    {
        return Err(AppError::Forbidden);
    }

    stServeUpload(
        FsPath::new(&stState.config.upload_dir)
            .join("images")
            .join(iImageId.to_string())
            .join(sFilename),
    )
    .await
}

/// Temporary gallery previews are session-private in the original only in
/// the sense that an authenticated session is required. The filename itself
/// already carries the uploader prefix and is validated on reuse.
pub async fn gallery_preview(
    State(stState): State<AppState>,
    CurrentUser(optUser): CurrentUser,
    Path(sFilename): Path<String>,
) -> Result<Response> {
    if optUser.is_none() || !bAllowedTopicImageFilename(&sFilename) {
        return Err(if optUser.is_none() {
            AppError::Forbidden
        } else {
            AppError::NotFound
        });
    }
    stServeUpload(
        FsPath::new(&stState.config.upload_dir)
            .join("gallery/preview")
            .join(sFilename),
    )
    .await
}

fn stFoundRedirect(sTarget: &str) -> Result<Response> {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, sTarget)
        .body(Body::empty())
        .map_err(|stError| AppError::Anyhow(stError.into()))
}

fn sUserpicFallback(optCurrentPhoto: Option<String>) -> String {
    optCurrentPhoto
        .filter(|sPhoto| !sPhoto.is_empty())
        .map_or_else(
            || "/img/p.gif".to_owned(),
            |sPhoto| format!("/photos/{sPhoto}"),
        )
}

/// `UserpicPermissionInterceptor`: the active photo is public. Historical
/// filenames are available only to their owner and moderators; other viewers
/// receive the same 302 redirect to the current photo or disabled-userpic
/// placeholder as the Java application.
pub async fn userpic(
    State(stState): State<AppState>,
    CurrentUser(optViewer): CurrentUser,
    Path(sFilename): Path<String>,
) -> Result<Response> {
    let Some(iOwnerId) = optPhotoOwnerId(&sFilename) else {
        return Err(AppError::NotFound);
    };
    let optCurrentPhoto: Option<Option<String>> =
        sqlx::query_scalar("SELECT photo FROM users WHERE id=$1")
            .bind(iOwnerId)
            .fetch_optional(&stState.pool)
            .await?;
    let Some(optCurrentPhoto) = optCurrentPhoto else {
        return Err(AppError::NotFound);
    };

    let bCurrent = optCurrentPhoto.as_deref() == Some(sFilename.as_str());
    let bMayViewHistorical = optViewer
        .as_ref()
        .is_some_and(|stViewer: &UserSummary| stViewer.id == iOwnerId || stViewer.canmod);
    if !bCurrent && !bMayViewHistorical {
        let sTarget = sUserpicFallback(optCurrentPhoto);
        return stFoundRedirect(&sTarget);
    }

    stServeUpload(
        FsPath::new(&stState.config.upload_dir)
            .join("photos")
            .join(sFilename),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_filenames_are_single_supported_image_components() {
        for sAllowed in ["original.png", "500px.jpg", "preview-42-abc.gif"] {
            assert!(bAllowedTopicImageFilename(sAllowed), "{sAllowed}");
        }
        for sRejected in [
            "",
            "original.webp",
            "../original.png",
            "nested/original.png",
            "original.PNG",
            "original.png\0ignored",
        ] {
            assert!(!bAllowedTopicImageFilename(sRejected), "{sRejected}");
        }
        assert!(bAllowedUploadFilename("42:123.webp"));
    }

    #[test]
    fn java_userpic_filename_contract_extracts_the_owner() {
        assert_eq!(optPhotoOwnerId("42.png"), Some(42));
        assert_eq!(optPhotoOwnerId("42:123456.jpg"), Some(42));
        assert_eq!(optPhotoOwnerId("42:-123456.gif"), Some(42));
        assert_eq!(
            optPhotoOwnerId("42-123e4567-e89b-12d3-a456-426614174000.webp"),
            Some(42)
        );
        assert_eq!(optPhotoOwnerId("other-42.png"), None);
        assert_eq!(optPhotoOwnerId("42/path.png"), None);
    }

    #[test]
    fn historical_userpic_redirect_preserves_java_filename() {
        assert_eq!(
            sUserpicFallback(Some("42:-123456.png".to_owned())),
            "/photos/42:-123456.png"
        );
        assert_eq!(sUserpicFallback(Some(String::new())), "/img/p.gif");
        assert_eq!(sUserpicFallback(None), "/img/p.gif");
    }
}
