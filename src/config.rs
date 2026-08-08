#[derive(Debug, Clone)]
pub struct StConfig {
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub public_url: String,
    /// Java SiteConfig.WSUrl. It includes the trailing slash because the
    /// original browser client appends the literal `ws` endpoint name.
    pub ws_url: String,
    pub static_dir: String,
    pub upload_dir: String,
    pub cookie_secret: String,
    pub site_secret: String,
    pub opensearch_url: Option<String>,
    pub page_size: i64,
    /// SiteConfig.enableHsts - off unless explicitly set, matching Java's
    /// property-absent-means-false default (HSTS is dangerous to enable
    /// prematurely: it makes HTTPS mandatory in the browser for the
    /// configured max-age, so it must be an explicit opt-in).
    pub enable_hsts: bool,
    /// Enables the `dev-activate`/`dev-permit` literal bypasses for account
    /// activation and the registration anti-bot permit token, used by
    /// local/CI test fixtures that can't run a real mail server or captcha.
    /// Off unless explicitly set - these bypasses must never be reachable
    /// in a real deployment.
    pub enable_dev_bypasses: bool,
}

pub type Config = StConfig;

impl StConfig {
    pub fn stFromEnv() -> Self {
        let sPublicUrl =
            std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://localhost:8181".to_string());
        let sWsUrl = std::env::var("WS_URL").unwrap_or_else(|_| sWsUrlFromPublic(&sPublicUrl));
        Self {
            host: std::env::var("LOR_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: std::env::var("LOR_PORT")
                .ok()
                .and_then(|sValue| sValue.parse().ok())
                .unwrap_or(8181),
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://linuxweb:linuxweb@localhost:5432/lor".to_string()),
            public_url: sPublicUrl,
            ws_url: sWsUrl,
            static_dir: std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string()),
            upload_dir: std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "uploads".to_string()),
            cookie_secret: std::env::var("COOKIE_SECRET")
                .unwrap_or_else(|_| "dev-only-change-me-change-me-change-me".to_string()),
            site_secret: std::env::var("SITE_SECRET")
                .ok()
                .or_else(|| std::env::var("COOKIE_SECRET").ok())
                .unwrap_or_else(|| "dev-only-change-me-change-me-change-me".to_string()),
            opensearch_url: std::env::var("OPENSEARCH_URL").ok(),
            page_size: std::env::var("PAGE_SIZE")
                .ok()
                .and_then(|sValue| sValue.parse().ok())
                .unwrap_or(30),
            enable_hsts: std::env::var("ENABLE_HSTS")
                .map(|sValue| sValue == "true" || sValue == "1")
                .unwrap_or(false),
            enable_dev_bypasses: std::env::var("ENABLE_DEV_BYPASSES")
                .map(|sValue| sValue == "true" || sValue == "1")
                .unwrap_or(false),
        }
    }

    pub fn from_env() -> Self {
        Self::stFromEnv()
    }
}

fn sWsUrlFromPublic(sPublicUrl: &str) -> String {
    let sBase = sPublicUrl.trim_end_matches('/');
    if let Some(sRest) = sBase.strip_prefix("https://") {
        format!("wss://{sRest}/")
    } else if let Some(sRest) = sBase.strip_prefix("http://") {
        format!("ws://{sRest}/")
    } else {
        format!("{sBase}/")
    }
}

#[cfg(test)]
mod tests {
    use super::sWsUrlFromPublic;

    #[test]
    fn websocket_url_fallback_matches_java_client_contract() {
        assert_eq!(
            sWsUrlFromPublic("https://www.linux.org.ru/"),
            "wss://www.linux.org.ru/"
        );
        assert_eq!(
            sWsUrlFromPublic("http://localhost:8181"),
            "ws://localhost:8181/"
        );
    }
}
