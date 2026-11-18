//! Settings and on-disk locations.

use crate::agent::{AgentKind, PermissionMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `MOTHR_HOME` relocates all config and data (useful for testing and portable installs).
fn home_override() -> Option<PathBuf> {
    std::env::var_os("MOTHR_HOME").filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn config_dir() -> PathBuf {
    if let Some(h) = home_override() {
        return h.join("config");
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mothr")
}

pub fn data_dir() -> PathBuf {
    if let Some(h) = home_override() {
        return h.join("data");
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mothr")
}

/// The app was called Shepherd before it was Mothr. Move its config and data folders over once,
/// so chats and settings survive the rename. Call before anything reads settings.
pub fn migrate_legacy_dirs() {
    if home_override().is_some() {
        return;
    }
    for (base, new) in [(dirs::config_dir(), config_dir()), (dirs::data_dir(), data_dir())] {
        let Some(old) = base.map(|b| b.join(LEGACY_NAME)) else { continue };
        if old.is_dir() && !new.exists() {
            match std::fs::rename(&old, &new) {
                Ok(()) => tracing::info!("moved {} to {}", old.display(), new.display()),
                Err(e) => tracing::warn!("couldn't move {} to {}: {e}", old.display(), new.display()),
            }
        }
    }
}

/// The app's previous name, used only to find data from before the rename.
pub const LEGACY_NAME: &str = "shepherd";

pub fn sessions_dir() -> PathBuf {
    data_dir().join("sessions")
}

/// Where images (agent output, pasted screenshots) are stored.
pub fn images_dir() -> PathBuf {
    data_dir().join("images")
}

/// File extension for an image MIME type.
pub fn image_ext(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        _ => return None,
    })
}

/// Write image bytes into [`images_dir`] under a content-derived name (same bytes, same file).
pub fn save_image(bytes: &[u8], mime: &str) -> anyhow::Result<PathBuf> {
    use std::hash::{Hash, Hasher};
    let ext = image_ext(mime).ok_or_else(|| anyhow::anyhow!("unsupported image type {mime}"))?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    let path = images_dir().join(format!("{:016x}-{}.{ext}", h.finish(), bytes.len()));
    if !path.exists() {
        std::fs::create_dir_all(images_dir())?;
        std::fs::write(&path, bytes)?;
    }
    Ok(path)
}

/// Decode base64 image data from an agent and store it. Returns the saved path.
pub fn save_image_base64(data: &str, mime: &str) -> anyhow::Result<PathBuf> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim())?;
    save_image(&bytes, mime)
}

/// Where design sessions without a project keep their files.
pub fn designs_dir() -> PathBuf {
    data_dir().join("designs")
}

