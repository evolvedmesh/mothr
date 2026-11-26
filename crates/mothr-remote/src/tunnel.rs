//! Finding the tunnel CLIs, and exposing the server through Microsoft Dev Tunnels.
//!
//! Tunnel problems never stop the server; they are reported through `RemoteStatus::error`.

use crate::RemoteStatus;
use mothr_core::TunnelKind;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch};

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
            TunnelKind::Tailscale => self.tailscale.is_some(),
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

/// What to run for a Dev Tunnel.
#[derive(Debug, Clone)]
pub struct TunnelConfig {
    /// Address the tunnel should forward to.
    pub target: SocketAddr,
    /// The CLI to run; `None` when it is not installed.
    pub program: Option<PathBuf>,
}

impl TunnelConfig {
    pub fn detect(bound: SocketAddr) -> Self {
        Self {
            target: bound,
            program: detect_tunnels().devtunnel,
        }
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
    let Some(program) = cfg.program.clone() else {
        return set_error(
            &status,
            "Dev Tunnels isn't installed: the `devtunnel` command wasn't found.".to_string(),
        );
    };
    run_devtunnel(&program, &cfg, &status, shutdown).await
}

fn set_error(status: &watch::Sender<RemoteStatus>, error: String) {
    tracing::warn!("remote tunnel: {error}");
    status.send_modify(|s| {
        s.error = Some(error);
        s.public_url = None;
    });
}

async fn run_devtunnel(
    program: &Path,
    cfg: &TunnelConfig,
    status: &watch::Sender<RemoteStatus>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut cmd = Command::new(program);
    mothr_core::process::no_window(cmd.as_std_mut());
    let mut child = match cmd
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devtunnel_command() {
        assert_eq!(
            devtunnel_args(7878),
            ["host", "-p", "7878", "--allow-anonymous"]
        );
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
    #[tokio::test]
    async fn devtunnel_with_fake_cli() {
        let _serial = crate::test_util::PROC_LOCK.lock().await;
        let dir = std::env::temp_dir().join(format!("mothr-remote-dt-{}", uuid::Uuid::new_v4()));
        let program = crate::test_util::fake_cli(
            &dir,
            r#"echo "Hosting port: $3"
echo "Connect via browser: https://zz9-$3.euw.devtunnels.ms, https://zz9.euw.devtunnels.ms:$3"
exec sleep 30"#,
        );
        let (status, mut rx) = watch::channel(RemoteStatus::default());
        let (stop, stop_rx) = watch::channel(false);
        let cfg = TunnelConfig {
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
        let (status, rx) = watch::channel(RemoteStatus::default());
        let (_stop, stop_rx) = watch::channel(false);
        let cfg = TunnelConfig {
            target: "127.0.0.1:1".parse().unwrap(),
            program: None,
        };
        run(cfg, status, stop_rx).await;
        assert!(rx.borrow().error.as_deref().unwrap().contains("devtunnel"));
    }
}
