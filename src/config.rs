use anyhow::Context;
use std::path::{Component, Path};

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
    pub site_secret: String,
    pub opensearch_url: Option<String>,
    pub captcha_public_key: Option<String>,
    pub captcha_private_key: Option<String>,
    pub captcha_verify_url: String,
    pub admin_email: Option<String>,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_helo_name: String,
    /// Java `telegram.token`; an absent value or the literal `false` disables
    /// the Best of LOR publisher without disabling other background jobs.
    pub telegram_token: Option<String>,
    /// Java `fallback.proxy.host/port`, represented as a full proxy URL for
    /// the Telegram direct-request fallback.
    pub fallback_proxy_url: Option<String>,
    /// Runs the Java-compatible scheduled maintenance jobs. This is normally
    /// enabled; tests and one-off administrative processes may turn it off.
    pub enable_background_jobs: bool,
    /// Java `cleanOldUserpics`: false is a safe dry-run that only logs old
    /// userpic candidates, true also removes them from the upload directory.
    pub clean_old_userpics: bool,
    /// Only these network peers may supply X-Forwarded-For/Proto. An empty
    /// list means all forwarded headers are ignored.
    pub trusted_proxy_cidrs: Vec<ipnetwork::IpNetwork>,
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
    pub fn stFromEnv() -> anyhow::Result<Self> {
        let sPublicUrl =
            std::env::var("PUBLIC_URL").unwrap_or_else(|_| "http://localhost:8181".to_string());
        let sWsUrl = std::env::var("WS_URL").unwrap_or_else(|_| sWsUrlFromPublic(&sPublicUrl));
        let sSiteSecret = optEnvOrFile("SITE_SECRET")?
            .unwrap_or_else(|| "dev-only-change-me-change-me-change-me".to_owned());
        Ok(Self {
            host: std::env::var("LOR_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: std::env::var("LOR_PORT")
                .ok()
                .and_then(|sValue| sValue.parse().ok())
                .unwrap_or(8181),
            database_url: optEnvOrFile("DATABASE_URL")?
                .unwrap_or_else(|| "postgres://linuxweb:linuxweb@localhost:5432/lor".to_string()),
            public_url: sPublicUrl,
            ws_url: sWsUrl,
            static_dir: std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string()),
            upload_dir: std::env::var("UPLOAD_DIR").unwrap_or_else(|_| "uploads".to_string()),
            site_secret: sSiteSecret,
            opensearch_url: std::env::var("OPENSEARCH_URL").ok(),
            captcha_public_key: std::env::var("CAPTCHA_PUBLIC_KEY").ok(),
            captcha_private_key: optEnvOrFile("CAPTCHA_PRIVATE_KEY")?,
            captcha_verify_url: std::env::var("CAPTCHA_VERIFY_URL")
                .unwrap_or_else(|_| "https://hcaptcha.com/siteverify".to_owned()),
            admin_email: std::env::var("ADMIN_EMAIL")
                .ok()
                .filter(|sValue| !sValue.trim().is_empty()),
            smtp_host: std::env::var("SMTP_HOST").unwrap_or_else(|_| "localhost".to_owned()),
            smtp_port: std::env::var("SMTP_PORT").map_or(25, |sValue| sValue.parse().unwrap_or(0)),
            smtp_helo_name: std::env::var("SMTP_HELO_NAME")
                .unwrap_or_else(|_| "localhost".to_owned()),
            telegram_token: optEnvOrFile("TELEGRAM_TOKEN")?
                .filter(|sValue| !sValue.trim().is_empty() && sValue != "false"),
            fallback_proxy_url: optHttpProxyUrl(
                std::env::var("FALLBACK_PROXY_URL")
                    .ok()
                    .filter(|sValue| !sValue.trim().is_empty()),
            )?,
            enable_background_jobs: bStrictEnvBoolean("ENABLE_BACKGROUND_JOBS", false)?,
            clean_old_userpics: std::env::var("CLEAN_OLD_USERPICS")
                .map(|sValue| sValue == "true" || sValue == "1")
                .unwrap_or(false),
            trusted_proxy_cidrs: vecTrustedProxyCidrs(
                &std::env::var("TRUSTED_PROXY_CIDRS").unwrap_or_default(),
            )?,
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
        })
    }

    pub fn from_env() -> anyhow::Result<Self> {
        Self::stFromEnv()
    }

    /// Production must never silently inherit the development fallbacks used
    /// by local Compose. Keep parsing backwards compatible, then fail closed
    /// before opening a database connection or binding the HTTP listener.
    pub fn vValidateForEnvironment(&self, sEnvironment: &str) -> anyhow::Result<()> {
        let sEnvironment = sEnvironment.trim().to_ascii_lowercase();
        if !matches!(
            sEnvironment.as_str(),
            "development" | "dev" | "test" | "production" | "prod"
        ) {
            anyhow::bail!(
                "invalid LOR_ENV {sEnvironment:?}: expected development, test or production"
            );
        }
        if !matches!(sEnvironment.as_str(), "production" | "prod") {
            return Ok(());
        }

        let mut vecProblems = Vec::new();
        let sSiteSecretLower = self.site_secret.to_ascii_lowercase();
        if self.site_secret.len() < 32
            || sSiteSecretLower.contains("change-me")
            || sSiteSecretLower.contains("dev-only")
            || sSiteSecretLower.contains("devcontainer")
            || self.site_secret.chars().any(char::is_control)
        {
            vecProblems.push(
                "SITE_SECRET must be an explicit production secret of at least 32 characters"
                    .to_owned(),
            );
        }
        if self.enable_dev_bypasses {
            vecProblems.push("ENABLE_DEV_BYPASSES must be disabled in production".to_owned());
        }
        let optPublicAuthority = self
            .public_url
            .parse::<http::Uri>()
            .ok()
            .filter(|stUri| stUri.scheme_str() == Some("https"))
            .and_then(|stUri| stUri.authority().map(ToString::to_string));
        if optPublicAuthority.is_none() {
            vecProblems
                .push("PUBLIC_URL must be an absolute https:// URL in production".to_owned());
        } else if !bOriginOnlyUri(&self.public_url) {
            vecProblems.push(
                "PUBLIC_URL must contain only the public HTTPS origin without credentials, query or fragment"
                    .to_owned(),
            );
        }
        let optWebSocketAuthority = self
            .ws_url
            .parse::<http::Uri>()
            .ok()
            .filter(|stUri| stUri.scheme_str() == Some("wss"))
            .and_then(|stUri| stUri.authority().map(ToString::to_string));
        if optWebSocketAuthority.is_none() {
            vecProblems.push("WS_URL must be an absolute wss:// URL in production".to_owned());
        } else if optWebSocketAuthority != optPublicAuthority {
            vecProblems.push("WS_URL must use the same authority as PUBLIC_URL".to_owned());
        } else if !bOriginOnlyUri(&self.ws_url) {
            vecProblems.push(
                "WS_URL must contain only the public WebSocket origin without credentials, query or fragment"
                    .to_owned(),
            );
        }
        if self.trusted_proxy_cidrs.is_empty() {
            vecProblems.push(
                "TRUSTED_PROXY_CIDRS must name the TLS reverse-proxy network in production"
                    .to_owned(),
            );
        }
        match self.opensearch_url.as_deref().and_then(|sValue| {
            reqwest::Url::parse(sValue).ok().filter(|stUrl| {
                matches!(stUrl.scheme(), "http" | "https") && stUrl.host_str().is_some()
            })
        }) {
            Some(_) => {}
            None => vecProblems
                .push("OPENSEARCH_URL must be an absolute HTTP(S) URL in production".to_owned()),
        }
        let bInvalidCaptchaKey = |optValue: Option<&str>| {
            optValue.is_none_or(|sValue| {
                sValue.trim().is_empty() || sValue.chars().any(char::is_control)
            })
        };
        if bInvalidCaptchaKey(self.captcha_public_key.as_deref())
            || bInvalidCaptchaKey(self.captcha_private_key.as_deref())
        {
            vecProblems.push(
                "CAPTCHA_PUBLIC_KEY and CAPTCHA_PRIVATE_KEY are required in production".to_owned(),
            );
        }
        let bValidCaptchaUrl = reqwest::Url::parse(&self.captcha_verify_url).is_ok_and(|stUrl| {
            stUrl.scheme() == "https"
                && stUrl.host_str().is_some()
                && stUrl.username().is_empty()
                && stUrl.password().is_none()
                && stUrl.fragment().is_none()
        });
        if !bValidCaptchaUrl {
            vecProblems.push(
                "CAPTCHA_VERIFY_URL must be an absolute credential-free HTTPS URL".to_owned(),
            );
        }
        if self.admin_email.as_deref().is_none_or(|sValue| {
            !sValue.contains('@') || sValue.contains('\r') || sValue.contains('\n')
        }) {
            vecProblems
                .push("ADMIN_EMAIL is required and must be a mailbox in production".to_owned());
        }
        if self.smtp_host.trim().is_empty()
            || self.smtp_host.chars().any(char::is_control)
            || self.smtp_port == 0
        {
            vecProblems.push("SMTP_HOST and SMTP_PORT must identify a valid MTA".to_owned());
        }
        if !bValidSmtpHelo(&self.smtp_helo_name) {
            vecProblems
                .push("SMTP_HELO_NAME must be a valid SMTP domain/address literal".to_owned());
        }
        if self.telegram_token.is_some() {
            if self
                .telegram_token
                .as_deref()
                .is_some_and(|sValue| sValue.chars().any(char::is_control))
            {
                vecProblems.push("TELEGRAM_TOKEN must not contain control characters".to_owned());
            }
            let bValidProxy = self.fallback_proxy_url.as_deref().is_some_and(|sUrl| {
                sUrl.parse::<http::Uri>().is_ok_and(|stUri| {
                    matches!(stUri.scheme_str(), Some("http" | "https"))
                        && stUri.authority().is_some()
                })
            });
            if !bValidProxy {
                vecProblems.push(
                    "FALLBACK_PROXY_URL is required and must be an absolute HTTP(S) URL in production when TELEGRAM_TOKEN is set"
                        .to_owned(),
                );
            }
        }
        if self.database_url.contains("linuxweb:linuxweb@")
            || self.database_url.contains("postgres:postgres@")
        {
            vecProblems.push("DATABASE_URL contains development credentials".to_owned());
        }

        match reqwest::Url::parse(&self.database_url) {
            Ok(stUrl) => {
                let optDecodedCredentials = stUrl.password().and_then(|sPassword| {
                    Some((
                        urlencoding::decode(stUrl.username()).ok()?.into_owned(),
                        urlencoding::decode(sPassword).ok()?.into_owned(),
                    ))
                });
                if !matches!(stUrl.scheme(), "postgres" | "postgresql")
                    || stUrl.username().is_empty()
                    || stUrl.password().is_none_or(str::is_empty)
                    || stUrl.path().trim_matches('/').is_empty()
                    || optDecodedCredentials.is_none()
                {
                    vecProblems.push(
                        "DATABASE_URL must be a complete PostgreSQL URL with runtime credentials and database name"
                            .to_owned(),
                    );
                }
                if optDecodedCredentials
                    .as_ref()
                    .is_some_and(|(sUser, sPassword)| {
                        matches!(
                            (sUser.as_str(), sPassword.as_str()),
                            ("linuxweb", "linuxweb")
                                | ("postgres", "postgres")
                                | ("maxcom", "maxcom")
                        )
                    })
                {
                    vecProblems.push("DATABASE_URL contains development credentials".to_owned());
                }
            }
            Err(_) => vecProblems.push("DATABASE_URL must be a valid PostgreSQL URL".to_owned()),
        }
        let stUploadDir = Path::new(&self.upload_dir);
        let bUnsafeUploadDir = !stUploadDir.is_absolute()
            || matches!(self.upload_dir.as_str(), "/" | "/app" | "/tmp" | "/var")
            || stUploadDir
                .components()
                .any(|stComponent| matches!(stComponent, Component::ParentDir));
        if bUnsafeUploadDir {
            vecProblems.push(
                "UPLOAD_DIR must be a dedicated absolute path without parent traversal".to_owned(),
            );
        }

        if vecProblems.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(
                "production configuration rejected:\n- {}",
                vecProblems.join("\n- ")
            )
        }
    }
}

