//! Auth/session/security compatibility layer.
//!
//! Original lorsource used Spring Security with stateless remember-me cookies,
//! roles derived from `users` flags, disabled global CSRF and explicit
//! permission checks in controller/service code. This module is the Rust port
//! target for that logic: password verification, role derivation, signed session
//! cookies and reusable permission predicates.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

/// Paths excluded from the main Spring Security filter chain by the current
/// `springapp-security.xml`. MVC response interceptors still run for these
/// resources, but CommonContextFilter (session hydration, CSRF-cookie creation
/// and the default `Cache-Control: private`) does not.
pub fn bSpringSecurityIgnoredPath(sPath: &str) -> bool {
    let bIgnoredPrefix = [
        "/css/",
        "/img/",
        "/js/",
        "/tango/",
        "/tango-auto/",
        "/tango-light/",
        "/waltz/",
        "/white/",
        "/zomg_ponies/",
        "/white2/",
        "/black/",
        "/adv/",
        "/fontello/",
        "/font/",
    ]
    .iter()
    .any(|sPrefix| sPath.starts_with(sPrefix));
    let bTopLevel = sPath
        .strip_prefix('/')
        .is_some_and(|sName| !sName.contains('/'));
    bIgnoredPrefix || (bTopLevel && (sPath.ends_with(".css") || sPath.ends_with(".ico")))
}

pub mod password {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use md5::{Digest, Md5};
    use unicode_normalization::UnicodeNormalization;

    /// Java registration/profile validators use `String.length()`, whose unit
    /// is a UTF-16 code unit. Keep password boundaries migration-compatible.
    pub fn bHasJavaMinimumLength(sRaw: &str, iMinimum: usize) -> bool {
        sRaw.encode_utf16().count() >= iMinimum
    }

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

    /// Verifies the 32-character Jasypt `BasicPasswordEncryptor` values used by
    /// Java before the bcrypt migration. Jasypt stores `salt || digest` as
    /// unpadded Base64, where the digest is MD5(salt || NFC(password)) followed
    /// by 999 further MD5 rounds.
    fn verify_legacy_jasypt(raw: &str, encoded: &str) -> bool {
        let Ok(stored) = STANDARD.decode(encoded) else {
            return false;
        };
        if stored.len() != 24 {
            return false;
        }

        let normalized: String = raw.nfc().collect();
        let (salt, expected) = stored.split_at(8);
        let mut digest = Md5::new();
        digest.update(salt);
        digest.update(normalized.as_bytes());
        let mut actual = digest.finalize().to_vec();
        for _ in 1..1_000 {
            actual = Md5::digest(&actual).to_vec();
        }

        super::constant_time_eq(expected, &actual)
    }

    /// Matches the current Java `PasswordEncoderImpl`: bcrypt is preferred,
    /// while old Jasypt hashes remain valid and are upgraded after login.
    pub fn verify(raw: &str, encoded: &str) -> bool {
        if raw.is_empty() || encoded.is_empty() {
            return false;
        }
        if is_bcrypt(encoded) {
            bcrypt::verify(truncate_for_bcrypt(raw), encoded).unwrap_or(false)
        } else {
            verify_legacy_jasypt(raw, encoded)
        }
    }

    pub fn hash(raw: &str) -> Result<String, bcrypt::BcryptError> {
        bcrypt::hash(truncate_for_bcrypt(raw), bcrypt::DEFAULT_COST)
    }
}

