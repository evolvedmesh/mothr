//! Sessions, transcript entries, the events agents emit and the reducer that applies them.
//!
//! The same reducer ([`SessionState::apply`]) runs in the hub, in the GPUI app and (ported
//! to JS) in the remote web client, so every view of a session converges on the same state.

use crate::agent::{AgentKind, PermissionMode};
use crate::usage::ContextUsage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

pub type SessionId = Uuid;
pub type EntryId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    #[default]
    Chat,
    /// Design-system / UI mockup session rendered on the Mothr canvas.
    Design,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "state", content = "message", rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Starting,
    Idle,
    Working,
    /// Waiting on a permission decision or question answer from the user.
    NeedsInput,
    Error(String),
    /// Process exited; can be resumed.
    Ended,
}

impl SessionStatus {
    pub fn label(&self) -> &str {
        match self {
            SessionStatus::Starting => "Starting",
            SessionStatus::Idle => "Ready",
            SessionStatus::Working => "Working",
            SessionStatus::NeedsInput => "Needs you",
            SessionStatus::Error(_) => "Error",
            SessionStatus::Ended => "Ended",
        }
    }
    pub fn is_live(&self) -> bool {
        matches!(
            self,
            SessionStatus::Starting | SessionStatus::Idle | SessionStatus::Working | SessionStatus::NeedsInput
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RemoteControlState {
    pub enabled: bool,
    /// URL to continue the session from another device, if the agent reports one.
    pub url: Option<String>,
    pub pending: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

/// One selectable value of a model setting (a context window size, say).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceInfo {
    pub id: String,
    /// Short label for menus: "200k", "1M".
    pub label: String,
}

impl ChoiceInfo {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Reasoning-effort levels this model accepts ("low" … "max"); empty = no such setting.
    #[serde(default)]
    pub efforts: Vec<String>,
    /// Context window sizes this model can run with; empty = a single fixed size.
    #[serde(default)]
    pub contexts: Vec<ChoiceInfo>,
}

impl ModelInfo {
    pub fn new(id: impl Into<String>, name: impl Into<String>, description: Option<String>) -> Self {
        Self { id: id.into(), name: name.into(), description, ..Default::default() }
    }
}

/// "xhigh" → "Extra high", for menus.
pub fn effort_label(effort: &str) -> String {
    match effort {
        "xhigh" => "Extra high".into(),
        "none" => "Off".into(),
        other => {
            let mut c = other.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    }
}

/// Details of a subagent session spawned by an orchestrator chat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SubagentInfo {
    /// The orchestrator's tool call that spawned this subagent.
    pub tool_use_id: String,
    /// Agent-side task id (used to stop it).
    pub task_id: Option<String>,
    /// e.g. "general-purpose", "Explore".
    pub agent_type: Option<String>,
    pub prompt: Option<String>,
    #[serde(default)]
    pub tokens: u64,
    #[serde(default)]
    pub tool_uses: u64,
    pub last_tool: Option<String>,
    #[serde(default)]
    pub background: bool,
}

/// Metadata about a session (cheap to clone, used for lists and grouping).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: SessionId,
    pub title: String,
    pub agent: AgentKind,
    pub kind: SessionKind,
    pub project: PathBuf,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: SessionStatus,
    pub permission_mode: PermissionMode,
    pub model: Option<String>,
    /// Reasoning effort (None = the agent's default).
    #[serde(default)]
    pub effort: Option<String>,
    /// Context window choice, a [`ChoiceInfo::id`] (None = the agent's default).
    #[serde(default)]
    pub context_limit: Option<String>,
    /// The title was written by Mothr (generated or renamed), so agent titles don't replace it.
    #[serde(default)]
    pub titled: bool,
    /// The agent's own session id (for resume / load).
    pub agent_session_id: Option<String>,
    pub pinned: bool,
    /// User-assigned group label; overrides smart grouping when set.
    pub group: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub remote_control: RemoteControlState,
    #[serde(default)]
    pub context: ContextUsage,
    #[serde(default)]
    pub unread: bool,
    /// Set on subagent sessions: the orchestrator chat that spawned them.
    #[serde(default)]
    pub parent: Option<SessionId>,
    #[serde(default)]
    pub subagent: Option<SubagentInfo>,
    /// A general chat that isn't tied to a project (runs from the home directory).
    #[serde(default)]
    pub general: bool,
    /// Messages waiting to be sent when the agent finishes its current turn.
    #[serde(default)]
    pub queue: Vec<QueuedPrompt>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedPrompt {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

/// A slash command an agent offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Argument hint, e.g. "<file>".
    #[serde(default)]
    pub hint: Option<String>,
}

/// How a prompt sent while the agent is busy should be delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// Send now if idle, otherwise queue until the turn ends.
    #[default]
    Normal,
    /// Inject into the running turn so the agent picks it up at its next step (Claude Code).
    Steer,
    /// Stop the current turn and send immediately.
    Interrupt,
}

impl SessionMeta {
    pub fn new(agent: AgentKind, kind: SessionKind, project: PathBuf, mode: PermissionMode) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title: match kind {
                SessionKind::Chat => "New chat".into(),
                SessionKind::Design => "New design".into(),
            },
            agent,
            kind,
            project,
            created_at: now,
            updated_at: now,
            status: SessionStatus::Starting,
            permission_mode: mode,
            model: None,
            effort: None,
            context_limit: None,
            titled: false,
            agent_session_id: None,
            pinned: false,
            group: None,
            archived: false,
            remote_control: RemoteControlState::default(),
            context: ContextUsage::default(),
            unread: false,
            parent: None,
            subagent: None,
            general: false,
            queue: Vec::new(),
        }
    }