pub fn settings_path() -> PathBuf {
    config_dir().join("settings.toml")
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Replace the home prefix with `~` for display.
pub fn tildify(path: &Path) -> String {
    let home = home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TunnelKind {
    /// Only reachable on the local network / bind address.
    #[default]
    None,
    /// Direct connection over the tailnet, on this machine's Tailscale address.
    /// `tailscale_funnel` was a separate option once; those settings land here.
    #[serde(alias = "tailscale_funnel")]
    Tailscale,
    /// Microsoft Dev Tunnels (`devtunnel host`).
    DevTunnel,
}

impl TunnelKind {
    pub fn label(self) -> &'static str {
        match self {
            TunnelKind::None => "Local network",
            TunnelKind::Tailscale => "Tailscale (tailnet only)",
            TunnelKind::DevTunnel => "Dev Tunnels",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentOverride {
    #[serde(default)]
    pub disabled: bool,
    /// Explicit program path (CLI or ACP adapter).
    pub program: Option<PathBuf>,
    /// Explicit argument list; when set the agent is launched over ACP with these args.
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteSettings {
    pub enabled: bool,
    pub bind: String,
    pub port: u16,
    /// Bearer token required by the remote client. Generated on first run.
    pub token: String,
    pub tunnel: TunnelKind,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "127.0.0.1".into(),
            port: 7878,
            token: String::new(),
            tunnel: TunnelKind::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemePreference,
    /// Directories scanned for development projects.
    pub project_roots: Vec<PathBuf>,
    pub scan_depth: usize,
    pub default_agent: Option<AgentKind>,
    pub default_mode: PermissionMode,
    pub agents: BTreeMap<AgentKind, AgentOverride>,
    pub remote: RemoteSettings,
    /// Seconds between account-limit refreshes.
    pub usage_poll_secs: u64,
    /// Warn in the UI when any window crosses this fraction.
    pub warn_threshold: f32,
    /// Notify (toast) when a background session needs input.
    pub notify_needs_input: bool,
    /// Interface scale: 1.0 compact, 1.15 default, 1.3 large.
    pub ui_scale: f32,
    /// Start new chats with the agent and model you used last.
    pub remember_last_selection: bool,
    pub last_agent: Option<AgentKind>,
    /// Last model chosen per agent.
    pub last_models: BTreeMap<AgentKind, String>,
    /// Whether the sidebar was open when the app last ran.
    pub sidebar_open: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let home = home_dir();
        let mut roots: Vec<PathBuf> = [
            "projects", "dev", "code", "src", "repos", "workspace", "work", "Developer", "git",
            "Documents/GitHub", "Documents/Projects", "source/repos",
        ]
        .iter()
        .map(|p| home.join(p))
        .filter(|p| p.is_dir())
        .collect();
        if roots.is_empty() {
            roots.push(home);
        }
        Self {
            theme: ThemePreference::System,
            project_roots: roots,
            scan_depth: 4,
            default_agent: None,
            default_mode: PermissionMode::Ask,
            agents: BTreeMap::new(),
            remote: RemoteSettings::default(),
            usage_poll_secs: 120,
            warn_threshold: 0.8,
            notify_needs_input: true,
            ui_scale: 1.15,
            remember_last_selection: true,
            last_agent: None,
            last_models: BTreeMap::new(),
            sidebar_open: true,
        }
    }
}

impl Settings {
    /// Agent new chats should start with.
    pub fn preferred_agent(&self) -> Option<AgentKind> {
        if self.remember_last_selection {
            self.last_agent.or(self.default_agent)
        } else {
            self.default_agent
        }
    }

    /// Model new chats with `agent` should start with (None = the agent's own default).
    pub fn preferred_model(&self, agent: AgentKind) -> Option<String> {
        let fixed = self.agents.get(&agent).and_then(|o| o.default_model.clone());
        if self.remember_last_selection {
            self.last_models.get(&agent).cloned().or(fixed)
        } else {
            fixed
        }
    }

    /// Record a selection (no-op unless remembering is enabled). Returns whether it changed.
    pub fn record_selection(&mut self, agent: AgentKind, model: Option<&str>) -> bool {
        if !self.remember_last_selection {
            return false;
        }
        let mut changed = self.last_agent != Some(agent);
        self.last_agent = Some(agent);
        if let Some(m) = model {
            changed |= self.last_models.get(&agent).map(String::as_str) != Some(m);
            self.last_models.insert(agent, m.to_string());
        }
        changed
    }

    pub fn load() -> Self {
        let path = settings_path();
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("invalid settings at {}: {e}", path.display());
                Settings::default()
            }),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = settings_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_atomic(&path, toml::to_string_pretty(self)?.as_bytes())
    }
}

/// Write via temp file + rename so a crash never leaves a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembered_selection_wins_when_enabled() {
        let mut s = Settings { default_agent: Some(AgentKind::Copilot), ..Settings::default() };
        assert!(s.record_selection(AgentKind::ClaudeCode, Some("opus")));
        assert_eq!(s.preferred_agent(), Some(AgentKind::ClaudeCode));
        assert_eq!(s.preferred_model(AgentKind::ClaudeCode).as_deref(), Some("opus"));
        // Recording the same thing again is not a change.
        assert!(!s.record_selection(AgentKind::ClaudeCode, Some("opus")));
    }

    #[test]
    fn fixed_defaults_apply_when_remembering_is_off() {
        let mut s = Settings { remember_last_selection: false, default_agent: Some(AgentKind::Hermes), ..Settings::default() };
        s.agents.entry(AgentKind::Hermes).or_default().default_model = Some("anthropic:claude-sonnet-5".into());
        assert!(!s.record_selection(AgentKind::ClaudeCode, Some("opus")));
        assert_eq!(s.preferred_agent(), Some(AgentKind::Hermes));
        assert_eq!(s.preferred_model(AgentKind::Hermes).as_deref(), Some("anthropic:claude-sonnet-5"));
    }

    #[test]
    fn old_funnel_setting_becomes_tailscale() {
        let remote: RemoteSettings = toml::from_str(
            "enabled = true\nbind = \"127.0.0.1\"\nport = 7878\ntoken = \"t\"\ntunnel = \"tailscale_funnel\"",
        )
        .unwrap();
        assert_eq!(remote.tunnel, TunnelKind::Tailscale);
    }

    #[test]
    fn settings_roundtrip_through_toml() {
        let mut s = Settings::default();
        s.record_selection(AgentKind::Codex, Some("gpt-6-sol"));
        let back: Settings = toml::from_str(&toml::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.last_models.get(&AgentKind::Codex).map(String::as_str), Some("gpt-6-sol"));
    }

    #[test]
    fn default_mode_falls_back_per_agent() {
        assert_eq!(PermissionMode::Auto.for_agent(AgentKind::ClaudeCode), PermissionMode::Auto);
        assert_eq!(PermissionMode::Auto.for_agent(AgentKind::Hermes), PermissionMode::Ask);
        assert_eq!(PermissionMode::Plan.for_agent(AgentKind::Codex), PermissionMode::Ask);
        assert_eq!(PermissionMode::AcceptEdits.for_agent(AgentKind::Copilot), PermissionMode::Ask);
        assert_eq!(PermissionMode::AllowAll.for_agent(AgentKind::OpenCode), PermissionMode::AllowAll);
    }
}
