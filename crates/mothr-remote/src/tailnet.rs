//! Reaching the server over Tailscale: listen directly on this machine's tailnet address.
//!
//! This deliberately avoids `tailscale serve`, which needs tailnet admin approval, operator
//! rights, HTTPS certificates and working MagicDNS on the phone. Plain http is fine here:
//! WireGuard encrypts the traffic, only tailnet devices can reach the address, and every request
//! still needs the pairing token.
//!
//! Problems never stop the server. They land in `RemoteStatus`, phrased as what to do next, and
//! the connection comes up on its own once they are fixed.

use crate::RemoteStatus;
use crate::tunnel::detect_tunnels;
use axum::Router;
use serde_json::Value;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::watch;
use tokio::task::JoinHandle;

const CMD_TIMEOUT: Duration = Duration::from_secs(15);
/// How soon to look again while something is wrong.
const RETRY: Duration = if cfg!(test) {
    Duration::from_millis(50)
} else {
    Duration::from_secs(3)
};
/// How often to re-check a healthy connection.
const RECHECK: Duration = if cfg!(test) {
    Duration::from_millis(50)
} else {
    Duration::from_secs(15)
};
/// Failed checks in a row before an open listener is closed, so a blip doesn't drop anyone.
const MISSES_BEFORE_CLOSING: u32 = 2;

pub(crate) struct TailnetConfig {
    /// The CLI to run; when `None` it is looked for on every check, so installing it just works.
    pub program: Option<PathBuf>,
    pub port: u16,
    pub app: Router,
}

#[derive(Debug, PartialEq, Eq)]
enum Health {
    Ready { ip: IpAddr, warning: Option<String> },
    /// Nothing to do but wait.
    Waiting(String),
    /// The user has to act.
    Problem(String),
}

/// Keep a listener on the tailnet address for as long as Tailscale is usable.
pub(crate) async fn run(
    cfg: TailnetConfig,
    status: watch::Sender<RemoteStatus>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut listener: Option<Listener> = None;
    let mut misses = 0;
    loop {
        let health = tokio::select! {
            _ = crate::stopped(&mut shutdown) => break,
            health = check(cfg.program.as_deref()) => health,
        };
        let mut healthy = false;
        match health {
            Health::Ready { ip, warning } => {
                misses = 0;
                if listener
                    .as_ref()
                    .is_none_or(|l| l.ip != ip || l.task.is_finished())
                {
                    close(&mut listener).await;
                    match Listener::start(ip, cfg.port, cfg.app.clone()).await {
                        Ok(l) => listener = Some(l),
                        Err(e) => publish(&status, None, Some(bind_problem(ip, cfg.port, &e)), None),
                    }
                }
                if listener.is_some() {
                    let url = crate::local_url_for(SocketAddr::new(ip, cfg.port));
                    publish(&status, Some(url), None, warning);
                    healthy = true;
                }
            }
            Health::Waiting(_) | Health::Problem(_)
                if listener.is_some() && misses + 1 < MISSES_BEFORE_CLOSING =>
            {
                misses += 1;
            }
            Health::Waiting(message) => {
                close(&mut listener).await;
                publish(&status, None, None, Some(message));
            }
            Health::Problem(message) => {
                close(&mut listener).await;
                publish(&status, None, Some(message), None);
            }
        }
        tokio::select! {
            _ = crate::stopped(&mut shutdown) => break,
            _ = tokio::time::sleep(if healthy { RECHECK } else { RETRY }) => {}
        }
    }
    close(&mut listener).await;
    publish(&status, None, None, None);
}

/// Update the status, waking the UI only when something changed.
fn publish(
    status: &watch::Sender<RemoteStatus>,
    url: Option<String>,
    error: Option<String>,
    warning: Option<String>,
) {
    status.send_if_modified(|s| {
        let changed = s.public_url != url || s.error != error || s.warning != warning;
        if changed {
            if let Some(e) = &error {
                tracing::warn!("remote tailscale: {e}");
            }
            s.public_url = url.clone();
            s.error = error.clone();
            s.warning = warning.clone();
        }
        changed
    });
}

async fn close(listener: &mut Option<Listener>) {
    if let Some(l) = listener.take() {
        l.stop().await;
    }
}

struct Listener {
    ip: IpAddr,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl Listener {
    async fn start(ip: IpAddr, port: u16, app: Router) -> std::io::Result<Self> {
        let tcp = TcpListener::bind((ip, port)).await?;
        let (stop, mut stop_rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            let serve = axum::serve(
                tcp,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async move { crate::stopped(&mut stop_rx).await });
            if let Err(e) = serve.await {
                tracing::warn!("remote tailnet listener failed: {e}");
            }
        });
        tracing::info!("remote control listening on tailnet address {ip}:{port}");
        Ok(Self { ip, stop, task })
    }

