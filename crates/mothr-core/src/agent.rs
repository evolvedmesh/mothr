//! Agent kinds, installations and the transports Mothr uses to talk to them.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The first-party agents Mothr knows how to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    Copilot,
    OpenCode,
    Hermes,
}

impl AgentKind {
    pub const ALL: [AgentKind; 5] = [
        AgentKind::ClaudeCode,
        AgentKind::Codex,
        AgentKind::Copilot,
        AgentKind::OpenCode,
        AgentKind::Hermes,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "Claude Code",
            AgentKind::Codex => "Codex",
            AgentKind::Copilot => "GitHub Copilot",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Hermes => "Hermes",
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "Claude",
            AgentKind::Codex => "Codex",
            AgentKind::Copilot => "Copilot",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Hermes => "Hermes",
        }
    }

    /// Stable id used in config files, URLs and the remote API.
    pub fn id(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "claude_code",
            AgentKind::Codex => "codex",
            AgentKind::Copilot => "copilot",
            AgentKind::OpenCode => "open_code",
            AgentKind::Hermes => "hermes",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }

    /// Brand accent as 0xRRGGBB.
    pub fn accent(self) -> u32 {
        match self {
            AgentKind::ClaudeCode => 0xD97757,
            AgentKind::Codex => 0x10A37F,
            AgentKind::Copilot => 0x8B5CF6,
            AgentKind::OpenCode => 0x3B82F6,
            AgentKind::Hermes => 0xF59E0B,
        }
    }

    /// One-glyph monogram for avatars.
    pub fn monogram(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "C",
            AgentKind::Codex => "X",
            AgentKind::Copilot => "G",
            AgentKind::OpenCode => "O",
            AgentKind::Hermes => "H",
        }
    }

    /// Which usage provider this agent's limits come from by default.
    pub fn provider(self) -> Provider {
        match self {
            AgentKind::ClaudeCode => Provider::Anthropic,
            AgentKind::Codex => Provider::OpenAi,
            AgentKind::Copilot => Provider::GitHubCopilot,
            AgentKind::OpenCode => Provider::OpenCode,
            AgentKind::Hermes => Provider::Hermes,
        }
    }

    /// Permission modes this agent can honour.
    pub fn supported_modes(self) -> &'static [PermissionMode] {
        use PermissionMode::*;
        match self {
            AgentKind::ClaudeCode => &[Ask, AcceptEdits, Auto, Plan, AllowAll],
            AgentKind::Copilot => &[Ask, Auto, Plan, AllowAll],
            AgentKind::Codex => &[Ask, AcceptEdits, Auto, AllowAll],
            AgentKind::OpenCode => &[Ask, Plan, AllowAll],
            AgentKind::Hermes => &[Ask, AllowAll],
        }
    }

    /// Messages can be sent into a running turn. Claude Code takes them at its next step; the ACP
    /// agents have no mid-turn input, so their driver redirects at the next safe point.
    pub fn supports_steering(self) -> bool {
        true
    }

    /// Sent messages can be edited: the conversation (and files) rewind and the edit is resent.
    pub fn supports_edit(self) -> bool {
        matches!(self, AgentKind::ClaudeCode)
    }

    /// Whether the agent supports Anthropic-style Remote Control of a session.
    pub fn supports_remote_control(self) -> bool {
        matches!(self, AgentKind::ClaudeCode)
    }
}

/// Usage/billing providers whose limits Mothr tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Anthropic,
    OpenAi,
    GitHubCopilot,
    OpenCode,
    Hermes,
}

impl Provider {
    pub fn display_name(self) -> &'static str {
        match self {
            Provider::Anthropic => "Claude",
            Provider::OpenAi => "OpenAI Codex",
            Provider::GitHubCopilot => "GitHub Copilot",
            Provider::OpenCode => "OpenCode",
            Provider::Hermes => "Hermes",
        }
    }

    pub fn agent(self) -> AgentKind {
        match self {
            Provider::Anthropic => AgentKind::ClaudeCode,
            Provider::OpenAi => AgentKind::Codex,
            Provider::GitHubCopilot => AgentKind::Copilot,
            Provider::OpenCode => AgentKind::OpenCode,
            Provider::Hermes => AgentKind::Hermes,
        }
    }
}