fn optEnvOrFile(sName: &str) -> anyhow::Result<Option<String>> {
    let sFileName = format!("{sName}_FILE");
    let optValue = std::env::var_os(sName);
    let optFile = std::env::var_os(&sFileName);
    if optValue.is_some() && optFile.is_some() {
        anyhow::bail!("set only one of {sName} and {sFileName}");
    }
    if let Some(sValue) = optValue {
        return sValue
            .into_string()
            .map(Some)
            .map_err(|_| anyhow::anyhow!("{sName} is not valid UTF-8"));
    }
    let Some(sPath) = optFile else {
        return Ok(None);
    };
    let sPath = sPath
        .into_string()
        .map_err(|_| anyhow::anyhow!("{sFileName} is not valid UTF-8"))?;
    sReadSingleLineValue(sName, &sPath).map(Some)
}

fn bStrictEnvBoolean(sName: &str, bDefault: bool) -> anyhow::Result<bool> {
    match std::env::var(sName) {
        Ok(sValue) => bStrictBooleanValue(sName, &sValue),
        Err(std::env::VarError::NotPresent) => Ok(bDefault),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("{sName} is not valid UTF-8")
        }
    }
}

fn bStrictBooleanValue(sName: &str, sValue: &str) -> anyhow::Result<bool> {
    match sValue {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => anyhow::bail!("{sName} must be the literal true or false; got {sValue:?}"),
    }
}

