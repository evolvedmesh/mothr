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
}

impl State {
    fn session_mut(&mut self, id: SessionId) -> Option<&mut SessionState> {
        self.sessions.iter_mut().find(|s| s.meta.id == id)
    }
    fn session(&self, id: SessionId) -> Option<&SessionState> {
        self.sessions.iter().find(|s| s.meta.id == id)
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
}

impl Hub {
    /// Create the hub and start its background tasks on `rt`.
    pub fn start(rt: Handle, settings: Settings) -> Arc<Hub> {
        let (events, _) = broadcast::channel(4096);
        let (driver_tx, driver_rx) = mpsc::unbounded_channel();
        let hub = Arc::new_cyclic(|me| Hub {
            me: me.clone(),
            rt: rt.clone(),
            state: RwLock::new(State::default()),
            events,
            driver_tx,
            projects: Arc::new(ProjectIndex::new()),
            usage: UsageFetcher::new(),
            settings: RwLock::new(settings),
        });
        hub.load_sessions();
        hub.load_models();
        let h = hub.clone();
        rt.spawn(async move { h.pump(driver_rx).await });
        let h = hub.clone();
        rt.spawn(async move { h.persist_loop().await });
        let h = hub.clone();
        rt.spawn(async move { h.usage_loop().await });
        hub.rescan_agents();
        hub.rescan_projects();
        hub
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().clone()
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
                    system_prompt: None,
                    title: "model probe".into(),
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
        let default_model = self.settings.read().agents.get(&meta.agent).and_then(|o| o.default_model.clone());
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
            system_prompt,
            title: meta.title.clone(),
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
        let mut meta = SessionMeta::new(agent, kind, project.clone(), mode.for_agent(agent));
        meta.model = model;
        meta.effort = effort.filter(|e| !e.is_empty());
        meta.context_limit = context.filter(|c| !c.is_empty());
        meta.general = general;
        let tx = self.spawn_driver(&meta, false)?;
        let id = meta.id;
        let state = SessionState::new(meta);
        {
            let mut st = self.state.write();
            st.sessions.push(state.clone());
            st.drivers.insert(id, tx.clone());
            st.dirty.insert(id);
        }
        self.broadcast(HubEvent::SessionCreated { state: Box::new(state) });
        if !general {
            self.projects.touch_recent(&project);
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
            let run = tokio::process::Command::new(&program)
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

    fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.events.subscribe()
    }



    fn dispatch(&self, command: AppCommand) -> anyhow::Result<Option<SessionId>> {
        match command {
            AppCommand::NewSession { agent, kind, project, mode, model, effort, context, prompt } => {
                return self.create_session(agent, kind, project, mode, model, effort, context, prompt).map(Some);
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
            AppCommand::Rename { id, title } => self.update_meta(id, |m| {
                m.title = title;
                m.titled = true;
            })?,
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
                self.broadcast(HubEvent::SessionRemoved { id });
            }
            AppCommand::PinProject { path, pinned } => {
                self.projects.set_pinned(&path, pinned);
                self.broadcast_projects();
            }
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
            || self.state.read().sessions.iter().any(|s| s.meta.project == project);
        if !known || !project.is_absolute() {
            anyhow::bail!("{} isn't a known project", project.display());
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
