//! The hub: Mothr's single source of truth.
//!
//! It owns every session (live or historical), spawns agent drivers, persists transcripts,
//! polls account usage, detects agents, indexes projects, and broadcasts [`HubEvent`]s to the
//! desktop UI and the remote-control server through the [`HubApi`] trait.

use parking_lot::RwLock;
use mothr_agents::{CommandTx, DriverOutput, SessionSpec};
use mothr_core::*;
use mothr_projects::ProjectIndex;
use mothr_usage::UsageFetcher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc};

mod skills;

pub use mothr_skills::{POPULAR as POPULAR_SKILLS, SkillCatalog};

#[derive(Default)]
struct State {
    /// Insertion-ordered sessions.
    sessions: Vec<SessionState>,
    drivers: HashMap<SessionId, CommandTx>,
    agents: Vec<AgentInstall>,
    usage: BTreeMap<Provider, ProviderUsage>,
    dirty: HashSet<SessionId>,
    /// (orchestrator, spawning tool id) → subagent session.
    subagents: HashMap<(SessionId, String), SessionId>,
    /// Sessions whose file rewind finished (edit & resend handshake).
    rewound: HashSet<SessionId>,
    /// Last known model list per agent (persisted to `models.json`).
    models: BTreeMap<AgentKind, Vec<ModelInfo>>,
    /// In order, never empty (persisted to `workspaces.json`).
    workspaces: Vec<Workspace>,
    /// The workspace the desktop shows; new chats without one go there.
    active_workspace: Option<WorkspaceId>,
    /// Mothr's skill library, offered to every agent.
    skills: Vec<Skill>,
    /// Skill installs running now.
    skills_installing: Vec<SkillSource>,
}

impl State {
    fn session_mut(&mut self, id: SessionId) -> Option<&mut SessionState> {
        self.sessions.iter_mut().find(|s| s.meta.id == id)
    }
    fn session(&self, id: SessionId) -> Option<&SessionState> {
        self.sessions.iter().find(|s| s.meta.id == id)
    }

    /// The workspace new chats go to when none is named: the active one, else the first.
    fn default_workspace(&self) -> Option<WorkspaceId> {
        self.active_workspace.filter(|a| self.workspaces.iter().any(|w| w.id == *a)).or_else(|| self.workspaces.first().map(|w| w.id))
    }
}

pub struct Hub {
    me: std::sync::Weak<Hub>,
    rt: Handle,
    state: RwLock<State>,
    events: broadcast::Sender<HubEvent>,
    driver_tx: mpsc::UnboundedSender<DriverOutput>,
    projects: Arc<ProjectIndex>,
    usage: UsageFetcher,
    settings: RwLock<Settings>,
    skills: mothr_skills::Library,
    /// The terminals beside chats, shared by the desktop and remote clients.
    terminals: Arc<mothr_terminal::Terminals>,
    /// Terminals with a new screen to send, and the wake-up for the task that sends them.
    terminals_dirty: Arc<(parking_lot::Mutex<HashSet<TerminalId>>, tokio::sync::Notify)>,
}

impl Hub {
    /// Create the hub and start its background tasks on `rt`.
    pub fn start(rt: Handle, settings: Settings) -> Arc<Hub> {
        let (events, _) = broadcast::channel(4096);
        let (driver_tx, driver_rx) = mpsc::unbounded_channel();
        let terminals_dirty: Arc<(parking_lot::Mutex<HashSet<TerminalId>>, tokio::sync::Notify)> = Arc::default();
        let dirty = terminals_dirty.clone();
        let terminals = Arc::new(mothr_terminal::Terminals::new(move |id| {
            dirty.0.lock().insert(id);
            dirty.1.notify_one();
        }));
        let hub = Arc::new_cyclic(|me| Hub {
            me: me.clone(),
            rt: rt.clone(),
            state: RwLock::new(State::default()),
            events,
            driver_tx,
            projects: Arc::new(ProjectIndex::new()),
            usage: UsageFetcher::new(),
            settings: RwLock::new(settings),
            skills: mothr_skills::Library::default(),
            terminals,
            terminals_dirty,
        });
        hub.load_sessions();
        hub.load_models();
        hub.load_workspaces();
        hub.state.write().skills = hub.skills.load();
        let h = hub.clone();
        rt.spawn(async move { h.pump(driver_rx).await });
        let h = hub.clone();
        rt.spawn(async move { h.persist_loop().await });
        let h = hub.clone();
        rt.spawn(async move { h.usage_loop().await });
        let h = hub.clone();
        rt.spawn(async move { h.terminal_loop().await });
        hub.rescan_agents();
        hub.rescan_projects();
        hub
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().clone()
    }

    /// The terminals beside chats (the desktop paints them from here).
    pub fn terminals(&self) -> &Arc<mothr_terminal::Terminals> {
        &self.terminals
    }

    /// File a new-chat draft's terminal under the chat it became.
    pub fn rekey_terminal(&self, draft: TerminalId, chat: SessionId) {
        if self.terminals.rekey(draft, chat).is_some() {
            self.broadcast(HubEvent::TerminalClosed { id: draft });
        }
    }

