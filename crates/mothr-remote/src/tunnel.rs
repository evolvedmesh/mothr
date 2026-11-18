//! Exposing the server beyond the LAN: Tailscale serve/funnel and Microsoft Dev Tunnels.
//!
//! Tunnel problems never stop the server; they are reported through `RemoteStatus::error`.

use crate::RemoteStatus;
use mothr_core::TunnelKind;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};

const CMD_TIMEOUT: Duration = Duration::from_secs(15);
const DEVTUNNEL_URL_TIMEOUT: Duration = Duration::from_secs(60);

/// Tunnel CLIs found on this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TunnelAvailability {
    pub tailscale: Option<PathBuf>,
    pub devtunnel: Option<PathBuf>,
}

impl TunnelAvailability {
    pub fn supports(&self, kind: TunnelKind) -> bool {
        match kind {
            TunnelKind::None => true,
            TunnelKind::Tailscale | TunnelKind::TailscaleFunnel => self.tailscale.is_some(),
            TunnelKind::DevTunnel => self.devtunnel.is_some(),
        }
    }
}

/// Look for the `tailscale` and `devtunnel` CLIs on PATH and in their usual install locations.
pub fn detect_tunnels() -> TunnelAvailability {
    let home = mothr_core::home_dir();
    let tailscale_extra = [
        PathBuf::from("/usr/bin/tailscale"),
        PathBuf::from("/usr/local/bin/tailscale"),
        PathBuf::from("/opt/homebrew/bin/tailscale"),
        PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale"),
        PathBuf::from(r"C:\Program Files\Tailscale\tailscale.exe"),
    ];
    let devtunnel_extra = [
        home.join(".local/bin/devtunnel"),
        home.join("bin/devtunnel"),
        PathBuf::from("/usr/local/bin/devtunnel"),
        PathBuf::from("/opt/homebrew/bin/devtunnel"),
        home.join(r"AppData\Local\Microsoft\DevTunnels\devtunnel.exe"),
    ];
    TunnelAvailability {
        tailscale: find_program("tailscale", &tailscale_extra),
        devtunnel: find_program("devtunnel", &devtunnel_extra),
    }
}