/// How Mothr speaks to an installed agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transport {
    /// Claude Code's native `--input-format stream-json` control protocol.
    ClaudeStreamJson { program: PathBuf },
    /// Any Agent Client Protocol server over stdio.
    Acp {
        program: PathBuf,
        args: Vec<String>,
        #[serde(default)]
        env: Vec<(String, String)>,
    },
}

impl Transport {
    pub fn label(&self) -> &'static str {
        match self {
            Transport::ClaudeStreamJson { .. } => "Claude SDK protocol",
            Transport::Acp { .. } => "Agent Client Protocol",
        }
    }
}

/// A detected (or configured) agent installation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentInstall {
    pub kind: AgentKind,
    /// The user-facing CLI that was found, if any.
    pub binary: Option<PathBuf>,
    pub version: Option<String>,
    /// Transport Mothr will use. `None` means the agent can't be started.
    pub transport: Option<Transport>,
    /// Human readable explanation when unavailable or degraded.
    pub note: Option<String>,
}

impl AgentInstall {
    pub fn available(&self) -> bool {
        self.transport.is_some()
    }
}

/// Unified permission policy across agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Ask before every sensitive action.
    #[default]
    Ask,
    /// Auto-approve file edits, ask for commands.
    AcceptEdits,
    /// Let the agent's classifier decide (Claude "auto", Copilot autopilot, Codex auto).
    Auto,
    /// Read-only planning; no edits.
    Plan,
    /// Approve everything ("yolo" / bypass permissions).
    AllowAll,
}

impl PermissionMode {
    pub const ALL: [PermissionMode; 5] = [
        PermissionMode::Ask,
        PermissionMode::AcceptEdits,
        PermissionMode::Auto,
        PermissionMode::Plan,
        PermissionMode::AllowAll,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PermissionMode::Ask => "Ask",
            PermissionMode::AcceptEdits => "Accept edits",
            PermissionMode::Auto => "Auto",
            PermissionMode::Plan => "Plan",
            PermissionMode::AllowAll => "Allow all",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            PermissionMode::Ask => "Ask before running tools or editing files",
            PermissionMode::AcceptEdits => "Edit files freely, ask before commands",
            PermissionMode::Auto => "Agent decides what is safe; asks when unsure",
            PermissionMode::Plan => "Research and plan only, no changes",
            PermissionMode::AllowAll => "Approve everything without asking",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            PermissionMode::Ask => "ask",
            PermissionMode::AcceptEdits => "accept_edits",
            PermissionMode::Auto => "auto",
            PermissionMode::Plan => "plan",
            PermissionMode::AllowAll => "allow_all",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    /// The closest mode `agent` supports (the app-wide default may not exist for every agent).
    pub fn for_agent(self, agent: AgentKind) -> PermissionMode {
        let supported = agent.supported_modes();
        let chain: &[PermissionMode] = match self {
            PermissionMode::Auto => &[PermissionMode::Auto, PermissionMode::AcceptEdits, PermissionMode::Ask],
            PermissionMode::AcceptEdits => &[PermissionMode::AcceptEdits, PermissionMode::Ask],
            PermissionMode::Plan => &[PermissionMode::Plan, PermissionMode::Ask],
            PermissionMode::AllowAll => &[PermissionMode::AllowAll, PermissionMode::Auto, PermissionMode::Ask],
            PermissionMode::Ask => &[PermissionMode::Ask],
        };
        chain.iter().copied().find(|m| supported.contains(m)).unwrap_or(PermissionMode::Ask)
    }

    pub fn is_dangerous(self) -> bool {
        matches!(self, PermissionMode::AllowAll)
    }
}
