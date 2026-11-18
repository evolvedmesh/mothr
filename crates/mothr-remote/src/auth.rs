//! Token extraction and constant-time verification.

use axum::http::{HeaderMap, Uri, header};

pub const COOKIE_NAME: &str = "mothr_token";
/// Crude brute-force damper applied to every failed authentication.
pub const FAILURE_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

/// Where a presented token came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    Bearer,
    Query,
    Cookie,
}

/// Compare two byte strings in time independent of where they differ.
/// (The length is not secret: tokens have a fixed length.)
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(diff) == 0
}

pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, rest) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| rest.trim().to_string())
}

pub fn query_token(uri: &Uri) -> Option<String> {
    query_param(uri.query()?, "token")
}

pub fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE_NAME)
        .map(|(_, v)| v.trim_matches('"').to_string())
}

/// Every token the request presents, in priority order.
pub fn presented_tokens(headers: &HeaderMap, uri: &Uri) -> Vec<(TokenSource, String)> {
    let mut out = Vec::with_capacity(3);
    if let Some(t) = bearer_token(headers) {
        out.push((TokenSource::Bearer, t));
    }
    if let Some(t) = query_token(uri) {
        out.push((TokenSource::Query, t));
    }
    if let Some(t) = cookie_token(headers) {
        out.push((TokenSource::Cookie, t));
    }
    out
}

/// The source of the first presented token that matches `expected`.
pub fn authorize(headers: &HeaderMap, uri: &Uri, expected: &str) -> Option<TokenSource> {
    presented_tokens(headers, uri)
        .into_iter()
        .find(|(_, t)| constant_time_eq(t.as_bytes(), expected.as_bytes()))
        .map(|(src, _)| src)
}

/// `Set-Cookie` value for a successful pairing.
pub fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=31536000{}",
        if secure { "; Secure" } else { "" }
    )
}

pub fn query_param(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (percent_decode(k) == key).then(|| percent_decode(v))
        })
        .next()
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn ct_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }

    #[test]
    fn extraction() {
        let mut h = HeaderMap::new();
        h.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer tok1"),
        );
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("a=b; mothr_token=tok3"),
        );
        let uri: Uri = "/api/x?q=1&token=tok%2D2".parse().unwrap();
        let toks = presented_tokens(&h, &uri);
        assert_eq!(
            toks,
            vec![
                (TokenSource::Bearer, "tok1".into()),
                (TokenSource::Query, "tok-2".into()),
                (TokenSource::Cookie, "tok3".into())
            ]
        );
        assert_eq!(authorize(&h, &uri, "tok3"), Some(TokenSource::Cookie));
        assert_eq!(authorize(&h, &uri, "nope"), None);
    }

    #[test]
    fn decoding() {
        assert_eq!(percent_decode("a%20b+c%"), "a b c%");
        assert_eq!(percent_decode("%4"), "%4");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(
            query_param("q=hello%20w&limit=5", "limit").as_deref(),
            Some("5")
        );
    }
}
