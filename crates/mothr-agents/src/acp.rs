//! Agent Client Protocol driver (newline-delimited JSON-RPC 2.0 over stdio).
//!
//! Implemented directly on `serde_json::Value` so it tolerates the small schema differences
//! between agents (Copilot, codex-acp, OpenCode, Hermes) and protocol revisions.

use crate::claude::truncate;
use crate::policy::{self, AutoDecision};
use crate::{Emitter, OutputTx, SessionSpec};
use serde_json::{json, Value};
use mothr_core::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Debug)]
enum Call {
    Initialize,
    NewSession,
    LoadSession,
    Prompt,
    SetMode(PermissionMode),
    SetModel(String),
    SetEffort(String),
}

struct PendingPermission {
    rpc_id: Value,
    question: bool,
    /// Set when the question came from Mothr's own MCP tool rather than an ACP request.
    ask: Option<tokio::sync::oneshot::Sender<Option<Vec<Vec<String>>>>>,
}

struct Driver {
    em: Emitter,
    spec: SessionSpec,
    writer: mpsc::UnboundedSender<String>,
    next_id: u64,
    calls: HashMap<u64, Call>,
    session_id: Option<String>,
    load_session: bool,
    image_prompts: bool,
    /// Replaying history during session/load — suppress duplicate transcript entries.
    replaying: bool,
    modes: Vec<ModeInfo>,
    current_mode_id: Option<String>,
    mode_config_id: Option<String>,
    model_config_id: Option<String>,
    /// The `thought_level` config option (reasoning effort), when the agent has one.
    effort_config_id: Option<String>,
    /// Effort the session runs with now (the agent's choice until the user picks one).
    effort: Option<String>,
    mode: PermissionMode,
    /// Agent message segment currently being streamed.
    segment: Option<(EntryId, bool)>,
    /// Copilot subagents: agentId → the tool call that spawned each.
    agents: HashMap<String, String>,
    /// Subagent tool calls whose agent hasn't sent an update yet, oldest first.
    unbound: Vec<String>,
    /// Every subagent tool call seen (so each is announced once).
    spawned: std::collections::HashSet<String>,
    /// Message segment being streamed per subagent (agents interleave, so each needs its own).
    sub_segments: HashMap<String, (EntryId, bool)>,
    /// Tool call → the subagent it belongs to.
    tool_targets: HashMap<String, String>,
    /// Tool calls a subagent has made, and the latest one.
    sub_stats: HashMap<String, (u64, Option<String>)>,
    tools: HashMap<String, ToolCall>,
    permissions: HashMap<EntryId, PendingPermission>,
    turn: u32,
    turn_started: Option<Instant>,
    queued: Vec<SessionCommand>,
    /// Prompts waiting for the running turn to end.
    deferred: Vec<SessionCommand>,
    turn_active: bool,
    /// A steering message is waiting for the in-flight tool calls to finish before it cuts in.
    steer_pending: bool,
    context_window: u64,
    /// Latest usage, shared with the background task that adds Copilot credits.
    usage: std::sync::Arc<std::sync::Mutex<ContextUsage>>,
    ready: bool,
    auth_methods: Vec<Value>,
    /// Mothr's question tool as an `mcpServers` entry, when the agent can reach HTTP MCP servers.
    ask_server: Option<Value>,
    /// Tool calls to Mothr's own question tool; the question card replaces them in the chat.
    ask_tools: std::collections::HashSet<String>,
}

pub async fn run(
    program: PathBuf,
    args: Vec<String>,
    env: Vec<(String, String)>,
    spec: SessionSpec,
    mut commands: mpsc::UnboundedReceiver<SessionCommand>,
    out: OutputTx,
) {
    let em = Emitter::new(spec.id, out);
    let mut args = args;
    if spec.agent == AgentKind::Copilot
        && let Some(tier) = spec.context.as_deref().filter(|t| !t.is_empty()) {
            args.extend(["--context".to_string(), tier.to_string()]);
        }
    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .envs(env)
        .current_dir(&spec.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            em.emit(SessionEvent::Status {
                status: SessionStatus::Error(format!("Failed to start {}: {e}", spec.agent.display_name())),
            });
            em.exited(Some(e.to_string()));
            return;
        }
    };
    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let (wtx, mut wrx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(line) = wrx.recv().await {
            if stdin.write_all(line.as_bytes()).await.is_err() || stdin.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = stdin.flush().await;
        }
    });
    let tail = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    {
        let tail = tail.clone();
        let agent = spec.agent;
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                tracing::debug!(target: "acp", "{}: {l}", agent.id());
                if let Ok(mut t) = tail.lock() {
                    t.push_str(&l);
                    t.push('\n');
                    if t.len() > 4000 {
                        let cut = t.len() - 4000;
                        t.drain(..cut);
                    }
                }
            }
        });
    }

    let mut d = Driver {
        em: em.clone(),
        mode: spec.mode,
        spec,
        writer: wtx,
        next_id: 0,
        calls: HashMap::new(),
        session_id: None,
        load_session: false,
        image_prompts: false,
        replaying: false,
        modes: vec![],
        current_mode_id: None,
        mode_config_id: None,
        model_config_id: None,
        effort_config_id: None,
        effort: None,
        segment: None,
        agents: HashMap::new(),
        unbound: vec![],
        spawned: Default::default(),
        sub_segments: HashMap::new(),
        tool_targets: HashMap::new(),
        sub_stats: HashMap::new(),
        tools: HashMap::new(),
        permissions: HashMap::new(),
        turn: 0,
        turn_started: None,
        queued: vec![],
        deferred: vec![],
        turn_active: false,
        steer_pending: false,
        context_window: 0,
        usage: Default::default(),
        ready: false,
        auth_methods: vec![],
        ask_server: None,
        ask_tools: Default::default(),
    };
    let (mut ask_config, mut ask_rx) = match crate::ask_mcp::start().await {
        Some(a) => (Some(a.config), a.requests),
        None => (None, mpsc::unbounded_channel().1),
    };
    d.ask_server = ask_config.take();
    d.call(
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": {"fs": {"readTextFile": true, "writeTextFile": true}, "terminal": false},
            "clientInfo": {"name": "mothr", "title": "Mothr", "version": env!("CARGO_PKG_VERSION")},
        }),
        Call::Initialize,
    );

    let mut lines = BufReader::new(stdout).lines();
    // Copilot logs usage per model call; while a turn runs, poll it for the live credit meter.
    let mut credit_tick = tokio::time::interval(std::time::Duration::from_millis(1200));
    credit_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = credit_tick.tick(), if d.turn_active && d.spec.agent == AgentKind::Copilot => d.refresh_credits(false),
            line = lines.next_line() => match line {
                Ok(Some(l)) if !l.trim().is_empty() => match crate::claude::parse_traced(&l) {
                    Ok(v) => d.on_message(v).await,
                    Err(_) => tracing::debug!(target: "acp", "non-json: {l}"),
                },
                Ok(Some(_)) => {}
                _ => break,
            },
            Some(req) = ask_rx.recv() => d.on_ask(req),
            cmd = commands.recv() => match cmd {
                Some(SessionCommand::Shutdown) | None => { let _ = child.start_kill(); break; }
                Some(c) => d.on_command(c),
            }
        }
    }
    let status = child.wait().await.ok();
    let err = match status {
        Some(s) if !s.success() && !format!("{s}").contains("signal: 9") => {
            let t = tail.lock().map(|t| t.trim().to_string()).unwrap_or_default();
            Some(if t.is_empty() { format!("{} exited ({s})", d.spec.agent.display_name()) } else { t })
        }
        _ => None,
    };
    if let Some(e) = &err {
        em.notice(NoticeLevel::Error, e.clone());
    }
    em.emit(SessionEvent::Status { status: SessionStatus::Ended });
    em.exited(err);
}

