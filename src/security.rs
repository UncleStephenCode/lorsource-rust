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

pub fn csrf_token(session_cookie: &str, secret: &str) -> String {
    sign_payload(session_cookie, secret)
}

pub fn verify_csrf(session_cookie: &str, token: &str, secret: &str) -> bool {
    csrf_token(session_cookie, secret) == token
}
