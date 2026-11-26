//! Updates for the desktop app.
//!
//! Checks Mothr's GitHub releases shortly after start and then every few hours. A newer
//! release is offered in the sidebar; clicking it downloads and verifies it, puts it in place
//! next to the running version and restarts into it (or waits for "Restart to update" while
//! chats are working). With "Install updates automatically" on, the download and install
//! happen on their own and the next start runs the new version.
//! Only copies installed with `install.sh` (Linux), the setup (Windows) or as an app the user
//! can replace (macOS: `install.sh` or the disk image) update themselves (see
//! `mothr_update::install`); others (`cargo run`, packages, the portable Windows zip) only say
//! that an update exists and where to get it.

use chrono::{DateTime, Utc};
use gpui_kit::*;
use mothr_hub::Hub;
use mothr_update::install::Install;
use mothr_update::{Client, Release};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;

/// This build's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The one-line installer, also shown where this copy can't update itself.
pub const INSTALL_COMMAND: &str = "curl -fsSL https://raw.githubusercontent.com/evolvedmesh/mothr/main/install.sh | bash";

const FIRST_CHECK: Duration = Duration::from_secs(20);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    /// Not checked yet.
    Idle,
    Checking,
    UpToDate,
    /// A newer release exists and isn't installed (automatic installs are off, or this copy
    /// can't update itself).
    Available { version: String, url: String },
    Installing { version: String },
    /// Installed; the next start runs it.
    Ready { version: String },
    Failed(String),
}

pub enum UpdateEvent {
    /// A new version was installed and takes effect on restart. `restart` is set when the
    /// user asked to update now, so the app should restart into it if nothing is running.
    Ready { version: String, restart: bool },
}

pub struct Updater {
    rt: Handle,
    hub: Arc<Hub>,
    install: Option<Install>,
    client: Option<Client>,
    state: UpdateState,
    checked_at: Option<DateTime<Utc>>,
    latest: Option<Release>,
    /// "Update" was clicked: restart once the install is done.
    restart_when_ready: bool,
    _schedule: Task<()>,
}

impl EventEmitter<UpdateEvent> for Updater {}

