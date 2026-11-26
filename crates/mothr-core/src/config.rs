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

/// Mothr's skill library: one folder per skill, each holding a `SKILL.md`.
pub fn skills_dir() -> PathBuf {
    data_dir().join("skills")
}

pub fn settings_path() -> PathBuf {
    config_dir().join("settings.toml")
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Expand a leading `~` to the home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    match path.strip_prefix('~') {
        Some("") => home_dir(),
        Some(rest) if rest.starts_with(['/', '\\']) => home_dir().join(&rest[1..]),
        _ => PathBuf::from(path),
    }
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

/// The accent color: violet by default, a preset, or your own (`Appearance::custom_accent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    #[default]
    Violet,
    Blue,
    Teal,
    Green,
    Orange,
    Rose,
    Pink,
    /// Monochrome: text color as the accent.
    Mono,
    Custom,
}

impl Accent {
    pub const PRESETS: [Accent; 8] = [Accent::Violet, Accent::Blue, Accent::Teal, Accent::Green, Accent::Orange, Accent::Rose, Accent::Pink, Accent::Mono];

    pub fn label(self) -> &'static str {
        match self {
            Accent::Violet => "Violet",
            Accent::Blue => "Blue",
            Accent::Teal => "Teal",
            Accent::Green => "Green",
            Accent::Orange => "Orange",
            Accent::Rose => "Rose",
            Accent::Pink => "Pink",
            Accent::Mono => "Mono",
            Accent::Custom => "Custom",
        }
    }

    /// The preset's color as 0xRRGGBB on a dark and a light background (None for `Custom`).
    pub fn hex(self, dark: bool) -> Option<u32> {
        Some(match (self, dark) {
            (Accent::Violet, true) => 0x8B7CF6,
            (Accent::Violet, false) => 0x6D5BEF,
            (Accent::Blue, true) => 0x5B9DFF,
            (Accent::Blue, false) => 0x2F6FE0,
            (Accent::Teal, true) => 0x2DD4BF,
            (Accent::Teal, false) => 0x0E9384,
            (Accent::Green, true) => 0x4ADE80,
            (Accent::Green, false) => 0x16A34A,
            (Accent::Orange, true) => 0xFB923C,
            (Accent::Orange, false) => 0xEA580C,
            (Accent::Rose, true) => 0xFB7185,
            (Accent::Rose, false) => 0xE11D48,
            (Accent::Pink, true) => 0xF472B6,
            (Accent::Pink, false) => 0xDB2777,
            (Accent::Mono, true) => 0xECECF1,
            (Accent::Mono, false) => 0x15161A,
            (Accent::Custom, _) => return None,
        })
    }
}

/// The tone of the surfaces (backgrounds, cards, borders).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// Neutral greys.
    #[default]
    Graphite,
    /// Cool, a little blue.
    Slate,
    /// Warm, a little brown.
    Warm,
    /// Pure black in dark mode (OLED).
    Black,
}

impl Tone {
    pub const ALL: [Tone; 4] = [Tone::Graphite, Tone::Slate, Tone::Warm, Tone::Black];

    pub fn label(self) -> &'static str {
        match self {
            Tone::Graphite => "Graphite",
            Tone::Slate => "Slate",
            Tone::Warm => "Warm",
            Tone::Black => "Black",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    #[default]
    Left,
    Right,
}

/// Where the terminal sits next to a chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Dock {
    Top,
    Right,
    #[default]
    Bottom,
    Left,
}

impl Dock {
    pub const ALL: [Dock; 4] = [Dock::Top, Dock::Right, Dock::Bottom, Dock::Left];

    pub fn label(self) -> &'static str {
        match self {
            Dock::Top => "Top",
            Dock::Right => "Right",
            Dock::Bottom => "Bottom",
            Dock::Left => "Left",
        }
    }

    /// Beside the chat rather than above or below it.
    pub fn is_side(self) -> bool {
        matches!(self, Dock::Left | Dock::Right)
    }
}

/// How Mothr looks and where things sit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub accent: Accent,
    /// "#RRGGBB" used when `accent` is `Custom`.
    pub custom_accent: Option<String>,
    pub tone: Tone,
    pub sidebar_side: Side,
    /// Where a chat's terminal sits.
    pub terminal_dock: Dock,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            accent: Accent::Violet,
            custom_accent: None,
            tone: Tone::Graphite,
            sidebar_side: Side::Left,
            terminal_dock: Dock::Bottom,
        }
    }
}

impl Appearance {
    /// The accent as 0xRRGGBB for a dark or light background.
    pub fn accent_hex(&self, dark: bool) -> u32 {
        match self.accent {
            Accent::Custom => self.custom_accent.as_deref().and_then(parse_hex_color).unwrap_or(Accent::Violet.hex(dark).unwrap_or(0x8B7CF6)),
            a => a.hex(dark).unwrap_or(0x8B7CF6),
        }
    }
}