fn find_program(name: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            name.to_string(),
        ]
    } else {
        vec![name.to_string()]
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .chain(extra.iter().cloned())
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// What to run for a tunnel.
#[derive(Debug, Clone)]
pub struct TunnelConfig {
    pub kind: TunnelKind,
    /// Address the tunnel should forward to.
    pub target: SocketAddr,
    /// The CLI to run; `None` when it is not installed.
    pub program: Option<PathBuf>,
}

impl TunnelConfig {
    pub fn detect(kind: TunnelKind, bound: SocketAddr) -> Self {
        let avail = detect_tunnels();
        let program = match kind {
            TunnelKind::None => None,
            TunnelKind::Tailscale | TunnelKind::TailscaleFunnel => avail.tailscale,
            TunnelKind::DevTunnel => avail.devtunnel,
        };
        Self {
            kind,
            target: bound,
            program,
        }
    }

    /// `http://host:port` the proxy should forward to (loopback when bound to all interfaces).
    fn target_url(&self) -> String {
        let ip = self.target.ip();
        let host = if ip.is_unspecified() || ip.is_loopback() {
            "127.0.0.1".to_string()
        } else if ip.is_ipv6() {
            format!("[{ip}]")
        } else {
            ip.to_string()
        };
        format!("http://{host}:{}", self.target.port())
    }
}

/// Command lines (without the program) that bring a Tailscale tunnel up, in fallback order.
pub(crate) fn tailscale_up_commands(cfg: &TunnelConfig) -> Vec<Vec<String>> {
    let port = cfg.target.port().to_string();
    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match cfg.kind {
        TunnelKind::Tailscale => vec![
            v(&["serve", "--bg", "--https=443", &cfg.target_url()]),
            v(&["serve", "--bg", &port]),
        ],
        TunnelKind::TailscaleFunnel => vec![v(&["funnel", "--bg", &port])],
        _ => vec![],
    }
}

/// Command lines that remove exactly the handler we configured (never `serve reset`).
pub(crate) fn tailscale_down_commands(kind: TunnelKind) -> Vec<Vec<String>> {
    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match kind {
        TunnelKind::Tailscale => vec![v(&["serve", "--https=443", "off"])],
        TunnelKind::TailscaleFunnel => vec![
            v(&["funnel", "--https=443", "off"]),
            v(&["serve", "--https=443", "off"]),
        ],
        _ => vec![],
    }
}

pub(crate) fn devtunnel_args(port: u16) -> Vec<String> {
    vec![
        "host".into(),
        "-p".into(),
        port.to_string(),
        "--allow-anonymous".into(),
    ]
}

/// Bring the tunnel up, publish its URL, wait for shutdown, tear it down.
pub async fn run(
    cfg: TunnelConfig,
    status: watch::Sender<RemoteStatus>,
    shutdown: watch::Receiver<bool>,
) {
    if cfg.kind == TunnelKind::None {
        return;
    }
    let Some(program) = cfg.program.clone() else {
        let name = if cfg.kind == TunnelKind::DevTunnel {
            "devtunnel"
        } else {
            "tailscale"
        };
        set_error(
            &status,
            format!("{} unavailable: `{name}` CLI not found", cfg.kind.label()),
        );
        return;
    };
    match cfg.kind {
        TunnelKind::DevTunnel => run_devtunnel(&program, &cfg, &status, shutdown).await,
        _ => run_tailscale(&program, &cfg, &status, shutdown).await,
    }
}

fn set_error(status: &watch::Sender<RemoteStatus>, error: String) {
    tracing::warn!("remote tunnel: {error}");
    status.send_modify(|s| {
        s.error = Some(error);
        s.public_url = None;
    });
}

async fn run_tailscale(
    program: &Path,
    cfg: &TunnelConfig,
    status: &watch::Sender<RemoteStatus>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut last_err = String::from("no command to run");
    let mut configured = false;
    for args in tailscale_up_commands(cfg) {
        match run_cmd(program, &args, CMD_TIMEOUT).await {
            Ok(out) if out.success => {
                configured = true;
                break;
            }
            Ok(out) => last_err = format!("`tailscale {}` {}", args.join(" "), out.describe()),
            Err(e) => last_err = format!("cannot run tailscale: {e}"),
        }
    }
    if !configured {
        set_error(status, last_err);
        return;
    }
    match tailscale_dns_name(program).await {
        Ok(name) => status.send_modify(|s| {
            s.public_url = Some(format!("https://{name}/"));
            s.error = None;
        }),
        Err(e) => set_error(
            status,
            format!("tailscale is serving but its DNS name is unknown: {e:#}"),
        ),
    }

    let _ = shutdown.wait_for(|stop| *stop).await;

    for args in tailscale_down_commands(cfg.kind) {
        match run_cmd(program, &args, CMD_TIMEOUT).await {
            Ok(out) if out.success => break,
            Ok(out) => tracing::warn!("`tailscale {}` {}", args.join(" "), out.describe()),
            Err(e) => tracing::warn!("cannot run tailscale: {e}"),
        }
    }
    status.send_modify(|s| s.public_url = None);
}

async fn tailscale_dns_name(program: &Path) -> anyhow::Result<String> {
    let out = run_cmd(
        program,
        &["status".to_string(), "--json".to_string()],
        CMD_TIMEOUT,
    )
    .await?;
    anyhow::ensure!(out.success, "`tailscale status --json` {}", out.describe());
    parse_tailscale_dns_name(&out.stdout)
}

/// `Self.DNSName` from `tailscale status --json`, without the trailing dot.
pub(crate) fn parse_tailscale_dns_name(json: &str) -> anyhow::Result<String> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    if let Some(state) = v.get("BackendState").and_then(|s| s.as_str())
        && state != "Running"
    {
        anyhow::bail!("Tailscale is not running (state: {state})");
    }
    let name = v
        .pointer("/Self/DNSName")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .trim_end_matches('.');
    anyhow::ensure!(
        !name.is_empty(),
        "this node has no MagicDNS name; enable MagicDNS and HTTPS certificates"
    );
    Ok(name.to_string())
}