fn constant_time_eq(expected: &[u8], supplied: &[u8]) -> bool {
    if expected.len() != supplied.len() {
        return false;
    }
    expected
        .iter()
        .zip(supplied)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// Binary-compatible implementation of Spring Security 6's
/// `TokenBasedRememberMeServices`, including LOR's `token_generation`
/// extension. This lets Java and Rust accept cookies issued by each other
/// during a rolling migration.
pub mod remember_me {
    use base64::{
        Engine,
        engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
    };
    use md5::Md5;
    use sha2::{Digest, Sha256};

    pub const COOKIE_NAME: &str = "remember_me";
    pub const VALIDITY_SECONDS: i64 = 365 * 24 * 60 * 60;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum EnAlgorithm {
        Md5,
        Sha256,
    }

    impl EnAlgorithm {
        fn sJavaName(self) -> &'static str {
            match self {
                Self::Md5 => "MD5",
                Self::Sha256 => "SHA256",
            }
        }

        fn optFromJavaName(sName: &str) -> Option<Self> {
            match sName {
                "MD5" => Some(Self::Md5),
                "SHA256" => Some(Self::Sha256),
                _ => None,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct StToken {
        pub sUsername: String,
        pub iExpiryMillis: i64,
        pub enAlgorithm: EnAlgorithm,
        pub sSignature: String,
    }

    fn sJavaUrlEncode(sValue: &str) -> String {
        // Usernames cannot contain spaces, but using '+' here keeps this helper
        // byte-for-byte compatible with java.net.URLEncoder for every token.
        urlencoding::encode(sValue).replace("%20", "+")
    }

    fn optJavaUrlDecode(sValue: &str) -> Option<String> {
        urlencoding::decode(&sValue.replace('+', " "))
            .ok()
            .map(std::borrow::Cow::into_owned)
    }

    pub fn sMakeSignature(
        iExpiryMillis: i64,
        sUsername: &str,
        sPasswordHash: &str,
        sSecret: &str,
        iTokenGeneration: i32,
        enAlgorithm: EnAlgorithm,
    ) -> String {
        let mut sData = format!("{sUsername}:{iExpiryMillis}:{sPasswordHash}:{sSecret}");
        if iTokenGeneration > 0 {
            sData.push(':');
            sData.push_str(&iTokenGeneration.to_string());
        }

        let vecDigest = match enAlgorithm {
            EnAlgorithm::Md5 => Md5::digest(sData.as_bytes()).to_vec(),
            EnAlgorithm::Sha256 => Sha256::digest(sData.as_bytes()).to_vec(),
        };
        vecDigest.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn sEncode(
        sUsername: &str,
        iExpiryMillis: i64,
        sPasswordHash: &str,
        sSecret: &str,
        iTokenGeneration: i32,
    ) -> String {
        let enAlgorithm = EnAlgorithm::Sha256;
        let sSignature = sMakeSignature(
            iExpiryMillis,
            sUsername,
            sPasswordHash,
            sSecret,
            iTokenGeneration,
            enAlgorithm,
        );
        let sPlain = [
            sUsername.to_owned(),
            iExpiryMillis.to_string(),
            enAlgorithm.sJavaName().to_owned(),
            sSignature,
        ]
        .iter()
        .map(|sValue| sJavaUrlEncode(sValue))
        .collect::<Vec<_>>()
        .join(":");
        STANDARD_NO_PAD.encode(sPlain.as_bytes())
    }

    pub fn optDecode(sCookie: &str) -> Option<StToken> {
        let vecRaw = STANDARD_NO_PAD
            .decode(sCookie)
            .or_else(|_| STANDARD.decode(sCookie))
            .ok()?;
        let sPlain = String::from_utf8(vecRaw).ok()?;
        let vecParts = sPlain
            .split(':')
            .map(optJavaUrlDecode)
            .collect::<Option<Vec<_>>>()?;

        let (sUsername, sExpiry, enAlgorithm, sSignature) = match vecParts.as_slice() {
            // Spring 5 cookies did not carry the algorithm. Spring 6 keeps
            // accepting them with its legacy matching algorithm (MD5).
            [sUsername, sExpiry, sSignature] => (
                sUsername.clone(),
                sExpiry,
                EnAlgorithm::Md5,
                sSignature.clone(),
            ),
            [sUsername, sExpiry, sAlgorithm, sSignature] => (
                sUsername.clone(),
                sExpiry,
                EnAlgorithm::optFromJavaName(sAlgorithm)?,
                sSignature.clone(),
            ),
            _ => return None,
        };

        Some(StToken {
            sUsername,
            iExpiryMillis: sExpiry.parse().ok()?,
            enAlgorithm,
            sSignature,
        })
    }

    pub fn bVerify(
        stToken: &StToken,
        sPasswordHash: &str,
        sSecret: &str,
        iTokenGeneration: i32,
        iNowMillis: i64,
    ) -> bool {
        if stToken.iExpiryMillis < iNowMillis {
            return false;
        }
        let sExpected = sMakeSignature(
            stToken.iExpiryMillis,
            &stToken.sUsername,
            sPasswordHash,
            sSecret,
            iTokenGeneration,
            stToken.enAlgorithm,
        );
        super::constant_time_eq(sExpected.as_bytes(), stToken.sSignature.as_bytes())
    }
}

/// Hex encoded HMAC-SHA256 compatible with the original `SecretTokenService`
/// activation/reset code strategy.
pub fn hmac_sha256_hex(secret: &str, payload: &str) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts keys of any size");
    mac.update(payload.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn verify_hash(expected: &str, supplied: &str) -> bool {
    if expected.len() != supplied.len() {
        return false;
    }
    expected
        .as_bytes()
        .iter()
        .zip(supplied.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// TLS terminates at a reverse proxy in front of this app, so "is this
/// request actually HTTPS" has to be read off `X-Forwarded-Proto` rather
/// than the connection axum sees directly. Used to decide whether to set
/// the `Secure` cookie flag (see `security_headers::apply` for the
/// equivalent HSTS-header check).
fn bTrustedProxy(stIp: std::net::IpAddr, vecTrusted: &[ipnetwork::IpNetwork]) -> bool {
    vecTrusted.iter().any(|stNetwork| stNetwork.contains(stIp))
}

/// Resolve the original client by walking the proxy chain from the trusted
/// TCP peer toward the left. Values supplied beyond the first untrusted hop
/// are ignored, preventing a client from spoofing the moderation/flood key.
pub fn stClientIp(
    stPeerIp: std::net::IpAddr,
    headers: &axum::http::HeaderMap,
    vecTrusted: &[ipnetwork::IpNetwork],
) -> std::net::IpAddr {
    if !bTrustedProxy(stPeerIp, vecTrusted) {
        return stPeerIp;
    }
    let Some(sForwarded) = headers
        .get("x-forwarded-for")
        .and_then(|stValue| stValue.to_str().ok())
    else {
        return stPeerIp;
    };
    let mut stCurrent = stPeerIp;
    for sCandidate in sForwarded.split(',').rev().map(str::trim) {
        if !bTrustedProxy(stCurrent, vecTrusted) {
            break;
        }
        let Ok(stCandidate) = sCandidate.parse::<std::net::IpAddr>() else {
            break;
        };
        stCurrent = stCandidate;
    }
    stCurrent
}

pub fn is_secure_request(
    headers: &axum::http::HeaderMap,
    optPeerIp: Option<std::net::IpAddr>,
    vecTrusted: &[ipnetwork::IpNetwork],
) -> bool {
    optPeerIp.is_some_and(|stPeerIp| bTrustedProxy(stPeerIp, vecTrusted))
        && headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("https"))
}

/// Token helpers compatible with the current Java `SecretTokenService`.
///
/// Java uses:
/// - PBKDF2WithHmacSHA256(base secret, random 16-byte salt, 65536 iterations, 256-bit key)
/// - AES/GCM/NoPadding with a random 12-byte IV and 128-bit tag
/// - Base64(salt || iv || ciphertext_with_tag)
pub mod secret_tokens {
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use pbkdf2::pbkdf2_hmac;
    use rand::{TryRng, rngs::SysRng};
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
        SysRng
            .try_fill_bytes(&mut salt)
            .map_err(|_| "system random source error".to_string())?;
        SysRng
            .try_fill_bytes(&mut iv)
            .map_err(|_| "system random source error".to_string())?;

        let key = derive_key(secret, &salt);
        let cipher =
            Aes256Gcm::new_from_slice(&key).map_err(|_| "aes-gcm key error".to_string())?;
        let nonce = Nonce::try_from(&iv[..]).map_err(|_| "aes-gcm nonce error".to_string())?;
        let ciphertext = cipher
            .encrypt(&nonce, plaintext.as_bytes())
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
        let nonce = Nonce::try_from(iv).ok()?;
        let plaintext = cipher.decrypt(&nonce, ciphertext).ok()?;
        String::from_utf8(plaintext).ok()
    }

    pub fn make_register_permit(secret: &str, now_millis: i64) -> Result<String, String> {
        let expiry = now_millis + 3_600_000;
        encrypt_java_secret(secret, &format!("permit:{expiry}"))
    }

    pub fn check_register_permit(secret: &str, permit: &str, now_millis: i64) -> bool {
        let Some(plaintext) = decrypt_java_secret(secret, permit) else {
            return false;
        };
        let Some(expiry) = plaintext
            .strip_prefix("permit:")
            .and_then(|v| v.parse::<i64>().ok())
        else {
            return false;
        };
        expiry > now_millis
    }

    pub fn activation_code(secret: &str, nick: &str, email: &str, regdate_millis: i64) -> String {
        super::hmac_sha256_hex(secret, &format!("{nick}:{email}:{regdate_millis}:activate"))
    }

    pub fn verify_activation_code(
        secret: &str,
        nick: &str,
        email: &str,
        regdate_millis: i64,
        code: &str,
    ) -> bool {
        let expected = activation_code(secret, nick, email, regdate_millis);
        super::verify_hash(&expected, code)
    }

    pub fn reset_code(secret: &str, nick: &str, email: &str, reset_millis: i64) -> String {
        super::hmac_sha256_hex(secret, &format!("{nick}:{email}:{reset_millis}:reset"))
    }

    pub fn verify_reset_code(
        secret: &str,
        nick: &str,
        email: &str,
        reset_millis: i64,
        code: &str,
    ) -> bool {
        let expected = reset_code(secret, nick, email, reset_millis);
        super::verify_hash(&expected, code)
    }
}

#[cfg(test)]
mod tests {
    use super::{bSpringSecurityIgnoredPath, is_secure_request, password, remember_me, stClientIp};

    #[test]
    fn spring_security_static_exclusions_match_current_xml_patterns() {
        for sPath in [
            "/js/lor.js",
            "/tango/combined.css",
            "/font/open.woff2",
            "/adv/banner.png",
            "/favicon.ico",
            "/site.css",
        ] {
            assert!(bSpringSecurityIgnoredPath(sPath), "{sPath}");
        }
        for sPath in [
            "/webjars/jquery/jquery.js",
            "/manifest.json",
            "/qrerror/combined.css",
            "/nested/site.css",
            "/favicon.png",
        ] {
            assert!(!bSpringSecurityIgnoredPath(sPath), "{sPath}");
        }
    }

    #[test]
    fn verifies_java_jasypt_basic_password_encryptor_fixture() {
        // Generated by org.jasypt.util.password.BasicPasswordEncryptor 1.9.3.
        assert!(password::verify(
            "password",
            "VEc1e68qZA1zkq6VSi+SYkFe08leeNrk"
        ));
        assert!(!password::verify(
            "wrong",
            "VEc1e68qZA1zkq6VSi+SYkFe08leeNrk"
        ));
        assert!(!password::verify("password", "not-a-valid-hash"));
        assert!(!password::verify("admin", "{noop}admin"));
    }

    #[test]
    fn forwarded_headers_are_used_only_from_trusted_proxy_chain() {
        let vecTrusted = vec!["10.0.0.0/8".parse().unwrap()];
        let mut stHeaders = axum::http::HeaderMap::new();
        stHeaders.insert("x-forwarded-for", "198.51.100.9, 10.1.2.3".parse().unwrap());
        stHeaders.insert("x-forwarded-proto", "https".parse().unwrap());
        assert_eq!(
            stClientIp("10.2.3.4".parse().unwrap(), &stHeaders, &vecTrusted),
            "198.51.100.9".parse::<std::net::IpAddr>().unwrap()
        );
        assert_eq!(
            stClientIp("203.0.113.4".parse().unwrap(), &stHeaders, &vecTrusted),
            "203.0.113.4".parse::<std::net::IpAddr>().unwrap()
        );
        assert!(is_secure_request(
            &stHeaders,
            Some("10.2.3.4".parse().unwrap()),
            &vecTrusted
        ));
        assert!(!is_secure_request(
            &stHeaders,
            Some("203.0.113.4".parse().unwrap()),
            &vecTrusted
        ));
    }

    #[test]
    fn emits_spring_6_cookie_with_lor_generation_signature() {
        // Fixture produced with Spring Security 6.5.11's encodeCookie and
        // GenerationBasedTokenRememberMeServices' SHA-256 payload.
        let sPassword = "$2a$10$abcdefghijklmnopqrstuvABCDEFGHIJKLMNOPQRSTUVWX";
        let sEncoded =
            remember_me::sEncode("Тест.User", 1_786_200_000_123, sPassword, "test-secret", 7);
        assert_eq!(
            sEncoded,
            "JUQwJUEyJUQwJUI1JUQxJTgxJUQxJTgyLlVzZXI6MTc4NjIwMDAwMDEyMzpTSEEyNTY6OGJiZTczN2QwZGUxYTI3ZGQyMjIyZjU3ZTFmNGU3NTA5YmFhMTljODU0ODlkOTY1MGM2Nzc4ZjYzZTMxMTIwNA"
        );

        let stToken = remember_me::optDecode(&sEncoded).expect("valid Spring cookie");
        assert_eq!(stToken.sUsername, "Тест.User");
        assert!(remember_me::bVerify(
            &stToken,
            sPassword,
            "test-secret",
            7,
            1_786_200_000_122,
        ));
        assert!(!remember_me::bVerify(
            &stToken,
            sPassword,
            "test-secret",
            8,
            1_786_200_000_122,
        ));
        assert!(!remember_me::bVerify(
            &stToken,
            sPassword,
            "test-secret",
            7,
            1_786_200_000_124,
        ));
    }

    #[test]
    fn accepts_legacy_three_part_spring_cookie_with_md5() {
        let sSignature = remember_me::sMakeSignature(
            2_000_000_000_000,
            "legacy",
            "password-hash",
            "secret",
            0,
            remember_me::EnAlgorithm::Md5,
        );
        let sPlain = format!("legacy:2000000000000:{sSignature}");
        let sCookie =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD_NO_PAD, sPlain);
        let stToken = remember_me::optDecode(&sCookie).expect("valid old cookie");
        assert!(remember_me::bVerify(
            &stToken,
            "password-hash",
            "secret",
            0,
            1_999_999_999_999,
        ));
    }
}