/// "#8B7CF6", "8b7cf6" or "#87f" → 0x8B7CF6.
pub fn parse_hex_color(s: &str) -> Option<u32> {
    let h = s.trim().trim_start_matches('#');
    let full: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 => h.to_string(),
        _ => return None,
    };
    u32::from_str_radix(&full, 16).ok()
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
    /// Show the agent's reasoning ("Thought · N words") in chats.
    pub show_thinking: bool,
    /// Interface scale: 1.0 compact, 1.15 default, 1.3 large.
    pub ui_scale: f32,
    /// Start new chats with the agent and model you used last.
    pub remember_last_selection: bool,
    pub last_agent: Option<AgentKind>,
    /// Last model chosen per agent.
    pub last_models: BTreeMap<AgentKind, String>,
    /// Last reasoning effort chosen per agent.
    pub last_efforts: BTreeMap<AgentKind, String>,
    /// Last context window chosen per agent.
    pub last_contexts: BTreeMap<AgentKind, String>,
    /// Whether the sidebar was open when the app last ran.
    pub sidebar_open: bool,
    /// Accent, tone and layout.
    pub appearance: Appearance,
    /// List the enabled skills in every chat's instructions, so agents can pick one on their
    /// own. Off, skills only run when typed as `/name`.
    pub skills_auto: bool,
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
            show_thinking: true,
            ui_scale: 1.15,
            remember_last_selection: true,
            last_agent: None,
            last_models: BTreeMap::new(),
            last_efforts: BTreeMap::new(),
            last_contexts: BTreeMap::new(),
            sidebar_open: true,
            appearance: Appearance::default(),
            skills_auto: true,
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

    /// Record the model chosen for `agent`; `None` (the agent's own default) clears it. No-op
    /// unless remembering is enabled. Returns whether it changed.
    pub fn record_model(&mut self, agent: AgentKind, model: Option<&str>) -> bool {
        if !self.remember_last_selection {
            return false;
        }
        let mut changed = self.last_agent != Some(agent);
        self.last_agent = Some(agent);
        match model {
            Some(m) => {
                changed |= self.last_models.get(&agent).map(String::as_str) != Some(m);
                self.last_models.insert(agent, m.to_string());
            }
            None => changed |= self.last_models.remove(&agent).is_some(),
        }
        changed
    }

    /// Reasoning effort new chats with `agent` should start with (None = the agent's default).
    pub fn preferred_effort(&self, agent: AgentKind) -> Option<String> {
        self.remember_last_selection.then(|| self.last_efforts.get(&agent).cloned()).flatten()
    }

    /// Context window new chats with `agent` should start with (None = the agent's default).
    pub fn preferred_context(&self, agent: AgentKind) -> Option<String> {
        self.remember_last_selection.then(|| self.last_contexts.get(&agent).cloned()).flatten()
    }

    /// Record the reasoning / context choice for `agent`; `None` clears it back to the agent's
    /// default. No-op unless remembering is enabled. Returns whether anything changed.
    pub fn record_options(&mut self, agent: AgentKind, effort: Option<&str>, context: Option<&str>) -> bool {
        if !self.remember_last_selection {
            return false;
        }
        fn set(map: &mut BTreeMap<AgentKind, String>, agent: AgentKind, v: Option<&str>) -> bool {
            match v {
                Some(v) if map.get(&agent).map(String::as_str) != Some(v) => {
                    map.insert(agent, v.to_string());
                    true
                }
                None => map.remove(&agent).is_some(),
                _ => false,
            }
        }
        let a = set(&mut self.last_efforts, agent, effort);
        let b = set(&mut self.last_contexts, agent, context);
        a | b
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
    fn appearance_round_trips_and_old_bar_settings_are_ignored() {
        let mut s = Settings::default();
        s.appearance.accent = Accent::Custom;
        s.appearance.custom_accent = Some("#12ab34".into());
        s.appearance.sidebar_side = Side::Right;
        let text = toml::to_string(&s).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back.appearance, s.appearance);
        assert_eq!(back.appearance.accent_hex(true), 0x12AB34);
        // Settings from the version with a separate bar still load.
        let old = text.replace("[appearance]", "[appearance]\nbar_position = \"bottom\"\nbar_items = [\"brand\"]");
        let back: Settings = toml::from_str(&old).unwrap();
        assert_eq!(back.appearance, s.appearance);
        // Older files without the section get the defaults.
        let old: Settings = toml::from_str("theme = \"dark\"").unwrap();
        assert_eq!(old.appearance, Appearance::default());
        assert_eq!(parse_hex_color("#87f"), Some(0x8877FF));
        assert_eq!(parse_hex_color("nope"), None);
    }

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
    fn remembered_options_are_per_agent_and_clearable() {
        let mut s = Settings::default();
        assert!(s.record_options(AgentKind::Copilot, Some("high"), Some("long_context")));
        assert!(!s.record_options(AgentKind::Copilot, Some("high"), Some("long_context")));
        assert_eq!(s.preferred_effort(AgentKind::Copilot).as_deref(), Some("high"));
        assert_eq!(s.preferred_context(AgentKind::Copilot).as_deref(), Some("long_context"));
        assert_eq!(s.preferred_effort(AgentKind::ClaudeCode), None);
        assert!(s.record_options(AgentKind::Copilot, None, None));
        assert_eq!(s.preferred_effort(AgentKind::Copilot), None);
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