    async fn stop(mut self) {
        let _ = self.stop.send(true);
        if tokio::time::timeout(Duration::from_secs(3), &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
        }
    }
}

fn bind_problem(ip: IpAddr, port: u16, e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::AddrInUse => format!(
            "Another app is using port {port} on your Tailscale address. Close it, then turn Remote off and on."
        ),
        std::io::ErrorKind::AddrNotAvailable => {
            "Tailscale's address isn't ready on this computer yet. Retrying…".to_string()
        }
        _ => format!("Mothr can't listen on {ip}:{port}: {e}"),
    }
}

async fn check(program: Option<&Path>) -> Health {
    let Some(program) = program
        .map(Path::to_path_buf)
        .or_else(|| detect_tunnels().tailscale)
    else {
        return Health::Problem(
            "Tailscale isn't installed. Get it from tailscale.com/download, then sign in.".into(),
        );
    };
    let out = match run_cmd(&program, &["status", "--json"], CMD_TIMEOUT).await {
        Ok(out) => out,
        Err(e) => return Health::Problem(format!("Mothr can't run Tailscale: {e}")),
    };
    if out.timed_out {
        return Health::Problem("Tailscale isn't responding. Try restarting it.".into());
    }
    let Ok(json) = serde_json::from_str::<Value>(&out.stdout) else {
        return Health::Problem(cli_failure(&out));
    };
    match read_status(&json) {
        Health::Ready { ip, warning } => {
            let warning = if shields_up(&program).await {
                Some(SHIELDS_UP.to_string())
            } else {
                warning
            };
            Health::Ready { ip, warning }
        }
        other => other,
    }
}

const SHIELDS_UP: &str = "Tailscale is blocking incoming connections, so your phone can't connect. Run: sudo tailscale set --shields-up=false";

/// Best effort: `debug prefs` isn't a stable interface, so any failure just means "don't know".
async fn shields_up(program: &Path) -> bool {
    let Ok(out) = run_cmd(program, &["debug", "prefs"], CMD_TIMEOUT).await else {
        return false;
    };
    serde_json::from_str::<Value>(&out.stdout)
        .ok()
        .and_then(|prefs| prefs.get("ShieldsUp")?.as_bool())
        .unwrap_or(false)
}

/// What to say when `tailscale status --json` produced no JSON.
fn cli_failure(out: &CmdOutput) -> String {
    let text = out.stderr.to_ascii_lowercase();
    let daemon_down = ["failed to connect", "is tailscale running", "connection refused"]
        .iter()
        .any(|needle| text.contains(needle));
    if daemon_down {
        return if cfg!(target_os = "linux") {
            "Tailscale isn't running on this computer. Run: sudo systemctl enable --now tailscaled"
                .into()
        } else {
            "Tailscale isn't running on this computer. Open the Tailscale app.".into()
        };
    }
    format!("Tailscale returned an error: {}", out.describe())
}

fn turn_on_hint() -> &'static str {
    if cfg!(target_os = "linux") {
        "Run: sudo tailscale up"
    } else {
        "Open the Tailscale app and turn it on."
    }
}

/// Turn `tailscale status --json` into what Remote should do.
fn read_status(json: &Value) -> Health {
    let text = |key: &str| json.get(key).and_then(Value::as_str).unwrap_or("");
    match text("BackendState") {
        "Running" => {}
        "NeedsLogin" => {
            return Health::Problem(match text("AuthURL") {
                "" => format!("Tailscale isn't signed in. {}", turn_on_hint()),
                url => format!("Finish signing in to Tailscale: {url}"),
            });
        }
        "NeedsMachineAuth" => {
            return Health::Problem(
                "This computer is waiting for approval. Approve it at login.tailscale.com/admin/machines"
                    .into(),
            );
        }
        "Stopped" => {
            return Health::Problem(format!("Tailscale is turned off. {}", turn_on_hint()));
        }
        _ => return Health::Waiting("Waiting for Tailscale to connect…".into()),
    }

    let ips: Vec<IpAddr> = json
        .get("TailscaleIPs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|ip| ip.as_str()?.parse().ok())
        .filter(is_tailnet_ip)
        .collect();
    // The IPv4 address is shorter in the QR code and works on every network.
    let Some(ip) = ips.iter().find(|ip| ip.is_ipv4()).or(ips.first()).copied() else {
        return Health::Problem(format!(
            "Tailscale is running, but this computer has no Tailscale address. {}",
            turn_on_hint()
        ));
    };

    let online = |node: &Value| node.get("Online").and_then(Value::as_bool);
    let peers_online = json
        .get("Peer")
        .and_then(Value::as_object)
        .is_some_and(|peers| peers.values().any(|p| online(p) == Some(true)));
    let warning = if json.get("Self").and_then(online) == Some(false) {
        Some("Tailscale can't reach its servers right now, so your phone may not connect.".into())
    } else if !peers_online {
        Some(
            "No other device is online on this tailnet. Open Tailscale on your phone and sign in to the same account."
                .into(),
        )
    } else {
        None
    };
    Health::Ready { ip, warning }
}

/// Only ever listen on a real Tailscale address, never on some other interface.
fn is_tailnet_ip(ip: &IpAddr) -> bool {
    let tailnet = match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            a == 100 && (64..128).contains(&b)
        }
        IpAddr::V6(v6) => v6.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    };
    tailnet || (cfg!(test) && ip.is_loopback())
}

