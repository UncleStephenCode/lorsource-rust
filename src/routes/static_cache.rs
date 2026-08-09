//! Cache policy from the original Tuckey `urlrewrite.xml` plus Spring's
//! `/webjars/**` resource handler. The deliberately odd GIF/JPG/PNG matcher
//! preserves the original ungrouped regular expression.

use axum::{
    extract::Request,
    http::{HeaderValue, StatusCode, Uri, header},
    middleware::Next,
    response::Response,
};

const S_ONE_HOUR: &str = "max-age=3600";
const S_TEN_YEARS: &str = "max-age=315360000";

fn bStaticPath(sPath: &str) -> bool {
    sPath == "/favicon.ico"
        || [
            "/static/",
            "/img/",
            "/font/",
            "/js/",
            "/webjars/",
            "/black/",
            "/tango/",
            "/white2/",
            "/waltz/",
            "/zomg_ponies/",
            "/adv/",
            "/qrerror/",
        ]
        .iter()
        .any(|sPrefix| sPath.starts_with(sPrefix))
}

fn bUploadedMedia(sPath: &str) -> bool {
    sPath.starts_with("/images/")
        || sPath.starts_with("/gallery/preview/")
        || sPath.starts_with("/gallery-uploads/preview/")
        || sPath.starts_with("/photos/")
}

fn bExtension(sPath: &str, arrExtensions: &[&str]) -> bool {
    arrExtensions
        .iter()
        .any(|sExtension| sPath.ends_with(sExtension))
}

fn vecCacheControls(stUri: &Uri, stStatus: StatusCode) -> Option<Vec<&'static str>> {
    let sPath = stUri.path();
    if stStatus.as_u16() < 200
        || stStatus.as_u16() >= 400
        || !bStaticPath(sPath)
        || bUploadedMedia(sPath)
    {
        return None;
    }

    if sPath.starts_with("/webjars/") {
        return Some(vec![S_TEN_YEARS]);
    }

    let optQuery = stUri.query().filter(|sQuery| !sQuery.trim().is_empty());
    if sPath.starts_with("/font/opensans/") {
        return Some(vec![S_TEN_YEARS]);
    }

    let bScriptOrStyle = bExtension(sPath, &[".css", ".js", ".woff", ".svg", ".ttf", ".woff2"]);
    if bScriptOrStyle {
        if optQuery.is_some() {
            return Some(vec![S_TEN_YEARS]);
        }
        if sPath.contains("jquery") && sPath.ends_with(".js") {
            // Both Tuckey rules append a value for the old unpackaged jquery
            // files; Java emits two Cache-Control header fields.
            return Some(vec![S_TEN_YEARS, S_ONE_HOUR]);
        }
        return Some(vec![S_ONE_HOUR]);
    }

    // Original regex: `\.(gif)|(jpg)|(png)$`. With use-query-string=true the
    // first two alternatives remain unanchored, while PNG only matches when
    // the complete path+query ends in `png`.
    let sMatch = optQuery.map_or_else(|| sPath.to_owned(), |sQuery| format!("{sPath}?{sQuery}"));
    let bImage = sMatch.contains(".gif") || sMatch.contains("jpg") || sMatch.ends_with("png");
    if bImage {
        return Some(if sPath.contains("/adv/") {
            vec!["no-cache"]
        } else {
            vec![S_TEN_YEARS]
        });
    }

    // Static resources not covered by any original rule (favicon, EOT, or a
    // PNG with a cachebuster) deliberately have no Cache-Control header.
    Some(Vec::new())
}

pub async fn apply(stRequest: Request, oNext: Next) -> Response {
    let stUri = stRequest.uri().clone();
    let mut stResponse = oNext.run(stRequest).await;
    let Some(vecPolicies) = vecCacheControls(&stUri, stResponse.status()) else {
        return stResponse;
    };

    // `/qrerror/**` is not excluded from Spring Security. Its default-servlet
    // response therefore carries both the Tuckey cache value and
    // CommonContextFilter's later `private` value, unlike `/js/**` and themes.
    let bPreservePrivate = stUri.path().starts_with("/qrerror/");
    let stHeaders = stResponse.headers_mut();
    let vecExisting = if bPreservePrivate {
        stHeaders
            .get_all(header::CACHE_CONTROL)
            .iter()
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    stHeaders.remove(header::CACHE_CONTROL);
    for sPolicy in vecPolicies {
        stHeaders.append(header::CACHE_CONTROL, HeaderValue::from_static(sPolicy));
    }
    for stPolicy in vecExisting {
        stHeaders.append(header::CACHE_CONTROL, stPolicy);
    }
    stResponse
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vecPolicy(sUri: &str) -> Option<Vec<&'static str>> {
        vecCacheControls(&sUri.parse().expect("valid test URI"), StatusCode::OK)
    }

    #[test]
    fn matches_observed_java_static_cache_matrix() {
        assert_eq!(vecPolicy("/tango/combined.css"), Some(vec![S_ONE_HOUR]));
        assert_eq!(
            vecPolicy("/tango/combined.css?v=1"),
            Some(vec![S_TEN_YEARS])
        );
        assert_eq!(
            vecPolicy("/font/opensans/open-sans.woff2"),
            Some(vec![S_TEN_YEARS])
        );
        assert_eq!(
            vecPolicy("/webjars/jquery/3.7.1/jquery.min.js"),
            Some(vec![S_TEN_YEARS])
        );
        assert_eq!(
            vecPolicy("/js/jquery.hotkeys.js"),
            Some(vec![S_TEN_YEARS, S_ONE_HOUR])
        );
        assert_eq!(vecPolicy("/img/p.gif?different=1"), Some(vec![S_TEN_YEARS]));
        assert_eq!(vecPolicy("/img/tuxlor.png?x=1"), Some(Vec::new()));
        assert_eq!(vecPolicy("/adv/banner.png"), Some(vec!["no-cache"]));
        assert_eq!(vecPolicy("/adv/banner.png?x=1"), Some(Vec::new()));
        assert_eq!(vecPolicy("/favicon.ico"), Some(Vec::new()));
        assert_eq!(vecPolicy("/qrerror/combined.css"), Some(vec![S_ONE_HOUR]));
    }

    #[test]
    fn leaves_errors_dynamic_pages_and_protected_media_untouched() {
        assert_eq!(
            vecCacheControls(
                &"/tango/missing.css".parse().unwrap(),
                StatusCode::NOT_FOUND
            ),
            None
        );
        assert_eq!(vecPolicy("/about"), None);
        assert_eq!(vecPolicy("/images/42/original.png"), None);
        assert_eq!(vecPolicy("/photos/42:1.png"), None);
        assert_eq!(vecPolicy("/gallery/preview/preview.png"), None);
    }
}