fn vecTrustedProxyCidrs(sValue: &str) -> anyhow::Result<Vec<ipnetwork::IpNetwork>> {
    sValue
        .split(',')
        .map(str::trim)
        .filter(|sCidr| !sCidr.is_empty())
        .map(|sCidr| {
            sCidr
                .parse::<ipnetwork::IpNetwork>()
                .with_context(|| format!("invalid TRUSTED_PROXY_CIDRS entry {sCidr:?}"))
        })
        .collect()
}

fn optHttpProxyUrl(optValue: Option<String>) -> anyhow::Result<Option<String>> {
    let Some(sValue) = optValue else {
        return Ok(None);
    };
    let stUrl = reqwest::Url::parse(&sValue)
        .with_context(|| "FALLBACK_PROXY_URL must be an absolute HTTP(S) URL")?;
    if !matches!(stUrl.scheme(), "http" | "https") || stUrl.host_str().is_none() {
        anyhow::bail!("FALLBACK_PROXY_URL must be an absolute HTTP(S) URL");
    }
    reqwest::Proxy::all(&sValue).with_context(|| "FALLBACK_PROXY_URL is not a valid proxy URL")?;
    Ok(Some(sValue))
}

fn sReadSingleLineValue(sName: &str, sPath: &str) -> anyhow::Result<String> {
    let sValue = std::fs::read_to_string(sPath)
        .with_context(|| format!("failed to read {sName} from {sPath}"))?;
    let sValue = sValue.trim_end_matches(['\r', '\n']).to_owned();
    if sValue.is_empty()
        || sValue
            .chars()
            .any(|cCharacter| matches!(cCharacter, '\r' | '\n' | '\0'))
    {
        anyhow::bail!("{sName}_FILE must contain one non-empty text line");
    }
    Ok(sValue)
}

