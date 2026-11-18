//! Mothr remote control.
//!
//! Serves an authenticated HTTP + WebSocket API on top of [`mothr_core::HubApi`] together
//! with an embedded single-page web client, so the whole app can be driven from a phone,
//! tablet or another computer. Reachability options: the local network (bind address),
//! Tailscale (`tailscale serve` / `tailscale funnel`) or Microsoft Dev Tunnels.
//!
//! Authentication: every API/WebSocket request must carry the pairing token, either as
//! `Authorization: Bearer <token>`, a `?token=<token>` query parameter or the `mothr_token`
//! cookie. Opening `/?token=<token>` in a browser sets that cookie (HttpOnly, SameSite=Strict)
//! and redirects to `/`, which is what the pairing QR code encodes.

mod auth;
mod server;
mod tunnel;

use mothr_core::{HubApi, RemoteSettings, TunnelKind};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub use tunnel::{TunnelAvailability, detect_tunnels};

/// Observable state of the remote server, for the settings UI.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct RemoteStatus {
    pub running: bool,
    /// URL reachable from this machine or the LAN: `http://<lan-ip>:<port>/` when bound to an
    /// unspecified address (0.0.0.0 / ::), otherwise `http://<bind>:<port>/`.
    pub local_url: Option<String>,
    /// Tunnel URL (Tailscale / Dev Tunnels) once established.
    pub public_url: Option<String>,
    pub tunnel: TunnelKind,
    /// Last error (tunnel failures never stop the server; they land here).
    pub error: Option<String>,
    pub connected_clients: usize,
}

/// A running remote-control server. Dropping it without [`RemoteServer::stop`] aborts the
/// server and lets tunnel teardown finish in the background (if the runtime is still alive);
/// prefer `stop`, which waits for the teardown.
pub struct RemoteServer {
    status: watch::Sender<RemoteStatus>,
    shutdown: watch::Sender<bool>,
    server_task: Option<JoinHandle<()>>,
    tunnel_task: Option<JoinHandle<()>>,
    token: String,
    addr: SocketAddr,
}

impl RemoteServer {
    /// Bind and start serving. Must be called from within a tokio runtime context.
    ///
    /// Binds `settings.bind:settings.port`; when that port is taken the next 10 ports are tried.
    /// `port == 0` binds an ephemeral port. An empty `settings.token` is rejected.
    pub async fn start(
        hub: Arc<dyn HubApi>,
        settings: RemoteSettings,
    ) -> anyhow::Result<RemoteServer> {
        let token = settings.token.trim().to_string();
        anyhow::ensure!(
            token.len() >= 16,
            "remote token is missing or too short; generate one with generate_token()"
        );
        let listener = bind_listener(&settings.bind, settings.port).await?;
        let addr = listener.local_addr()?;

        let initial = RemoteStatus {
            running: true,
            local_url: Some(local_url_for(addr)),
            public_url: None,
            tunnel: settings.tunnel,
            error: None,
            connected_clients: 0,
        };
        let (status, _) = watch::channel(initial);
        let (shutdown, _) = watch::channel(false);

        let state = server::AppState {
            hub,
            token: Arc::from(token.as_str()),
            status: status.clone(),
            shutdown: shutdown.subscribe(),
        };
        let app = server::router(state);
        let mut stop_rx = shutdown.subscribe();
        let status_for_server = status.clone();
        let server_task = tokio::spawn(async move {
            let result = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                let _ = stop_rx.wait_for(|v| *v).await;
            })
            .await;
            if let Err(e) = result {
                tracing::error!("remote server failed: {e}");
                status_for_server.send_modify(|s| s.error = Some(format!("server error: {e}")));
            }
            status_for_server.send_modify(|s| s.running = false);
        });
        tracing::info!("remote control listening on {addr}");

        let tunnel_task = (settings.tunnel != TunnelKind::None).then(|| {
            let cfg = tunnel::TunnelConfig::detect(settings.tunnel, addr);
            tokio::spawn(tunnel::run(cfg, status.clone(), shutdown.subscribe()))
        });

        Ok(RemoteServer {
            status,
            shutdown,
            server_task: Some(server_task),
            tunnel_task,
            token,
            addr,
        })
    }

    pub fn status(&self) -> RemoteStatus {
        self.status.borrow().clone()
    }

    pub fn subscribe_status(&self) -> watch::Receiver<RemoteStatus> {
        self.status.subscribe()
    }

    /// The socket address actually bound.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Best URL for pairing (tunnel URL if up, else the local/LAN URL) including `?token=…`.
    pub fn pairing_url(&self) -> Option<String> {
        let status = self.status.borrow();
        let base = status.public_url.as_ref().or(status.local_url.as_ref())?;
        Some(with_token(base, &self.token))
    }

    /// Stop serving, close every client and tear down any tunnel we set up.
    pub async fn stop(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = self.tunnel_task.take() {
            // Teardown runs `tailscale … off` / kills devtunnel; give it time but don't hang.
            if tokio::time::timeout(Duration::from_secs(20), task)
                .await
                .is_err()
            {
                tracing::warn!("remote tunnel teardown timed out");
            }
        }
        if let Some(mut task) = self.server_task.take()
            && tokio::time::timeout(Duration::from_secs(5), &mut task)
                .await
                .is_err()
        {
            task.abort();
        }
        self.status.send_modify(|s| {
            s.running = false;
            s.public_url = None;
            s.connected_clients = 0;
        });
    }
}