impl Driver {
    fn send(&self, v: Value) {
        crate::claude::trace(&format!("> {v}"));
        let _ = self.writer.send(v.to_string());
    }

    fn call(&mut self, method: &str, params: Value, kind: Call) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.calls.insert(id, kind);
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    fn notify(&self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn reply(&self, id: &Value, result: Value) {
        self.send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    fn reply_err(&self, id: &Value, code: i64, msg: &str) {
        self.send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}}));
    }

    fn sid(&self) -> Value {
        json!(self.session_id.clone().unwrap_or_default())
    }

    fn set_status(&self, s: SessionStatus) {
        self.em.emit(SessionEvent::Status { status: s });
    }

    fn end_segment(&mut self) {
        if let Some((id, _)) = self.segment.take() {
            self.em.emit(SessionEvent::StreamEnd { entry_id: id });
        }
        let open: Vec<String> = self.sub_segments.keys().cloned().collect();
        for t in open {
            self.end_segment_for(&Some(t));
        }
    }

    /// Emit to the chat itself (`None`) or to the subagent session spawned by that tool call.
    fn out(&self, target: &Option<String>, e: SessionEvent) {
        match target {
            None => self.em.emit(e),
            Some(t) => self.em.sub(t.clone(), e),
        }
    }

    fn end_segment_for(&mut self, target: &Option<String>) {
        let seg = match target {
            None => self.segment.take(),
            Some(t) => self.sub_segments.remove(t),
        };
        if let Some((id, _)) = seg {
            self.out(target, SessionEvent::StreamEnd { entry_id: id });
        }
    }

    /// Which subagent (the tool call that spawned it) an update belongs to. Copilot tags every
    /// update from a subagent with its `agentId` and says nothing else, so the first sight of
    /// an id is matched to its spawning tool call from the session log, else to the oldest
    /// subagent still waiting for one.
    fn target_of(&mut self, v: &Value) -> Option<String> {
        let meta = |m: &Value| m["github.com/copilot"]["agentId"].as_str().map(str::to_string);
        let agent = meta(&v["params"]["update"]["_meta"]).or_else(|| meta(&v["params"]["_meta"]))?;
        if let Some(t) = self.agents.get(&agent) {
            return Some(t.clone());
        }
        let from_log = self.session_id.as_deref().and_then(|sid| copilot_subagent_tool(sid, &agent)).filter(|t| self.spawned.contains(t));
        let tool = match from_log {
            Some(t) => {
                self.unbound.retain(|u| *u != t);
                t
            }
            None if !self.unbound.is_empty() => self.unbound.remove(0),
            None => return None,
        };
        self.agents.insert(agent, tool.clone());
        Some(tool)
    }

    // ------------------------------------------------------------------ commands

    fn on_command(&mut self, c: SessionCommand) {
        if !self.ready && !matches!(c, SessionCommand::RespondPermission { .. } | SessionCommand::AnswerQuestion { .. }) {
            self.queued.push(c);
            return;
        }
        match c {
            SessionCommand::Prompt { text, attachments, delivery } => {
                if self.turn_active {
                    // ACP has no mid-turn input, so a message that can't wait cuts the turn short and
                    // goes out first (the session and its context carry on): `Interrupt` does it
                    // at once, `Steer` as soon as no tool call is mid-run so no edit is left
                    // half-done. A plain message waits its turn.
                    let urgent = delivery != Delivery::Normal;
                    let prompt = SessionCommand::Prompt { text, attachments, delivery: Delivery::Normal };
                    if urgent {
                        self.deferred.insert(0, prompt);
                    } else {
                        self.deferred.push(prompt);
                    }
                    match delivery {
                        Delivery::Interrupt => self.cancel_turn(),
                        Delivery::Steer if self.tools.is_empty() => self.cancel_turn(),
                        Delivery::Steer => self.steer_pending = true,
                        Delivery::Normal => {}
                    }
                    return;
                }
                self.em.emit(SessionEvent::Upsert {
                    entry: Entry::User { id: new_entry_id(), text: text.clone(), attachments: attachments.clone(), at: chrono::Utc::now(), agent_ref: None, prev_ref: None },
                });
                let mut prompt = vec![json!({"type": "text", "text": text})];
                for a in attachments {
                    let uri = format!("file://{}", a.path.display());
                    let name = a.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let is_image = matches!(
                        a.path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
                        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
                    );
                    if is_image && self.image_prompts
                        && let Ok(bytes) = std::fs::read(&a.path) {
                            let mime = a.mime.clone().unwrap_or_else(|| "image/png".into());
                            prompt.push(json!({"type": "image", "mimeType": mime, "data": crate::claude::base64_lite::encode(&bytes), "uri": uri}));
                            continue;
                        }
                    prompt.push(json!({"type": "resource_link", "uri": uri, "name": name}));
                }
                self.end_segment();
                self.turn += 1;
                self.turn_started = Some(Instant::now());
                self.set_status(SessionStatus::Working);
                self.turn_active = true;
                self.call("session/prompt", json!({"sessionId": self.sid(), "prompt": prompt}), Call::Prompt);
            }
            SessionCommand::Cancel => {
                // Stopping means stopping: nothing queued to steer in behind it.
                self.deferred.clear();
                self.cancel_turn();
            }
            SessionCommand::SetMode { mode } => self.apply_mode(mode),
            SessionCommand::SetModel { model } => {
                if let Some(cfg) = self.model_config_id.clone() {
                    self.call(
                        "session/set_config_option",
                        json!({"sessionId": self.sid(), "configId": cfg, "value": model}),
                        Call::SetModel(model),
                    );
                } else {
                    self.call("session/set_model", json!({"sessionId": self.sid(), "modelId": model}), Call::SetModel(model));
                }
            }
            SessionCommand::SetEffort { effort } => match self.effort_config_id.clone() {
                Some(cfg) => {
                    self.call("session/set_config_option", json!({"sessionId": self.sid(), "configId": cfg, "value": effort}), Call::SetEffort(effort));
                }
                None => self.em.notice(NoticeLevel::Info, format!("{} has no reasoning setting.", self.spec.agent.display_name())),
            },
            // The context window is fixed when the process starts; the hub restarts the chat for it.
            SessionCommand::SetContext { context } => {
                self.spec.context = Some(context);
                self.emit_options();
            }
            SessionCommand::RespondPermission { entry_id, option_id } => {
                if let Some(p) = self.permissions.remove(&entry_id) {
                    let outcome = if option_id == "cancelled" {
                        json!({"outcome": "cancelled"})
                    } else {
                        json!({"outcome": "selected", "optionId": option_id})
                    };
                    self.reply(&p.rpc_id, json!({"outcome": outcome}));
                    self.em.emit(SessionEvent::Resolved { entry_id, choice: Some(option_id), answers: None });
                }
            }
            SessionCommand::AnswerQuestion { entry_id, answers } => {
                // ACP has no question primitive yet; agents ask via elicitation-like requests.
                if let Some(p) = self.permissions.remove(&entry_id) {
                    if let Some(tx) = p.ask {
                        let _ = tx.send(Some(answers.clone()));
                    } else if p.question {
                        self.reply(&p.rpc_id, json!({"action": "accept", "content": {"answers": answers}}));
                    }
                    self.em.emit(SessionEvent::Resolved { entry_id, choice: None, answers: Some(answers) });
                }
            }
            SessionCommand::SetRemoteControl { .. } => {
                self.em.emit(SessionEvent::RemoteControl {
                    state: RemoteControlState {
                        enabled: false,
                        url: None,
                        pending: false,
                        error: Some(format!(
                            "{} doesn't offer Remote Control. Use Mothr Remote to reach this chat from another device.",
                            self.spec.agent.display_name()
                        )),
                    },
                });
            }
            SessionCommand::RewindFiles { .. } => {
                self.em.emit(SessionEvent::FilesRewound { ok: false, detail: Some(format!("{} can't revert files", self.spec.agent.display_name())) });
            }
            SessionCommand::StopSubagent { .. } => {
                self.em.notice(NoticeLevel::Info, format!("{} doesn't support stopping individual subagents; stop the whole chat instead.", self.spec.agent.display_name()));
            }
            SessionCommand::Shutdown => {}
        }
    }

    /// Stop the running turn and turn down anything still waiting on the user.
    fn cancel_turn(&mut self) {
        self.steer_pending = false;
        self.notify("session/cancel", json!({"sessionId": self.sid()}));
        let pending: Vec<EntryId> = self.permissions.keys().cloned().collect();
        for eid in pending {
            if let Some(p) = self.permissions.remove(&eid) {
                match p.ask {
                    Some(tx) => {
                        let _ = tx.send(None);
                    }
                    None if p.question => self.reply(&p.rpc_id, json!({"action": "cancel"})),
                    None => self.reply(&p.rpc_id, json!({"outcome": {"outcome": "cancelled"}})),
                }
                self.em.emit(SessionEvent::Resolved { entry_id: eid, choice: Some("cancelled".into()), answers: None });
            }
        }
    }

    fn apply_mode(&mut self, mode: PermissionMode) {
        self.mode = mode;
        // Mothr enforces the policy client-side; also move the agent's own mode if it has one.
        if let Some(id) = policy::match_mode(mode, &self.modes) {
            if self.current_mode_id.as_deref() == Some(id.as_str()) {
                self.em.emit(SessionEvent::Mode { mode });
                return;
            }
            self.current_mode_id = Some(id.clone());
            if let Some(cfg) = self.mode_config_id.clone() {
                self.call("session/set_config_option", json!({"sessionId": self.sid(), "configId": cfg, "value": id}), Call::SetMode(mode));
            } else {
                self.call("session/set_mode", json!({"sessionId": self.sid(), "modeId": id}), Call::SetMode(mode));
            }
        } else {
            self.em.emit(SessionEvent::Mode { mode });
        }
    }

    // ------------------------------------------------------------------ inbound

    async fn on_message(&mut self, v: Value) {
        let has_method = v.get("method").and_then(|m| m.as_str()).is_some();
        let id = v.get("id").cloned();
        match (has_method, id) {
            (true, Some(id)) if !id.is_null() => self.on_request(id, &v).await,
            (true, _) => self.on_notification(&v),
            (false, Some(id)) => self.on_response(id, &v),
            _ => {}
        }
    }

    fn on_response(&mut self, id: Value, v: &Value) {
        let Some(n) = id.as_u64() else { return };
        let Some(kind) = self.calls.remove(&n) else { return };
        let result = &v["result"];
        let error = v.get("error").filter(|e| !e.is_null());
        let err_msg = error.map(|e| {
            let m = e["message"].as_str().unwrap_or("error").to_string();
            match e["data"].as_str().or(e["data"]["message"].as_str()) {
                Some(d) if !d.is_empty() && d != m => format!("{m}: {d}"),
                _ => m,
            }
        });
        match kind {
            Call::Initialize => {
                if let Some(e) = err_msg {
                    self.set_status(SessionStatus::Error(format!("Initialize failed: {e}")));
                    return;
                }
                let caps = &result["agentCapabilities"];
                self.load_session = caps["loadSession"].as_bool().unwrap_or(false);
                self.image_prompts = caps["promptCapabilities"]["image"].as_bool().unwrap_or(false);
                if !caps["mcpCapabilities"]["http"].as_bool().unwrap_or(false) {
                    self.ask_server = None;
                }
                self.auth_methods = result["authMethods"].as_array().cloned().unwrap_or_default();
                let cwd = self.spec.cwd.display().to_string();
                match (&self.spec.resume, self.load_session) {
                    (Some(sid), true) => {
                        self.replaying = true;
                        self.session_id = Some(sid.clone());
                        self.call("session/load", json!({"sessionId": sid, "cwd": cwd, "mcpServers": self.mcp_servers()}), Call::LoadSession);
                    }
                    _ => {
                        self.call("session/new", json!({"cwd": cwd, "mcpServers": self.mcp_servers()}), Call::NewSession);
                    }
                }
            }
            Call::NewSession | Call::LoadSession => {
                self.replaying = false;
                if let Some(e) = err_msg {
                    let is_load = matches!(kind, Call::LoadSession);
                    if is_load {
                        // Fall back to a fresh session.
                        self.em.notice(NoticeLevel::Warning, format!("Couldn't restore the previous conversation ({e}); starting fresh."));
                        let cwd = self.spec.cwd.display().to_string();
                        self.call("session/new", json!({"cwd": cwd, "mcpServers": self.mcp_servers()}), Call::NewSession);
                        return;
                    }
                    let auth = error.and_then(|e| e["code"].as_i64()) == Some(-32000)
                        || e.to_ascii_lowercase().contains("auth");
                    let hint = if auth { self.auth_hint() } else { String::new() };
                    self.set_status(SessionStatus::Error(format!("{e}{hint}")));
                    self.em.notice(NoticeLevel::Error, format!("Couldn't start the session: {e}{hint}"));
                    return;
                }
                if let Some(s) = result["sessionId"].as_str() {
                    self.session_id = Some(s.to_string());
                }
                self.on_session_setup(result);
                self.ready = true;
                let mode = self.mode;
                if mode != PermissionMode::Ask || policy::match_mode(mode, &self.modes).is_some() {
                    self.apply_mode(mode);
                }
                if let Some(m) = self.spec.model.clone() {
                    self.on_command(SessionCommand::SetModel { model: m });
                }
                if let Some(e) = self.spec.effort.clone()
                    && self.effort_config_id.is_some()
                    && self.effort.as_deref() != Some(e.as_str()) {
                        self.on_command(SessionCommand::SetEffort { effort: e });
                    }
                self.emit_options();
                self.refresh_credits(false);
                for c in std::mem::take(&mut self.queued) {
                    self.on_command(c);
                }
            }
            Call::Prompt => {
                self.end_segment();
                let stale: Vec<ToolCall> = self.tools.drain().map(|(_, t)| t).collect();
                for mut t in stale {
                    if matches!(t.status, ToolStatus::Pending | ToolStatus::Running) {
                        t.status = ToolStatus::Failed;
                        let target = self.tool_targets.get(&t.id).cloned();
                        if self.spawned.contains(&t.id) {
                            self.em.emit(SessionEvent::SubagentFinished { tool_use_id: t.id.clone(), status: "failed".into() });
                        }
                        self.out(&target, SessionEvent::Upsert { entry: Entry::Tool(t) });
                    }
                }
                let stop = result["stopReason"].as_str().unwrap_or(if err_msg.is_some() { "error" } else { "end_turn" });
                if let Some(e) = &err_msg {
                    self.em.notice(NoticeLevel::Error, e.clone());
                }
                if let Some(u) = result.get("usage").filter(|u| u.is_object()) {
                    self.on_turn_usage(u);
                }
                self.em.emit(SessionEvent::Upsert {
                    entry: Entry::TurnEnd {
                        id: new_entry_id(),
                        stop_reason: stop.to_string(),
                        duration_ms: self.turn_started.map(|t| t.elapsed().as_millis() as u64),
                        cost_usd: None,
                    },
                });
                self.set_status(SessionStatus::Idle);
                self.turn_active = false;
                self.steer_pending = false;
                self.refresh_credits(true);
                if !self.deferred.is_empty() {
                    let next = self.deferred.remove(0);
                    self.on_command(next);
                }
            }
            Call::SetMode(m) => {
                if let Some(e) = err_msg {
                    self.em.notice(NoticeLevel::Warning, format!("{} didn't accept the mode change: {e}", self.spec.agent.display_name()));
                }
                // Client-side policy still applies.
                self.em.emit(SessionEvent::Mode { mode: m });
            }
            Call::SetEffort(e) => match err_msg {
                Some(msg) => self.em.notice(NoticeLevel::Warning, format!("Couldn't change the reasoning effort: {msg}")),
                None => {
                    self.effort = Some(e);
                    self.emit_options();
                }
            },
            Call::SetModel(m) => match err_msg {
                Some(e) => self.em.notice(NoticeLevel::Warning, format!("Couldn't switch model: {e}")),
                None => self.em.emit(SessionEvent::Model { model: m }),
            },
        }
    }

    fn auth_hint(&self) -> String {
        let cmd = self
            .auth_methods
            .iter()
            .find_map(|m| m["description"].as_str().map(str::to_string))
            .unwrap_or_else(|| format!("sign in with the {} CLI", self.spec.agent.display_name()));
        format!(" — authentication required: {cmd}")
    }

    fn on_session_setup(&mut self, r: &Value) {
        let mut models: Vec<ModelInfo> = r["models"]["availableModels"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        Some(ModelInfo::new(
                            m["modelId"].as_str()?,
                            m["name"].as_str().unwrap_or(m["modelId"].as_str()?),
                            m["description"].as_str().map(str::to_string),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut current_model = r["models"]["currentModelId"].as_str().map(str::to_string);
        let mut efforts: Vec<String> = vec![];
        self.current_mode_id = r["modes"]["currentModeId"].as_str().map(str::to_string);
        self.modes = r["modes"]["availableModes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        Some(ModeInfo {
                            id: m["id"].as_str()?.to_string(),
                            name: m["name"].as_str().unwrap_or("").to_string(),
                            description: m["description"].as_str().map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        for opt in r["configOptions"].as_array().into_iter().flatten() {
            let cat = opt["category"].as_str().unwrap_or("");
            let id = opt["id"].as_str().unwrap_or("").to_string();
            let options: Vec<(String, String, Option<String>)> = opt["options"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|o| {
                            Some((
                                o["value"].as_str()?.to_string(),
                                o["name"].as_str().unwrap_or("").to_string(),
                                o["description"].as_str().map(str::to_string),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            match cat {
                "model" => {
                    self.model_config_id = Some(id);
                    if models.is_empty() {
                        models = options.iter().map(|(v, n, d)| ModelInfo::new(v.clone(), n.clone(), d.clone())).collect();
                    }
                    if current_model.is_none() {
                        current_model = opt["currentValue"].as_str().map(str::to_string);
                    }
                }
                "thought_level" => {
                    self.effort = opt["currentValue"].as_str().map(str::to_string);
                    self.effort_config_id = Some(id);
                    efforts = options.iter().map(|(v, ..)| v.clone()).collect();
                }
                "mode" if (id == "mode" || self.modes.is_empty())
                    // Only adopt the config option if it carries the same ids as session modes.
                    && self.modes.is_empty() => {
                        self.modes = options.iter().map(|(v, n, d)| ModeInfo { id: v.clone(), name: n.clone(), description: d.clone() }).collect();
                        self.mode_config_id = Some(id);
                    }
                _ => {}
            }
        }
        // Dedupe models by id (Copilot lists "auto" twice).
        let mut seen = std::collections::HashSet::new();
        models.retain(|m| seen.insert(m.id.clone()));
        for m in &mut models {
            // "auto" lets Copilot choose, so it has no effort setting of its own.
            if m.id != "auto" {
                m.efforts = efforts.clone();
            }
            // Copilot's long-context tier (≈1M) exists for its Claude models.
            if self.spec.agent == AgentKind::Copilot && m.id.starts_with("claude-") {
                m.contexts = vec![ChoiceInfo::new("default", "Standard"), ChoiceInfo::new("long_context", "1M")];
            }
        }
        self.em.emit(SessionEvent::Ready {
            agent_session_id: self.session_id.clone(),
            model: current_model,
            models,
            modes: self.modes.clone(),
        });
    }

    fn on_notification(&mut self, v: &Value) {
        if v["method"] != "session/update" {
            return;
        }
        let u = &v["params"]["update"];
        let kind = u["sessionUpdate"].as_str().unwrap_or("");
        let target = self.target_of(v);
        if self.replaying && !matches!(kind, "current_mode_update" | "usage_update" | "available_commands_update") {
            return;
        }
        match kind {
            "agent_message_chunk" | "agent_thought_chunk" => {
                let thinking = kind == "agent_thought_chunk";
                if let Some(entry) = image_entry(&u["content"]) {
                    self.end_segment_for(&target);
                    self.out(&target, SessionEvent::Upsert { entry });
                    return;
                }
                let text = content_text(&u["content"]);
                if text.is_empty() {
                    return;
                }
                let open = match &target {
                    None => self.segment.clone(),
                    Some(t) => self.sub_segments.get(t).cloned(),
                };
                let id = match open {
                    Some((id, t)) if t == thinking => id,
                    _ => {
                        self.end_segment_for(&target);
                        let id = new_entry_id();
                        match &target {
                            None => self.segment = Some((id.clone(), thinking)),
                            Some(t) => {
                                self.sub_segments.insert(t.clone(), (id.clone(), thinking));
                            }
                        }
                        id
                    }
                };
                self.out(&target, SessionEvent::AppendText { entry_id: id, thinking, delta: text });
            }
            "tool_call" | "tool_call_update" => {
                self.end_segment_for(&target);
                self.on_tool(u, kind == "tool_call", target);
            }
            "plan" => {
                self.end_segment_for(&target);
                let items = u["entries"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|e| PlanItem {
                                content: e["content"].as_str().unwrap_or("").to_string(),
                                status: match e["status"].as_str() {
                                    Some("completed") => PlanStatus::Completed,
                                    Some("in_progress") => PlanStatus::InProgress,
                                    _ => PlanStatus::Pending,
                                },
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let plan_id = match &target {
                    None => format!("plan-{}", self.turn),
                    Some(t) => format!("plan-{}-{t}", self.turn),
                };
                self.out(&target, SessionEvent::Upsert { entry: Entry::Plan { id: plan_id, items } });
            }
            "current_mode_update" => {
                let id = u["currentModeId"].as_str().or(u["modeId"].as_str());
                self.current_mode_id = id.map(str::to_string);
                if let Some(m) = id.and_then(policy::mode_from_agent) {
                    self.mode = m;
                    self.em.emit(SessionEvent::Mode { mode: m });
                }
            }
            "usage_update" => self.on_usage_update(u),
            "available_commands_update" => {
                let commands: Vec<CommandInfo> = u["availableCommands"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| {
                                Some(CommandInfo {
                                    name: c["name"].as_str()?.trim_start_matches('/').to_string(),
                                    description: c["description"].as_str().map(str::to_string),
                                    hint: c["input"]["hint"].as_str().map(str::to_string),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.em.emit(SessionEvent::Commands { commands });
            }
            "session_info_update" => {
                if let Some(t) = u["title"].as_str().filter(|t| !t.is_empty()) {
                    self.em.emit(SessionEvent::Title { title: t.to_string() });
                }
            }
            "config_option_update" => {
                for opt in u["configOptions"].as_array().into_iter().flatten() {
                    if opt["category"] == "thought_level"
                        && let Some(e) = opt["currentValue"].as_str()
                        && self.effort.as_deref() != Some(e) {
                            self.effort = Some(e.to_string());
                            self.emit_options();
                        }
                    if opt["category"] == "model"
                        && let Some(m) = opt["currentValue"].as_str() {
                            self.em.emit(SessionEvent::Model { model: m.to_string() });
                        }
                }
            }
            _ => {}
        }
    }

    /// `usage_update`: how full the context window is right now.
    fn on_usage_update(&mut self, u: &Value) {
        let used = u["used"].as_u64().or(u["totalTokens"].as_u64()).or(u["total_tokens"].as_u64());
        let size = u["size"].as_u64().or(u["contextWindow"].as_u64()).filter(|s| *s > 0);
        if let Some(size) = size {
            self.context_window = size;
        }
        let cost = u["cost"]["amount"].as_f64().or(u["cost"].as_f64());
        self.update_usage(|c| {
            if let Some(used) = used {
                c.used_tokens = used;
            }
            if self.context_window > 0 {
                c.window_tokens = self.context_window;
            }
            if cost.is_some() {
                c.cost_usd = cost;
            }
        });
    }

    /// The `usage` on a finished prompt: tokens billed this turn, summed over every model call
    /// (tool loops re-read the whole context each time). It says nothing about how full the
    /// window is, so it only adds to the session's running totals.
    fn on_turn_usage(&mut self, u: &Value) {
        let input = u["inputTokens"].as_u64().unwrap_or(0);
        let output = u["outputTokens"].as_u64().unwrap_or(0);
        self.update_usage(|c| {
            c.input_tokens += input;
            c.output_tokens += output;
        });
    }

    fn update_usage(&self, f: impl FnOnce(&mut ContextUsage)) {
        let usage = {
            let mut c = self.usage.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut c);
            *c
        };
        self.em.emit(SessionEvent::Context { usage });
    }

    fn emit_options(&self) {
        self.em.emit(SessionEvent::Options { effort: self.effort.clone().or_else(|| self.spec.effort.clone()), context: self.spec.context.clone() });
    }

    /// GitHub Copilot keeps a running credit total (AIC) in its session log, not in ACP. Read it
    /// once the log has caught up with the turn.
    fn refresh_credits(&self, after_turn: bool) {
        if self.spec.agent != AgentKind::Copilot {
            return;
        }
        let Some(sid) = self.session_id.clone() else { return };
        let (em, usage) = (self.em.clone(), self.usage.clone());
        tokio::spawn(async move {
            if after_turn {
                tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            }
            let Some(credits) = tokio::task::spawn_blocking(move || copilot_credits(&sid)).await.ok().flatten() else { return };
            let snapshot = {
                let mut c = usage.lock().unwrap_or_else(|e| e.into_inner());
                if c.credits == Some(credits) {
                    return;
                }
                c.credits = Some(credits);
                *c
            };
            em.emit(SessionEvent::Context { usage: snapshot });
        });
    }

    fn on_tool(&mut self, u: &Value, create: bool, target: Option<String>) {
        let Some(id) = u["toolCallId"].as_str().map(str::to_string) else { return };
        if u["title"].as_str().is_some_and(|t| t.contains(crate::ask_mcp::TOOL)) {
            self.ask_tools.insert(id.clone());
        }
        if self.ask_tools.contains(&id) {
            return;
        }
        let mut call = self.tools.remove(&id).unwrap_or_else(|| ToolCall {
            id: id.clone(),
            name: String::new(),
            title: "Tool".into(),
            kind: ToolKind::Other,
            status: ToolStatus::Pending,
            input: None,
            output: None,
            diffs: vec![],
            locations: vec![],
        });
        let _ = create;
        if let Some(t) = &target {
            self.tool_targets.insert(id.clone(), t.clone());
            if create {
                let stats = self.sub_stats.entry(t.clone()).or_default();
                stats.0 += 1;
                stats.1 = u["title"].as_str().map(str::to_string);
                let (tool_uses, last_tool) = stats.clone();
                self.em.emit(SessionEvent::SubagentProgress { tool_use_id: t.clone(), tokens: 0, tool_uses, last_tool });
            }
        }
        if let Some(t) = u["title"].as_str() {
            call.title = t.to_string();
            if call.name.is_empty() {
                call.name = t.to_string();
            }
        }
        if let Some(k) = u["kind"].as_str() {
            call.kind = acp_kind(k);
        }
        if let Some(s) = u["status"].as_str() {
            call.status = match s {
                "pending" => ToolStatus::Pending,
                "in_progress" => ToolStatus::Running,
                "completed" => ToolStatus::Completed,
                "failed" => ToolStatus::Failed,
                _ => call.status,
            };
        }
        if let Some(raw) = u.get("rawInput").filter(|r| r.is_object()) {
            let (_, detail) = crate::summarize_input(&call.name, raw);
            if detail.is_some() {
                call.input = detail;
            }
        }
        if let Some(locs) = u["locations"].as_array() {
            call.locations = locs.iter().filter_map(|l| l["path"].as_str().map(PathBuf::from)).collect();
            if call.input.is_none() {
                call.input = call.locations.first().map(|p| p.display().to_string());
            }
        }
        let mut images: Vec<Entry> = vec![];
        if let Some(content) = u["content"].as_array() {
            let mut text = String::new();
            for c in content {
                match c["type"].as_str() {
                    Some("diff") => {
                        let diff = file_diff(c);
                        call.diffs.retain(|d| d.path != diff.path);
                        call.diffs.push(diff);
                    }
                    Some("content") => {
                        if let Some(entry) = image_entry(&c["content"]) {
                            images.push(entry);
                            continue;
                        }
                        let t = content_text(&c["content"]);
                        if !t.is_empty() {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(&t);
                        }
                    }
                    _ => {}
                }
            }
            if !text.is_empty() {
                call.output = Some(truncate(&text, 20_000));
            }
        }
        if let Some(raw) = u.get("rawOutput").filter(|r| !r.is_null())
            && call.output.is_none() {
                let s = raw.get("output").and_then(|o| o.as_str()).map(str::to_string).unwrap_or_else(|| {
                    raw.as_str().map(str::to_string).unwrap_or_else(|| serde_json::to_string_pretty(raw).unwrap_or_default())
                });
                call.output = Some(truncate(&s, 20_000));
            }
        // A tool call that starts an agent is a subagent: give it its own session.
        if target.is_none()
            && let Some(raw) = u.get("rawInput").filter(|r| r["agent_type"].is_string())
            && self.spawned.insert(id.clone())
        {
            self.unbound.push(id.clone());
            self.em.emit(SessionEvent::SubagentStarted {
                tool_use_id: id.clone(),
                task_id: None,
                title: raw["description"].as_str().or(raw["name"].as_str()).unwrap_or("Subagent").to_string(),
                agent_type: raw["agent_type"].as_str().map(str::to_string),
                prompt: raw["prompt"].as_str().map(str::to_string),
                background: raw["mode"].as_str() == Some("background"),
            });
        }
        self.out(&target, SessionEvent::Upsert { entry: Entry::Tool(call.clone()) });
        for entry in images {
            self.out(&target, SessionEvent::Upsert { entry });
        }
        if self.spawned.contains(&id) && matches!(call.status, ToolStatus::Completed | ToolStatus::Failed) {
            let status = if call.status == ToolStatus::Failed { "failed" } else { "completed" };
            self.em.emit(SessionEvent::SubagentFinished { tool_use_id: id.clone(), status: status.to_string() });
        }
        if !matches!(call.status, ToolStatus::Completed | ToolStatus::Failed) {
            self.tools.insert(id, call);
        }
        // The last running tool call finished: this is where a steering message cuts in.
        if self.steer_pending && self.tools.is_empty() {
            self.cancel_turn();
        }
    }

    async fn on_request(&mut self, id: Value, v: &Value) {
        let method = v["method"].as_str().unwrap_or("");
        let p = &v["params"];
        match method {
            "session/request_permission" => self.on_permission(id, p),
            "fs/read_text_file" => {
                let path = PathBuf::from(p["path"].as_str().unwrap_or(""));
                match tokio::fs::read_to_string(&path).await {
                    Ok(content) => {
                        let line = p["line"].as_u64().map(|l| l.saturating_sub(1) as usize);
                        let limit = p["limit"].as_u64().map(|l| l as usize);
                        let content = match (line, limit) {
                            (None, None) => content,
                            (l, n) => content.lines().skip(l.unwrap_or(0)).take(n.unwrap_or(usize::MAX)).collect::<Vec<_>>().join("\n"),
                        };
                        self.reply(&id, json!({"content": content}));
                    }
                    Err(e) => self.reply_err(&id, -32002, &format!("{}: {e}", path.display())),
                }
            }
            "fs/write_text_file" => {
                let path = PathBuf::from(p["path"].as_str().unwrap_or(""));
                let content = p["content"].as_str().unwrap_or("");
                if let Some(dir) = path.parent() {
                    let _ = tokio::fs::create_dir_all(dir).await;
                }
                match tokio::fs::write(&path, content).await {
                    Ok(()) => self.reply(&id, Value::Null),
                    Err(e) => self.reply_err(&id, -32603, &format!("{}: {e}", path.display())),
                }
            }
            "session/elicitation" | "elicitation/create" => self.on_elicitation(id, p),
            _ => self.reply_err(&id, -32601, &format!("Mothr doesn't implement {method}")),
        }
    }

    fn on_permission(&mut self, id: Value, p: &Value) {
        let tc = &p["toolCall"];
        let options: Vec<PermissionOption> = p["options"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|o| {
                        Some(PermissionOption {
                            id: o["optionId"].as_str()?.to_string(),
                            label: o["name"].as_str().unwrap_or("Option").to_string(),
                            kind: match o["kind"].as_str() {
                                Some("allow_always") => PermissionOptionKind::AllowAlways,
                                Some("reject_once") => PermissionOptionKind::RejectOnce,
                                Some("reject_always") => PermissionOptionKind::RejectAlways,
                                _ => PermissionOptionKind::AllowOnce,
                            },
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let tool_id = tc["toolCallId"].as_str().map(str::to_string);
        let target = tool_id.as_ref().and_then(|t| self.tool_targets.get(t)).cloned();
        let known = tool_id.as_ref().and_then(|t| self.tools.get(t));
        let kind = tc["kind"].as_str().map(acp_kind).or(known.map(|k| k.kind)).unwrap_or(ToolKind::Other);
        let title = tc["title"].as_str().map(str::to_string).or(known.map(|k| k.title.clone())).unwrap_or_else(|| "Permission requested".into());
        let detail = tc
            .get("rawInput")
            .filter(|r| r.is_object())
            .and_then(|r| crate::summarize_input(&title, r).1)
            .or(known.and_then(|k| k.input.clone()));

        // Mothr's own question tool is always allowed; the question card is the prompt.
        let is_ask = title.contains(crate::ask_mcp::TOOL) || tool_id.as_ref().is_some_and(|t| self.ask_tools.contains(t));
        // Otherwise Mothr's policy can answer without bothering the user.
        let decision = if is_ask { AutoDecision::Allow } else { policy::decide(self.mode, kind) };
        let pairs: Vec<(String, PermissionOptionKind)> = options.iter().map(|o| (o.id.clone(), o.kind)).collect();
        if decision != AutoDecision::Ask
            && let Some(opt) = policy::pick_option(decision, &pairs) {
                self.reply(&id, json!({"outcome": {"outcome": "selected", "optionId": opt}}));
                return;
            }
        // The request usually carries the proposed edit; otherwise it was announced earlier.
        let proposed: Vec<FileDiff> = tc["content"].as_array().into_iter().flatten().filter(|c| c["type"] == "diff").map(file_diff).collect();
        let diffs = if proposed.is_empty() { known.map(|k| k.diffs.clone()).unwrap_or_default() } else { proposed };
        let entry_id = new_entry_id();
        self.permissions.insert(entry_id.clone(), PendingPermission { rpc_id: id, question: false, ask: None });
        self.out(
            &target,
            SessionEvent::Upsert {
                entry: Entry::Permission(PermissionRequest { id: entry_id, tool_call_id: tool_id, title, detail, kind, options, resolved: None, diffs }),
            },
        );
    }

    fn mcp_servers(&self) -> Value {
        json!(self.ask_server.iter().collect::<Vec<_>>())
    }

    /// The agent called Mothr's question tool: show a question card and hold the call open.
    fn on_ask(&mut self, req: crate::ask_mcp::AskRequest) {
        let entry_id = new_entry_id();
        self.permissions.insert(entry_id.clone(), PendingPermission { rpc_id: Value::Null, question: true, ask: Some(req.reply) });
        self.em.emit(SessionEvent::Upsert { entry: Entry::Question(QuestionRequest { id: entry_id, questions: req.questions, answers: None }) });
    }

    /// Best-effort support for agent questions sent as elicitation requests.
    fn on_elicitation(&mut self, id: Value, p: &Value) {
        let message = p["message"].as_str().unwrap_or("The agent has a question").to_string();
        let mut questions = crate::claude::parse_questions(p);
        if questions.is_empty() {
            let props = p["requestedSchema"]["properties"].as_object();
            questions = props
                .map(|props| {
                    props
                        .iter()
                        .map(|(k, s)| Question {
                            header: Some(k.clone()),
                            question: s["description"].as_str().or(s["title"].as_str()).unwrap_or(&message).to_string(),
                            options: s["enum"]
                                .as_array()
                                .map(|e| e.iter().filter_map(|x| x.as_str()).map(|l| QuestionOption { label: l.into(), description: None }).collect())
                                .unwrap_or_default(),
                            multi_select: s["type"] == "array",
                        })
                        .collect()
                })
                .unwrap_or_else(|| vec![Question { header: None, question: message.clone(), options: vec![], multi_select: false }]);
        }
        let entry_id = new_entry_id();
        self.permissions.insert(entry_id.clone(), PendingPermission { rpc_id: id, question: true, ask: None });
        self.em.emit(SessionEvent::Upsert { entry: Entry::Question(QuestionRequest { id: entry_id, questions, answers: None }) });
    }
}

/// An ACP `diff` content block. ACP sends whole files, so no line offset applies.
fn file_diff(c: &Value) -> FileDiff {
    FileDiff {
        path: PathBuf::from(c["path"].as_str().unwrap_or("")),
        old_text: c["oldText"].as_str().map(str::to_string),
        new_text: c["newText"].as_str().unwrap_or("").to_string(),
        start_line: None,
    }
}

fn acp_kind(k: &str) -> ToolKind {
    match k {
        "read" => ToolKind::Read,
        "edit" => ToolKind::Edit,
        "delete" => ToolKind::Delete,
        "move" => ToolKind::Move,
        "search" => ToolKind::Search,
        "execute" => ToolKind::Execute,
        "think" => ToolKind::Think,
        "fetch" => ToolKind::Fetch,
        "switch_mode" => ToolKind::Other,
        _ => ToolKind::Other,
    }
}

/// An [`Entry::Image`] from an ACP `image` content block (`data` + `mimeType`), saved to disk.
/// The id derives from the stored file, so a repeated update doesn't duplicate the picture.
fn image_entry(c: &Value) -> Option<Entry> {
    if c["type"] != "image" {
        return None;
    }
    let mime = c["mimeType"].as_str().unwrap_or("image/png");
    match save_image_base64(c["data"].as_str()?, mime) {
        Ok(path) => {
            let id = format!("img-{}", path.file_stem()?.to_string_lossy());
            Some(Entry::Image { id, path, mime: Some(mime.to_string()) })
        }
        Err(e) => {
            tracing::warn!("couldn't store agent image: {e}");
            None
        }
    }
}

fn content_text(c: &Value) -> String {
    match c["type"].as_str() {
        Some("text") => c["text"].as_str().unwrap_or("").to_string(),
        Some("resource_link") => format!("[{}]({})", c["name"].as_str().unwrap_or("link"), c["uri"].as_str().unwrap_or("")),
        Some("resource") => c["resource"]["text"].as_str().unwrap_or("").to_string(),
        _ => c.as_str().unwrap_or("").to_string(),
    }
}


/// The tool call that spawned a Copilot subagent, from its session log (`subagent.started`).
fn copilot_subagent_tool(session_id: &str, agent_id: &str) -> Option<String> {
    let home = std::env::var_os("COPILOT_HOME").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".copilot")))?;
    subagent_tool_in(&home, session_id, agent_id)
}

fn subagent_tool_in(home: &std::path::Path, session_id: &str, agent_id: &str) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(home.join("session-state").join(session_id).join("events.jsonl")).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(2 << 20))).ok()?;
    let mut bytes = Vec::new();
    (&mut f).take(2 << 20).read_to_end(&mut bytes).ok()?;
    String::from_utf8_lossy(&bytes).lines().rev().filter(|l| l.contains("\"subagent.started\"") && l.contains(agent_id)).find_map(|l| {
        let v: Value = serde_json::from_str(l).ok()?;
        (v["agentId"] == agent_id).then(|| v["data"]["toolCallId"].as_str().map(str::to_string)).flatten()
    })
}

/// Credits (AIC) a Copilot session has used: the last `session.usage_checkpoint` (written when a
/// turn ends) plus every `session.usage_record` logged since, one per model call, so the total
/// keeps climbing while a turn is still running.
fn copilot_credits(session_id: &str) -> Option<f64> {
    let home = std::env::var_os("COPILOT_HOME").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".copilot")))?;
    copilot_credits_in(&home, session_id)
}

fn copilot_credits_in(home: &std::path::Path, session_id: &str) -> Option<f64> {
    use std::io::{Read, Seek, SeekFrom};
    let path = home.join("session-state").join(session_id).join("events.jsonl");
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    // Polled every second or so during a turn, so read a small tail first and only widen it when
    // it holds no checkpoint (a long turn without one).
    for window in [1u64 << 20, 16 << 20] {
        f.seek(SeekFrom::Start(len.saturating_sub(window))).ok()?;
        let mut bytes = Vec::new();
        (&mut f).take(window).read_to_end(&mut bytes).ok()?;
        let tail = String::from_utf8_lossy(&bytes);
        let mut since = 0.0;
        let mut seen = false;
        for l in tail.lines().rev() {
            if l.contains("\"session.usage_checkpoint\"") {
                if let Some(total) = serde_json::from_str::<Value>(l).ok().and_then(|v| v["data"]["totalNanoAiu"].as_f64()) {
                    return Some(total / 1e9 + since);
                }
            } else if l.contains("\"session.usage_record\"")
                && let Some(n) = serde_json::from_str::<Value>(l).ok().and_then(|v| v["data"]["usage"]["copilotUsage"]["totalNanoAiu"].as_f64())
            {
                since += n / 1e9;
                seen = true;
            }
        }
        // No checkpoint in reach: with the whole log in view, the records are the total.
        if len <= window {
            return seen.then_some(since);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subagent_is_matched_to_the_tool_call_that_spawned_it() {
        let home = std::env::temp_dir().join(format!("mothr-copilot-sub-{}", std::process::id()));
        let dir = home.join("session-state").join("s1");
        std::fs::create_dir_all(&dir).unwrap();
        let log = [
            r#"{"type":"subagent.started","data":{"toolCallId":"toolu_a"},"agentId":"agent-1"}"#,
            r#"{"type":"subagent.started","data":{"toolCallId":"toolu_b"},"agentId":"agent-2"}"#,
        ]
        .join("\n");
        std::fs::write(dir.join("events.jsonl"), log).unwrap();
        assert_eq!(subagent_tool_in(&home, "s1", "agent-2").as_deref(), Some("toolu_b"));
        assert_eq!(subagent_tool_in(&home, "s1", "agent-9"), None);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn credits_come_from_the_last_usage_checkpoint() {
        let home = std::env::temp_dir().join(format!("mothr-copilot-test-{}", std::process::id()));
        let dir = home.join("session-state").join("abc");
        std::fs::create_dir_all(&dir).unwrap();
        let log = [
            r#"{"type":"session.usage_checkpoint","data":{"totalNanoAiu":1000000000}}"#,
            r#"{"type":"assistant.message","data":{}}"#,
            r#"{"type":"session.usage_checkpoint","data":{"totalNanoAiu":6244550000,"totalPremiumRequests":1}}"#,
        ]
        .join("\n");
        std::fs::write(dir.join("events.jsonl"), &log).unwrap();
        assert_eq!(copilot_credits_in(&home, "abc"), Some(6.24455));
        assert_eq!(copilot_credits_in(&home, "missing"), None);
        // Records written after the checkpoint (a turn in flight) are added on top.
        let live = format!(
            "{log}\n{}\n{}",
            r#"{"type":"session.usage_record","data":{"usage":{"copilotUsage":{"totalNanoAiu":500000000}}}}"#,
            r#"{"type":"session.usage_record","data":{"usage":{"copilotUsage":{"totalNanoAiu":250000000}}}}"#
        );
        std::fs::write(dir.join("events.jsonl"), live).unwrap();
        assert_eq!(copilot_credits_in(&home, "abc"), Some(6.99455));
        // Before any checkpoint exists the records alone are the total.
        std::fs::write(dir.join("events.jsonl"), r#"{"type":"session.usage_record","data":{"usage":{"copilotUsage":{"totalNanoAiu":2000000000}}}}"#).unwrap();
        assert_eq!(copilot_credits_in(&home, "abc"), Some(2.0));
        let _ = std::fs::remove_dir_all(home);
    }
}
