use crate::{
    config::Config,
    error::{AppError, Result},
    models::UserSummary,
    state::AppState,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::Mutex as AsyncMutex;

const LOGIN_ATTEMPT_TTL: Duration = Duration::from_secs(30 * 60);
const LOGIN_ATTEMPT_MAX_KEYS: usize = 1_000_000;

#[derive(Debug, Clone)]
pub struct StPostingIdentity {
    pub stUser: UserSummary,
    /// This is the posting session's authorization state. It is false only
    /// for the dedicated password-less `anonymous` database user.
    pub bAuthorized: bool,
}

#[derive(Debug, Clone)]
pub struct StPostingResolution {
    pub stIdentity: StPostingIdentity,
    /// Java's `AuthUtil.postingUser` keeps the anonymous session and adds a
    /// binding error when credentials supplied in a public posting form are
    /// invalid. Keeping the error separate lets the route re-render the form
    /// and guarantees that no mutation is attempted.
    pub optError: Option<String>,
}

type TyPostingAuthRow = (bool, bool, Option<String>);

async fn stLoadPostingUser(stState: &AppState, sNick: &str) -> Result<Option<UserSummary>> {
    Ok(sqlx::query_as::<_, UserSummary>(
        r#"SELECT id, nick, name, score, max_score, photo, town, regdate,
                  COALESCE(canmod,false) AS canmod,
                  COALESCE(candel,false) AS candel,
                  (COALESCE(corrector,false)
                   AND NOT COALESCE(frozen_until > CURRENT_TIMESTAMP,false)) AS corrector,
                  blocked, userinfo
           FROM users WHERE nick=$1"#,
    )
    .bind(sNick)
    .fetch_optional(&stState.pool)
    .await?)
}

async fn stAnonymousPostingIdentity(stState: &AppState) -> Result<StPostingIdentity> {
    let stUser = sqlx::query_as::<_, UserSummary>(
        r#"SELECT id, nick, name, score, max_score, photo, town, regdate,
                  COALESCE(canmod,false) AS canmod,
                  COALESCE(candel,false) AS candel,
                  false AS corrector, blocked, userinfo
           FROM users
           WHERE nick='anonymous' AND COALESCE(passwd,'')=''
           ORDER BY id LIMIT 1"#,
    )
    .fetch_optional(&stState.pool)
    .await?
    .ok_or_else(|| {
        AppError::Anyhow(anyhow::anyhow!(
            "canonical password-less anonymous user is missing"
        ))
    })?;
    Ok(StPostingIdentity {
        stUser,
        bAuthorized: false,
    })
}

/// Resolve the effective author exactly as Java's `AuthUtil.postingUser`:
/// an authenticated HTTP session wins over form fields; otherwise the form
/// may either select `anonymous` or authenticate a registered account for this
/// one write operation without creating a remember-me session.
pub async fn stResolvePostingIdentity(
    stState: &AppState,
    optSessionUser: Option<&UserSummary>,
    optNick: Option<&str>,
    optPassword: Option<&str>,
) -> Result<StPostingResolution> {
    if let Some(stUser) = optSessionUser {
        return Ok(StPostingResolution {
            stIdentity: StPostingIdentity {
                stUser: stUser.clone(),
                bAuthorized: true,
            },
            optError: None,
        });
    }

    let stAnonymous = stAnonymousPostingIdentity(stState).await?;
    // UserPropertyEditor passes the form value verbatim to UserDao. In
    // particular whitespace is not trimmed into the anonymous identity: a
    // whitespace-padded nick is an invalid/unknown user and must remain a
    // form error.
    let sNick = optNick.unwrap_or("anonymous");
    if sNick.is_empty() || sNick == stAnonymous.stUser.nick {
        return Ok(StPostingResolution {
            stIdentity: stAnonymous,
            optError: None,
        });
    }

    let Some(stUser) = stLoadPostingUser(stState, sNick).await? else {
        return Ok(StPostingResolution {
            stIdentity: stAnonymous,
            optError: Some(format!("Пользователь \"{sNick}\" не найден")),
        });
    };
    let (bActivated, bBlocked, optHash): TyPostingAuthRow =
        sqlx::query_as("SELECT activated, COALESCE(blocked,false), passwd FROM users WHERE id=$1")
            .bind(stUser.id)
            .fetch_one(&stState.pool)
            .await?;
    if bBlocked || !bActivated {
        return Ok(StPostingResolution {
            stIdentity: stAnonymous,
            optError: Some(format!(
                "Пользователь \"{}\" заблокирован или не активирован",
                stUser.nick
            )),
        });
    }
    let sPassword = optPassword.unwrap_or("");
    if !optHash
        .as_deref()
        .is_some_and(|sHash| crate::security::password::verify(sPassword, sHash))
    {
        return Ok(StPostingResolution {
            stIdentity: stAnonymous,
            optError: Some(format!(
                "Пароль для пользователя \"{}\" задан неверно!",
                stUser.nick
            )),
        });
    }

    Ok(StPostingResolution {
        stIdentity: StPostingIdentity {
            stUser,
            bAuthorized: true,
        },
        optError: None,
    })
}

#[derive(Default)]
struct StLoginAttemptState {
    mapIp: HashMap<String, Instant>,
    mapUser: HashMap<String, Instant>,
}

/// Java LoginAttemptCache semantics: one failed attempt requires CAPTCHA for
/// the same remote IP and lower-cased username for 30 minutes.
pub struct CLoginAttemptCache {
    stState: Mutex<StLoginAttemptState>,
    dtTtl: Duration,
    iMaxKeys: usize,
}

impl Default for CLoginAttemptCache {
    fn default() -> Self {
        Self {
            stState: Mutex::new(StLoginAttemptState::default()),
            dtTtl: LOGIN_ATTEMPT_TTL,
            iMaxKeys: LOGIN_ATTEMPT_MAX_KEYS,
        }
    }
}

impl CLoginAttemptCache {
    fn stLock(&self) -> std::sync::MutexGuard<'_, StLoginAttemptState> {
        self.stState
            .lock()
            .unwrap_or_else(|stPoisoned| stPoisoned.into_inner())
    }

    fn vPrune(mapValues: &mut HashMap<String, Instant>, dtNow: Instant, iMaxKeys: usize) {
        mapValues.retain(|_, dtDeadline| *dtDeadline > dtNow);
        if mapValues.len() >= iMaxKeys
            && let Some(sOldest) = mapValues
                .iter()
                .min_by_key(|(_, dtDeadline)| **dtDeadline)
                .map(|(sKey, _)| sKey.clone())
        {
            mapValues.remove(&sOldest);
        }
    }

    pub fn bRequireForIp(&self, sIp: &str) -> bool {
        let dtNow = Instant::now();
        self.stLock()
            .mapIp
            .get(sIp)
            .is_some_and(|dtDeadline| *dtDeadline > dtNow)
    }

    pub fn bRequireForUser(&self, sUser: &str) -> bool {
        let dtNow = Instant::now();
        self.stLock()
            .mapUser
            .get(&sUser.to_lowercase())
            .is_some_and(|dtDeadline| *dtDeadline > dtNow)
    }

    pub fn vRecordFailedAttempt(&self, sIp: &str, sUser: &str) {
        let dtNow = Instant::now();
        let dtDeadline = dtNow + self.dtTtl;
        let mut stState = self.stLock();
        Self::vPrune(&mut stState.mapIp, dtNow, self.iMaxKeys);
        Self::vPrune(&mut stState.mapUser, dtNow, self.iMaxKeys);
        stState.mapIp.insert(sIp.to_owned(), dtDeadline);
        stState.mapUser.insert(sUser.to_lowercase(), dtDeadline);
    }
}

