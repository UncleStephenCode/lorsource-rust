//! Auth/session/security compatibility layer.
//!
//! Original lorsource used Spring Security with stateless remember-me cookies,
//! roles derived from `users` flags, disabled global CSRF and explicit
//! permission checks in controller/service code. This module is the Rust port
//! target for that logic: password verification, role derivation, signed session
//! cookies and reusable permission predicates.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

pub mod password {
    /// BCrypt uses the first 72 bytes. Match the Scala PasswordEncoderImpl
    /// behaviour: truncate on a UTF-8 boundary before hashing/verifying.
    pub fn truncate_for_bcrypt(raw: &str) -> String {
        let bytes = raw.as_bytes();
        if bytes.len() <= 72 {
            return raw.to_owned();
        }
        let mut end = 72;
        while end > 0 && (bytes[end] & 0b1100_0000) == 0b1000_0000 {
            end -= 1;
        }
        String::from_utf8_lossy(&bytes[..end]).to_string()
    }

    pub fn is_bcrypt(encoded: &str) -> bool {
        encoded.starts_with("$2a$") || encoded.starts_with("$2b$") || encoded.starts_with("$2y$")
    }

    /// Verifies modern bcrypt passwords. Legacy Jasypt BasicPasswordEncryptor
    /// hashes from pre-2026 dumps are intentionally not silently accepted here;
    /// import/migration should rehash them or run a transitional verifier.
    pub fn verify(raw: &str, encoded: &str) -> bool {
        if raw.is_empty() || encoded.is_empty() {
            return false;
        }
        if let Some(noop) = encoded.strip_prefix("{noop}") {
            return raw == noop;
        }
        if is_bcrypt(encoded) {
            bcrypt::verify(truncate_for_bcrypt(raw), encoded).unwrap_or(false)
        } else {
            false
        }
    }

    pub fn hash(raw: &str) -> Result<String, bcrypt::BcryptError> {
        bcrypt::hash(truncate_for_bcrypt(raw), bcrypt::DEFAULT_COST)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Anonymous,
    User,
    Corrector,
    Moderator,
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    ViewDeleted,
    EditOwnPost,
    EditAnyPost,
    DeleteOwnPost,
    DeleteAnyPost,
    ModeratePremoderation,
    ManageUsers,
    ManageGroups,
    ManageTags,
    PostWarning,
}

#[derive(Debug, Clone)]
pub struct Principal {
    pub user_id: Option<i32>,
    pub activated: bool,
    pub blocked: bool,
    pub canmod: bool,
    pub candel: bool,
    pub corrector: bool,
}

impl Principal {
    pub fn anonymous() -> Self {
        Self { user_id: None, activated: false, blocked: false, canmod: false, candel: false, corrector: false }
    }

    pub fn roles(&self) -> Vec<Role> {
        if self.user_id.is_none() || !self.activated || self.blocked {
            return vec![Role::Anonymous];
        }
        let mut roles = vec![Role::User];
        if self.corrector { roles.push(Role::Corrector); }
        if self.canmod { roles.push(Role::Moderator); }
        if self.canmod && self.candel { roles.push(Role::Admin); }
        roles
    }

