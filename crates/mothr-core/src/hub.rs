//! The hub contract: the single API both the desktop UI and the remote-control server use.

use crate::agent::{AgentInstall, AgentKind, PermissionMode};
use crate::session::{ModelInfo, SessionCommand, SessionEvent, SessionId, SessionKind, SessionMeta, SessionState};
use crate::usage::ProviderUsage;
use crate::workspace::{Workspace, WorkspaceId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
        effort: Option<String>,
        /// A [`crate::ChoiceInfo::id`] from the model's `contexts`.
        #[serde(default)]
        context: Option<String>,
        #[serde(default)]
        prompt: Option<String>,
        /// Where the chat goes; None = the workspace open on the desktop.
        #[serde(default)]
        workspace: Option<WorkspaceId>,
        /// More projects the chat works across besides `project`, its primary one.
        #[serde(default)]
        extra_projects: Vec<PathBuf>,
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
    /// Replace the projects a chat works across besides its primary one. A running chat picks
    /// them up from its next message (Claude Code restarts, resuming the conversation).
    SetExtraProjects { id: SessionId, projects: Vec<PathBuf> },
    PinSession { id: SessionId, pinned: bool },
    Archive { id: SessionId, archived: bool },
    SetGroup { id: SessionId, group: Option<String> },
    MarkRead { id: SessionId },
    Delete { id: SessionId },
    PinProject { path: PathBuf, pinned: bool },
    /// Add a workspace (the caller picks its id). At most [`crate::MAX_WORKSPACES`].
    CreateWorkspace { workspace: Workspace },
    /// Rename a workspace or change its icon.
    UpdateWorkspace { workspace: Workspace },
    /// Remove a workspace; its chats move to the first one left. The last one can't go.
    DeleteWorkspace { id: WorkspaceId },
    /// Move a workspace to `index` in the order.
    MoveWorkspace { id: WorkspaceId, index: usize },
    /// Move a chat (and its subagents) to another workspace.
    SetWorkspace { id: SessionId, workspace: WorkspaceId },
    /// Remember which workspace the desktop shows; new chats without one go there.
    SelectWorkspace { id: WorkspaceId },
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
    /// The models an agent offers (learned from a session or a startup probe).
    AgentModels { agent: AgentKind, models: Vec<ModelInfo> },
    Projects { pinned: Vec<ProjectRef>, recent: Vec<ProjectRef> },
    /// Project index finished (re)scanning.
    ProjectsIndexed { count: usize },
    Toast { level: crate::session::NoticeLevel, text: String },
    /// The workspaces changed (added, renamed, removed, reordered or selected).
    Workspaces { workspaces: Vec<Workspace>, active: Option<WorkspaceId> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct HubSnapshot {
    pub sessions: Vec<SessionState>,
    pub agents: Vec<AgentInstall>,
    pub usage: Vec<ProviderUsage>,
    pub pinned_projects: Vec<ProjectRef>,
    pub recent_projects: Vec<ProjectRef>,
    /// Last known model list per agent, so pickers work before the first chat.
    #[serde(default)]
    pub agent_models: BTreeMap<AgentKind, Vec<ModelInfo>>,
    /// In order; never empty once the hub has started.
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    /// The workspace the desktop shows.
    #[serde(default)]
    pub active_workspace: Option<WorkspaceId>,
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
    /// List a folder of a known project (`dir` is relative, `""` for the root).
    fn list_dir(&self, project: &std::path::Path, dir: &str) -> anyhow::Result<crate::DirListing> {
        let _ = (project, dir);
        anyhow::bail!("browsing projects isn't available here")
    }
    /// Read a file of a known project (`file` is relative).
    fn read_file(&self, project: &std::path::Path, file: &str) -> anyhow::Result<crate::FileContent> {
        let _ = (project, file);
        anyhow::bail!("browsing projects isn't available here")
    }
    /// Fuzzy-find files of a known project by path.
    fn find_files(&self, project: &std::path::Path, query: &str, limit: usize) -> anyhow::Result<Vec<crate::FileHit>> {
        let _ = (project, query, limit);
        anyhow::bail!("browsing projects isn't available here")
    }
}