/// FloodProtector's process-local AddComment cache. Successful checks record
/// the action immediately and rejected checks do not extend its deadline.
pub struct CCommentFloodCache {
    bEnabled: bool,
    mapPerformed: AsyncMutex<HashMap<String, Instant>>,
}

impl CCommentFloodCache {
    pub fn new(sPublicUrl: &str) -> Self {
        let bEnabled = reqwest::Url::parse(sPublicUrl)
            .ok()
            .and_then(|stUrl| stUrl.host_str().map(str::to_owned))
            .is_none_or(|sHost| sHost != "127.0.0.1");
        Self {
            bEnabled,
            mapPerformed: AsyncMutex::new(HashMap::new()),
        }
    }

    pub async fn optCheck(&self, sRemoteIp: &str, iThresholdSeconds: u64) -> Option<String> {
        if !self.bEnabled {
            return None;
        }
        let dtNow = Instant::now();
        let mut mapPerformed = self.mapPerformed.lock().await;
        mapPerformed.retain(|_, dtAction| {
            dtNow
                .checked_duration_since(*dtAction)
                .is_some_and(|stAge| stAge < Duration::from_secs(30 * 60))
        });
        let sKey = format!("AddComment:{sRemoteIp}");
        if mapPerformed.get(&sKey).is_some_and(|dtAction| {
            dtAction
                .checked_add(Duration::from_secs(iThresholdSeconds))
                .is_some_and(|dtDeadline| dtDeadline > dtNow)
        }) {
            return Some(format!(
                "Следующее сообщение может быть записано не менее чем через {iThresholdSeconds} секунд после предыдущего"
            ));
        }
        mapPerformed.insert(sKey, dtNow);
        None
    }
}