    pub fn has(&self, permission: Permission) -> bool {
        match permission {
            Permission::ViewDeleted | Permission::DeleteAnyPost | Permission::ModeratePremoderation | Permission::ManageTags | Permission::PostWarning => self.canmod,
            Permission::EditAnyPost | Permission::ManageUsers | Permission::ManageGroups => self.canmod && self.candel,
            Permission::EditOwnPost | Permission::DeleteOwnPost => self.user_id.is_some() && !self.blocked && self.activated,
        }
    }
}


/// Hex encoded HMAC-SHA256 compatible with the original `SecretTokenService`
/// activation/reset code strategy.
pub fn hmac_sha256_hex(secret: &str, payload: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts keys of any size");
    mac.update(payload.as_bytes());
    mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

pub fn verify_hash(expected: &str, supplied: &str) -> bool {
    if expected.len() != supplied.len() {
        return false;
    }
    expected.as_bytes().iter().zip(supplied.as_bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// TLS terminates at a reverse proxy in front of this app, so "is this
/// request actually HTTPS" has to be read off `X-Forwarded-Proto` rather
/// than the connection axum sees directly. Used to decide whether to set
/// the `Secure` cookie flag (see `security_headers::apply` for the
/// equivalent HSTS-header check).
pub fn is_secure_request(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

pub fn sign_payload(payload: &str, secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":");
    hasher.update(payload.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

pub fn make_timed_session(user_id: i32, secret: &str) -> String {
    let payload = format!("v1.{user_id}.{}", Utc::now().timestamp());
    let sig = sign_payload(&payload, secret);
    format!("{}.{}", URL_SAFE_NO_PAD.encode(payload), sig)
}

pub fn verify_timed_session(value: &str, secret: &str, max_age_seconds: i64) -> Option<i32> {
    let (payload64, sig) = value.split_once('.')?;
    let payload = String::from_utf8(URL_SAFE_NO_PAD.decode(payload64).ok()?).ok()?;
    if sign_payload(&payload, secret) != sig {
        return None;
    }
    let mut parts = payload.split('.');
    if parts.next()? != "v1" {
        return None;
    }
    let user_id = parts.next()?.parse().ok()?;
    let issued_at: i64 = parts.next()?.parse().ok()?;
    if Utc::now().timestamp().saturating_sub(issued_at) > max_age_seconds {
        return None;
    }
    Some(user_id)
}

/// Token helpers compatible with the current Java `SecretTokenService`.
///
/// Java uses:
/// - PBKDF2WithHmacSHA256(base secret, random 16-byte salt, 65536 iterations, 256-bit key)
/// - AES/GCM/NoPadding with a random 12-byte IV and 128-bit tag
/// - Base64(salt || iv || ciphertext_with_tag)
pub mod secret_tokens {
    use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
    use base64::{engine::general_purpose::STANDARD, Engine};
    use pbkdf2::pbkdf2_hmac;
    use rand::RngCore;
    use sha2::Sha256;

    const SALT_LEN: usize = 16;
    const IV_LEN: usize = 12;
    const KEY_LEN: usize = 32;
    const PBKDF2_ITERATIONS: u32 = 65_536;

    fn derive_key(secret: &str, salt: &[u8]) -> [u8; KEY_LEN] {
        let mut key = [0u8; KEY_LEN];
        pbkdf2_hmac::<Sha256>(secret.as_bytes(), salt, PBKDF2_ITERATIONS, &mut key);
        key
    }

    pub fn encrypt_java_secret(secret: &str, plaintext: &str) -> Result<String, String> {
        let mut salt = [0u8; SALT_LEN];
        let mut iv = [0u8; IV_LEN];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        rand::rngs::OsRng.fill_bytes(&mut iv);

        let key = derive_key(secret, &salt);
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| "aes-gcm key error".to_string())?;
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&iv), plaintext.as_bytes())
            .map_err(|_| "aes-gcm encrypt error".to_string())?;

        let mut out = Vec::with_capacity(SALT_LEN + IV_LEN + ciphertext.len());
        out.extend_from_slice(&salt);
        out.extend_from_slice(&iv);
        out.extend_from_slice(&ciphertext);
        Ok(STANDARD.encode(out))
    }

    pub fn decrypt_java_secret(secret: &str, encoded: &str) -> Option<String> {
        let data = STANDARD.decode(encoded).ok()?;
        if data.len() < SALT_LEN + IV_LEN + 16 {
            return None;
        }
        let salt = &data[..SALT_LEN];
        let iv = &data[SALT_LEN..SALT_LEN + IV_LEN];
        let ciphertext = &data[SALT_LEN + IV_LEN..];

        let key = derive_key(secret, salt);
        let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
        let plaintext = cipher.decrypt(Nonce::from_slice(iv), ciphertext).ok()?;
        String::from_utf8(plaintext).ok()
    }

    pub fn make_register_permit(secret: &str, now_millis: i64) -> Result<String, String> {
        let expiry = now_millis + 3_600_000;
        encrypt_java_secret(secret, &format!("permit:{expiry}"))
    }

    pub fn check_register_permit(secret: &str, permit: &str, now_millis: i64) -> bool {
        let Some(plaintext) = decrypt_java_secret(secret, permit) else { return false; };
        let Some(expiry) = plaintext.strip_prefix("permit:").and_then(|v| v.parse::<i64>().ok()) else {
            return false;
        };
        expiry > now_millis
    }

    pub fn activation_code(secret: &str, nick: &str, email: &str, regdate_millis: i64) -> String {
        super::hmac_sha256_hex(secret, &format!("{nick}:{email}:{regdate_millis}:activate"))
    }

    pub fn verify_activation_code(secret: &str, nick: &str, email: &str, regdate_millis: i64, code: &str) -> bool {
        let expected = activation_code(secret, nick, email, regdate_millis);
        super::verify_hash(&expected, code)
    }

    pub fn reset_code(secret: &str, nick: &str, email: &str, reset_millis: i64) -> String {
        super::hmac_sha256_hex(secret, &format!("{nick}:{email}:{reset_millis}:reset"))
    }

    pub fn verify_reset_code(secret: &str, nick: &str, email: &str, reset_millis: i64, code: &str) -> bool {
        let expected = reset_code(secret, nick, email, reset_millis);
        super::verify_hash(&expected, code)
    }
}
