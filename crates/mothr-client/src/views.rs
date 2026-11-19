//! Flat, UI-ready records exported to Kotlin/Swift. Everything here is plain data.

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentRef {
    /// Stable id: "claude_code", "codex", "copilot", "open_code", "hermes".
    pub id: String,
    pub name: String,
    pub short_name: String,
    /// Brand tint as 0xRRGGBB.
    pub color: u32,
    pub monogram: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ChatStatus {
    Starting,
    Ready,
    Working,
    NeedsYou,
    Error,
    Ended,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ChatSummary {
    pub id: String,
    pub title: String,
    pub agent: AgentRef,
    pub project_name: String,
    pub project_path: String,
    pub general: bool,
    pub design: bool,
    pub status: ChatStatus,
    pub status_label: String,
    pub error: Option<String>,
    pub unread: bool,
    pub pinned: bool,
    pub updated_ms: i64,
    /// "now", "5m", "3h", "2d", "Nov 3".
    pub updated_label: String,
    /// Latest meaningful line (or what the agent is waiting on).
    pub preview: String,
    pub subagents_running: u32,
    pub subagents_total: u32,
    pub queued: u32,
    /// Set on subagent chats.
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ChatGroup {
    pub title: String,
    pub chats: Vec<ChatSummary>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ModeOption {
    /// "ask", "accept_edits", "auto", "plan", "allow_all".
    pub id: String,
    pub label: String,
    pub description: String,
    pub dangerous: bool,
}

/// One value of a setting: a reasoning effort or a context window size.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ChoiceOption {
    /// What to send back ("high", "1m", "long_context").
    pub id: String,
    /// What to show ("High", "1M").
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ModelOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Reasoning levels this model accepts; empty when it has no such setting.
    pub efforts: Vec<ChoiceOption>,
    /// Context window sizes this model can run with; empty when it has one fixed size.
    pub contexts: Vec<ChoiceOption>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CommandView {
    pub name: String,
    pub description: Option<String>,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct QueuedView {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ToolState {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PlanItemView {
    pub content: String,
    /// "pending", "in_progress", "completed".
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PermissionChoice {
    pub id: String,
    pub label: String,
    pub allow: bool,
    pub always: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct QuestionChoice {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct QuestionView {
    pub header: Option<String>,
    pub question: String,
    pub options: Vec<QuestionChoice>,
    pub multi_select: bool,
}

/// One transcript item.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum EntryView {
    User { id: String, text: String, at_ms: i64 },
    Assistant { id: String, text: String, streaming: bool },
    Thinking { id: String, text: String, streaming: bool },
    Tool {
        id: String,
        title: String,
        /// "read", "edit", "execute", "search", "fetch", "task", …
        kind: String,
        status: ToolState,
        input: Option<String>,
        output: Option<String>,
        added: u32,
        removed: u32,
        /// Unified-style changed lines ("+…"/"-…"), capped.
        diff: Option<String>,
    },
    Plan { id: String, items: Vec<PlanItemView> },
    Permission {
        id: String,
        title: String,
        detail: Option<String>,
        options: Vec<PermissionChoice>,
        /// Label of the chosen option once resolved.
        resolved: Option<String>,
    },
    Question {
        id: String,
        questions: Vec<QuestionView>,
        answers: Option<Vec<Vec<String>>>,
    },
    Notice { id: String, level: String, text: String },
    TurnEnd { id: String, summary: String },
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ChatDetail {
    pub summary: ChatSummary,
    pub mode: ModeOption,
    pub modes: Vec<ModeOption>,
    pub model: Option<String>,
    pub models: Vec<ModelOption>,
    /// The reasoning effort the chat runs with, a [`ChoiceOption::id`]; None = the agent's default.
    pub effort: Option<String>,
    /// The context window the chat runs with, a [`ChoiceOption::id`]; None = the agent's default.
    pub context_limit: Option<String>,
    pub commands: Vec<CommandView>,
    pub queue: Vec<QueuedView>,
    pub entries: Vec<EntryView>,
    pub context_fraction: f32,
    /// "61.0k / 200.0k", empty when unknown.
    pub context_label: String,
    pub cost_label: Option<String>,
    /// Credits used so far ("6.2 AIC"), for agents that bill in them (GitHub Copilot).
    pub credits_label: Option<String>,
    /// Working, starting or waiting on the user: new messages queue.
    pub busy: bool,
    /// The agent process is running (otherwise the chat can be resumed).
    pub live: bool,
    pub can_steer: bool,
    pub subagents: Vec<ChatSummary>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentView {
    pub agent: AgentRef,
    pub available: bool,
    pub version: Option<String>,
    pub note: Option<String>,
    pub modes: Vec<ModeOption>,
    /// Models new chats can start with, as last reported by the desktop.
    pub models: Vec<ModelOption>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ProjectView {
    pub path: String,
    pub name: String,
    pub display_path: String,
    pub pinned: bool,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct WindowView {
    /// "5h", "Weekly", "AIC" …
    pub label: String,
    pub fraction: f32,
    /// "ok", "warn", "critical", "exhausted".
    pub level: String,
    /// "in 2h 14m".
    pub resets: Option<String>,
    /// "212 / 300" when absolute numbers are known.
    pub amount: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct UsageView {
    pub agent: AgentRef,
    pub provider_name: String,
    pub plan: Option<String>,
    pub error: Option<String>,
    pub worst_fraction: f32,
    pub level: String,
    pub updated_label: String,
    pub windows: Vec<WindowView>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Toast {
    pub level: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConnectionState {
    Idle,
    Connecting,
    Connected,
    /// Lost the connection; retrying automatically.
    Reconnecting,
    /// The server rejected the pairing token. Re-pair to continue.
    Unauthorized,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ConnectionInfo {
    pub state: ConnectionState,
    /// The desktop's address (without the token).
    pub server: String,
    /// The desktop's own name, once it has told us.
    pub computer: Option<String>,
    pub message: Option<String>,
    /// The first snapshot has arrived.
    pub loaded: bool,
    pub needs_you: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Delivery {
    /// Send now if idle, otherwise queue until the turn ends.
    Normal,
    /// Inject into the running turn (Claude Code).
    Steer,
    /// Stop the current turn and send immediately.
    Interrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AttentionKind {
    /// Answer with [`crate::MothrClient::respond_permission`] (reply id = option id).
    Permission,
    /// Answer with [`crate::MothrClient::answer_question`] (reply id = option label).
    Question,
}

/// A reply that can be sent straight from a notification.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct QuickReply {
    pub id: String,
    pub label: String,
    /// The approving choice (shown first).
    pub allow: bool,
}

/// Something an agent is blocked on until a person acts: a permission request or a question.
/// The phone turns each one into a notification.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Attention {
    /// Stable for the lifetime of the request ("<chat id>/<entry id>"). Use it to dedupe and to
    /// withdraw the notification once the request is answered anywhere.
    pub key: String,
    pub chat_id: String,
    pub entry_id: String,
    /// Chats to open, outermost first (an orchestrator, then its subagent).
    pub open_path: Vec<String>,
    pub kind: AttentionKind,
    pub agent: AgentRef,
    pub chat_title: String,
    pub project_name: String,
    /// "Claude Code needs permission", "Codex has a question".
    pub title: String,
    /// What it's asking.
    pub body: String,
    /// Replies safe to send without opening the chat: one-time allow/deny, or the options of a
    /// single, single-choice question with at most three options. Empty: open the chat.
    pub quick_replies: Vec<QuickReply>,
    pub updated_ms: i64,
}