    /// Send the screens of terminals that changed, at most every 80 ms each: a busy shell
    /// would otherwise flood remote clients.
    async fn terminal_loop(self: Arc<Self>) {
        loop {
            self.terminals_dirty.1.notified().await;
            tokio::time::sleep(Duration::from_millis(40)).await;
            let ids: Vec<TerminalId> = self.terminals_dirty.0.lock().drain().collect();
            for id in ids {
                if let Some(t) = self.terminals.get(id) {
                    self.broadcast(HubEvent::Terminal { screen: Box::new(t.screen()) });
                }
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }

    /// Where a chat's terminal starts: its primary repo (home for a general chat).
    fn terminal_cwd(&self, id: TerminalId, cwd: Option<PathBuf>) -> PathBuf {
        cwd.filter(|c| c.is_dir())
            .or_else(|| self.state.read().session(id).map(|s| s.meta.project.clone()).filter(|p| p.is_dir()))
            .unwrap_or_else(home_dir)
    }

    /// Replace settings, persist them, and re-run detection if agent overrides changed.
    pub fn update_settings(self: &Arc<Self>, new: Settings) {
        let old = std::mem::replace(&mut *self.settings.write(), new.clone());
        if let Err(e) = new.save() {
            tracing::warn!("saving settings failed: {e}");
        }
        if old.agents != new.agents {
            self.rescan_agents();
        }
        if old.project_roots != new.project_roots || old.scan_depth != new.scan_depth {
            self.rescan_projects();
        }
    }

    pub fn projects(&self) -> &Arc<ProjectIndex> {
        &self.projects
    }

    fn broadcast(&self, e: HubEvent) {
        let _ = self.events.send(e);
    }

    // ------------------------------------------------------------------ discovery

    pub fn rescan_agents(self: &Arc<Self>) {
        let h = self.clone();
        self.rt.spawn(async move {
            let overrides = h.settings.read().agents.clone();
            let agents = tokio::task::spawn_blocking(move || mothr_agents::detect_all(&overrides))
                .await
                .unwrap_or_default();
            h.state.write().agents = agents.clone();
            h.broadcast(HubEvent::Agents { agents: agents.clone() });
            h.probe_models(agents);
        });
    }

    pub fn rescan_projects(self: &Arc<Self>) {
        let h = self.clone();
        self.rt.spawn(async move {
            let (roots, depth) = {
                let s = h.settings.read();
                (s.project_roots.clone(), s.scan_depth)
            };
            let idx = h.projects.clone();
            let count = tokio::task::spawn_blocking(move || idx.scan(&roots, depth)).await.unwrap_or(0);
            h.broadcast(HubEvent::ProjectsIndexed { count });
            h.broadcast_projects();
        });
    }

    fn broadcast_projects(&self) {
        self.broadcast(HubEvent::Projects { pinned: self.projects.pinned(), recent: self.projects.recent(12) });
    }

    async fn usage_loop(self: Arc<Self>) {
        loop {
            self.refresh_usage().await;
            let secs = self.settings.read().usage_poll_secs.max(30);
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
    }

    async fn refresh_usage(&self) {
        for u in self.usage.fetch_all().await {
            self.merge_usage(u);
        }
    }

    fn merge_usage(&self, u: ProviderUsage) {
        let merged = {
            let mut st = self.state.write();
            let entry = st.usage.entry(u.provider).or_insert_with(|| ProviderUsage::new(u.provider, u.source.clone()));
            entry.merge(u);
            entry.clone()
        };
        self.broadcast(HubEvent::Usage { usage: merged });
    }

    // ------------------------------------------------------------------ model catalog

    fn load_models(&self) {
        let loaded: BTreeMap<AgentKind, Vec<ModelInfo>> = std::fs::read_to_string(models_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        self.state.write().models = loaded;
    }

    /// Remember an agent's model list (persisted, broadcast) when it changed.
    fn cache_models(&self, st: &mut State, agent: AgentKind, models: Vec<ModelInfo>) {
        if st.models.get(&agent) == Some(&models) {
            return;
        }
        st.models.insert(agent, models.clone());
        let path = models_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_vec_pretty(&st.models) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(&path, bytes) {
                    tracing::warn!("saving {}: {e}", path.display());
                }
            }
            Err(e) => tracing::warn!("encoding models: {e}"),
        }
        self.broadcast(HubEvent::AgentModels { agent, models });
    }

    /// Start each detected agent briefly (no prompt) to learn its model list, so the model
    /// picker is populated before the user's first chat. Skips agents already cached.
    fn probe_models(self: &Arc<Self>, agents: Vec<AgentInstall>) {
        for install in agents {
            let Some(transport) = install.transport.clone() else { continue };
            let agent = install.kind;
            let cached_ok = self.state.read().models.get(&agent).is_some_and(|ms| {
                // Lists cached before reasoning/context options existed are learned again.
                !matches!(agent, AgentKind::ClaudeCode | AgentKind::Copilot) || ms.iter().any(|m| !m.efforts.is_empty() || !m.contexts.is_empty())
            });
            if cached_ok {
                continue;
            }
            let h = self.clone();
            self.rt.spawn(async move {
                let id = SessionId::new_v4();
                let spec = SessionSpec {
                    id,
                    agent,
                    kind: SessionKind::Chat,
                    cwd: std::env::temp_dir(),
                    mode: PermissionMode::Ask.for_agent(agent),
                    model: None,
                    effort: None,
                    context: None,
                    resume: None,
                    resume_at: None,
                    replay: false,
                    system_prompt: None,
                    extra_dirs: vec![],
                    title: "model probe".into(),
                    skills: vec![],
                    skills_note: None,
                };
                let (tx, mut rx) = mpsc::unbounded_channel();
                let Ok(cmd) = mothr_agents::spawn(&transport, spec, tx) else { return };
                let found = tokio::time::timeout(Duration::from_secs(45), async {
                    while let Some(out) = rx.recv().await {
                        match out {
                            DriverOutput::Event(_, SessionEvent::Ready { models, .. }) if !models.is_empty() => return Some(models),
                            DriverOutput::Exited(..) => return None,
                            _ => {}
                        }
                    }
                    None
                })
                .await
                .ok()
                .flatten();
                let _ = cmd.send(SessionCommand::Shutdown);
                if let Some(models) = found {
                    let mut st = h.state.write();
                    h.cache_models(&mut st, agent, models);
                }
            });
        }
    }

    // ------------------------------------------------------------------ workspaces

    /// Read the workspaces, make sure there is one, and put every chat in one.
    fn load_workspaces(&self) {
        let saved: SavedWorkspaces = std::fs::read_to_string(workspaces_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let mut st = self.state.write();
        st.workspaces = saved.workspaces;
        st.workspaces.truncate(MAX_WORKSPACES);
        let fresh = st.workspaces.is_empty();
        if fresh {
            st.workspaces.push(Workspace::new(DEFAULT_WORKSPACE_NAME));
        }
        st.active_workspace = saved.active;
        let fallback = st.default_workspace();
        let known: HashSet<WorkspaceId> = st.workspaces.iter().map(|w| w.id).collect();
        // Chats from before workspaces (or from one that's gone) join the default one.
        let mut moved = vec![];
        for s in &mut st.sessions {
            if !s.meta.workspace.is_some_and(|w| known.contains(&w)) {
                s.meta.workspace = fallback;
                moved.push(s.meta.id);
            }
        }
        st.dirty.extend(moved);
        if fresh {
            self.save_workspaces(&st);
        }
    }

    fn save_workspaces(&self, st: &State) {
        let saved = SavedWorkspaces { workspaces: st.workspaces.clone(), active: st.active_workspace };
        let path = workspaces_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_vec_pretty(&saved) {
            Ok(bytes) => {
                if let Err(e) = write_atomic(&path, &bytes) {
                    tracing::warn!("saving {}: {e}", path.display());
                }
            }
            Err(e) => tracing::warn!("encoding workspaces: {e}"),
        }
    }

    /// Change the workspaces under the lock, then save and broadcast them.
    fn edit_workspaces(&self, f: impl FnOnce(&mut State) -> anyhow::Result<()>) -> anyhow::Result<()> {
        let event = {
            let mut st = self.state.write();
            f(&mut st)?;
            self.save_workspaces(&st);
            HubEvent::Workspaces { workspaces: st.workspaces.clone(), active: st.default_workspace() }
        };
        self.broadcast(event);
        Ok(())
    }

    fn create_workspace(&self, mut workspace: Workspace) -> anyhow::Result<()> {
        self.edit_workspaces(|st| {
            anyhow::ensure!(st.workspaces.len() < MAX_WORKSPACES, "You can have up to {MAX_WORKSPACES} workspaces");
            anyhow::ensure!(!st.workspaces.iter().any(|w| w.id == workspace.id), "That workspace already exists");
            workspace.name = clean_workspace_name(&workspace.name).unwrap_or_else(|| next_workspace_name(&st.workspaces));
            st.workspaces.push(workspace);
            Ok(())
        })
    }

    fn update_workspace(&self, workspace: Workspace) -> anyhow::Result<()> {
        let name = clean_workspace_name(&workspace.name).ok_or_else(|| anyhow::anyhow!("A workspace needs a name"))?;
        self.edit_workspaces(|st| {
            let w = st.workspaces.iter_mut().find(|w| w.id == workspace.id).ok_or_else(|| anyhow::anyhow!("no such workspace"))?;
            w.name = name;
            w.icon = workspace.icon.filter(|i| WORKSPACE_ICONS.contains(&i.as_str()));
            Ok(())
        })
    }

    fn delete_workspace(&self, id: WorkspaceId) -> anyhow::Result<()> {
        let mut moved = vec![];
        self.edit_workspaces(|st| {
            anyhow::ensure!(st.workspaces.len() > 1, "Mothr needs at least one workspace");
            let before = st.workspaces.len();
            st.workspaces.retain(|w| w.id != id);
            anyhow::ensure!(st.workspaces.len() < before, "no such workspace");
            if st.active_workspace == Some(id) {
                st.active_workspace = None;
            }
            let to = st.default_workspace();
            for s in &mut st.sessions {
                if s.meta.workspace == Some(id) {
                    s.meta.workspace = to;
                    moved.push(s.meta.clone());
                }
            }
            st.dirty.extend(moved.iter().map(|m| m.id));
            Ok(())
        })?;
        for meta in moved {
            self.broadcast(HubEvent::SessionMeta { meta });
        }
        Ok(())
    }

    fn move_workspace(&self, id: WorkspaceId, index: usize) -> anyhow::Result<()> {
        self.edit_workspaces(|st| {
            let from = st.workspaces.iter().position(|w| w.id == id).ok_or_else(|| anyhow::anyhow!("no such workspace"))?;
            let w = st.workspaces.remove(from);
            let to = index.min(st.workspaces.len());
            st.workspaces.insert(to, w);
            Ok(())
        })
    }

    fn select_workspace(&self, id: WorkspaceId) -> anyhow::Result<()> {
        if self.state.read().active_workspace == Some(id) {
            return Ok(());
        }
        self.edit_workspaces(|st| {
            anyhow::ensure!(st.workspaces.iter().any(|w| w.id == id), "no such workspace");
            st.active_workspace = Some(id);
            Ok(())
        })
    }

    /// Move a chat and its subagents to another workspace.
    fn set_workspace(&self, id: SessionId, workspace: WorkspaceId) -> anyhow::Result<()> {
        let moved: Vec<SessionMeta> = {
            let mut st = self.state.write();
            anyhow::ensure!(st.workspaces.iter().any(|w| w.id == workspace), "no such workspace");
            let mut out = vec![];
            for s in &mut st.sessions {
                if (s.meta.id == id || s.meta.parent == Some(id)) && s.meta.workspace != Some(workspace) {
                    s.meta.workspace = Some(workspace);
                    out.push(s.meta.clone());
                }
            }
            st.dirty.extend(out.iter().map(|m| m.id));
            out
        };
        for meta in moved {
            self.broadcast(HubEvent::SessionMeta { meta });
        }
        Ok(())
    }

    // ------------------------------------------------------------------ persistence

    fn load_sessions(&self) {
        let dir = sessions_dir();
        let Ok(read) = std::fs::read_dir(&dir) else { return };
        let mut sessions: Vec<SessionState> = read
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|t| serde_json::from_str::<SessionState>(&t).ok())
            .collect();
        for s in &mut sessions {
            if s.meta.status.is_live() {
                s.meta.status = SessionStatus::Ended;
            }
            s.meta.remote_control = RemoteControlState::default();
            // Older versions counted tokens billed across a whole turn as context; that can't
            // exceed the window, so it's unknown until the agent next reports it.
            let c = &mut s.meta.context;
            if c.window_tokens > 0 && c.used_tokens > c.window_tokens {
                c.used_tokens = 0;
            }
            // Dangling questions can't be answered after a restart.
            for e in &mut s.entries {
                match e {
                    Entry::Permission(p) if p.resolved.is_none() => p.resolved = Some("expired".into()),
                    Entry::Question(q) if q.answers.is_none() => q.answers = Some(vec![]),
                    Entry::Assistant { streaming, .. } | Entry::Thinking { streaming, .. } => *streaming = false,
                    // Tools still "running" when Mothr exited were interrupted.
                    Entry::Tool(t) if matches!(t.status, ToolStatus::Pending | ToolStatus::Running) => {
                        t.status = ToolStatus::Failed;
                        if t.output.is_none() {
                            t.output = Some("Interrupted (Mothr was closed)".into());
                        }
                    }
                    _ => {}
                }
            }
        }
        sessions.sort_by_key(|s| s.meta.created_at);
        let mut st = self.state.write();
        for s in &sessions {
            if let (Some(parent), Some(info)) = (s.meta.parent, &s.meta.subagent) {
                st.subagents.insert((parent, info.tool_use_id.clone()), s.meta.id);
            }
        }
        st.sessions = sessions;
    }

    async fn persist_loop(self: Arc<Self>) {
        let dir = sessions_dir();
        let _ = std::fs::create_dir_all(&dir);
        loop {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            self.flush(&dir);
        }
    }

    fn flush(&self, dir: &std::path::Path) {
        let to_write: Vec<SessionState> = {
            let mut st = self.state.write();
            let ids: Vec<SessionId> = st.dirty.drain().collect();
            ids.into_iter().filter_map(|id| st.session(id).cloned()).collect()
        };
        for s in to_write {
            let path = dir.join(format!("{}.json", s.meta.id));
            match serde_json::to_vec(&s) {
                Ok(bytes) => {
                    if let Err(e) = write_atomic(&path, &bytes) {
                        tracing::warn!("persist {}: {e}", path.display());
                    }
                }
                Err(e) => tracing::warn!("serialize session: {e}"),
            }
        }
    }

    /// Flush all pending writes now (call on app exit).
    pub fn shutdown(&self) {
        let ids: Vec<SessionId> = {
            let st = self.state.read();
            st.drivers.keys().copied().collect()
        };
        for id in ids {
            if let Some(tx) = self.state.read().drivers.get(&id) {
                let _ = tx.send(SessionCommand::Shutdown);
            }
        }
        {
            let mut st = self.state.write();
            let all: Vec<SessionId> = st.sessions.iter().map(|s| s.meta.id).collect();
            st.dirty.extend(all);
        }
        self.flush(&sessions_dir());
        self.terminals.close_all();
    }

    // ------------------------------------------------------------------ drivers

    async fn pump(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<DriverOutput>) {
        while let Some(out) = rx.recv().await {
            match out {
                DriverOutput::Event(id, event) => {
                    self.on_subagent_lifecycle(id, &event);
                    self.on_session_event(id, event);
                }
                DriverOutput::SubEvent(parent, tool_use_id, event) => {
                    let child = self.ensure_subagent(parent, &tool_use_id, None, None, None, None, false);
                    if let Some(child) = child {
                        self.on_session_event(child, event);
                    }
                }
                DriverOutput::Usage(u) => self.merge_usage(u),
                DriverOutput::Exited(id, err) => {
                    let mut st = self.state.write();
                    st.drivers.remove(&id);
                    let children: Vec<SessionId> = st.sessions.iter().filter(|s| s.meta.parent == Some(id) && s.meta.status.is_live()).map(|s| s.meta.id).collect();
                    for c in children {
                        if let Some(cs) = st.session_mut(c) {
                            cs.meta.status = SessionStatus::Ended;
                            let meta = cs.meta.clone();
                            st.dirty.insert(c);
                            self.broadcast(HubEvent::SessionMeta { meta });
                        }
                    }
                    if let Some(s) = st.session_mut(id) {
                        if let Some(e) = err
                            && !matches!(s.meta.status, SessionStatus::Error(_)) {
                                s.meta.status = SessionStatus::Error(e);
                            }
                        s.meta.remote_control = RemoteControlState::default();
                        let meta = s.meta.clone();
                        st.dirty.insert(id);
                        drop(st);
                        self.broadcast(HubEvent::SessionMeta { meta });
                    }
                }
            }
        }
    }

    fn on_session_event(&self, id: SessionId, event: SessionEvent) {
        let mut st = self.state.write();
        // The library's skills join every agent's own commands in the `/` menu.
        let event = match event {
            SessionEvent::Commands { commands } => SessionEvent::Commands { commands: merge_skill_commands(&commands, &st.skills) },
            e => e,
        };
        let Some(s) = st.session_mut(id) else { return };
        let before = s.meta.status.clone();
        s.apply(&event);
        let learned = match &event {
            SessionEvent::Ready { models, .. } if !models.is_empty() => Some((s.meta.agent, models.clone())),
            _ => None,
        };
        if let Some((agent, models)) = learned {
            self.cache_models(&mut st, agent, models);
        }
        let Some(s) = st.session_mut(id) else { return };
        if matches!(event, SessionEvent::Upsert { entry: Entry::TurnEnd { .. } }) {
            s.meta.unread = true;
        }
        let needs_input_now = s.meta.status == SessionStatus::NeedsInput && before != SessionStatus::NeedsInput;
        // Turn finished: send the next queued message.
        let mut next: Option<QueuedPrompt> = None;
        if matches!(event, SessionEvent::Status { status: SessionStatus::Idle }) && !s.meta.queue.is_empty() {
            next = Some(s.meta.queue.remove(0));
        }
        let meta_after = next.as_ref().map(|_| s.meta.clone());
        let rewound = match &event {
            SessionEvent::FilesRewound { ok, detail } => Some((*ok, detail.clone())),
            _ => None,
        };
        let title = s.meta.title.clone();
        // The first message of a chat gets a generated title.
        let titling = match &event {
            SessionEvent::Upsert { entry: Entry::User { text, .. } }
                if !s.meta.titled && s.meta.parent.is_none() && s.meta.kind == SessionKind::Chat && s.entries.iter().filter(|e| matches!(e, Entry::User { .. })).count() == 1 =>
            {
                Some(text.clone())
            }
            _ => None,
        };
        // Persist on anything except streaming deltas.
        if !matches!(event, SessionEvent::AppendText { .. }) {
            st.dirty.insert(id);
        }
        // Broadcast while holding the lock so snapshot_and_subscribe() is exact.
        self.broadcast(HubEvent::Session { id, event });
        if let Some(meta) = meta_after {
            self.broadcast(HubEvent::SessionMeta { meta });
        }
        if let Some(q) = next
            && let Some(tx) = st.drivers.get(&id) {
                let _ = tx.send(SessionCommand::Prompt { text: q.text, attachments: q.attachments, delivery: Delivery::Normal });
            }
        if let Some((ok, detail)) = rewound {
            st.rewound.insert(id);
            if let Some(d) = detail {
                self.broadcast(HubEvent::Toast { level: if ok { NoticeLevel::Info } else { NoticeLevel::Warning }, text: d });
            }
        }
        drop(st);
        if let Some(text) = titling {
            self.generate_title(id, text);
        }
        if needs_input_now && self.settings.read().notify_needs_input {
            self.broadcast(HubEvent::Toast { level: NoticeLevel::Info, text: format!("“{title}” needs your input") });
        }
    }

    /// Create (or find) the child session for a subagent.
    #[allow(clippy::too_many_arguments)]
    fn ensure_subagent(
        &self,
        parent: SessionId,
        tool_use_id: &str,
        task_id: Option<String>,
        title: Option<String>,
        agent_type: Option<String>,
        prompt: Option<String>,
        background: bool,
    ) -> Option<SessionId> {
        let mut st = self.state.write();
        if let Some(id) = st.subagents.get(&(parent, tool_use_id.to_string())).copied() {
            // Fill in details that arrive after the first message.
            if let Some(s) = st.session_mut(id)
                && let Some(info) = s.meta.subagent.as_mut() {
                    let mut changed = false;
                    if info.task_id.is_none() && task_id.is_some() {
                        info.task_id = task_id;
                        changed = true;
                    }
                    if let Some(t) = title {
                        s.meta.title = t;
                        changed = true;
                    }
                    if changed {
                        let meta = s.meta.clone();
                        self.broadcast(HubEvent::SessionMeta { meta });
                    }
                }
            return Some(id);
        }
        let pm = st.session(parent)?.meta.clone();
        let mut meta = SessionMeta::new(pm.agent, SessionKind::Chat, pm.project.clone(), pm.permission_mode);
        meta.title = title.unwrap_or_else(|| "Subagent".into());
        meta.parent = Some(parent);
        meta.workspace = pm.workspace;
        meta.extra_projects = pm.extra_projects.clone();
        meta.model = pm.model.clone();
        meta.status = SessionStatus::Working;
        meta.subagent = Some(SubagentInfo {
            tool_use_id: tool_use_id.to_string(),
            task_id,
            agent_type,
            prompt: prompt.clone(),
            background,
            ..Default::default()
        });
        let id = meta.id;
        let mut state = SessionState::new(meta);
        if let Some(p) = prompt {
            state.entries.push(Entry::User { id: new_entry_id(), text: p, attachments: vec![], at: chrono::Utc::now(), agent_ref: None, prev_ref: None });
            // Keep the subagent's own title rather than deriving it from the prompt.
        }
        st.subagents.insert((parent, tool_use_id.to_string()), id);
        st.sessions.push(state.clone());
        st.dirty.insert(id);
        self.broadcast(HubEvent::SessionCreated { state: Box::new(state) });
        Some(id)
    }

    fn on_subagent_lifecycle(&self, parent: SessionId, event: &SessionEvent) {
        match event {
            SessionEvent::SubagentStarted { tool_use_id, task_id, title, agent_type, prompt, background } => {
                self.ensure_subagent(parent, tool_use_id, task_id.clone(), Some(title.clone()), agent_type.clone(), prompt.clone(), *background);
            }
            SessionEvent::SubagentProgress { tool_use_id, tokens, tool_uses, last_tool } => {
                let Some(id) = self.ensure_subagent(parent, tool_use_id, None, None, None, None, false) else { return };
                let _ = self.update_meta(id, |m| {
                    if let Some(info) = m.subagent.as_mut() {
                        info.tokens = *tokens;
                        info.tool_uses = *tool_uses;
                        info.last_tool = last_tool.clone();
                    }
                    m.context.used_tokens = *tokens;
                });
            }
            SessionEvent::SubagentFinished { tool_use_id, status } => {
                let Some(id) = self.ensure_subagent(parent, tool_use_id, None, None, None, None, false) else { return };
                let new_status = match status.as_str() {
                    "failed" => SessionStatus::Error("Subagent failed".into()),
                    _ => SessionStatus::Ended,
                };
                self.on_session_event(id, SessionEvent::Status { status: new_status });
            }
            _ => {}
        }
    }

    /// Commands aimed at a subagent are carried out by its orchestrator's process.
    fn dispatch_to_subagent(&self, id: SessionId, parent: SessionId, tool_use_id: String, command: SessionCommand) -> anyhow::Result<()> {
        let tx = self.state.read().drivers.get(&parent).cloned();
        let Some(tx) = tx else {
            anyhow::bail!("The orchestrator chat has stopped, so this subagent can't be controlled anymore.");
        };
        let forward = match command {
            c @ (SessionCommand::RespondPermission { .. } | SessionCommand::AnswerQuestion { .. }) => c,
            SessionCommand::Cancel | SessionCommand::StopSubagent { .. } => SessionCommand::StopSubagent { tool_use_id },
            SessionCommand::Prompt { .. } => {
                anyhow::bail!("Subagents work on their own and can't take direct messages. Message the orchestrator chat to steer them.")
            }
            _ => return Ok(()),
        };
        let _ = id;
        tx.send(forward).map_err(|_| anyhow::anyhow!("The orchestrator chat has stopped"))
    }

    /// Edit & resend: revert files to before `entry_id`, rewind the conversation to the message
    /// before it, then send `text` in its place.
    fn edit_message(&self, id: SessionId, entry_id: String, text: String) -> anyhow::Result<()> {
        let (meta, agent_ref, prev_ref) = {
            let st = self.state.read();
            let s = st.session(id).ok_or_else(|| anyhow::anyhow!("no such session"))?;
            let Some(Entry::User { agent_ref, prev_ref, .. }) = s.entries.iter().find(|e| e.id() == entry_id) else {
                anyhow::bail!("Only your own messages can be edited");
            };
            (s.meta.clone(), agent_ref.clone(), prev_ref.clone())
        };
        anyhow::ensure!(meta.agent.supports_edit(), "{} can't rewind a conversation, so messages can't be edited", meta.agent.display_name());
        anyhow::ensure!(meta.parent.is_none(), "Subagent messages can't be edited");
        let Some(agent_ref) = agent_ref else {
            anyhow::bail!("This message was sent before Mothr could track it, so it can't be edited");
        };
        let hub = self.me.upgrade().ok_or_else(|| anyhow::anyhow!("hub is shutting down"))?;
        self.rt.spawn(async move {
            // 1. Revert file changes made since that message (needs a live process).
            let tx = match hub.resume(id) {
                Ok(tx) => tx,
                Err(e) => {
                    hub.broadcast(HubEvent::Toast { level: NoticeLevel::Error, text: format!("Couldn't edit: {e}") });
                    return;
                }
            };
            hub.state.write().rewound.remove(&id);
            let _ = tx.send(SessionCommand::RewindFiles { user_message_id: agent_ref });
            for _ in 0..160 {
                if hub.state.read().rewound.contains(&id) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            // 2. Stop the process and wait for it to exit.
            let _ = tx.send(SessionCommand::Shutdown);
            for _ in 0..160 {
                if !hub.state.read().drivers.contains_key(&id) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            // 3. Drop the edited message and everything after it.
            hub.on_session_event(id, SessionEvent::Truncate { from_entry_id: entry_id });
            // 4. Resume up to the previous message (or start fresh) and send the edit.
            let meta = match hub.state.read().session(id) {
                Some(s) => s.meta.clone(),
                None => return,
            };
            let spawned = match &prev_ref {
                Some(at) => hub.spawn_driver_at(&meta, true, Some(at.clone())),
                None => hub.spawn_driver_at(&meta, false, None),
            };
            match spawned {
                Ok(tx) => {
                    hub.state.write().drivers.insert(id, tx.clone());
                    let _ = hub.update_meta(id, |m| {
                        m.status = SessionStatus::Starting;
                        m.queue.clear();
                    });
                    let _ = tx.send(SessionCommand::Prompt { text, attachments: vec![], delivery: Delivery::Normal });
                }
                Err(e) => hub.broadcast(HubEvent::Toast { level: NoticeLevel::Error, text: format!("Couldn't restart the chat: {e}") }),
            }
        });
        Ok(())
    }

    fn transport_for(&self, agent: AgentKind) -> anyhow::Result<Transport> {
        let st = self.state.read();
        let install = st.agents.iter().find(|a| a.kind == agent);
        match install.and_then(|i| i.transport.clone()) {
            Some(t) => Ok(t),
            None => {
                let note = install.and_then(|i| i.note.clone());
                anyhow::bail!(note.unwrap_or_else(|| format!("{} isn't available", agent.display_name())))
            }
        }
    }

    fn spawn_driver(&self, meta: &SessionMeta, resume: bool) -> anyhow::Result<CommandTx> {
        self.spawn_driver_at(meta, resume, None)
    }

    fn spawn_driver_at(&self, meta: &SessionMeta, resume: bool, resume_at: Option<String>) -> anyhow::Result<CommandTx> {
        let transport = self.transport_for(meta.agent)?;
        let system_prompt = (meta.kind == SessionKind::Design).then(|| mothr_design::INSTRUCTIONS.to_string());
        if meta.kind == SessionKind::Design {
            mothr_design::ensure_scaffold(&meta.project, &meta.project_name())?;
        }
        let (default_model, skills_auto) = {
            let s = self.settings.read();
            (s.agents.get(&meta.agent).and_then(|o| o.default_model.clone()), s.skills_auto)
        };
        let (skills, empty) = {
            let st = self.state.read();
            (st.skills.clone(), st.session(meta.id).is_none_or(|s| s.entries.is_empty()))
        };
        let skills_note = if skills_auto { catalog_note(&skills) } else { None };
        let spec = SessionSpec {
            id: meta.id,
            agent: meta.agent,
            kind: meta.kind,
            cwd: meta.project.clone(),
            mode: meta.permission_mode,
            model: meta.model.clone().or(default_model),
            effort: meta.effort.clone(),
            context: meta.context_limit.clone(),
            resume: if resume { meta.agent_session_id.clone() } else { None },
            resume_at,
            replay: resume && empty,
            system_prompt,
            extra_dirs: if meta.kind == SessionKind::Chat { meta.extra_projects.clone() } else { vec![] },
            title: meta.title.clone(),
            skills,
            skills_note,
        };
        let _guard = self.rt.enter();
        mothr_agents::spawn(&transport, spec, self.driver_tx.clone())
    }

    #[allow(clippy::too_many_arguments)]
    fn create_session(
        &self,
        agent: AgentKind,
        kind: SessionKind,
        mut project: PathBuf,
        mode: PermissionMode,
        model: Option<String>,
        effort: Option<String>,
        context: Option<String>,
        prompt: Option<String>,
        workspace: Option<WorkspaceId>,
        extra_projects: Vec<PathBuf>,
    ) -> anyhow::Result<SessionId> {
        let general = project.as_os_str().is_empty() && kind == SessionKind::Chat;
        if general {
            // General chats aren't tied to a project; they run from the home directory so the
            // agent can reach files anywhere (with the usual permission prompts).
            project = home_dir();
        }
        if project.as_os_str().is_empty() && kind == SessionKind::Design {
            // A design without a project gets its own workspace folder.
            project = designs_dir().join(format!("design-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
            std::fs::create_dir_all(&project)?;
        }
        anyhow::ensure!(project.is_dir(), "{} is not a folder", project.display());
        // The nearest mode the agent runs with this model (Claude's auto needs a model with it).
        let modes = permission_modes(agent, self.state.read().models.get(&agent).map(Vec::as_slice).unwrap_or_default(), model.as_deref());
        let mut meta = SessionMeta::new(agent, kind, project.clone(), mode.closest_in(&modes));
        meta.model = model;
        meta.effort = effort.filter(|e| !e.is_empty());
        meta.context_limit = context.filter(|c| !c.is_empty());
        meta.general = general;
        if kind == SessionKind::Chat && !general {
            meta.extra_projects = normalize_extra_projects(&project, extra_projects);
        }
        meta.workspace = {
            let st = self.state.read();
            workspace.filter(|w| st.workspaces.iter().any(|x| x.id == *w)).or_else(|| st.default_workspace())
        };
        let tx = self.spawn_driver(&meta, false)?;
        let id = meta.id;
        let extra = meta.extra_projects.clone();
        let mut state = SessionState::new(meta);
        // Skills are in the `/` menu before the agent reports its own commands.
        state.commands = merge_skill_commands(&[], &self.state.read().skills);
        {
            let mut st = self.state.write();
            st.sessions.push(state.clone());
            st.drivers.insert(id, tx.clone());
            st.dirty.insert(id);
        }
        self.broadcast(HubEvent::SessionCreated { state: Box::new(state) });
        if !general {
            for p in extra.iter().rev().chain(std::iter::once(&project)) {
                self.projects.touch_recent(p);
            }
            self.broadcast_projects();
        }
        if let Some(text) = prompt.filter(|p| !p.trim().is_empty()) {
            let _ = tx.send(SessionCommand::Prompt { text, attachments: vec![], delivery: Delivery::Normal });
        }
        Ok(id)
    }

    /// Change the context window of a running chat. Agents fix it when they start, so the
    /// process is restarted, resuming the same conversation.
    fn change_context(&self, id: SessionId, context: String) -> anyhow::Result<()> {
        let (agent, status, current) = {
            let st = self.state.read();
            let s = st.session(id).ok_or_else(|| anyhow::anyhow!("no such session"))?;
            (s.meta.agent, s.meta.status.clone(), s.meta.context_limit.clone())
        };
        if current.as_deref() == Some(context.as_str()) {
            return Ok(());
        }
        anyhow::ensure!(
            !matches!(status, SessionStatus::Working | SessionStatus::NeedsInput | SessionStatus::Starting),
            "Wait for the current turn to finish before changing the context window"
        );
        self.update_meta(id, |m| m.context_limit = Some(context))?;
        let Some(tx) = self.state.read().drivers.get(&id).cloned() else { return Ok(()) };
        if !matches!(agent, AgentKind::ClaudeCode | AgentKind::Copilot) {
            let _ = tx; // Other agents have no context choice to apply.
            return Ok(());
        }
        self.restart_resuming(id, tx)
    }

    /// Change the projects a chat covers besides its primary one. Claude Code takes folders
    /// only when it starts, so a running Claude chat restarts, resuming the conversation;
    /// other agents hear about them with their next message.
    fn set_extra_projects(&self, id: SessionId, projects: Vec<PathBuf>) -> anyhow::Result<()> {
        let meta = self.state.read().session(id).map(|s| s.meta.clone()).ok_or_else(|| anyhow::anyhow!("no such session"))?;
        anyhow::ensure!(meta.parent.is_none(), "Subagents work in their orchestrator's repos");
        anyhow::ensure!(meta.kind == SessionKind::Chat && !meta.general, "Only chats in a repo can add more repos");
        let projects = normalize_extra_projects(&meta.project, projects);
        if projects == meta.extra_projects {
            return Ok(());
        }
        let live = self.state.read().drivers.get(&id).cloned();
        let restart = live.is_some() && meta.agent == AgentKind::ClaudeCode;
        anyhow::ensure!(
            !restart || !matches!(meta.status, SessionStatus::Working | SessionStatus::NeedsInput | SessionStatus::Starting),
            "Wait for the current turn to finish before changing the chat's repos"
        );
        let children: Vec<SessionId> = self.state.read().sessions.iter().filter(|s| s.meta.parent == Some(id)).map(|s| s.meta.id).collect();
        for c in std::iter::once(id).chain(children) {
            self.update_meta(c, |m| m.extra_projects = projects.clone())?;
        }
        for p in &projects {
            self.projects.touch_recent(p);
        }
        self.broadcast_projects();
        match live {
            Some(tx) if restart => self.restart_resuming(id, tx)?,
            Some(tx) => {
                let _ = tx.send(SessionCommand::SetExtraProjects { projects });
            }
            None => {}
        }
        Ok(())
    }

    /// Stop a chat's agent process and start it again on the same conversation, so settings
    /// fixed at start-up (context window, folders) take effect.
    fn restart_resuming(&self, id: SessionId, tx: CommandTx) -> anyhow::Result<()> {
        let hub = self.me.upgrade().ok_or_else(|| anyhow::anyhow!("hub is shutting down"))?;
        self.rt.spawn(async move {
            let _ = tx.send(SessionCommand::Shutdown);
            for _ in 0..160 {
                if !hub.state.read().drivers.contains_key(&id) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if let Err(e) = hub.resume(id) {
                hub.broadcast(HubEvent::Toast { level: NoticeLevel::Error, text: format!("Couldn't restart the chat: {e}") });
            }
        });
        Ok(())
    }

    /// Write a short title for a chat from its first message, with a small Claude model when
    /// Claude Code is installed. Until then (and without Claude) the chat keeps the message's
    /// first line as its title.
    fn generate_title(&self, id: SessionId, first_message: String) {
        let program = self.state.read().agents.iter().find_map(|a| match &a.transport {
            Some(Transport::ClaudeStreamJson { program }) => Some(program.clone()),
            _ => None,
        });
        let (Some(program), Some(hub)) = (program, self.me.upgrade()) else { return };
        self.rt.spawn(async move {
            let excerpt: String = first_message.chars().take(1200).collect();
            let mut cmd = tokio::process::Command::new(&program);
            mothr_core::process::no_window(cmd.as_std_mut());
            let run = cmd
                .args([
                    "-p",
                    &excerpt,
                    "--model",
                    "haiku",
                    "--output-format",
                    "text",
                    "--no-session-persistence",
                    "--tools",
                    "",
                    "--disable-slash-commands",
                    "--strict-mcp-config",
                    "--setting-sources",
                    "",
                    "--system-prompt",
                    TITLE_PROMPT,
                ])
                .current_dir(std::env::temp_dir())
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output();
            let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(45), run).await else { return };
            if !out.status.success() {
                return;
            }
            let Some(title) = clean_title(&String::from_utf8_lossy(&out.stdout)) else { return };
            let _ = hub.update_meta(id, |m| {
                if !m.titled {
                    m.title = title;
                    m.titled = true;
                }
            });
        });
    }

    /// Pick up a conversation the agent keeps on its own as a Mothr chat, and resume it.
    fn resume_past(&self, agent: AgentKind, agent_session_id: String, project: PathBuf, title: Option<String>, workspace: Option<WorkspaceId>) -> anyhow::Result<SessionId> {
        // A chat that already carries the conversation is opened again instead.
        let existing = self.state.read().sessions.iter().find(|s| s.meta.agent == agent && s.meta.agent_session_id.as_deref() == Some(agent_session_id.as_str())).map(|s| (s.meta.id, s.meta.archived));
        if let Some((id, archived)) = existing {
            if archived {
                self.update_meta(id, |m| m.archived = false)?;
                if let Some(state) = self.session(id) {
                    self.broadcast(HubEvent::SessionCreated { state: Box::new(state) });
                }
            }
            self.resume(id)?;
            return Ok(id);
        }
        anyhow::ensure!(project.is_dir(), "{} is no longer there", project.display());
        let mode = self.settings.read().default_mode.for_agent(agent);
        let mut meta = SessionMeta::new(agent, SessionKind::Chat, project.clone(), mode);
        meta.agent_session_id = Some(agent_session_id.clone());
        meta.general = project == home_dir();
        if let Some(t) = title.filter(|t| !t.trim().is_empty() && t != "Untitled chat") {
            meta.title = t;
            meta.titled = true;
        }
        meta.status = SessionStatus::Ended;
        meta.workspace = {
            let st = self.state.read();
            workspace.filter(|w| st.workspaces.iter().any(|x| x.id == *w)).or_else(|| st.default_workspace())
        };
        let id = meta.id;
        let mut state = SessionState::new(meta);
        // Claude Code doesn't replay a resumed conversation; its transcript file has it.
        if agent == AgentKind::ClaudeCode {
            state.entries = mothr_agents::history::claude_history(&agent_session_id, &project);
        }
        state.commands = merge_skill_commands(&[], &self.state.read().skills);
        {
            let mut st = self.state.write();
            st.sessions.push(state.clone());
            st.dirty.insert(id);
        }
        self.broadcast(HubEvent::SessionCreated { state: Box::new(state) });
        if project != home_dir() {
            self.projects.touch_recent(&project);
            self.broadcast_projects();
        }
        self.resume(id)?;
        Ok(id)
    }

    fn resume(&self, id: SessionId) -> anyhow::Result<CommandTx> {
        if let Some(tx) = self.state.read().drivers.get(&id) {
            return Ok(tx.clone());
        }
        let meta = self.state.read().session(id).map(|s| s.meta.clone()).ok_or_else(|| anyhow::anyhow!("no such session"))?;
        let tx = self.spawn_driver(&meta, true)?;
        let mut st = self.state.write();
        st.drivers.insert(id, tx.clone());
        if let Some(s) = st.session_mut(id) {
            s.meta.status = SessionStatus::Starting;
            let meta = s.meta.clone();
            drop(st);
            self.broadcast(HubEvent::SessionMeta { meta });
        }
        Ok(tx)
    }

    fn update_meta(&self, id: SessionId, f: impl FnOnce(&mut SessionMeta)) -> anyhow::Result<()> {
        let meta = {
            let mut st = self.state.write();
            let s = st.session_mut(id).ok_or_else(|| anyhow::anyhow!("no such session"))?;
            f(&mut s.meta);
            let m = s.meta.clone();
            st.dirty.insert(id);
            m
        };
        self.broadcast(HubEvent::SessionMeta { meta });
        Ok(())
    }
}

impl Hub {
    fn snapshot_from(&self, st: &State) -> HubSnapshot {
        HubSnapshot {
            sessions: st.sessions.iter().filter(|s| !s.meta.archived).cloned().collect(),
            agents: st.agents.clone(),
            usage: st.usage.values().cloned().collect(),
            pinned_projects: self.projects.pinned(),
            recent_projects: self.projects.recent(12),
            agent_models: st.models.clone(),
            workspaces: st.workspaces.clone(),
            active_workspace: st.default_workspace(),
            skills: st.skills.clone(),
            skills_installing: st.skills_installing.clone(),
            terminals: self.terminals.screens(),
        }
    }
}

impl HubApi for Hub {
    fn snapshot(&self) -> HubSnapshot {
        let st = self.state.read();
        self.snapshot_from(&st)
    }

    fn snapshot_and_subscribe(&self) -> (HubSnapshot, broadcast::Receiver<HubEvent>) {
        // Session events are broadcast while the write lock is held, so subscribing and
        // snapshotting under one read guard yields an exact, gap-free starting point.
        let st = self.state.read();
        let rx = self.events.subscribe();
        (self.snapshot_from(&st), rx)
    }

    fn session(&self, id: SessionId) -> Option<SessionState> {
        self.state.read().session(id).cloned()
    }

    fn past_chats(&self, agent: AgentKind) -> anyhow::Result<Vec<PastChat>> {
        let transport = self.transport_for(agent)?;
        let mut chats = mothr_agents::history::past_chats(&transport, agent)?;
        let st = self.state.read();
        let known: HashMap<&str, SessionId> = st
            .sessions
            .iter()
            .filter(|s| s.meta.agent == agent && !s.meta.is_subagent())
            .filter_map(|s| Some((s.meta.agent_session_id.as_deref()?, s.meta.id)))
            .collect();
        for c in &mut chats {
            c.chat = known.get(c.id.as_str()).copied();
        }
        Ok(chats)
    }

    fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.events.subscribe()
    }



    fn dispatch(&self, command: AppCommand) -> anyhow::Result<Option<SessionId>> {
        match command {
            AppCommand::NewSession { agent, kind, project, mode, model, effort, context, prompt, workspace, extra_projects } => {
                return self.create_session(agent, kind, project, mode, model, effort, context, prompt, workspace, extra_projects).map(Some);
            }
            AppCommand::Session { id, command } => {
                let sub = self.state.read().session(id).and_then(|s| Some((s.meta.parent?, s.meta.subagent.as_ref()?.tool_use_id.clone())));
                if let Some((parent, tool_use_id)) = sub {
                    self.dispatch_to_subagent(id, parent, tool_use_id, command)?;
                    return Ok(None);
                }
                let live = self.state.read().drivers.get(&id).cloned();
                if let (Some(_), SessionCommand::Prompt { text, attachments, delivery: Delivery::Normal }) = (&live, &command) {
                    let busy = self.state.read().session(id).is_some_and(|s| matches!(s.meta.status, SessionStatus::Working | SessionStatus::NeedsInput));
                    if busy {
                        let q = QueuedPrompt { id: new_entry_id(), text: text.clone(), attachments: attachments.clone() };
                        self.update_meta(id, |m| m.queue.push(q))?;
                        return Ok(None);
                    }
                }
                if let (Some(_), SessionCommand::SetContext { context }) = (&live, &command) {
                    self.change_context(id, context.clone())?;
                    return Ok(None);
                }
                match (live, command) {
                    (Some(tx), c) => {
                        tx.send(c).map_err(|_| anyhow::anyhow!("session has stopped"))?;
                    }
                    (None, c @ SessionCommand::Prompt { .. }) => {
                        self.resume(id)?.send(c).map_err(|_| anyhow::anyhow!("session has stopped"))?;
                    }
                    (None, SessionCommand::SetMode { mode }) => self.update_meta(id, |m| m.permission_mode = mode)?,
                    (None, SessionCommand::SetModel { model }) => self.update_meta(id, |m| m.model = Some(model))?,
                    (None, SessionCommand::SetEffort { effort }) => self.update_meta(id, |m| m.effort = Some(effort))?,
                    (None, SessionCommand::SetContext { context }) => self.update_meta(id, |m| m.context_limit = Some(context))?,
                    (None, SessionCommand::RespondPermission { entry_id, .. } | SessionCommand::AnswerQuestion { entry_id, .. }) => {
                        self.on_session_event(id, SessionEvent::Resolved { entry_id, choice: Some("expired".into()), answers: None });
                    }
                    (None, SessionCommand::SetRemoteControl { enabled: true }) => {
                        self.resume(id)?.send(SessionCommand::SetRemoteControl { enabled: true }).ok();
                    }
                    (None, _) => {}
                }
            }
            AppCommand::Dequeue { id, queue_id } => self.update_meta(id, |m| m.queue.retain(|q| q.id != queue_id))?,
            AppCommand::EditMessage { id, entry_id, text } => self.edit_message(id, entry_id, text)?,
            AppCommand::Resume { id } => {
                self.resume(id)?;
            }
            AppCommand::ResumePast { agent, agent_session_id, project, title, workspace } => {
                return self.resume_past(agent, agent_session_id, project, title, workspace).map(Some);
            }
            AppCommand::TerminalOpen { id, cwd } => {
                let cwd = self.terminal_cwd(id, cwd);
                let t = self.terminals.open(id, cwd);
                // An exited shell starts again when someone opens it.
                if t.exited().is_some() {
                    t.restart();
                }
            }
            AppCommand::TerminalInput { id, data } => {
                let t = self.terminals.get(id).ok_or_else(|| anyhow::anyhow!("no terminal is open there"))?;
                t.write(data.into_bytes());
            }
            AppCommand::TerminalResize { id, cols, rows } => {
                let t = self.terminals.get(id).ok_or_else(|| anyhow::anyhow!("no terminal is open there"))?;
                t.resize(cols, rows, None);
            }
            AppCommand::TerminalRestart { id } => {
                let t = self.terminals.get(id).ok_or_else(|| anyhow::anyhow!("no terminal is open there"))?;
                t.restart();
            }
            AppCommand::TerminalClose { id } => {
                if self.terminals.close(id) {
                    self.broadcast(HubEvent::TerminalClosed { id });
                }
            }
            AppCommand::Rename { id, title } => self.update_meta(id, |m| {
                m.title = title;
                m.titled = true;
            })?,
            AppCommand::SetExtraProjects { id, projects } => self.set_extra_projects(id, projects)?,
            AppCommand::PinSession { id, pinned } => self.update_meta(id, |m| m.pinned = pinned)?,
            AppCommand::Archive { id, archived } => {
                if archived
                    && let Some(tx) = self.state.read().drivers.get(&id) {
                        let _ = tx.send(SessionCommand::Shutdown);
                    }
                self.update_meta(id, |m| m.archived = archived)?;
                if archived {
                    self.broadcast(HubEvent::SessionRemoved { id });
                }
            }
            AppCommand::SetGroup { id, group } => self.update_meta(id, |m| m.group = group.filter(|g| !g.trim().is_empty()))?,
            AppCommand::MarkRead { id } => {
                if self.state.read().session(id).is_some_and(|s| s.meta.unread) {
                    self.update_meta(id, |m| m.unread = false)?;
                }
            }
            AppCommand::Delete { id } => {
                let children: Vec<SessionId> = self.state.read().sessions.iter().filter(|s| s.meta.parent == Some(id)).map(|s| s.meta.id).collect();
                for c in children {
                    let _ = self.dispatch(AppCommand::Delete { id: c });
                }
                let tx = {
                    let mut st = self.state.write();
                    st.subagents.retain(|_, v| *v != id);
                    st.sessions.retain(|s| s.meta.id != id);
                    st.dirty.remove(&id);
                    st.drivers.remove(&id)
                };
                if let Some(tx) = tx {
                    let _ = tx.send(SessionCommand::Shutdown);
                }
                let _ = std::fs::remove_file(sessions_dir().join(format!("{id}.json")));
                if self.terminals.close(id) {
                    self.broadcast(HubEvent::TerminalClosed { id });
                }
                self.broadcast(HubEvent::SessionRemoved { id });
            }
            AppCommand::PinProject { path, pinned } => {
                self.projects.set_pinned(&path, pinned);
                self.broadcast_projects();
            }
            AppCommand::CreateWorkspace { workspace } => self.create_workspace(workspace)?,
            AppCommand::UpdateWorkspace { workspace } => self.update_workspace(workspace)?,
            AppCommand::DeleteWorkspace { id } => self.delete_workspace(id)?,
            AppCommand::MoveWorkspace { id, index } => self.move_workspace(id, index)?,
            AppCommand::SetWorkspace { id, workspace } => self.set_workspace(id, workspace)?,
            AppCommand::SelectWorkspace { id } => self.select_workspace(id)?,
            AppCommand::RefreshUsage => {
                if let Some(h) = self.me.upgrade() {
                    self.rt.spawn(async move { h.refresh_usage().await });
                }
            }
            AppCommand::RescanAgents => {
                if let Some(h) = self.me.upgrade() {
                    h.rescan_agents();
                }
            }
            AppCommand::RescanProjects => {
                if let Some(h) = self.me.upgrade() {
                    h.rescan_projects();
                }
            }
            AppCommand::InstallSkills { source } => self.install_skills(source)?,
            AppCommand::UpdateSkill { name } => {
                let source = self.state.read().skills.iter().find(|s| s.name == name).map(|s| s.source.clone());
                match source {
                    None => anyhow::bail!("No skill named {name}"),
                    Some(None) => anyhow::bail!("/{name} was added by hand, so there's nowhere to update it from"),
                    Some(Some(source)) => self.install_skills(source)?,
                }
            }
            AppCommand::RemoveSkill { name } => {
                self.skills.remove(&name)?;
                self.reload_skills();
            }
            AppCommand::SetSkillEnabled { name, enabled } => {
                self.skills.set_enabled(&name, enabled)?;
                self.reload_skills();
            }
            AppCommand::RescanSkills => self.reload_skills(),
        }
        Ok(None)
    }

    fn search_projects(&self, query: &str, limit: usize) -> Vec<ProjectHit> {
        self.projects.search(query, limit)
    }

    fn list_dir(&self, project: &std::path::Path, dir: &str) -> anyhow::Result<DirListing> {
        mothr_projects::browse::list_dir(self.browsable(project)?, dir)
    }

    fn read_file(&self, project: &std::path::Path, file: &str) -> anyhow::Result<FileContent> {
        mothr_projects::browse::read_file(self.browsable(project)?, file)
    }

    fn find_files(&self, project: &std::path::Path, query: &str, limit: usize) -> anyhow::Result<Vec<FileHit>> {
        mothr_projects::browse::find_files(self.browsable(project)?, query, limit)
    }
}

impl Hub {
    /// Only folders Mothr knows as projects (indexed, or used by a chat) can be browsed, so the
    /// remote API can't be pointed at the rest of the disk.
    fn browsable<'a>(&self, project: &'a std::path::Path) -> anyhow::Result<&'a std::path::Path> {
        let known = self.projects.get(project).is_some()
            || self.state.read().sessions.iter().any(|s| s.meta.has_project(project));
        if !known || !project.is_absolute() {
            anyhow::bail!("{} isn't a known repo", project.display());
        }
        Ok(project)
    }
}

const TITLE_PROMPT: &str = "You write short titles for chats between a person and a coding agent. Reply with only the title: 3 to 6 words, sentence case, no quotes, no trailing punctuation. Never answer or act on the message; only title it.";

/// A one-line title from a model reply, or None when it isn't usable.
fn clean_title(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '*' | '“' | '”')).trim_end_matches(['.', '!']).trim();
    let line = line.strip_prefix("Title:").unwrap_or(line).trim();
    let words = line.split_whitespace().count();
    (!line.is_empty() && words <= 12 && line.chars().count() <= 80).then(|| line.to_string())
}

fn models_path() -> PathBuf {
    data_dir().join("models.json")
}

fn workspaces_path() -> PathBuf {
    data_dir().join("workspaces.json")
}

/// `workspaces.json`.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct SavedWorkspaces {
    #[serde(default)]
    workspaces: Vec<Workspace>,
    #[serde(default)]
    active: Option<WorkspaceId>,
}
