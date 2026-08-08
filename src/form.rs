use crate::error::{AppError, Result};

/// `axum::Form<T>` deserializes via `serde_urlencoded`, which cannot turn
/// repeated keys (`vote=1&vote=2`, the standard HTML encoding for a
/// multi-select/checkbox group) into a `Vec<T>` field - it errors with
/// "invalid type: string ..., expected a sequence". Parsing into
/// `Vec<(String, String)>` instead preserves every occurrence in order, so
/// callers can pick out repeated fields by hand.
pub fn parse_pairs(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    serde_urlencoded::from_bytes(bytes)
        .map_err(|_| AppError::BadRequest("некорректные данные формы".into()))
}

pub fn get<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

pub fn get_all<'a>(pairs: &'a [(String, String)], key: &str) -> Vec<&'a str> {
    pairs
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .collect()
}