async fn run_devtunnel(
    program: &Path,
    cfg: &TunnelConfig,
    status: &watch::Sender<RemoteStatus>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut child = match Command::new(program)
        .args(devtunnel_args(cfg.target.port()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return set_error(status, format!("cannot start devtunnel: {e}")),
    };
    let (line_tx, mut lines) = mpsc::channel::<String>(64);
    if let Some(out) = child.stdout.take() {
        tokio::spawn(forward_lines(out, line_tx.clone()));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(forward_lines(err, line_tx));
    }

    let mut tail: VecDeque<String> = VecDeque::new();
    let mut found = false;
    let deadline = tokio::time::sleep(DEVTUNNEL_URL_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            Some(line) = lines.recv() => {
                tracing::debug!("devtunnel: {line}");
                if !found && let Some(url) = parse_devtunnel_url(&line) {
                    found = true;
                    status.send_modify(|s| {
                        s.public_url = Some(url);
                        s.error = None;
                    });
                }
                tail.push_back(line);
                if tail.len() > 6 {
                    tail.pop_front();
                }
            }
            exit = child.wait() => {
                let code = exit.ok().and_then(|e| e.code()).map(|c| c.to_string()).unwrap_or_else(|| "?".into());
                let detail = tail.iter().map(String::as_str).collect::<Vec<_>>().join(" | ");
                return set_error(status, format!("devtunnel exited (code {code}): {detail}"));
            }
            _ = &mut deadline, if !found => {
                let detail = tail.iter().map(String::as_str).collect::<Vec<_>>().join(" | ");
                let _ = child.kill().await;
                return set_error(status, format!("devtunnel did not report a URL (are you logged in? `devtunnel user login`): {detail}"));
            }
            _ = crate::stopped(&mut shutdown) => break,
        }
    }
    let _ = child.kill().await;
    status.send_modify(|s| s.public_url = None);
}

async fn forward_lines(reader: impl AsyncRead + Unpin, tx: mpsc::Sender<String>) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if tx.send(line).await.is_err() {
            break;
        }
    }
}

/// The browser URL from a `devtunnel host` output line, e.g.
/// `Connect via browser: https://abc-7878.usw2.devtunnels.ms, https://abc.usw2.devtunnels.ms:7878`.
pub(crate) fn parse_devtunnel_url(line: &str) -> Option<String> {
    let mut candidates: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == ',' || c == '"' || c == '\'')
        .map(|t| t.trim_end_matches(['.', ';', ')', ']']))
        .filter(|t| t.starts_with("https://"))
        .filter(|t| {
            let host = t["https://".len()..].split('/').next().unwrap_or("");
            host.split(':')
                .next()
                .unwrap_or("")
                .ends_with(".devtunnels.ms")
                && !host.contains("-inspect.")
        })
        .collect();
    // Prefer the form with the port in the hostname (no explicit `:port`).
    candidates.sort_by_key(|t| {
        t["https://".len()..]
            .split('/')
            .next()
            .unwrap_or("")
            .contains(':')
    });
    candidates.first().map(|u| {
        if u.ends_with('/') {
            u.to_string()
        } else {
            format!("{u}/")
        }
    })
}

pub(crate) struct CmdOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl CmdOutput {
    pub fn describe(&self) -> String {
        let text = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        let text = text.trim();
        let chars: Vec<char> = text.chars().collect();
        let text: String = if chars.len() > 400 {
            chars[chars.len() - 400..].iter().collect()
        } else {
            text.to_string()
        };
        let text = text.replace('\n', " ");
        if self.timed_out {
            format!("did not finish in {}s: {text}", CMD_TIMEOUT.as_secs())
        } else {
            format!(
                "failed (exit {}): {text}",
                self.code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into())
            )
        }
    }
}