#[derive(Deserialize)]
struct StCaptchaResponse {
    success: bool,
    #[serde(rename = "error-codes", default)]
    vecErrorCodes: Vec<String>,
}

pub async fn sValidateCaptcha(
    stConfig: &Config,
    cHttp: &reqwest::Client,
    optResponse: Option<&str>,
    sRemoteIp: &str,
) -> std::result::Result<(), String> {
    // CaptchaService distinguishes a missing parameter (local validation
    // error) from a present-but-empty parameter (sent to hCaptcha, whose
    // error-codes are then shown). Preserve that distinction.
    let Some(sResponse) = optResponse else {
        return Err("Код проверки защиты от роботов не указан".to_owned());
    };
    if stConfig.enable_dev_bypasses && sResponse == "dev-captcha" {
        return Ok(());
    }
    let Some(sPrivateKey) = stConfig.captcha_private_key.as_deref() else {
        return Err("Unable to check captcha: CAPTCHA_PRIVATE_KEY is not configured".to_owned());
    };
    let Some(sPublicKey) = stConfig.captcha_public_key.as_deref() else {
        return Err("Unable to check captcha: CAPTCHA_PUBLIC_KEY is not configured".to_owned());
    };
    let stResponse = cHttp
        .post(&stConfig.captcha_verify_url)
        .form(&[
            ("secret", sPrivateKey),
            ("response", sResponse),
            ("remoteip", sRemoteIp),
            ("sitekey", sPublicKey),
        ])
        .send()
        .await
        .map_err(|stError| format!("Unable to check captcha: {stError}"))?
        .error_for_status()
        .map_err(|stError| format!("Unable to check captcha: {stError}"))?
        .json::<StCaptchaResponse>()
        .await
        .map_err(|stError| format!("Unable to check captcha: {stError}"))?;

    if stResponse.success {
        Ok(())
    } else {
        Err(format!(
            "Код проверки защиты от роботов не совпадает ({})",
            stResponse.vecErrorCodes.join(",")
        ))
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::{CCommentFloodCache, CLoginAttemptCache, sValidateCaptcha};

    fn stCaptchaConfig(sVerifyUrl: String) -> crate::config::Config {
        crate::config::Config {
            host: "127.0.0.1".to_owned(),
            port: 8181,
            database_url: "postgres://unused".to_owned(),
            public_url: "https://example.test".to_owned(),
            ws_url: "wss://example.test/".to_owned(),
            static_dir: "static".to_owned(),
            upload_dir: "uploads".to_owned(),
            site_secret: "unused-site-secret".to_owned(),
            opensearch_url: None,
            captcha_public_key: Some("public-key".to_owned()),
            captcha_private_key: Some("private-key".to_owned()),
            captcha_verify_url: sVerifyUrl,
            admin_email: None,
            smtp_host: "localhost".to_owned(),
            smtp_port: 25,
            smtp_helo_name: "localhost".to_owned(),
            telegram_token: None,
            fallback_proxy_url: None,
            enable_background_jobs: false,
            clean_old_userpics: false,
            trusted_proxy_cidrs: Vec::new(),
            page_size: 30,
            enable_hsts: false,
            enable_dev_bypasses: false,
        }
    }

    async fn stCaptchaEndpoint(
        sResponseBody: &str,
    ) -> (crate::config::Config, tokio::task::JoinHandle<String>) {
        let stListener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("captcha test listener");
        let stAddress = stListener.local_addr().expect("captcha listener address");
        let sResponseBody = sResponseBody.to_owned();
        let hServer = tokio::spawn(async move {
            let (mut stStream, _) = stListener.accept().await.expect("captcha request");
            let mut vecRequest = vec![0_u8; 4096];
            let iRead = stStream.read(&mut vecRequest).await.expect("read request");
            let sRequest = String::from_utf8_lossy(&vecRequest[..iRead]).to_string();
            let sResponse = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sResponseBody}",
                sResponseBody.len()
            );
            stStream
                .write_all(sResponse.as_bytes())
                .await
                .expect("write captcha response");
            sRequest
        });
        (
            stCaptchaConfig(format!("http://{stAddress}/siteverify")),
            hServer,
        )
    }

    #[test]
    fn failed_attempt_is_keyed_by_ip_and_case_insensitive_username() {
        let cCache = CLoginAttemptCache::default();
        assert!(!cCache.bRequireForIp("192.0.2.1"));
        assert!(!cCache.bRequireForUser("Alice"));

        cCache.vRecordFailedAttempt("192.0.2.1", "Alice");

        assert!(cCache.bRequireForIp("192.0.2.1"));
        assert!(cCache.bRequireForUser("alice"));
        assert!(cCache.bRequireForUser("ALICE"));
        assert!(!cCache.bRequireForIp("192.0.2.2"));
    }

    #[tokio::test]
    async fn comment_flood_cache_matches_java_recording_semantics() {
        let cCache = CCommentFloodCache::new("https://linux.org.ru");
        assert!(cCache.optCheck("192.0.2.1", 30).await.is_none());
        assert!(cCache.optCheck("192.0.2.1", 30).await.is_some());
        assert!(cCache.optCheck("192.0.2.2", 30).await.is_none());

        let cDisabled = CCommentFloodCache::new("http://127.0.0.1:8181");
        assert!(cDisabled.optCheck("192.0.2.1", 30).await.is_none());
        assert!(cDisabled.optCheck("192.0.2.1", 30).await.is_none());
    }

    #[tokio::test]
    async fn captcha_posts_the_java_parameter_contract() {
        let (stConfig, hServer) = stCaptchaEndpoint(r#"{"success":true}"#).await;
        sValidateCaptcha(
            &stConfig,
            &reqwest::Client::new(),
            Some("captcha answer"),
            "192.0.2.10",
        )
        .await
        .expect("successful captcha");

        let sRequest = hServer.await.unwrap();
        assert!(sRequest.starts_with("POST /siteverify HTTP/1.1"));
        assert!(sRequest.contains("secret=private-key"));
        assert!(sRequest.contains("response=captcha+answer"));
        assert!(sRequest.contains("remoteip=192.0.2.10"));
        assert!(sRequest.contains("sitekey=public-key"));
    }

    #[tokio::test]
    async fn captcha_distinguishes_missing_from_api_rejection_like_java() {
        let stUnusedConfig = stCaptchaConfig("http://127.0.0.1:1/unused".to_owned());
        assert_eq!(
            sValidateCaptcha(&stUnusedConfig, &reqwest::Client::new(), None, "192.0.2.10").await,
            Err("Код проверки защиты от роботов не указан".to_owned())
        );

        let (stConfig, hServer) = stCaptchaEndpoint(
            r#"{"success":false,"error-codes":["missing-input-response","bad-request"]}"#,
        )
        .await;
        assert_eq!(
            sValidateCaptcha(&stConfig, &reqwest::Client::new(), Some(""), "192.0.2.10").await,
            Err(
                "Код проверки защиты от роботов не совпадает (missing-input-response,bad-request)"
                    .to_owned()
            )
        );
        let sRequest = hServer.await.unwrap();
        assert!(sRequest.contains("response=&"));
    }
}