impl Drop for RemoteServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = self.server_task.take() {
            task.abort();
        }
    }
}

/// Resolves once `rx` reads `true` (or its sender is gone).
pub(crate) async fn stopped(rx: &mut watch::Receiver<bool>) {
    let _ = rx.wait_for(|stop| *stop).await;
}

/// 32 random bytes, URL-safe base64 without padding (43 chars).
pub fn generate_token() -> String {
    use base64::Engine as _;
    let bytes: [u8; 32] = rand::random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// QR code modules (row-major, `true` = dark) without the quiet zone; the UI adds the margin.
/// Returns an empty matrix if `data` does not fit in a QR code.
pub fn qr_matrix(data: &str) -> Vec<Vec<bool>> {
    let Ok(code) = qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::M)
    else {
        return Vec::new();
    };
    let width = code.width();
    code.to_colors()
        .chunks(width)
        .map(|row| row.iter().map(|c| *c == qrcode::Color::Dark).collect())
        .collect()
}

fn with_token(base: &str, token: &str) -> String {
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}token={}", percent_encode(token))
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

async fn bind_listener(bind: &str, port: u16) -> anyhow::Result<TcpListener> {
    let host = bind.trim().trim_start_matches('[').trim_end_matches(']');
    let host = if host.is_empty() { "127.0.0.1" } else { host };
    if port == 0 {
        return Ok(TcpListener::bind((host, 0)).await?);
    }
    let mut last_err = None;
    for p in port..=port.saturating_add(10) {
        match TcpListener::bind((host, p)).await {
            Ok(l) => return Ok(l),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                tracing::debug!("remote: port {p} in use, trying next");
                last_err = Some(e);
            }
            Err(e) => return Err(anyhow::anyhow!("cannot bind {host}:{p}: {e}")),
        }
    }
    Err(anyhow::anyhow!(
        "ports {port}-{} on {host} are all in use ({})",
        port.saturating_add(10),
        last_err.map(|e| e.to_string()).unwrap_or_default()
    ))
}

fn local_url_for(addr: SocketAddr) -> String {
    let ip = match addr.ip() {
        ip if ip.is_unspecified() => primary_local_ip().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        ip => ip,
    };
    let host = match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };
    format!("http://{host}:{}/", addr.port())
}

/// The primary LAN address: "connect" a UDP socket (no packets are sent) and read the
/// local address the OS routed it through.
pub fn primary_local_ip() -> Option<IpAddr> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    sock.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_shape() {
        let t = generate_token();
        assert_eq!(t.len(), 43);
        assert!(
            t.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_ne!(t, generate_token());
    }

    #[test]
    fn qr_is_square() {
        let m =
            qr_matrix("http://192.168.1.2:7878/?token=abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG");
        assert!(!m.is_empty());
        assert!(m.iter().all(|r| r.len() == m.len()));
        // Finder pattern: top-left corner module is dark.
        assert!(m[0][0]);
    }

    #[test]
    fn pairing_url_building() {
        assert_eq!(
            with_token("https://h.ts.net/", "a-b_c"),
            "https://h.ts.net/?token=a-b_c"
        );
        assert_eq!(
            with_token("http://x/?a=1", "t+/"),
            "http://x/?a=1&token=t%2B%2F"
        );
    }

    #[test]
    fn local_urls() {
        assert_eq!(
            local_url_for("127.0.0.1:80".parse().unwrap()),
            "http://127.0.0.1:80/"
        );
        assert_eq!(local_url_for("[::1]:9".parse().unwrap()), "http://[::1]:9/");
        let lan = local_url_for("0.0.0.0:7878".parse().unwrap());
        assert!(lan.starts_with("http://") && lan.ends_with(":7878/"));
    }
}