    pub fn is_subagent(&self) -> bool {
        self.parent.is_some()
    }

    pub fn project_name(&self) -> String {
        if self.general {
            return "General".into();
        }
        self.project
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.project.display().to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    Task,
    Other,
}

impl ToolKind {
    pub fn from_tool_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        if n.contains("read") || n == "ls" || n.contains("view") || n.contains("notebookread") {
            ToolKind::Read
        } else if n.contains("edit") || n.contains("write") || n.contains("patch") || n.contains("multiedit") {
            ToolKind::Edit
        } else if n.contains("grep") || n.contains("glob") || n.contains("search") || n.contains("find") {
            ToolKind::Search
        } else if n.contains("bash") || n.contains("shell") || n.contains("exec") || n.contains("terminal") || n.contains("command") {
            ToolKind::Execute
        } else if n.contains("fetch") || n.contains("web") {
            ToolKind::Fetch
        } else if n.contains("task") || n.contains("agent") {
            ToolKind::Task
        } else if n.contains("delete") || n.contains("remove") {
            ToolKind::Delete
        } else {
            ToolKind::Other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: PathBuf,
    /// The text being replaced; `None` when the file is new.
    pub old_text: Option<String>,
    pub new_text: String,
    /// 1-based line of `old_text` in the file, when `old_text` is only part of it.
    #[serde(default)]
    pub start_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: EntryId,
    pub name: String,
    pub title: String,
    pub kind: ToolKind,
    pub status: ToolStatus,
    /// Pretty input (command line, path, pattern …).
    pub input: Option<String>,
    pub output: Option<String>,
    pub diffs: Vec<FileDiff>,
    pub locations: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    pub content: String,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionOption {
    pub id: String,
    pub label: String,
    pub kind: PermissionOptionKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: EntryId,
    pub tool_call_id: Option<EntryId>,
    pub title: String,
    pub detail: Option<String>,
    pub kind: ToolKind,
    pub options: Vec<PermissionOption>,
    /// Option id chosen, once resolved.
    pub resolved: Option<String>,
    /// The file changes this request would make, for review before they happen.
    #[serde(default)]
    pub diffs: Vec<FileDiff>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub header: Option<String>,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestionRequest {
    pub id: EntryId,
    pub questions: Vec<Question>,
    /// One answer list per question once answered (labels or free text).
    pub answers: Option<Vec<Vec<String>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub path: PathBuf,
    pub mime: Option<String>,
}

/// One item in a chat transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    User {
        id: EntryId,
        text: String,
        #[serde(default)]
        attachments: Vec<Attachment>,
        at: DateTime<Utc>,
        /// The agent's id for this message (Claude message uuid) — enables edit & rewind.
        #[serde(default)]
        agent_ref: Option<String>,
        /// The agent's id of the last message before this one (resume point when editing).
        #[serde(default)]
        prev_ref: Option<String>,
    },
    Assistant {
        id: EntryId,
        text: String,
        #[serde(default)]
        streaming: bool,
    },
    Thinking {
        id: EntryId,
        text: String,
        #[serde(default)]
        streaming: bool,
    },
    Tool(ToolCall),
    /// An image produced by the agent (or one of its tools), stored under [`images_dir`].
    Image {
        id: EntryId,
        path: PathBuf,
        #[serde(default)]
        mime: Option<String>,
    },
    Plan {
        id: EntryId,
        items: Vec<PlanItem>,
    },
    Permission(PermissionRequest),
    Question(QuestionRequest),
    Notice {
        id: EntryId,
        level: NoticeLevel,
        text: String,
    },
    /// Agent finished a turn.
    TurnEnd {
        id: EntryId,
        stop_reason: String,
        duration_ms: Option<u64>,
        cost_usd: Option<f64>,
    },
}

impl Entry {
    pub fn id(&self) -> &str {
        match self {
            Entry::User { id, .. }
            | Entry::Assistant { id, .. }
            | Entry::Thinking { id, .. }
            | Entry::Plan { id, .. }
            | Entry::Notice { id, .. }
            | Entry::Image { id, .. }
            | Entry::TurnEnd { id, .. } => id,
            Entry::Tool(t) => &t.id,
            Entry::Permission(p) => &p.id,
            Entry::Question(q) => &q.id,
        }
    }

