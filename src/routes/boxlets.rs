use askama::Template;
use axum::{
    extract::State,
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};

use crate::{
    application::boxlet::CBoxletService,
    domain::boxlet::model::{StGalleryBoxletItem, StTagCloudItem},
    error::Result,
    infra::postgres::boxlet_repository::CBoxletPgRepository,
    state::AppState,
};

#[derive(Template)]
#[template(path = "gallery_boxlet.html")]
struct StGalleryBoxletTemplate {
    items: Vec<StGalleryBoxletItem>,
}

#[derive(Template)]
#[template(path = "tagcloud_boxlet.html")]
struct StTagCloudBoxletTemplate {
    tags: Vec<StTagCloudItem>,
}

/// `GalleryBoxlet.getData`: this intentionally has no auth or query
/// extractor. `AbstractBoxlet` accepts every HTTP method and its optional
/// `edit` request parameter only populates an unused model value for this JSP.
pub async fn gallery(State(stState): State<AppState>) -> Result<Response> {
    let cService = CBoxletService::new(
        CBoxletPgRepository::new(stState.pool.clone()),
        &stState.config.upload_dir,
    );
    let sBody = StGalleryBoxletTemplate {
        items: cService.vecGallery().await?,
    }
    .render()?;
    Ok(stHtmlFragment(sBody))
}

/// `TagCloudBoxlet.getData`: public, session-independent and method-agnostic,
/// like the original `AbstractBoxlet` controller.
pub async fn tagCloud(State(stState): State<AppState>) -> Result<Response> {
    let cService = CBoxletService::new(
        CBoxletPgRepository::new(stState.pool.clone()),
        &stState.config.upload_dir,
    );
    let sBody = StTagCloudBoxletTemplate {
        tags: cService.vecTagCloud().await?,
    }
    .render()?;
    Ok(stHtmlFragment(sBody))
}

fn stHtmlFragment(sBody: String) -> Response {
    let mut stResponse = sBody.into_response();
    stResponse.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html;charset=UTF-8"),
    );
    stResponse
}

#[cfg(test)]
mod tests {
    use askama::Template;
    use axum::http::header;

    use super::{StGalleryBoxletTemplate, StTagCloudBoxletTemplate, stHtmlFragment};
    use crate::domain::boxlet::model::{StGalleryBoxletItem, StTagCloudItem};

    #[test]
    fn gallery_fragment_keeps_original_dom_contract() {
        let sHtml = StGalleryBoxletTemplate {
            items: vec![StGalleryBoxletItem {
                sTitle: "Скриншот".to_string(),
                sAltTitle: "Скриншот".to_string(),
                iStat: 3,
                sUserNick: "tester".to_string(),
                sLink: "/gallery/screenshots/42".to_string(),
                sImageMedium: "images/7/1000px.jpg".to_string(),
                sImageSrcset: "images/7/500px.jpg 500w".to_string(),
                iImageWidth: 640,
                iImageHeight: 480,
                sImagePaddingPercent: "75.0".to_string(),
            }],
        }
        .render()
        .expect("gallery template");

        assert!(sHtml.contains("<h2><a href=\"/gallery/\">Галерея</a></h2>"));
        assert!(sHtml.contains("class=\"boxlet_content boxlet-gallery\""));
        assert!(sHtml.contains("padding-bottom: 75.0%"));
        assert!(sHtml.contains("src=\"images/7/1000px.jpg\""));
        assert!(sHtml.contains("width=640 height=480"));
        assert!(sHtml.contains("Скриншот</a> от tester (3)"));
        assert!(sHtml.contains("другие скриншоты..."));
    }

    #[test]
    fn tag_cloud_fragment_uses_calculated_weight_and_encoded_url() {
        let sHtml = StTagCloudBoxletTemplate {
            tags: vec![StTagCloudItem {
                sValue: "c++".to_string(),
                iWeight: 7,
                sUrl: "/tag/c%2B%2B".to_string(),
            }],
        }
        .render()
        .expect("tag cloud template");

        assert!(sHtml.contains("<h2>Облако Меток</h2>"));
        assert!(sHtml.contains("<p align=\"center\">"));
        assert!(sHtml.contains("class=\"cloud7\" href=\"/tag/c%2B%2B\">c++</a>"));
        assert!(sHtml.contains("href=\"/tags/\">все метки...</a>"));
    }

    #[test]
    fn boxlet_database_values_are_html_escaped() {
        let sGallery = StGalleryBoxletTemplate {
            items: vec![StGalleryBoxletItem {
                sTitle: "<script>alert(1)</script>".to_owned(),
                sAltTitle: "\"><script>alert(2)</script>".to_owned(),
                iStat: 1,
                sUserNick: "<img src=x onerror=alert(3)>".to_owned(),
                sLink: "/gallery/screenshots/1".to_owned(),
                sImageMedium: "images/1/1000px.jpg".to_owned(),
                sImageSrcset: "images/1/500px.jpg 500w".to_owned(),
                iImageWidth: 640,
                iImageHeight: 480,
                sImagePaddingPercent: "75.0".to_owned(),
            }],
        }
        .render()
        .expect("gallery template");
        assert!(!sGallery.contains("<script>"));
        assert!(!sGallery.contains("<img src=x"));
        assert!(sGallery.contains("alert(1)"));
        assert!(sGallery.contains('&'));

        let sTags = StTagCloudBoxletTemplate {
            tags: vec![StTagCloudItem {
                sValue: "<svg onload=alert(4)>".to_owned(),
                iWeight: 1,
                sUrl: "/tag/test".to_owned(),
            }],
        }
        .render()
        .expect("tag template");
        assert!(!sTags.contains("<svg"));
        assert!(sTags.contains("alert(4)"));
        assert!(sTags.contains('&'));
    }

    #[test]
    fn fragment_content_type_matches_jsp_declaration() {
        let stResponse = stHtmlFragment(String::new());
        assert_eq!(
            stResponse.headers().get(header::CONTENT_TYPE),
            Some(&"text/html;charset=UTF-8".parse().expect("header value"))
        );
    }
}