/// Run a command, capturing output; kill it after `timeout` but keep what it printed
/// (e.g. `tailscale serve` printing an "enable Serve at https://…" link and waiting).
pub(crate) async fn run_cmd(
    program: &Path,
    args: &[String],
    timeout: Duration,
) -> std::io::Result<CmdOutput> {
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
    let collect = |t: tokio::task::JoinHandle<String>| async move {
        tokio::time::timeout(Duration::from_secs(2), t)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
    };
    let stdout = collect(out_task).await;
    let stderr = collect(err_task).await;
    Ok(CmdOutput {
        success: exit.is_some_and(|e| e.success()),
        code: exit.and_then(|e| e.code()),
        stdout,
        stderr,
        timed_out,
    })
}

#[cfg(test)]
mod tests {
    /// Tests that write and exec fake CLIs race with concurrent forks (ETXTBSY); run them serially.
    static PROC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    use super::*;

    fn cfg(kind: TunnelKind, target: &str) -> TunnelConfig {
        TunnelConfig {
            kind,
            target: target.parse().unwrap(),
            program: None,
        }
    }

    #[test]
    fn tailscale_args() {
        let up = tailscale_up_commands(&cfg(TunnelKind::Tailscale, "0.0.0.0:7878"));
        assert_eq!(
            up[0],
            ["serve", "--bg", "--https=443", "http://127.0.0.1:7878"]
        );
        assert_eq!(up[1], ["serve", "--bg", "7878"]);
        let up = tailscale_up_commands(&cfg(TunnelKind::Tailscale, "192.168.1.4:9000"));
        assert_eq!(up[0][3], "http://192.168.1.4:9000");
        let up = tailscale_up_commands(&cfg(TunnelKind::TailscaleFunnel, "127.0.0.1:7878"));
        assert_eq!(up, vec![vec!["funnel", "--bg", "7878"]]);
        assert_eq!(
            tailscale_down_commands(TunnelKind::Tailscale),
            vec![vec!["serve", "--https=443", "off"]]
        );
        assert_eq!(
            tailscale_down_commands(TunnelKind::TailscaleFunnel)[0],
            ["funnel", "--https=443", "off"]
        );
        assert_eq!(
            devtunnel_args(7878),
            ["host", "-p", "7878", "--allow-anonymous"]
        );
    }