struct CmdOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

impl CmdOutput {
    fn describe(&self) -> String {
        let text = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        let chars: Vec<char> = text.trim().chars().collect();
        let tail: String = chars[chars.len().saturating_sub(300)..].iter().collect();
        let code = self.code.map_or_else(|| "?".into(), |c| c.to_string());
        format!("{} (exit {code})", tail.replace('\n', " "))
    }
}

/// Run a command, capturing output; kill it after `timeout`.
async fn run_cmd(program: &Path, args: &[&str], timeout: Duration) -> std::io::Result<CmdOutput> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let read_all = |r: Option<Box<dyn AsyncRead + Unpin + Send>>| {
        tokio::spawn(async move {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut buf).await;
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out_task = read_all(child.stdout.take().map(|s| Box::new(s) as _));
    let err_task = read_all(child.stderr.take().map(|s| Box::new(s) as _));
    let (exit, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(r) => (Some(r?), false),
        Err(_) => {
            let _ = child.kill().await;
            (None, true)
        }
    };
    // A daemonised grandchild could keep the pipes open; don't wait on it forever.
    let collect = |t: JoinHandle<String>| async move {
        tokio::time::timeout(Duration::from_secs(2), t)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
    };
    let stdout = collect(out_task).await;
    let stderr = collect(err_task).await;
    Ok(CmdOutput {
        code: exit.and_then(|e| e.code()),
        stdout,
        stderr,
        timed_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const IP: &str = "100.64.0.10";

    fn running(peers: Value) -> Value {
        json!({
            "BackendState": "Running",
            "TailscaleIPs": ["fd7a:115c:a1e0::1", IP],
            "Self": { "Online": true },
            "Peer": peers,
        })
    }

    fn online_peer() -> Value {
        json!({ "key": { "Online": true } })
    }

    fn problem(json: Value) -> String {
        match read_status(&json) {
            Health::Problem(message) => message,
            other => panic!("expected a problem, got {other:?}"),
        }
    }

    #[test]
    fn ready_prefers_ipv4() {
        assert_eq!(
            read_status(&running(online_peer())),
            Health::Ready {
                ip: IP.parse().unwrap(),
                warning: None
            }
        );
    }

    #[test]
    fn falls_back_to_ipv6() {
        let mut status = running(online_peer());
        status["TailscaleIPs"] = json!(["fd7a:115c:a1e0::1"]);
        let Health::Ready { ip, .. } = read_status(&status) else {
            panic!("not ready");
        };
        assert!(ip.is_ipv6());
    }

    #[test]
    fn explains_each_state() {
        let state = |s: &str| json!({ "BackendState": s });
        assert!(problem(state("NeedsLogin")).contains("isn't signed in"));
        assert!(problem(state("NeedsMachineAuth")).contains("approval"));
        assert!(problem(state("Stopped")).contains("turned off"));
        let login = json!({ "BackendState": "NeedsLogin", "AuthURL": "https://login.example/a" });
        assert!(problem(login).contains("https://login.example/a"));
        assert!(matches!(
            read_status(&state("Starting")),
            Health::Waiting(_)
        ));
    }

    #[test]
    fn refuses_addresses_outside_tailscale() {
        let mut status = running(online_peer());
        status["TailscaleIPs"] = json!(["192.168.1.4", "8.8.8.8", "not an ip"]);
        assert!(problem(status).contains("no Tailscale address"));
        assert!(is_tailnet_ip(&"100.64.0.1".parse().unwrap()));
        assert!(is_tailnet_ip(&"100.127.255.254".parse().unwrap()));
        assert!(!is_tailnet_ip(&"100.128.0.1".parse().unwrap()));
        assert!(!is_tailnet_ip(&"100.63.0.1".parse().unwrap()));
        assert!(!is_tailnet_ip(&"fd00::1".parse().unwrap()));
    }

    #[test]
    fn warns_when_nothing_could_connect() {
        let warning = |status: Value| match read_status(&status) {
            Health::Ready { warning, .. } => warning.unwrap_or_default(),
            other => panic!("not ready: {other:?}"),
        };
        assert!(warning(running(json!(null))).contains("No other device"));
        assert!(warning(running(json!({ "a": { "Online": false } }))).contains("No other device"));
        let mut offline = running(online_peer());
        offline["Self"]["Online"] = json!(false);
        assert!(warning(offline).contains("can't reach its servers"));
    }

    #[test]
    fn recognizes_a_stopped_daemon() {
        let out = |stderr: &str| CmdOutput {
            code: Some(1),
            stdout: String::new(),
            stderr: stderr.into(),
            timed_out: false,
        };
        let down = cli_failure(&out(
            "failed to connect to local Tailscale service; is Tailscale running?",
        ));
        assert!(down.contains("isn't running"));
        assert!(cli_failure(&out("boom")).contains("boom"));
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn config(program: PathBuf, port: u16) -> TailnetConfig {
        TailnetConfig {
            program: Some(program),
            port,
            app: Router::new(),
        }
    }

    /// Follows Tailscale's state: signed out, then running, then turned off again.
    #[cfg(unix)]
    #[tokio::test]
    async fn listens_only_while_tailscale_is_usable() {
        let _serial = crate::test_util::PROC_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("mothr-tailnet-{}", uuid::Uuid::new_v4()));
        let state = dir.join("status.json");
        let program = crate::test_util::fake_cli(
            &dir,
            &format!(
                r#"case "$*" in
  "status --json") cat '{state}' ;;
  "debug prefs") echo '{{"ShieldsUp":true}}' ;;
esac"#,
                state = state.display()
            ),
        );
        std::fs::write(&state, json!({ "BackendState": "NeedsLogin" }).to_string()).unwrap();
        let port = free_port();
        let (status, mut rx) = watch::channel(RemoteStatus::default());
        let (stop, stop_rx) = watch::channel(false);
        let task = tokio::spawn(run(config(program, port), status, stop_rx));
        let wait = Duration::from_secs(10);

        let s = tokio::time::timeout(wait, rx.wait_for(|s| s.error.is_some()))
            .await
            .unwrap()
            .unwrap()
            .clone();
        assert!(s.error.unwrap().contains("isn't signed in"));
        assert_eq!(s.public_url, None);
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err());

        let mut up = running(online_peer());
        up["TailscaleIPs"] = json!(["127.0.0.1"]);
        std::fs::write(&state, up.to_string()).unwrap();
        let s = tokio::time::timeout(wait, rx.wait_for(|s| s.public_url.is_some()))
            .await
            .unwrap()
            .unwrap()
            .clone();
        assert_eq!(s.public_url, Some(format!("http://127.0.0.1:{port}/")));
        assert_eq!(s.error, None);
        assert_eq!(s.warning.as_deref(), Some(SHIELDS_UP));
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();

        std::fs::write(&state, json!({ "BackendState": "Stopped" }).to_string()).unwrap();
        let s = tokio::time::timeout(wait, rx.wait_for(|s| s.public_url.is_none()))
            .await
            .unwrap()
            .unwrap()
            .clone();
        assert!(s.error.unwrap().contains("turned off"));
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err());

        stop.send(true).unwrap();
        task.await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Stopping doesn't wait for a hung `tailscale` command.
    #[cfg(unix)]
    #[tokio::test]
    async fn stop_interrupts_a_hung_cli() {
        let _serial = crate::test_util::PROC_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("mothr-tailnet-{}", uuid::Uuid::new_v4()));
        let started = dir.join("started");
        let program = crate::test_util::fake_cli(
            &dir,
            &format!("touch '{}'\nexec sleep 30", started.display()),
        );
        let (status, _rx) = watch::channel(RemoteStatus::default());
        let (stop, stop_rx) = watch::channel(false);
        let task = tokio::spawn(run(config(program, free_port()), status, stop_rx));
        tokio::time::timeout(Duration::from_secs(10), async {
            while !started.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
