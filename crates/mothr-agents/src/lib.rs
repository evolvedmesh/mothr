//! Agent detection and session drivers.
//!
//! A driver owns one agent process and translates between its wire protocol and Mothr's
//! [`SessionEvent`]/[`SessionCommand`] model. Two protocols cover every first-party agent:
//! - [`claude`]: Claude Code's native stream-json control protocol (no Node needed, exposes
//!   rate limits, permission modes and Remote Control).
//! - [`acp`]: the Agent Client Protocol (Copilot, Codex via codex-acp, OpenCode, Hermes).

pub mod acp;
mod ask_mcp;
pub mod claude;
mod edits;
pub mod detect;
mod policy;

use mothr_core::{
    AgentKind, PermissionMode, ProviderUsage, SessionCommand, SessionEvent, SessionId, SessionKind, Transport,
};
use std::path::PathBuf;
use tokio::sync::mpsc;

pub use detect::{detect, detect_all};

/// Everything needed to start (or resume) an agent session.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub id: SessionId,
    pub agent: AgentKind,
    pub kind: SessionKind,
    pub cwd: PathBuf,
    pub mode: PermissionMode,
    pub model: Option<String>,
    /// Reasoning effort ("low" … "max"); None = the agent's default.
    pub effort: Option<String>,
    /// Context window choice (a `ChoiceInfo::id`); None = the agent's default.
    pub context: Option<String>,
    /// Agent-side session id to resume, if any.
    pub resume: Option<String>,
    /// Resume only up to this agent message (truncating resume, used by edit & resend).
    pub resume_at: Option<String>,
    /// Extra instructions (used by Design mode).
    pub system_prompt: Option<String>,
    pub title: String,
}

/// Output of a driver.
#[derive(Debug, Clone)]
pub enum DriverOutput {
    Event(SessionId, SessionEvent),
    /// An event for a subagent of `SessionId`, keyed by the spawning tool call id.
    SubEvent(SessionId, String, SessionEvent),
    Usage(ProviderUsage),
    Exited(SessionId, Option<String>),
}

pub type OutputTx = mpsc::UnboundedSender<DriverOutput>;
pub type CommandTx = mpsc::UnboundedSender<SessionCommand>;

/// Spawn a driver task on the current tokio runtime. Returns the command channel.
pub fn spawn(transport: &Transport, spec: SessionSpec, out: OutputTx) -> anyhow::Result<CommandTx> {
    let (tx, rx) = mpsc::unbounded_channel();
    match transport.clone() {
        Transport::ClaudeStreamJson { program } => {
            tokio::spawn(claude::run(program, spec, rx, out));
        }
        Transport::Acp { program, args, env } => {
            tokio::spawn(acp::run(program, args, env, spec, rx, out));
        }
    }
    Ok(tx)
}

/// Helper owned by drivers for emitting events.
#[derive(Clone)]
pub(crate) struct Emitter {
    id: SessionId,
    out: OutputTx,
}

impl Emitter {
    pub fn new(id: SessionId, out: OutputTx) -> Self {
        Self { id, out }
    }
    pub fn emit(&self, e: SessionEvent) {
        let _ = self.out.send(DriverOutput::Event(self.id, e));
    }
    pub fn sub(&self, tool_use_id: String, e: SessionEvent) {
        let _ = self.out.send(DriverOutput::SubEvent(self.id, tool_use_id, e));
    }
    pub fn usage(&self, u: ProviderUsage) {
        let _ = self.out.send(DriverOutput::Usage(u));
    }
    pub fn exited(&self, err: Option<String>) {
        let _ = self.out.send(DriverOutput::Exited(self.id, err));
    }
    pub fn notice(&self, level: mothr_core::NoticeLevel, text: impl Into<String>) {
        self.emit(SessionEvent::Upsert {
            entry: mothr_core::Entry::Notice {
                id: mothr_core::new_entry_id(),
                level,
                text: text.into(),
            },
        });
    }
}

/// Short single-line preview of a JSON tool input.
pub(crate) fn summarize_input(name: &str, input: &serde_json::Value) -> (String, Option<String>) {
    let s = |k: &str| input.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let n = name.to_ascii_lowercase();
    let is_agent = matches!(n.as_str(), "task" | "agent");
    let detail = if is_agent { s("subagent_type") } else { None }
        .or_else(|| s("command"))
        .or_else(|| s("file_path"))
        .or_else(|| s("path"))
        .or_else(|| s("notebook_path"))
        .or_else(|| s("pattern").map(|p| match s("path") { Some(d) => format!("{p}  in {d}"), None => p }))
        .or_else(|| s("url"))
        .or_else(|| s("query"))
        .or_else(|| s("description"))
        .or_else(|| s("prompt").map(|p| p.chars().take(200).collect()));
    let title = match (n.as_str(), &detail) {
        ("bash", Some(_)) => "Run command".to_string(),
        ("read", Some(d)) => format!("Read {}", short_path(d)),
        ("write", Some(d)) => format!("Write {}", short_path(d)),
        ("edit" | "multiedit", Some(d)) => format!("Edit {}", short_path(d)),
        ("grep", Some(_)) => "Search files".to_string(),
        ("glob", Some(_)) => "Find files".to_string(),
        ("webfetch", Some(_)) => "Fetch".to_string(),
        ("websearch", Some(_)) => "Web search".to_string(),
        ("task" | "agent", _) => match s("description") {
            Some(d) => format!("Subagent · {d}"),
            None => "Subagent".to_string(),
        },
        ("todowrite", _) => "Update plan".to_string(),
        _ => name.to_string(),
    };
    (title, detail)
}

fn short_path(p: &str) -> String {
    let path = std::path::Path::new(p);
    path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
}