    pub fn is_pending_input(&self) -> bool {
        match self {
            Entry::Permission(p) => p.resolved.is_none(),
            Entry::Question(q) => q.answers.is_none(),
            _ => false,
        }
    }
}

pub fn new_entry_id() -> EntryId {
    Uuid::new_v4().simple().to_string()
}

/// Events flowing from an agent driver to every view of a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SessionEvent {
    /// Agent connected and reported its capabilities.
    Ready {
        agent_session_id: Option<String>,
        model: Option<String>,
        models: Vec<ModelInfo>,
        modes: Vec<ModeInfo>,
    },
    /// Insert or replace an entry with the same id.
    Upsert { entry: Entry },
    /// Append streamed text to an Assistant/Thinking entry (created if missing).
    AppendText {
        entry_id: EntryId,
        thinking: bool,
        delta: String,
    },
    /// Mark streaming entries as done.
    StreamEnd { entry_id: EntryId },
    Status { status: SessionStatus },
    Context { usage: ContextUsage },
    Mode { mode: PermissionMode },
    Model { model: String },
    /// Reasoning effort / context window the session now runs with.
    Options { effort: Option<String>, context: Option<String> },
    Title { title: String },
    RemoteControl { state: RemoteControlState },
    /// The agent acknowledged a user message and told us its ids.
    UserRef { entry_id: EntryId, agent_ref: String, prev_ref: Option<String> },
    /// Drop this entry and everything after it (edit & resend).
    Truncate { from_entry_id: EntryId },
    /// The agent's available slash commands.
    Commands { commands: Vec<CommandInfo> },
    /// Result of reverting file changes for an edit.
    FilesRewound { ok: bool, detail: Option<String> },
    /// The orchestrator spawned a subagent (emitted on the parent session).
    SubagentStarted {
        tool_use_id: String,
        task_id: Option<String>,
        title: String,
        agent_type: Option<String>,
        prompt: Option<String>,
        background: bool,
    },
    SubagentProgress {
        tool_use_id: String,
        tokens: u64,
        tool_uses: u64,
        last_tool: Option<String>,
    },
    /// `status`: "completed", "failed", "killed" or "stopped".
    SubagentFinished { tool_use_id: String, status: String },
    /// Resolution of a permission/question entry (by entry id).
    Resolved {
        entry_id: EntryId,
        choice: Option<String>,
        answers: Option<Vec<Vec<String>>>,
    },
}

/// Full state of one session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub meta: SessionMeta,
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub models: Vec<ModelInfo>,
    #[serde(default)]
    pub modes: Vec<ModeInfo>,
    #[serde(default)]
    pub commands: Vec<CommandInfo>,
}

impl SessionState {
    pub fn new(meta: SessionMeta) -> Self {
        Self {
            meta,
            entries: Vec::new(),
            models: Vec::new(),
            modes: Vec::new(),
            commands: Vec::new(),
        }
    }

    /// An agent "title" that is just the first message again.
    fn echoes_first_prompt(&self, title: &str) -> bool {
        let t = title.trim().trim_end_matches('…').trim();
        let Some(Entry::User { text, .. }) = self.entries.iter().find(|e| matches!(e, Entry::User { .. })) else { return false };
        let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        !t.is_empty() && first.starts_with(t)
    }

