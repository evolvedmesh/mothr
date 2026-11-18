//! Parse the pairing link the desktop shows (its QR code): `https://host[:port]/?token=…`.

use std::net::IpAddr;

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

    /// What to tell the user when this address can't be reached, based on what kind it is.
    pub fn unreachable_message(&self) -> &'static str {
        let authority = self.base.split_once("://").map_or(self.base.as_str(), |(_, a)| a);
        let host = match authority.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(v6),
            None => authority.split(':').next().unwrap_or(authority),
        }
        .to_ascii_lowercase();
        let ip = host.parse::<IpAddr>().ok();
        if host == "localhost" || ip.is_some_and(|ip| ip.is_loopback()) {
            "This link points at this phone, not your computer. Unpair from the menu, then scan the code in Mothr → Settings → Remote."
        } else if host.ends_with(".ts.net") || ip.is_some_and(is_tailnet) {
            "Can't reach your computer over Tailscale. Turn Tailscale on on this phone, sign in to the same account, and check that Remote is on in Mothr."
        } else if host.ends_with(".local") || ip.is_some_and(is_private) {
            "Can't reach your computer. Join the same Wi-Fi and check that Remote is on in Mothr."
        } else {
            "Can't reach Mothr. Is the desktop app running with Remote on?"
        }
    }

    /// The pairing link, for saving and re-pairing.
    pub fn link(&self) -> String {
        format!("{}/?token={}", self.base, self.token)
    }
}

fn is_tailnet(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]),
        IpAddr::V6(v6) => v6.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
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
    fn unreachable_message_fits_the_address() {
        let message = |link: &str| Pairing::parse(link).unwrap().unreachable_message();
        for link in ["http://127.0.0.1:7878/?token=t", "http://localhost:1/?token=t", "http://[::1]:7878/?token=t"] {
            assert!(message(link).contains("this phone"), "{link}");
        }
        for link in ["http://100.64.0.10:7878/?token=t", "http://[fd7a:115c:a1e0::1]:7878/?token=t", "https://box.tail1.ts.net/?token=t"] {
            assert!(message(link).contains("Tailscale"), "{link}");
        }
        for link in ["http://192.168.1.4:7878/?token=t", "http://10.0.2.2:7979/?token=t", "http://box.local:7878/?token=t"] {
            assert!(message(link).contains("Wi-Fi"), "{link}");
        }
        assert!(message("https://abc.devtunnels.ms/?token=t").contains("Remote on"));
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
