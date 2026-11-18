//! Parse the pairing link the desktop shows (its QR code): `https://host[:port]/?token=…`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pairing {
    /// `http(s)://host[:port]` without a trailing slash.
    pub base: String,
    pub token: String,
}

impl Pairing {
    pub fn parse(input: &str) -> Result<Self, String> {
        let s = input.trim();
        if s.is_empty() {
            return Err("Paste the link shown in Mothr → Settings → Remote.".into());
        }
        let s = if s.contains("://") { s.to_string() } else { format!("http://{s}") };
        let (scheme, rest) = s.split_once("://").ok_or("Not a link")?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(format!("Unsupported link type “{scheme}”"));
        }
        let (authority, tail) = match rest.find(['/', '?', '#']) {
            Some(i) => rest.split_at(i),
            None => (rest, ""),
        };
        if authority.is_empty() || authority.contains('@') || authority.contains(char::is_whitespace) {
            return Err("The link has no valid host".into());
        }
        let query = tail.split_once('?').map(|(_, q)| q.split('#').next().unwrap_or("")).unwrap_or("");
        let token = query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| *k == "token")
            .map(|(_, v)| v.to_string())
            .filter(|t| !t.is_empty())
            .ok_or("The link has no pairing token. Copy the full link, including ?token=…")?;
        if !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err("The pairing token looks damaged. Copy the link again.".into());
        }
        Ok(Self { base: format!("{scheme}://{authority}"), token })
    }

    pub fn ws_url(&self) -> String {
        let ws = if let Some(rest) = self.base.strip_prefix("https://") {
            format!("wss://{rest}")
        } else {
            format!("ws://{}", self.base.trim_start_matches("http://"))
        };
        format!("{ws}/api/ws?token={}", self.token)
    }

    /// The pairing link, for saving and re-pairing.
    pub fn link(&self) -> String {
        format!("{}/?token={}", self.base, self.token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_links() {
        let p = Pairing::parse(" https://box.tail1234.ts.net/?token=abc-DEF_123#x ").unwrap();
        assert_eq!(p.base, "https://box.tail1234.ts.net");
        assert_eq!(p.token, "abc-DEF_123");
        assert_eq!(p.ws_url(), "wss://box.tail1234.ts.net/api/ws?token=abc-DEF_123");
        let p = Pairing::parse("192.168.1.4:7878?token=t1").unwrap();
        assert_eq!(p.base, "http://192.168.1.4:7878");
        assert_eq!(p.ws_url(), "ws://192.168.1.4:7878/api/ws?token=t1");
        assert_eq!(p.link(), "http://192.168.1.4:7878/?token=t1");
    }

    #[test]
    fn rejects_bad_links() {
        assert!(Pairing::parse("").is_err());
        assert!(Pairing::parse("https://host/").is_err());
        assert!(Pairing::parse("ftp://host/?token=x").is_err());
        assert!(Pairing::parse("https://user@host/?token=x").is_err());
        assert!(Pairing::parse("https://host/?token=a b").is_err());
    }
}