    fn position(&self, id: &str) -> Option<usize> {
        // Recent entries are touched most; search from the back.
        self.entries.iter().rposition(|e| e.id() == id)
    }

    pub fn pending_inputs(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.is_pending_input())
    }

    /// Apply an event. Returns `true` when the transcript (not just metadata) changed.
    pub fn apply(&mut self, event: &SessionEvent) -> bool {
        self.meta.updated_at = Utc::now();
        match event {
            SessionEvent::Ready {
                agent_session_id,
                model,
                models,
                modes,
            } => {
                if agent_session_id.is_some() {
                    self.meta.agent_session_id = agent_session_id.clone();
                }
                if model.is_some() {
                    self.meta.model = model.clone();
                }
                if !models.is_empty() {
                    self.models = models.clone();
                }
                if !modes.is_empty() {
                    self.modes = modes.clone();
                }
                if self.meta.status == SessionStatus::Starting {
                    self.meta.status = SessionStatus::Idle;
                }
                false
            }
            SessionEvent::Upsert { entry } => {
                if let Some(i) = self.position(entry.id()) {
                    self.entries[i] = entry.clone();
                } else {
                    if let Entry::User { text, .. } = entry
                        && self.meta.title.starts_with("New ") {
                            self.meta.title = derive_title(text);
                        }
                    self.entries.push(entry.clone());
                }
                if entry.is_pending_input() {
                    self.meta.status = SessionStatus::NeedsInput;
                }
                true
            }
            SessionEvent::AppendText {
                entry_id,
                thinking,
                delta,
            } => {
                match self.position(entry_id).map(|i| &mut self.entries[i]) {
                    Some(Entry::Assistant { text, streaming, .. })
                    | Some(Entry::Thinking { text, streaming, .. }) => {
                        text.push_str(delta);
                        *streaming = true;
                    }
                    _ => {
                        let entry = if *thinking {
                            Entry::Thinking {
                                id: entry_id.clone(),
                                text: delta.clone(),
                                streaming: true,
                            }
                        } else {
                            Entry::Assistant {
                                id: entry_id.clone(),
                                text: delta.clone(),
                                streaming: true,
                            }
                        };
                        self.entries.push(entry);
                    }
                }
                true
            }
            SessionEvent::StreamEnd { entry_id } => {
                if let Some(i) = self.position(entry_id) {
                    match &mut self.entries[i] {
                        Entry::Assistant { streaming, .. } | Entry::Thinking { streaming, .. } => {
                            *streaming = false
                        }
                        _ => {}
                    }
                }
                true
            }
            SessionEvent::Status { status } => {
                // Never drop "needs input" while something is still pending.
                if *status == SessionStatus::Working && self.pending_inputs().next().is_some() {
                    self.meta.status = SessionStatus::NeedsInput;
                } else {
                    self.meta.status = status.clone();
                }
                if matches!(status, SessionStatus::Idle | SessionStatus::Ended | SessionStatus::Error(_)) {
                    for e in &mut self.entries {
                        if let Entry::Assistant { streaming, .. } | Entry::Thinking { streaming, .. } = e {
                            *streaming = false;
                        }
                    }
                }
                false
            }
            SessionEvent::Context { usage } => {
                self.meta.context = *usage;
                false
            }
            SessionEvent::Mode { mode } => {
                self.meta.permission_mode = *mode;
                false
            }
            SessionEvent::Model { model } => {
                self.meta.model = Some(model.clone());
                false
            }
            SessionEvent::Options { effort, context } => {
                self.meta.effort = effort.clone();
                self.meta.context_limit = context.clone();
                false
            }
            SessionEvent::Title { title } => {
                // Mothr's own title wins, and so does anything better than echoing the prompt.
                if !self.meta.titled && !self.echoes_first_prompt(title) {
                    self.meta.title = title.clone();
                }
                false
            }
            SessionEvent::RemoteControl { state } => {
                self.meta.remote_control = state.clone();
                false
            }
            SessionEvent::SubagentStarted { .. } | SessionEvent::SubagentProgress { .. } | SessionEvent::SubagentFinished { .. } => false,
            SessionEvent::UserRef { entry_id, agent_ref: r, prev_ref: pr } => {
                if let Some(i) = self.position(entry_id)
                    && let Entry::User { agent_ref, prev_ref, .. } = &mut self.entries[i] {
                        *agent_ref = Some(r.clone());
                        *prev_ref = pr.clone();
                    }
                false
            }
            SessionEvent::Truncate { from_entry_id } => {
                if let Some(i) = self.position(from_entry_id) {
                    self.entries.truncate(i);
                }
                true
            }
            SessionEvent::Commands { commands } => {
                self.commands = commands.clone();
                false
            }
            SessionEvent::FilesRewound { .. } => false,
            SessionEvent::Resolved {
                entry_id,
                choice,
                answers,
            } => {
                if let Some(i) = self.position(entry_id) {
                    match &mut self.entries[i] {
                        Entry::Permission(p) => p.resolved = choice.clone().or(Some("dismissed".into())),
                        Entry::Question(q) => q.answers = answers.clone().or(Some(vec![])),
                        _ => {}
                    }
                }
                if self.meta.status == SessionStatus::NeedsInput && self.pending_inputs().next().is_none() {
                    self.meta.status = SessionStatus::Working;
                }
                true
            }
        }
    }
}