fn bOriginOnlyUri(sValue: &str) -> bool {
    sValue.parse::<http::Uri>().is_ok_and(|stUri| {
        stUri
            .authority()
            .is_some_and(|stAuthority| !stAuthority.as_str().contains('@'))
            && stUri
                .path_and_query()
                .is_none_or(|stPath| stPath.as_str() == "/")
    })
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

fn bValidSmtpHelo(sValue: &str) -> bool {
    !sValue.is_empty()
        && sValue.chars().all(|cCharacter| {
            cCharacter.is_ascii_alphanumeric() || matches!(cCharacter, '.' | '-' | ':' | '[' | ']')
        })
}

#[cfg(test)]
mod tests {
    use super::{
        StConfig, bStrictBooleanValue, optHttpProxyUrl, sReadSingleLineValue, sWsUrlFromPublic,
        vecTrustedProxyCidrs,
    };

    fn stConfig() -> StConfig {
        StConfig {
            host: "127.0.0.1".to_owned(),
            port: 8181,
            database_url: "postgres://lor:strong-password@db/lor".to_owned(),
            public_url: "https://www.linux.org.ru".to_owned(),
            ws_url: "wss://www.linux.org.ru/".to_owned(),
            static_dir: "static".to_owned(),
            upload_dir: "/srv/lorsource/uploads".to_owned(),
            site_secret: "site-production-secret-01234567890".to_owned(),
            opensearch_url: Some("https://opensearch:9200".to_owned()),
            captcha_public_key: Some("production-site-key".to_owned()),
            captcha_private_key: Some("production-private-key".to_owned()),
            captcha_verify_url: "https://hcaptcha.com/siteverify".to_owned(),
            admin_email: Some("admin@linux.org.ru".to_owned()),
            smtp_host: "mail.internal".to_owned(),
            smtp_port: 25,
            smtp_helo_name: "www.linux.org.ru".to_owned(),
            telegram_token: None,
            fallback_proxy_url: None,
            enable_background_jobs: true,
            clean_old_userpics: false,
            trusted_proxy_cidrs: vec!["127.0.0.1/32".parse().unwrap()],
            page_size: 30,
            enable_hsts: true,
            enable_dev_bypasses: false,
        }
    }

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

    #[test]
    fn trusted_proxy_cidrs_fail_closed_on_a_typo() {
        let vecCidrs =
            vecTrustedProxyCidrs("127.0.0.1/32, 10.0.0.0/8").expect("valid proxy networks");
        assert_eq!(vecCidrs.len(), 2);

        let sError = vecTrustedProxyCidrs("127.0.0.1/32, 10.0.0.0/99")
            .expect_err("one malformed entry must reject the complete setting")
            .to_string();
        assert!(sError.contains("TRUSTED_PROXY_CIDRS"));
        assert!(sError.contains("10.0.0.0/99"));
    }

    #[test]
    fn background_job_activation_boolean_fails_closed_on_a_typo() {
        assert!(bStrictBooleanValue("ENABLE_BACKGROUND_JOBS", "true").unwrap());
        assert!(!bStrictBooleanValue("ENABLE_BACKGROUND_JOBS", "false").unwrap());
        for sInvalid in ["1", "0", "TRUE", "yes", "treu", " true"] {
            let sError = bStrictBooleanValue("ENABLE_BACKGROUND_JOBS", sInvalid)
                .expect_err("ambiguous scheduler activation must be rejected")
                .to_string();
            assert!(sError.contains("ENABLE_BACKGROUND_JOBS"));
            assert!(sError.contains("literal true or false"));
        }
    }

    #[test]
    fn malformed_fallback_proxy_is_rejected_during_configuration_loading() {
        assert_eq!(
            optHttpProxyUrl(Some("http://proxy.internal:3128".to_owned())).unwrap(),
            Some("http://proxy.internal:3128".to_owned())
        );
        for sValue in ["not a URL", "file:///tmp/socket", "http://"] {
            assert!(
                optHttpProxyUrl(Some(sValue.to_owned())).is_err(),
                "{sValue}"
            );
        }
    }

    #[test]
    fn production_configuration_fails_closed_on_development_values() {
        let mut stConfig = stConfig();
        stConfig.site_secret = "change-me-in-production-change-me-in-production".to_owned();
        stConfig.public_url = "http://localhost:8181".to_owned();
        stConfig.ws_url = "ws://localhost:8181/".to_owned();
        stConfig.opensearch_url = None;
        stConfig.captcha_private_key = None;
        stConfig.enable_dev_bypasses = true;
        stConfig.smtp_port = 0;
        stConfig.smtp_helo_name = "localhost\r\nMAIL FROM:<evil>".to_owned();

        let sError = stConfig
            .vValidateForEnvironment("production")
            .expect_err("insecure production values must fail")
            .to_string();

        assert!(sError.contains("SITE_SECRET"));
        assert!(sError.contains("PUBLIC_URL"));
        assert!(sError.contains("WS_URL"));
        assert!(sError.contains("OPENSEARCH_URL"));
        assert!(sError.contains("CAPTCHA_PRIVATE_KEY"));
        assert!(sError.contains("ENABLE_DEV_BYPASSES"));
        assert!(sError.contains("SMTP_HOST and SMTP_PORT"));
        assert!(sError.contains("SMTP_HELO_NAME"));
    }

    #[test]
    fn secure_production_and_development_defaults_are_accepted() {
        stConfig()
            .vValidateForEnvironment("production")
            .expect("secure production config");

        let mut stDevelopment = stConfig();
        stDevelopment.public_url = "http://localhost:8181".to_owned();
        stDevelopment
            .vValidateForEnvironment("development")
            .expect("development keeps explicit local defaults");
    }

    #[test]
    fn production_rejects_cross_origin_websocket() {
        let mut stConfig = stConfig();
        stConfig.ws_url = "wss://socket.example/".to_owned();

        let sError = stConfig
            .vValidateForEnvironment("production")
            .expect_err("cross-origin websocket must fail")
            .to_string();
        assert!(sError.contains("same authority"));
    }

    #[test]
    fn production_telegram_requires_the_java_fallback_proxy() {
        let mut stConfig = stConfig();
        stConfig.telegram_token = Some("secret-bot-token".to_owned());
        let sError = stConfig
            .vValidateForEnvironment("production")
            .expect_err("Telegram without fallback proxy must fail closed")
            .to_string();
        assert!(sError.contains("FALLBACK_PROXY_URL"));

        stConfig.fallback_proxy_url = Some("http://proxy.internal:3128".to_owned());
        stConfig
            .vValidateForEnvironment("production")
            .expect("Telegram with fallback proxy is production-valid");
    }

    #[test]
    fn production_rejects_unsafe_dependency_and_storage_configuration() {
        let mut stConfig = stConfig();
        stConfig.database_url = "postgres://l%69nuxweb:l%69nuxweb@database/lor".to_owned();
        stConfig.public_url = "https://www.linux.org.ru/unexpected?debug=1".to_owned();
        stConfig.opensearch_url = Some("ftp://search.internal/messages".to_owned());
        stConfig.captcha_verify_url = "http://captcha.internal/siteverify".to_owned();
        stConfig.upload_dir = "uploads/../shared".to_owned();

        let sError = stConfig
            .vValidateForEnvironment("production")
            .expect_err("unsafe production endpoints and paths must fail")
            .to_string();
        assert!(sError.contains("development credentials"));
        assert!(sError.contains("PUBLIC_URL must contain only"));
        assert!(sError.contains("OPENSEARCH_URL"));
        assert!(sError.contains("CAPTCHA_VERIFY_URL"));
        assert!(sError.contains("UPLOAD_DIR"));
    }

    #[test]
    fn secret_file_accepts_one_line_and_rejects_multiline_content() {
        let stPath =
            std::env::temp_dir().join(format!("lorsource-secret-file-{}", uuid::Uuid::new_v4()));
        std::fs::write(&stPath, "production-secret\n").expect("write one-line secret");
        assert_eq!(
            sReadSingleLineValue("SITE_SECRET", stPath.to_str().unwrap())
                .expect("read one-line secret"),
            "production-secret"
        );

        std::fs::write(&stPath, "first\nsecond\n").expect("write multiline secret");
        let sError = sReadSingleLineValue("SITE_SECRET", stPath.to_str().unwrap())
            .expect_err("multiline secret must fail")
            .to_string();
        assert!(sError.contains("one non-empty text line"));
        std::fs::remove_file(stPath).expect("remove secret fixture");
    }
}
