//! The hub contract: the single API both the desktop UI and the remote-control server use.

use crate::agent::{AgentInstall, AgentKind, PermissionMode};
use crate::session::{SessionCommand, SessionEvent, SessionId, SessionKind, SessionMeta, SessionState};
use crate::usage::ProviderUsage;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::sync::broadcast;

/// A development project as presented to UIs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectRef {
    pub path: PathBuf,
    pub name: String,
    /// Detected stack tags: "rust", "node", "python", "git" …
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub git_branch: Option<String>,
}

impl ProjectRef {
    pub fn from_path(path: PathBuf) -> Self {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        Self { path, name, tags: vec![], pinned: false, git_branch: None }
    }
}

/// A fuzzy search hit with the matched character indices into `project.name`/path for highlighting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectHit {
    pub project: ProjectRef,
    pub score: u32,
    /// Char indices matched in `project.name`.
    #[serde(default)]
    pub name_indices: Vec<u32>,
    /// Char indices matched in `display_path`.
    #[serde(default)]
    pub path_indices: Vec<u32>,
    /// The path as displayed and matched (`~/…`).
    #[serde(default)]
    pub display_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppCommand {
    NewSession {
        agent: AgentKind,
        kind: SessionKind,
        project: PathBuf,
        mode: PermissionMode,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        prompt: Option<String>,
    },
    Session {
        id: SessionId,
        command: SessionCommand,
    },
    /// Rewind the conversation to before `entry_id` (a user message) and send `text` instead.
    EditMessage { id: SessionId, entry_id: String, text: String },
    /// Remove a queued message.
    Dequeue { id: SessionId, queue_id: String },
    /// Re-spawn an ended session, resuming the agent's conversation if possible.
    Resume { id: SessionId },
    Rename { id: SessionId, title: String },
    PinSession { id: SessionId, pinned: bool },
    Archive { id: SessionId, archived: bool },
    SetGroup { id: SessionId, group: Option<String> },
    MarkRead { id: SessionId },
    Delete { id: SessionId },
    PinProject { path: PathBuf, pinned: bool },
    RefreshUsage,
    RescanAgents,
    RescanProjects,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // Events are moved through channels, never stored in bulk.
pub enum HubEvent {
    SessionCreated { state: Box<SessionState> },
    Session { id: SessionId, event: SessionEvent },
    SessionMeta { meta: SessionMeta },
    SessionRemoved { id: SessionId },
    Usage { usage: ProviderUsage },
    Agents { agents: Vec<AgentInstall> },
    Projects { pinned: Vec<ProjectRef>, recent: Vec<ProjectRef> },
    /// Project index finished (re)scanning.
    ProjectsIndexed { count: usize },
    Toast { level: crate::session::NoticeLevel, text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HubSnapshot {
    pub sessions: Vec<SessionState>,
    pub agents: Vec<AgentInstall>,
    pub usage: Vec<ProviderUsage>,
    pub pinned_projects: Vec<ProjectRef>,
    pub recent_projects: Vec<ProjectRef>,
}

/// Implemented by the hub; consumed by the GPUI app and the remote server.
pub trait HubApi: Send + Sync + 'static {
    fn snapshot(&self) -> HubSnapshot;
    fn session(&self, id: SessionId) -> Option<SessionState>;
    fn subscribe(&self) -> broadcast::Receiver<HubEvent>;
    /// Subscribe and snapshot atomically, so no event is both in the snapshot and replayed.
    fn snapshot_and_subscribe(&self) -> (HubSnapshot, broadcast::Receiver<HubEvent>) {
        let rx = self.subscribe();
        (self.snapshot(), rx)
    }
    /// Execute a command. `NewSession` returns the new id.
    fn dispatch(&self, command: AppCommand) -> anyhow::Result<Option<SessionId>>;
    fn search_projects(&self, query: &str, limit: usize) -> Vec<ProjectHit>;
}