/// A short title from the first user message.
pub fn derive_title(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let mut out: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        out.push('…');
    }
    if out.is_empty() { "Untitled".into() } else { out }
}

/// Commands flowing from a view (GPUI or remote) to a session driver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum SessionCommand {
    Prompt {
        text: String,
        #[serde(default)]
        attachments: Vec<Attachment>,
        #[serde(default)]
        delivery: Delivery,
    },
    Cancel,
    SetMode { mode: PermissionMode },
    SetModel { model: String },
    SetEffort { effort: String },
    SetContext { context: String },
    RespondPermission { entry_id: EntryId, option_id: String },
    AnswerQuestion { entry_id: EntryId, answers: Vec<Vec<String>> },
    SetRemoteControl { enabled: bool },
    /// Revert file changes made since this agent message id (Claude file checkpoints).
    RewindFiles { user_message_id: String },
    /// Stop one subagent of an orchestrator chat.
    StopSubagent { tool_use_id: String },
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> SessionState {
        SessionState::new(SessionMeta::new(AgentKind::ClaudeCode, SessionKind::Chat, "/tmp".into(), PermissionMode::Ask))
    }

    #[test]
    fn streaming_text_accumulates_and_finishes() {
        let mut s = state();
        for d in ["Hel", "lo"] {
            s.apply(&SessionEvent::AppendText { entry_id: "a".into(), thinking: false, delta: d.into() });
        }
        assert!(matches!(&s.entries[0], Entry::Assistant { text, streaming: true, .. } if text == "Hello"));
        s.apply(&SessionEvent::Status { status: SessionStatus::Idle });
        assert!(matches!(&s.entries[0], Entry::Assistant { streaming: false, .. }));
    }

    #[test]
    fn first_user_message_titles_the_session() {
        let mut s = state();
        s.apply(&SessionEvent::Upsert {
            entry: Entry::User { id: "u".into(), text: "\n  Fix the login bug\nmore".into(), attachments: vec![], at: chrono::Utc::now(), agent_ref: None, prev_ref: None },
        });
        assert_eq!(s.meta.title, "Fix the login bug");
    }

    #[test]
    fn agent_titles_that_echo_the_prompt_are_ignored() {
        let mut s = state();
        s.apply(&SessionEvent::Upsert {
            entry: Entry::User { id: "u".into(), text: "Fix the login bug please".into(), attachments: vec![], at: chrono::Utc::now(), agent_ref: None, prev_ref: None },
        });
        s.meta.title = "Login redirect loop".into();
        s.meta.titled = true;
        s.apply(&SessionEvent::Title { title: "Fix the login bug please".into() });
        assert_eq!(s.meta.title, "Login redirect loop");
        // Without a generated title, a real agent title is taken but an echo isn't.
        s.meta.titled = false;
        s.apply(&SessionEvent::Title { title: "Fix the login…".into() });
        assert_eq!(s.meta.title, "Login redirect loop");
        s.apply(&SessionEvent::Title { title: "Auth redirect fix".into() });
        assert_eq!(s.meta.title, "Auth redirect fix");
    }

    #[test]
    fn options_event_sets_effort_and_context() {
        let mut s = state();
        s.apply(&SessionEvent::Options { effort: Some("high".into()), context: Some("1m".into()) });
        assert_eq!((s.meta.effort.as_deref(), s.meta.context_limit.as_deref()), (Some("high"), Some("1m")));
    }

    #[test]
    fn pending_permission_holds_needs_input_until_resolved() {
        let mut s = state();
        s.apply(&SessionEvent::Upsert {
            entry: Entry::Permission(PermissionRequest {
                id: "p".into(),
                tool_call_id: None,
                title: "Run".into(),
                detail: None,
                kind: ToolKind::Execute,
                options: vec![],
                resolved: None,
                diffs: vec![],
            }),
        });
        assert_eq!(s.meta.status, SessionStatus::NeedsInput);
        // A "working" status must not hide the outstanding request.
        s.apply(&SessionEvent::Status { status: SessionStatus::Working });
        assert_eq!(s.meta.status, SessionStatus::NeedsInput);
        s.apply(&SessionEvent::Resolved { entry_id: "p".into(), choice: Some("allow".into()), answers: None });
        assert_eq!(s.meta.status, SessionStatus::Working);
        assert!(s.pending_inputs().next().is_none());
    }

    #[test]
    fn events_roundtrip_through_json() {
        let e = SessionEvent::AppendText { entry_id: "x".into(), thinking: true, delta: "hm".into() };
        let back: SessionEvent = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(e, back);
    }

    fn user(id: &str, text: &str) -> Entry {
        Entry::User { id: id.into(), text: text.into(), attachments: vec![], at: chrono::Utc::now(), agent_ref: None, prev_ref: None }
    }

    #[test]
    fn user_refs_and_truncate_support_edit() {
        let mut s = state();
        s.apply(&SessionEvent::Upsert { entry: user("u1", "one") });
        s.apply(&SessionEvent::AppendText { entry_id: "a1".into(), thinking: false, delta: "ok".into() });
        s.apply(&SessionEvent::Upsert { entry: user("u2", "two") });
        s.apply(&SessionEvent::AppendText { entry_id: "a2".into(), thinking: false, delta: "ok".into() });
        s.apply(&SessionEvent::UserRef { entry_id: "u2".into(), agent_ref: "uuid-2".into(), prev_ref: Some("uuid-a1".into()) });
        assert!(matches!(&s.entries[2], Entry::User { agent_ref: Some(r), prev_ref: Some(p), .. } if r == "uuid-2" && p == "uuid-a1"));
        s.apply(&SessionEvent::Truncate { from_entry_id: "u2".into() });
        assert_eq!(s.entries.len(), 2);
        assert_eq!(s.entries.last().map(|e| e.id()), Some("a1"));
    }

    #[test]
    fn image_entries_upsert_and_roundtrip() {
        let mut s = state();
        let img = Entry::Image { id: "i1".into(), path: "/tmp/x.png".into(), mime: Some("image/png".into()) };
        s.apply(&SessionEvent::Upsert { entry: img.clone() });
        s.apply(&SessionEvent::Upsert { entry: img.clone() });
        assert_eq!(s.entries, vec![img.clone()]);
        assert_eq!(img.id(), "i1");
        let back: Entry = serde_json::from_str(&serde_json::to_string(&img).unwrap()).unwrap();
        assert_eq!(back, img);
    }

    #[test]
    fn image_types_map_to_extensions() {
        assert_eq!(crate::image_ext("image/jpeg"), Some("jpg"));
        assert_eq!(crate::image_ext("image/png"), Some("png"));
        assert_eq!(crate::image_ext("application/pdf"), None);
    }

    #[test]
    fn commands_and_queue_roundtrip() {
        let mut s = state();
        s.apply(&SessionEvent::Commands { commands: vec![CommandInfo { name: "compact".into(), description: None, hint: None }] });
        assert_eq!(s.commands.len(), 1);
        s.meta.queue.push(QueuedPrompt { id: "q".into(), text: "later".into(), attachments: vec![] });
        let back: SessionState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.meta.queue.len(), 1);
        // Older transcripts without the new fields still load.
        let legacy = r#"{"type":"user","id":"x","text":"hi","at":"2026-01-01T00:00:00Z"}"#;
        assert!(serde_json::from_str::<Entry>(legacy).is_ok());
        let cmd: SessionCommand = serde_json::from_str(r#"{"command":"prompt","text":"hi"}"#).unwrap();
        assert!(matches!(cmd, SessionCommand::Prompt { delivery: Delivery::Normal, .. }));
    }
}