impl Updater {
    pub fn new(rt: Handle, hub: Arc<Hub>, cx: &mut Context<Self>) -> Self {
        let install = Install::detect();
        // What an update on Windows left behind: the binary it replaced, now that it's stopped.
        if let Some(i) = &install {
            i.clean_up();
        }
        // An update installed earlier that this run hasn't switched to yet.
        let state = install
            .as_ref()
            .and_then(|i| i.current_version())
            .filter(|v| is_newer(v, VERSION))
            .map(|version| UpdateState::Ready { version })
            .unwrap_or(UpdateState::Idle);
        let client = Client::new(&format!("mothr/{VERSION}")).inspect_err(|e| tracing::warn!("updates: {e:#}")).ok();
        let schedule = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FIRST_CHECK).await;
            loop {
                if this.update(cx, |u, cx| u.check(false, cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(CHECK_EVERY).await;
            }
        });
        Self { rt, hub, install, client, state, checked_at: None, latest: None, restart_when_ready: false, _schedule: schedule }
    }

    pub fn state(&self) -> &UpdateState {
        &self.state
    }

    pub fn checked_at(&self) -> Option<DateTime<Utc>> {
        self.checked_at
    }

    /// Whether this copy installs updates itself (see `Install::detect`).
    pub fn can_install(&self) -> bool {
        self.install.is_some()
    }

    /// The newest version seen by the last successful check.
    pub fn latest_version(&self) -> Option<String> {
        self.latest.as_ref().map(|r| r.version.to_string())
    }

    /// The release page of the newest version seen.
    pub fn release_url(&self) -> Option<&str> {
        self.latest.as_ref().map(|r| r.url.as_str())
    }

    /// Where to get the newest version by hand, for copies that don't update themselves: the
    /// Windows installer itself on Windows, the disk image on macOS, else the release page.
    pub fn download_url(&self) -> Option<&str> {
        let r = self.latest.as_ref()?;
        let direct = if cfg!(windows) {
            Some(mothr_update::windows_setup_asset(&r.version))
        } else if cfg!(target_os = "macos") {
            Some(mothr_update::macos_dmg_asset(&r.version, mothr_update::macos_arch()))
        } else {
            None
        };
        if let Some(asset) = direct.and_then(|name| r.asset(&name)) {
            return Some(&asset.url);
        }
        Some(&r.url)
    }

    /// Ask GitHub for the newest release. `manual` comes from "Check now".
    pub fn check(&mut self, manual: bool, cx: &mut Context<Self>) {
        if matches!(self.state, UpdateState::Checking | UpdateState::Installing { .. }) {
            return;
        }
        if !manual && matches!(self.state, UpdateState::Ready { .. }) {
            return;
        }
        let Some(client) = self.client.clone() else {
            self.state = UpdateState::Failed("Couldn't set up update checks".into());
            cx.notify();
            return;
        };
        let previous = std::mem::replace(&mut self.state, UpdateState::Checking);
        cx.notify();
        let job = self.rt.spawn(async move { client.latest().await });
        cx.spawn(async move |this, cx| {
            let result = job.await.map_err(anyhow::Error::from).and_then(|r| r);
            let _ = this.update(cx, |u, cx| u.checked(result, manual, previous, cx));
        })
        .detach();
    }

    fn checked(&mut self, result: anyhow::Result<Release>, manual: bool, previous: UpdateState, cx: &mut Context<Self>) {
        self.checked_at = Some(Utc::now());
        match result {
            Err(e) => {
                tracing::info!("update check failed: {e:#}");
                // A background check that fails keeps what we knew; "Check now" says why.
                self.state = if manual || previous == UpdateState::Idle { UpdateState::Failed(format!("{e:#}")) } else { previous };
            }
            Ok(release) if !release.is_newer_than(VERSION) => {
                self.state = UpdateState::UpToDate;
                self.latest = Some(release);
            }
            Ok(release) => {
                let version = release.version.to_string();
                let staged = self.install.as_ref().and_then(|i| i.current_version()).as_deref() == Some(version.as_str());
                let auto = self.hub.settings().auto_update;
                self.state = if staged {
                    UpdateState::Ready { version }
                } else {
                    UpdateState::Available { version, url: release.url.clone() }
                };
                self.latest = Some(release);
                if !staged && auto && self.can_install() {
                    self.install(cx);
                }
            }
        }
        cx.notify();
    }

    /// Download and put in place the newest release found by the last check.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        let (Some(install), Some(client), Some(release)) = (self.install.clone(), self.client.clone(), self.latest.clone()) else { return };
        if !release.is_newer_than(VERSION) || matches!(self.state, UpdateState::Installing { .. }) {
            return;
        }
        let version = release.version.to_string();
        self.state = UpdateState::Installing { version: version.clone() };
        cx.notify();
        let job = self.rt.spawn(async move { install.stage(&client, &release).await });
        cx.spawn(async move |this, cx| {
            let result = job.await.map_err(anyhow::Error::from).and_then(|r| r);
            let _ = this.update(cx, |u, cx| {
                match result {
                    Ok(()) => {
                        tracing::info!("installed Mothr {version}; it starts next time");
                        u.state = UpdateState::Ready { version: version.clone() };
                        let restart = std::mem::take(&mut u.restart_when_ready);
                        cx.emit(UpdateEvent::Ready { version, restart });
                    }
                    Err(e) => {
                        tracing::warn!("installing the update failed: {e:#}");
                        u.restart_when_ready = false;
                        u.state = UpdateState::Failed(format!("{e:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The sidebar's "Update": install the new version and restart into it, or restart if
    /// it's already installed. Copies that can't update themselves aren't offered this.
    pub fn update_now(this: &Entity<Self>, cx: &mut App) {
        match this.read(cx).state.clone() {
            UpdateState::Ready { .. } => Self::restart(this, cx),
            UpdateState::Available { .. } | UpdateState::Failed(_) if this.read(cx).can_install() => this.update(cx, |u, cx| {
                u.restart_when_ready = true;
                u.install(cx);
            }),
            _ => {}
        }
    }

    /// Quit and start the installed version once this process has exited.
    pub fn restart(this: &Entity<Self>, cx: &mut App) {
        let Some(binary) = this.read(cx).install.as_ref().map(Install::current_binary) else { return };
        match Self::restart_command(&binary).spawn() {
            Ok(_) => cx.quit(),
            Err(e) => tracing::warn!("couldn't restart into {}: {e}", binary.display()),
        }
    }

    /// Windows: start the new binary now; it waits for this process to exit (see `main`).
    #[cfg(windows)]
    fn restart_command(binary: &std::path::Path) -> std::process::Command {
        let mut cmd = std::process::Command::new(binary);
        cmd.env(WAIT_PID_VAR, std::process::id().to_string());
        if let Some(dir) = binary.parent() {
            cmd.current_dir(dir);
        }
        cmd
    }

    /// A shell that waits for this process to exit and then starts the new binary. On macOS too
    /// it runs the binary inside the bundle rather than `open`ing the bundle: LaunchServices
    /// would start it with launchd's environment instead of this one's.
    #[cfg(not(windows))]
    fn restart_command(binary: &std::path::Path) -> std::process::Command {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c")
            .arg(r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; exec "$2""#)
            .arg("mothr-restart")
            .arg(std::process::id().to_string())
            .arg(binary)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            // Its own process group, so it outlives this one.
            cmd.process_group(0);
        }
        cmd
    }
}

/// Set on a Windows restart: the process id of the Mothr that started this one, to wait for.
#[cfg(windows)]
pub const WAIT_PID_VAR: &str = "MOTHR_WAIT_PID";

/// After a restart into a new version (Windows), wait for the previous Mothr to finish saving
/// and exit. Call first thing in `main`, while the process has one thread.
pub fn wait_for_previous() {
    #[cfg(windows)]
    if let Some(pid) = std::env::var(WAIT_PID_VAR).ok().and_then(|p| p.parse::<u32>().ok()) {
        // SAFETY: called at the top of `main`, before any other thread exists.
        unsafe { std::env::remove_var(WAIT_PID_VAR) };
        mothr_update::install::wait_for_exit(pid, Duration::from_secs(30));
    }
}

fn is_newer(candidate: &str, current: &str) -> bool {
    match (semver::Version::parse(candidate), semver::Version::parse(current)) {
        (Ok(a), Ok(b)) => a > b,
        _ => false,
    }
}
