//! UI-side mirror of the hub, kept in sync from the hub's broadcast stream.

use gpui_kit::*;
use shepherd_core::*;
use shepherd_hub::Hub;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

pub struct Store {
    pub hub: Arc<Hub>,
    pub sessions: Vec<SessionState>,
    pub agents: Vec<AgentInstall>,
    pub agents_scanned: bool,
    pub usage: BTreeMap<Provider, ProviderUsage>,
    pub pinned: Vec<ProjectRef>,
    pub recent: Vec<ProjectRef>,
    pub project_count: Option<usize>,
    /// Limit windows we already warned about (provider, label, level).
    warned: HashSet<(Provider, String, UsageLevel)>,
    _task: Task<()>,
}

#[derive(Debug, Clone)]
pub enum StoreEvent {
    Session(SessionId, SessionEvent),
    #[allow(dead_code)]
    Created(SessionId),
    Removed(SessionId),
    Changed,
    Toast(NoticeLevel, String),
}

impl EventEmitter<StoreEvent> for Store {}

impl Store {
    pub fn new(hub: Arc<Hub>, cx: &mut Context<Self>) -> Self {
        let (snap, mut rx) = hub.snapshot_and_subscribe();
        let task = cx.spawn(async move |this, cx| {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if this.update(cx, |s, cx| s.on_event(event, cx)).is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {
                        if this.update(cx, |s, cx| s.resync(cx)).is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        });
        let mut store = Self {
            hub,
            sessions: vec![],
            agents: vec![],
            agents_scanned: false,
            usage: BTreeMap::new(),
            pinned: vec![],
            recent: vec![],
            project_count: None,
            warned: HashSet::new(),
            _task: task,
        };
        store.apply_snapshot(snap);
        let n = store.hub.projects().len();
        if n > 0 {
            store.project_count = Some(n);
        }
        store
    }

    fn apply_snapshot(&mut self, snap: HubSnapshot) {
        self.sessions = snap.sessions;
        if !snap.agents.is_empty() {
            self.agents = snap.agents;
            self.agents_scanned = true;
        }
        self.usage = snap.usage.into_iter().map(|u| (u.provider, u)).collect();
        self.pinned = snap.pinned_projects;
        self.recent = snap.recent_projects;
    }

    fn resync(&mut self, cx: &mut Context<Self>) {
        let snap = self.hub.snapshot();
        self.apply_snapshot(snap);
        cx.emit(StoreEvent::Changed);
        cx.notify();
    }

    fn on_event(&mut self, event: HubEvent, cx: &mut Context<Self>) {
        match event {
            HubEvent::SessionCreated { state } => {
                let id = state.meta.id;
                self.sessions.retain(|s| s.meta.id != id);
                self.sessions.push(*state);
                cx.emit(StoreEvent::Created(id));
            }
            HubEvent::Session { id, event } => {
                if let Some(s) = self.sessions.iter_mut().find(|s| s.meta.id == id) {
                    s.apply(&event);
                }
                cx.emit(StoreEvent::Session(id, event));
            }
            HubEvent::SessionMeta { meta } => {
                if let Some(s) = self.sessions.iter_mut().find(|s| s.meta.id == meta.id) {
                    s.meta = meta;
                } else if !meta.archived {
                    // Un-archived: pull the full state.
                    if let Some(s) = self.hub_session(meta.id) {
                        self.sessions.push(s);
                    }
                }
                self.sessions.retain(|s| !s.meta.archived);
                cx.emit(StoreEvent::Changed);
            }
            HubEvent::SessionRemoved { id } => {
                self.sessions.retain(|s| s.meta.id != id);
                cx.emit(StoreEvent::Removed(id));
            }
            HubEvent::Usage { usage } => {
                let warn_at = self.hub.settings().warn_threshold;
                for w in &usage.windows {
                    let level = w.level();
                    if w.used_fraction >= warn_at && self.warned.insert((usage.provider, w.label.clone(), level)) {
                        let reset = w.resets_at.map(|r| format!(" — resets {}", format_reset(r))).unwrap_or_default();
                        cx.emit(StoreEvent::Toast(
                            if level >= UsageLevel::Critical { NoticeLevel::Error } else { NoticeLevel::Warning },
                            format!(
                                "{} {} limit at {:.0}%{}",
                                usage.provider.display_name(),
                                w.label,
                                w.used_fraction * 100.0,
                                reset
                            ),
                        ));
                    }
                }
                self.usage.insert(usage.provider, usage);
            }
            HubEvent::Agents { agents } => {
                self.agents = agents;
                self.agents_scanned = true;
            }
            HubEvent::Projects { pinned, recent } => {
                self.pinned = pinned;
                self.recent = recent;
            }
            HubEvent::ProjectsIndexed { count } => self.project_count = Some(count),
            HubEvent::Toast { level, text } => cx.emit(StoreEvent::Toast(level, text)),
        }
        cx.notify();
    }

    fn hub_session(&self, id: SessionId) -> Option<SessionState> {
        self.hub.session(id)
    }

    pub fn session(&self, id: SessionId) -> Option<&SessionState> {
        self.sessions.iter().find(|s| s.meta.id == id)
    }

    /// Subagent sessions spawned by an orchestrator chat, oldest first.
    pub fn children(&self, id: SessionId) -> Vec<&SessionState> {
        self.sessions.iter().filter(|s| s.meta.parent == Some(id)).collect()
    }

    /// (running, total, needing input) subagents of a chat.
    pub fn subagent_counts(&self, id: SessionId) -> (usize, usize, usize) {
        let kids = self.children(id);
        let running = kids.iter().filter(|s| s.meta.status.is_live()).count();
        let needs = kids.iter().filter(|s| s.meta.status == SessionStatus::NeedsInput).count();
        (running, kids.len(), needs)
    }

    /// Models an agent reported in the most recent chat that listed any.
    pub fn known_models(&self, agent: AgentKind) -> Vec<ModelInfo> {
        self.sessions
            .iter()
            .rev()
            .find(|s| s.meta.agent == agent && !s.models.is_empty())
            .map(|s| s.models.clone())
            .unwrap_or_default()
    }

    /// Slash commands an agent reported in its most recent chat.
    pub fn known_commands(&self, agent: AgentKind) -> Vec<CommandInfo> {
        self.sessions
            .iter()
            .rev()
            .find(|s| s.meta.agent == agent && !s.commands.is_empty())
            .map(|s| s.commands.clone())
            .unwrap_or_default()
    }

    /// Persist the agent/model the user just chose, if "remember" is on.
    pub fn record_selection(&self, agent: AgentKind, model: Option<&str>) {
        let mut s = self.hub.settings();
        if s.record_selection(agent, model) {
            self.hub.update_settings(s);
        }
    }

    pub fn available_agents(&self) -> Vec<AgentKind> {
        self.agents.iter().filter(|a| a.available()).map(|a| a.kind).collect()
    }

    /// Send a command to the hub; surface failures as a notification.
    pub fn dispatch(&self, command: AppCommand, window: &mut Window, cx: &mut App) -> Option<SessionId> {
        match self.hub.dispatch(command) {
            Ok(id) => id,
            Err(e) => {
                use gpui_kit::component::WindowExt as _;
                window.push_notification(gpui_kit::component::notification::Notification::error(e.to_string()), cx);
                None
            }
        }
    }
}
