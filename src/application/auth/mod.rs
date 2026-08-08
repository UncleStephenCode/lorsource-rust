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
    let sNick = optNick.unwrap_or("anonymous").trim();
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
    let Some(sResponse) = optResponse.filter(|sValue| !sValue.trim().is_empty()) else {
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
    use super::{CCommentFloodCache, CLoginAttemptCache};

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
}