    #[test]
    fn dns_name_parsing() {
        let json = r#"{"BackendState":"Running","Self":{"HostName":"box","DNSName":"box.tail1234.ts.net."}}"#;
        assert_eq!(
            parse_tailscale_dns_name(json).unwrap(),
            "box.tail1234.ts.net"
        );
        assert!(
            parse_tailscale_dns_name(r#"{"BackendState":"Stopped","Self":{"DNSName":"a.b."}}"#)
                .is_err()
        );
        assert!(
            parse_tailscale_dns_name(r#"{"BackendState":"Running","Self":{"DNSName":""}}"#)
                .is_err()
        );
    }

    /// Exercises the real CLI read-only (`tailscale status --json`) when it is installed.
    #[tokio::test]
    async fn real_tailscale_status_if_installed() {
        let _serial = PROC_LOCK.lock().await;
        let Some(ts) = detect_tunnels().tailscale else {
            return;
        };
        if let Ok(name) = tailscale_dns_name(&ts).await {
            assert!(!name.ends_with('.') && name.contains('.'), "{name}");
        }
    }

    #[test]
    fn devtunnel_url_parsing() {
        let line = "Connect via browser: https://abc12x-7878.usw2.devtunnels.ms, https://abc12x.usw2.devtunnels.ms:7878";
        assert_eq!(
            parse_devtunnel_url(line).unwrap(),
            "https://abc12x-7878.usw2.devtunnels.ms/"
        );
        let line = "Inspect network activity: https://abc12x-7878-inspect.usw2.devtunnels.ms";
        assert_eq!(parse_devtunnel_url(line), None);
        assert_eq!(parse_devtunnel_url("Hosting port: 7878"), None);
        assert_eq!(parse_devtunnel_url("see https://example.com/"), None);
    }

    #[cfg(unix)]
    fn fake_cli(dir: &Path, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("fake-cli");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Full Tailscale lifecycle against a fake CLI: first syntax fails, fallback succeeds,
    /// DNS name is published, and the targeted `off` runs on shutdown.
    #[cfg(unix)]
    #[tokio::test]
    async fn tailscale_lifecycle_with_fake_cli() {
        let _serial = PROC_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("mothr-remote-ts-{}", uuid::Uuid::new_v4()));
        let log = dir.join("calls.log");
        let program = fake_cli(
            &dir,
            &format!(
                r#"echo "$*" >> '{log}'
case "$*" in
  "status --json") echo '{{"BackendState":"Running","Self":{{"DNSName":"fake.tailnet.ts.net."}}}}' ;;
  "serve --bg --https=443 "*) echo "flag provided but not defined" >&2; exit 1 ;;
esac
exit 0"#,
                log = log.display()
            ),
        );
        let (status, mut rx) = watch::channel(RemoteStatus::default());
        let (stop, stop_rx) = watch::channel(false);
        let cfg = TunnelConfig {
            kind: TunnelKind::Tailscale,
            target: "127.0.0.1:4321".parse().unwrap(),
            program: Some(program),
        };
        let task = tokio::spawn(run(cfg, status, stop_rx));
        let s = tokio::time::timeout(
            Duration::from_secs(10),
            rx.wait_for(|s| s.public_url.is_some()),
        )
        .await
        .unwrap()
        .unwrap()
        .clone();
        assert_eq!(
            s.public_url.as_deref(),
            Some("https://fake.tailnet.ts.net/")
        );
        assert_eq!(s.error, None);
        stop.send(true).unwrap();
        task.await.unwrap();
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.lines().collect::<Vec<_>>(),
            [
                "serve --bg --https=443 http://127.0.0.1:4321",
                "serve --bg 4321",
                "status --json",
                "serve --https=443 off"
            ]
        );
        assert_eq!(rx.borrow().public_url, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn devtunnel_with_fake_cli() {
        let _serial = PROC_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("mothr-remote-dt-{}", uuid::Uuid::new_v4()));
        let program = fake_cli(
            &dir,
            r#"echo "Hosting port: $3"
echo "Connect via browser: https://zz9-$3.euw.devtunnels.ms, https://zz9.euw.devtunnels.ms:$3"
exec sleep 30"#,
        );
        let (status, mut rx) = watch::channel(RemoteStatus::default());
        let (stop, stop_rx) = watch::channel(false);
        let cfg = TunnelConfig {
            kind: TunnelKind::DevTunnel,
            target: "0.0.0.0:5555".parse().unwrap(),
            program: Some(program),
        };
        let task = tokio::spawn(run(cfg, status, stop_rx));
        let url = tokio::time::timeout(
            Duration::from_secs(10),
            rx.wait_for(|s| s.public_url.is_some()),
        )
        .await
        .unwrap()
        .unwrap()
        .public_url
        .clone();
        assert_eq!(url.as_deref(), Some("https://zz9-5555.euw.devtunnels.ms/"));
        stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rx.borrow().public_url, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn missing_cli_sets_error() {
        let _serial = PROC_LOCK.lock().await;
        let (status, rx) = watch::channel(RemoteStatus::default());
        let (_stop, stop_rx) = watch::channel(false);
        run(cfg(TunnelKind::DevTunnel, "127.0.0.1:1"), status, stop_rx).await;
        assert!(rx.borrow().error.as_deref().unwrap().contains("devtunnel"));
    }
}
