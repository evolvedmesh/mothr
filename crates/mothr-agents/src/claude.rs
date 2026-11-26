//! Claude Code driver over the native `--input-format stream-json` control protocol.

use crate::policy::{self, AutoDecision};
use crate::{edits, summarize_input, CommandTx, Emitter, OutputTx, SessionSpec};
use serde_json::{json, Value};
use mothr_core::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

fn claude_mode(m: PermissionMode) -> &'static str {
    match m {
        PermissionMode::Ask => "default",
        PermissionMode::AcceptEdits => "acceptEdits",
        PermissionMode::Auto => "auto",
        PermissionMode::Plan => "plan",
        PermissionMode::AllowAll => "bypassPermissions",
    }
}

/// The mode a permission suggestion would switch the session to.
fn suggested_mode(suggestions: &Value) -> Option<PermissionMode> {
    suggestions.as_array()?.iter().find(|s| s["type"] == "setMode").and_then(|s| mode_from_claude(s["mode"].as_str()?))
}

fn mode_from_claude(m: &str) -> Option<PermissionMode> {
    Some(match m {
        "default" | "manual" => PermissionMode::Ask,
        "acceptEdits" => PermissionMode::AcceptEdits,
        "auto" | "dontAsk" => PermissionMode::Auto,
        "plan" => PermissionMode::Plan,
        "bypassPermissions" => PermissionMode::AllowAll,
        _ => return None,
    })
}

enum Pending {
    Init,
    /// `get_context_usage`: how full the window is and how big it is.
    ContextUsage,
    SetMode(PermissionMode),
    SetModel(String),
    SetEffort(String),
    RemoteControl(bool),
    Rewind,
    Other,
}

/// Where an event belongs: `None` = the orchestrator chat, `Some(tool_use_id)` = a subagent.
type Target = Option<String>;

/// An inbound `can_use_tool` request waiting on the user.
struct Incoming {
    target: Target,
    request_id: String,
    tool_name: String,
    input: Value,
    suggestions: Value,
    questions: Vec<Question>,
}

#[derive(Clone, Copy, PartialEq)]
enum BlockKind {
    Text,
    Thinking,
}

struct Driver {
    em: Emitter,
    spec: SessionSpec,
    writer: mpsc::UnboundedSender<String>,
    next_req: u64,
    pending: HashMap<String, Pending>,
    incoming: HashMap<EntryId, Incoming>,
    /// msg id → streamed blocks (stream index, kind, entry id, confirmed by full message)
    blocks: HashMap<String, Vec<(u64, BlockKind, EntryId, bool)>>,
    current_msg: HashMap<Target, String>,
    tools: HashMap<String, (ToolCall, Target)>,
    /// task_id → tool_use_id of the spawning Agent/Task call.
    tasks: HashMap<String, String>,
    /// Our user entries awaiting the CLI's replay (to learn their message uuids).
    pending_user: std::collections::VecDeque<(EntryId, String)>,
    /// uuid of the latest top-level message (resume point for edits).
    last_uuid: Option<String>,
    turn_started: Option<Instant>,
    turn: u32,
    model: Option<String>,
    context_window: u64,
    last_context: u64,
    /// What the last `Context` event said, so partial updates keep the rest (cost, totals).
    usage: ContextUsage,
    mode: PermissionMode,
    working: bool,
    /// The CLI's own commands and skills; a library skill of the same name gives way to them.
    native: HashSet<String>,
}

pub async fn run(program: PathBuf, spec: SessionSpec, mut commands: mpsc::UnboundedReceiver<SessionCommand>, out: OutputTx) {
    let em = Emitter::new(spec.id, out);
    let mut cmd = Command::new(&program);
    mothr_core::process::no_window(cmd.as_std_mut());
    cmd.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-prompt-tool",
        "stdio",
        "--allow-dangerously-skip-permissions",
        "--replay-user-messages",
        "--permission-mode",
        claude_mode(spec.mode),
    ]);
    if let Some(m) = &spec.model {
        cmd.args(["--model", &model_with_context(m, spec.context.as_deref())]);
    }
    if let Some(e) = &spec.effort {
        cmd.args(["--effort", e]);
    }
    if spec.context.as_deref() == Some("200k") {
        // The Claude 5 models default to a 1M window; this keeps the session at 200k.
        cmd.env("CLAUDE_CODE_DISABLE_1M_CONTEXT", "1");
    }
    if let Some(r) = &spec.resume {
        cmd.args(["--resume", r]);
        if let Some(at) = &spec.resume_at {
            cmd.args(["--resume-session-at", at, "--fork-session"]);
        }
    }
    let system_prompt = [spec.system_prompt.as_deref(), spec.skills_note.as_deref()].into_iter().flatten().collect::<Vec<_>>().join("\n\n");
    if !system_prompt.is_empty() {
        cmd.args(["--append-system-prompt", &system_prompt]);
    }
    // The chat's other projects: Claude Code may read and edit files in them too.
    for dir in &spec.extra_dirs {
        cmd.arg("--add-dir").arg(dir);
    }
    cmd.current_dir(&spec.cwd)
        .env("CLAUDE_CODE_ENTRYPOINT", "sdk-mothr")
        // File checkpoints make "edit & resend" able to revert the agent's file changes.
        .env("CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            em.emit(SessionEvent::Status { status: SessionStatus::Error(format!("Failed to start Claude Code: {e}")) });
            em.exited(Some(e.to_string()));
            return;
        }
    };
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");

    let (wtx, mut wrx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(line) = wrx.recv().await {
            if stdin.write_all(line.as_bytes()).await.is_err() || stdin.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = stdin.flush().await;
        }
    });

    let stderr_tail = std::sync::Arc::new(parking_lot_lite::Mutex::new(String::new()));
    {
        let tail = stderr_tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                tracing::debug!(target: "claude", "{l}");
                let mut t = tail.lock();
                t.push_str(&l);
                t.push('\n');
                if t.len() > 4000 {
                    let cut = t.len() - 4000;
                    t.drain(..cut);
                }
            }
        });
    }

    let mut d = Driver {
        em: em.clone(),
        mode: spec.mode,
        spec,
        writer: wtx,
        next_req: 0,
        pending: HashMap::new(),
        incoming: HashMap::new(),
        blocks: HashMap::new(),
        current_msg: HashMap::new(),
        tasks: HashMap::new(),
        pending_user: std::collections::VecDeque::new(),
        last_uuid: None,
        tools: HashMap::new(),
        turn_started: None,
        turn: 0,
        model: None,
        context_window: 0,
        last_context: 0,
        usage: ContextUsage::default(),
        working: false,
        native: HashSet::new(),
    };
    d.control(json!({"subtype": "initialize", "hooks": null}), Pending::Init);

    let mut lines = BufReader::new(stdout).lines();
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(l)) if !l.trim().is_empty() => match parse_traced(&l) {
                    Ok(v) => d.on_message(v),
                    Err(_) => tracing::debug!(target: "claude", "non-json: {l}"),
                },
                Ok(Some(_)) => {}
                _ => break,
            },
            cmd = commands.recv() => match cmd {
                Some(SessionCommand::Shutdown) | None => {
                    let _ = child.start_kill();
                    break;
                }
                Some(c) => d.on_command(c),
            }
        }
    }
    let status = child.wait().await.ok();
    let err = match status {
        Some(s) if s.success() || format!("{s}").contains("signal: 9") => None,
        Some(s) => {
            let tail = stderr_tail.lock().trim().to_string();
            Some(if tail.is_empty() { format!("Claude Code exited ({s})") } else { tail })
        }
        None => None,
    };
    if let Some(e) = &err
        && !e.contains("signal: 9") {
            em.notice(NoticeLevel::Error, e.clone());
        }
    em.emit(SessionEvent::Status { status: SessionStatus::Ended });
    em.exited(err);
}

/// Tiny mutex shim to avoid an extra dependency for one stderr buffer.
mod parking_lot_lite {
    pub struct Mutex<T>(std::sync::Mutex<T>);
    impl<T> Mutex<T> {
        pub fn new(t: T) -> Self {
            Self(std::sync::Mutex::new(t))
        }
        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
            self.0.lock().unwrap_or_else(|e| e.into_inner())
        }
    }
}

impl Driver {
    fn out(&self, target: &Target, e: SessionEvent) {
        match target {
            None => self.em.emit(e),
            Some(t) => self.em.sub(t.clone(), e),
        }
    }

    fn send(&self, v: Value) {
        let _ = self.writer.send(v.to_string());
    }

    fn control(&mut self, request: Value, kind: Pending) {
        self.next_req += 1;
        let id = format!("mothr-{}", self.next_req);
        self.pending.insert(id.clone(), kind);
        self.send(json!({"type": "control_request", "request_id": id, "request": request}));
    }

    fn respond(&self, request_id: &str, response: Value) {
        self.send(json!({"type": "control_response", "response": {"subtype": "success", "request_id": request_id, "response": response}}));
    }

    fn respond_error(&self, request_id: &str, error: &str) {
        self.send(json!({"type": "control_response", "response": {"subtype": "error", "request_id": request_id, "error": error}}));
    }

    fn set_working(&mut self, w: bool) {
        if self.working != w {
            self.working = w;
            self.em.emit(SessionEvent::Status { status: if w { SessionStatus::Working } else { SessionStatus::Idle } });
        }
    }

    // ---------------------------------------------------------------- commands

    fn on_command(&mut self, c: SessionCommand) {
        match c {
            SessionCommand::Prompt { text, attachments, delivery } => {
                // `/name` for a library skill sends the skill itself; the chat shows what was typed.
                let sent = expand_prompt(&text, &self.spec.skills, &self.native).unwrap_or_else(|| text.clone());
                let mut content = vec![json!({"type": "text", "text": sent})];
                for a in &attachments {
                    if let Some(img) = image_block(&a.path) {
                        content.push(img);
                    } else {
                        content[0] = json!({"type": "text", "text": format!("{}\n\n@{}", content[0]["text"].as_str().unwrap_or(""), a.path.display())});
                    }
                }
                let entry_id = new_entry_id();
                self.em.emit(SessionEvent::Upsert {
                    entry: Entry::User { id: entry_id.clone(), text: text.clone(), attachments, at: chrono::Utc::now(), agent_ref: None, prev_ref: None },
                });
                self.pending_user.push_back((entry_id, sent));
                if delivery == Delivery::Interrupt && self.working {
                    // Stop the running turn first; the CLI processes our message right after.
                    self.control(json!({"subtype": "interrupt"}), Pending::Other);
                }
                self.send(json!({
                    "type": "user",
                    "message": {"role": "user", "content": content},
                    "parent_tool_use_id": null,
                    "session_id": "",
                }));
                if !self.working {
                    self.turn += 1;
                    self.turn_started = Some(Instant::now());
                }
                self.set_working(true);
            }
            SessionCommand::Cancel => {
                self.control(json!({"subtype": "interrupt"}), Pending::Other);
                // Reject anything still waiting on the user so the turn can wind down.
                let ids: Vec<EntryId> = self.incoming.keys().cloned().collect();
                for id in ids {
                    if let Some(inc) = self.incoming.remove(&id) {
                        self.respond(&inc.request_id, json!({"behavior": "deny", "message": "The user interrupted.", "interrupt": true}));
                        self.out(&inc.target, SessionEvent::Resolved { entry_id: id, choice: Some("cancelled".into()), answers: None });
                    }
                }
            }
            SessionCommand::SetMode { mode } => {
                self.control(json!({"subtype": "set_permission_mode", "mode": claude_mode(mode)}), Pending::SetMode(mode));
            }
            SessionCommand::SetModel { model } => {
                let wire = model_with_context(&model, self.spec.context.as_deref());
                self.control(json!({"subtype": "set_model", "model": wire}), Pending::SetModel(model));
            }
            SessionCommand::SetEffort { effort } => {
                self.control(json!({"subtype": "apply_flag_settings", "settings": {"effortLevel": effort}}), Pending::SetEffort(effort));
            }
            // The context window is fixed when the process starts; the hub restarts the chat for it.
            SessionCommand::SetContext { context } => {
                self.spec.context = Some(context);
                self.emit_options();
            }
            SessionCommand::RespondPermission { entry_id, option_id } => self.resolve_permission(entry_id, option_id),
            SessionCommand::AnswerQuestion { entry_id, answers } => {
                if let Some(inc) = self.incoming.remove(&entry_id) {
                    let mut input = inc.input.clone();
                    let mut map = serde_json::Map::new();
                    for (q, a) in inc.questions.iter().zip(answers.iter()) {
                        map.insert(q.question.clone(), Value::String(a.join(", ")));
                    }
                    if let Some(obj) = input.as_object_mut() {
                        obj.insert("answers".into(), Value::Object(map));
                    }
                    self.respond(&inc.request_id, json!({"behavior": "allow", "updatedInput": input}));
                    self.out(&inc.target, SessionEvent::Resolved { entry_id, choice: None, answers: Some(answers) });
                }
            }
            SessionCommand::SetRemoteControl { enabled } => {
                self.em.emit(SessionEvent::RemoteControl {
                    state: RemoteControlState { enabled, url: None, pending: true, error: None },
                });
                // No `name`: Claude Code titles the remote session itself and follows the title it
                // gives the chat. Mothr's own title is only a placeholder ("New chat") until then.
                self.control(json!({"subtype": "remote_control", "enabled": enabled}), Pending::RemoteControl(enabled));
            }
            SessionCommand::RewindFiles { user_message_id } => {
                self.control(json!({"subtype": "rewind_files", "user_message_id": user_message_id}), Pending::Rewind);
            }
            SessionCommand::StopSubagent { tool_use_id } => {
                if let Some(task_id) = self.tasks.iter().find(|(_, t)| **t == tool_use_id).map(|(k, _)| k.clone()) {
                    self.control(json!({"subtype": "stop_task", "task_id": task_id}), Pending::Other);
                }
            }
            // Folders are given when Claude Code starts; the hub restarts the chat to change them.
            SessionCommand::SetExtraProjects { .. } => {}
            SessionCommand::SetSkills { skills } => self.spec.skills = skills,
            SessionCommand::Shutdown => {}
        }
    }

    fn resolve_permission(&mut self, entry_id: EntryId, option_id: String) {
        let Some(inc) = self.incoming.remove(&entry_id) else { return };
        let response = match option_id.as_str() {
            "allow" => json!({"behavior": "allow", "updatedInput": inc.input}),
            "allow_always" => {
                // A suggestion can switch the session's mode ("accept all edits"): show it.
                if let Some(mode) = suggested_mode(&inc.suggestions) {
                    self.mode_changed(mode);
                }
                json!({"behavior": "allow", "updatedInput": inc.input, "updatedPermissions": inc.suggestions})
            }
            "plan_auto" => {
                self.control(json!({"subtype": "set_permission_mode", "mode": "acceptEdits"}), Pending::SetMode(PermissionMode::AcceptEdits));
                json!({"behavior": "allow", "updatedInput": inc.input})
            }
            "plan_manual" => {
                self.control(json!({"subtype": "set_permission_mode", "mode": "default"}), Pending::SetMode(PermissionMode::Ask));
                json!({"behavior": "allow", "updatedInput": inc.input})
            }
            "plan_keep" => json!({"behavior": "deny", "message": "The user wants to keep planning. Refine the plan based on their next message."}),
            _ => json!({"behavior": "deny", "message": format!("The user denied {}.", inc.tool_name)}),
        };
        self.respond(&inc.request_id, response);
        self.out(&inc.target, SessionEvent::Resolved { entry_id, choice: Some(option_id), answers: None });
    }

    /// The session's mode changed. Claude Code applies it to the next tool call, but a request
    /// already waiting on the user stays open; answer the ones the new mode lets through, as
    /// Claude Code would have.
    fn mode_changed(&mut self, mode: PermissionMode) {
        self.mode = mode;
        self.em.emit(SessionEvent::Mode { mode });
        let allowed: Vec<EntryId> = self
            .incoming
            .iter()
            .filter(|(_, i)| i.questions.is_empty() && !matches!(i.tool_name.as_str(), "AskUserQuestion" | "ExitPlanMode"))
            .filter(|(_, i)| policy::decide(mode, ToolKind::from_tool_name(&i.tool_name)) == AutoDecision::Allow)
            .map(|(id, _)| id.clone())
            .collect();
        for id in allowed {
            self.resolve_permission(id, "allow".into());
        }
    }

    // ---------------------------------------------------------------- inbound

    fn on_message(&mut self, v: Value) {
        match v["type"].as_str().unwrap_or("") {
            "control_response" => self.on_control_response(&v["response"]),
            "control_request" => self.on_control_request(&v),
            "control_cancel_request" => {
                let rid = v["request_id"].as_str().unwrap_or("");
                if let Some(eid) = self.incoming.iter().find(|(_, i)| i.request_id == rid).map(|(k, _)| k.clone())
                    && let Some(inc) = self.incoming.remove(&eid) {
                        self.out(&inc.target, SessionEvent::Resolved { entry_id: eid, choice: Some("cancelled".into()), answers: None });
                    }
            }
            "system" => self.on_system(&v),
            "stream_event" => {
                let target = v["parent_tool_use_id"].as_str().map(str::to_string);
                self.on_stream(&v["event"], target);
            }
            "assistant" => {
                let target = v["parent_tool_use_id"].as_str().map(str::to_string);
                self.on_assistant(&v["message"], target.clone());
                if target.is_none()
                    && let Some(u) = v["uuid"].as_str() {
                        self.last_uuid = Some(u.to_string());
                    }
            }
            "user" => self.on_user(&v),
            "result" => self.on_result(&v),
            "rate_limit_event" => {
                if let Some(u) = mothr_usage::claude_rate_limit_event(&v) {
                    self.em.usage(u);
                }
            }
            _ => {}
        }
    }

    fn on_control_response(&mut self, r: &Value) {
        let rid = r["request_id"].as_str().unwrap_or("").to_string();
        let ok = r["subtype"] == "success";
        let body = &r["response"];
        let err = r["error"].as_str().map(str::to_string);
        match self.pending.remove(&rid) {
            Some(Pending::Init) if ok => {
                let models = body["models"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|m| {
                                let supports_effort = m["supportsEffort"].as_bool().unwrap_or(false);
                                let mut info = ModelInfo::new(
                                    m["value"].as_str()?,
                                    m["displayName"].as_str().unwrap_or(m["value"].as_str()?),
                                    m["description"].as_str().map(str::to_string),
                                );
                                info.efforts = m["supportedEffortLevels"]
                                    .as_array()
                                    .map(|l| l.iter().filter_map(|e| e.as_str().map(str::to_string)).collect())
                                    .unwrap_or_default();
                                // Models with adjustable effort are the current generation, which can run
                                // with a 1M window or be held to 200k.
                                if supports_effort {
                                    info.contexts = vec![ChoiceInfo::new("1m", "1M"), ChoiceInfo::new("200k", "200k")];
                                }
                                // Claude Code refuses auto mode on a model without it.
                                if !m["supportsAutoMode"].as_bool().unwrap_or(false) {
                                    info.unsupported_modes = vec![PermissionMode::Auto];
                                }
                                Some(info)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(m) = body["current_permission_mode"].as_str().and_then(mode_from_claude) {
                    self.mode = m;
                    self.em.emit(SessionEvent::Mode { mode: m });
                }
                self.em.emit(SessionEvent::Ready { agent_session_id: None, model: None, models, modes: vec![] });
                // The window size is known before the first message.
                self.control(json!({"subtype": "get_context_usage"}), Pending::ContextUsage);
                let commands: Vec<CommandInfo> = body["commands"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| {
                                Some(CommandInfo {
                                    name: c["name"].as_str()?.trim_start_matches('/').to_string(),
                                    description: c["description"].as_str().map(str::to_string),
                                    hint: c["argumentHint"].as_str().filter(|h| !h.is_empty()).map(str::to_string),
                                    skill: false,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if !commands.is_empty() {
                    self.native = commands.iter().map(|c| c.name.clone()).collect();
                    self.em.emit(SessionEvent::Commands { commands });
                }
            }
            Some(Pending::SetMode(m)) => {
                if ok {
                    self.mode_changed(m);
                } else {
                    self.em.notice(NoticeLevel::Warning, format!("Couldn't switch to {}: {}", m.label(), err.unwrap_or_default()));
                }
            }
            Some(Pending::ContextUsage) => {
                if ok {
                    let max = body["maxTokens"].as_u64().filter(|m| *m > 0);
                    let total = body["totalTokens"].as_u64();
                    if let Some(max) = max {
                        self.context_window = max;
                    }
                    if let Some(total) = total.filter(|t| *t > 0) {
                        self.last_context = total;
                    }
                    if max.is_some() || total.is_some() {
                        self.usage.window_tokens = self.context_window;
                        self.usage.used_tokens = self.last_context;
                        self.em.emit(SessionEvent::Context { usage: self.usage });
                    }
                }
            }
            Some(Pending::SetEffort(e)) => {
                if ok {
                    self.spec.effort = Some(e);
                    self.emit_options();
                } else {
                    self.em.notice(NoticeLevel::Warning, format!("Couldn't change the reasoning effort: {}", err.unwrap_or_default()));
                }
            }
            Some(Pending::SetModel(m)) => {
                if ok {
                    self.em.emit(SessionEvent::Model { model: m });
                    self.control(json!({"subtype": "get_context_usage"}), Pending::ContextUsage);
                } else {
                    self.em.notice(NoticeLevel::Warning, format!("Couldn't switch model: {}", err.unwrap_or_default()));
                }
            }
            Some(Pending::RemoteControl(enabled)) => {
                let state = if ok {
                    RemoteControlState { enabled, url: if enabled { find_url(body) } else { None }, pending: false, error: None }
                } else {
                    RemoteControlState { enabled: false, url: None, pending: false, error: err.or(Some("Remote Control unavailable".into())) }
                };
                self.em.emit(SessionEvent::RemoteControl { state });
            }
            Some(Pending::Init) => {
                self.em.notice(NoticeLevel::Error, format!("Claude Code failed to initialize: {}", err.unwrap_or_default()));
            }
            Some(Pending::Rewind) => {
                let detail = if ok {
                    body["filesChanged"].as_array().map(|f| format!("{} file(s) reverted", f.len())).or_else(|| body["message"].as_str().map(str::to_string))
                } else {
                    err.clone()
                };
                self.em.emit(SessionEvent::FilesRewound { ok, detail });
            }
            Some(Pending::Other) | None => {}
        }
    }

    fn on_control_request(&mut self, v: &Value) {
        let rid = v["request_id"].as_str().unwrap_or("").to_string();
        let req = &v["request"];
        match req["subtype"].as_str().unwrap_or("") {
            "can_use_tool" => {
                let tool = req["tool_name"].as_str().unwrap_or("tool").to_string();
                let input = req["input"].clone();
                let target: Target = req["agent_id"].as_str().and_then(|a| self.tasks.get(a).cloned());
                let entry_id = new_entry_id();
                let tool_call_id = req["tool_use_id"].as_str().map(str::to_string);
                if tool == "AskUserQuestion" {
                    let questions = parse_questions(&input);
                    self.incoming.insert(entry_id.clone(), Incoming {
                        target: target.clone(), request_id: rid, tool_name: tool, input, suggestions: Value::Null, questions: questions.clone(),
                    });
                    self.out(&target, SessionEvent::Upsert {
                        entry: Entry::Question(QuestionRequest { id: entry_id, questions, answers: None }),
                    });
                    return;
                }
                let diffs = edits::edit_diffs(&input);
                let (title, detail, options) = if tool == "ExitPlanMode" {
                    (
                        "Ready to start? Claude finished planning".to_string(),
                        input["plan"].as_str().map(str::to_string),
                        vec![
                            opt("plan_auto", "Yes, auto-accept edits", PermissionOptionKind::AllowAlways),
                            opt("plan_manual", "Yes, approve each edit", PermissionOptionKind::AllowOnce),
                            opt("plan_keep", "No, keep planning", PermissionOptionKind::RejectOnce),
                        ],
                    )
                } else {
                    let (t, d) = summarize_input(&tool, &input);
                    let mut options = vec![opt("allow", "Allow", PermissionOptionKind::AllowOnce)];
                    if req["permission_suggestions"].as_array().is_some_and(|a| !a.is_empty()) {
                        let label = if suggested_mode(&req["permission_suggestions"]) == Some(PermissionMode::AcceptEdits) {
                            "Accept all edits"
                        } else {
                            "Always allow"
                        };
                        options.push(opt("allow_always", label, PermissionOptionKind::AllowAlways));
                    }
                    options.push(opt("deny", "Deny", PermissionOptionKind::RejectOnce));
                    let desc = req["description"].as_str().map(str::to_string);
                    (t, d.or(desc), options)
                };
                self.incoming.insert(entry_id.clone(), Incoming {
                    target: target.clone(),
                    request_id: rid,
                    tool_name: tool.clone(),
                    input,
                    suggestions: req["permission_suggestions"].clone(),
                    questions: vec![],
                });
                self.out(&target, SessionEvent::Upsert {
                    entry: Entry::Permission(PermissionRequest {
                        id: entry_id,
                        tool_call_id,
                        title,
                        detail,
                        kind: ToolKind::from_tool_name(&tool),
                        options,
                        resolved: None,
                        diffs,
                    }),
                });
            }
            "hook_callback" => self.respond(&rid, json!({})),
            other => self.respond_error(&rid, &format!("{other} is not supported by Mothr")),
        }
    }

    fn emit_options(&self) {
        self.em.emit(SessionEvent::Options { effort: self.spec.effort.clone(), context: self.spec.context.clone() });
    }

    fn on_system(&mut self, v: &Value) {
        match v["subtype"].as_str().unwrap_or("") {
            "init" => {
                self.model = v["model"].as_str().map(str::to_string);
                if self.spec.effort.is_some() || self.spec.context.is_some() {
                    self.emit_options();
                }
                if let Some(m) = v["permissionMode"].as_str().and_then(mode_from_claude) {
                    self.mode = m;
                    self.em.emit(SessionEvent::Mode { mode: m });
                }
                self.em.emit(SessionEvent::Ready {
                    agent_session_id: v["session_id"].as_str().map(str::to_string),
                    model: self.model.clone(),
                    models: vec![],
                    modes: vec![],
                });
            }
            "status" => {
                if v["status"] == "requesting" {
                    self.set_working(true);
                }
                // Claude Code reports every mode change, its own included (leaving plan mode).
                if let Some(m) = v["permissionMode"].as_str().and_then(mode_from_claude)
                    && m != self.mode {
                        self.mode_changed(m);
                    }
            }
            "task_started" => {
                let (Some(task), Some(tu)) = (v["task_id"].as_str(), v["tool_use_id"].as_str()) else { return };
                // Only agent tasks are subagents; background shell commands are plain tool runs.
                let is_agent = v["task_type"].as_str().map(|t| t == "local_agent" || t.contains("agent")).unwrap_or(v["subagent_type"].is_string());
                if !is_agent {
                    return;
                }
                self.tasks.insert(task.to_string(), tu.to_string());
                self.em.emit(SessionEvent::SubagentStarted {
                    tool_use_id: tu.to_string(),
                    task_id: Some(task.to_string()),
                    title: v["description"].as_str().unwrap_or("Subagent").to_string(),
                    agent_type: v["subagent_type"].as_str().map(str::to_string),
                    prompt: v["prompt"].as_str().map(str::to_string),
                    background: v["is_backgrounded"].as_bool().unwrap_or(false),
                });
            }
            "task_progress" => {
                let Some(tu) = v["task_id"].as_str().and_then(|t| self.tasks.get(t).cloned()) else { return };
                self.em.emit(SessionEvent::SubagentProgress {
                    tool_use_id: tu,
                    tokens: v["usage"]["total_tokens"].as_u64().unwrap_or(0),
                    tool_uses: v["usage"]["tool_uses"].as_u64().unwrap_or(0),
                    last_tool: v["last_tool_name"].as_str().map(str::to_string),
                });
            }
            "task_notification" | "task_updated" => {
                let status = v["status"].as_str().or(v["patch"]["status"].as_str());
                let tu = v["task_id"].as_str().and_then(|t| self.tasks.get(t).cloned());
                if let (Some(status), Some(tu)) = (status, tu)
                    && matches!(status, "completed" | "failed" | "killed" | "stopped" | "cancelled") {
                        self.em.emit(SessionEvent::SubagentFinished { tool_use_id: tu, status: status.to_string() });
                    }
            }
            "compact_boundary" => self.em.notice(NoticeLevel::Info, "Conversation compacted to free up context"),
            "session_title_changed" => {
                if let Some(t) = v["title"].as_str() {
                    self.em.emit(SessionEvent::Title { title: t.to_string() });
                }
            }
            "api_retry" => {
                let msg = v["error"]["message"].as_str().or(v["message"].as_str()).unwrap_or("retrying");
                self.em.notice(NoticeLevel::Warning, format!("API error, retrying: {msg}"));
            }
            _ => {}
        }
    }

    fn on_stream(&mut self, e: &Value, target: Target) {
        match e["type"].as_str().unwrap_or("") {
            "message_start" => {
                if let Some(id) = e["message"]["id"].as_str() {
                    self.current_msg.insert(target.clone(), id.to_string());
                }
                if target.is_none() {
                    self.set_working(true);
                }
            }
            "content_block_start" => {
                let (Some(msg), Some(index)) = (self.current_msg.get(&target).cloned(), e["index"].as_u64()) else { return };
                let kind = match e["content_block"]["type"].as_str() {
                    Some("text") => BlockKind::Text,
                    Some("thinking") => BlockKind::Thinking,
                    _ => return,
                };
                let id = format!("{msg}:{index}");
                self.blocks.entry(msg).or_default().push((index, kind, id, false));
            }
            "content_block_delta" => {
                let (Some(msg), Some(index)) = (self.current_msg.get(&target), e["index"].as_u64()) else { return };
                let Some((_, kind, id, _)) = self.blocks.get(msg).and_then(|b| b.iter().find(|b| b.0 == index)) else { return };
                let delta = &e["delta"];
                let text = match delta["type"].as_str() {
                    Some("text_delta") => delta["text"].as_str(),
                    Some("thinking_delta") => delta["thinking"].as_str(),
                    _ => None,
                };
                if let Some(t) = text.filter(|t| !t.is_empty()) {
                    self.out(&target, SessionEvent::AppendText { entry_id: id.clone(), thinking: *kind == BlockKind::Thinking, delta: t.to_string() });
                }
            }
            "content_block_stop" => {
                let (Some(msg), Some(index)) = (self.current_msg.get(&target), e["index"].as_u64()) else { return };
                if let Some((_, _, id, _)) = self.blocks.get(msg).and_then(|b| b.iter().find(|b| b.0 == index)) {
                    self.out(&target, SessionEvent::StreamEnd { entry_id: id.clone() });
                }
            }
            _ => {}
        }
    }

    fn on_assistant(&mut self, m: &Value, target: Target) {
        let msg = m["id"].as_str().unwrap_or("").to_string();
        if let Some(model) = m["model"].as_str()
            && self.model.as_deref() != Some(model) {
                self.model = Some(model.to_string());
            }
        // Context = everything the model saw on this call plus what it produced.
        let u = &m["usage"];
        let ctx = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens", "output_tokens"]
            .iter()
            .filter_map(|k| u[*k].as_u64())
            .sum::<u64>();
        if ctx > 0 && target.is_none() {
            self.last_context = ctx;
            self.usage.used_tokens = ctx;
            self.usage.window_tokens = self.context_window;
            self.em.emit(SessionEvent::Context { usage: self.usage });
        }
        for (bi, block) in m["content"].as_array().into_iter().flatten().enumerate() {
            match block["type"].as_str().unwrap_or("") {
                "image" => {
                    if let Some(entry) = image_entry(block, format!("{msg}:img{bi}")) {
                        self.out(&target, SessionEvent::Upsert { entry });
                    }
                }
                t @ ("text" | "thinking") => {
                    let kind = if t == "text" { BlockKind::Text } else { BlockKind::Thinking };
                    let text = block[t].as_str().unwrap_or("").to_string();
                    let id = self
                        .blocks
                        .get_mut(&msg)
                        .and_then(|bs| bs.iter_mut().find(|b| b.1 == kind && !b.3))
                        .map(|b| {
                            b.3 = true;
                            b.2.clone()
                        })
                        .unwrap_or_else(new_entry_id);
                    if text.is_empty() {
                        continue; // redacted / summarized-away thinking
                    }
                    let entry = if kind == BlockKind::Text {
                        Entry::Assistant { id, text, streaming: false }
                    } else {
                        Entry::Thinking { id, text, streaming: false }
                    };
                    self.out(&target, SessionEvent::Upsert { entry });
                }
                "tool_use" => self.on_tool_use(block, target.clone()),
                _ => {}
            }
        }
    }

    fn on_tool_use(&mut self, block: &Value, target: Target) {
        let id = block["id"].as_str().unwrap_or("").to_string();
        let name = block["name"].as_str().unwrap_or("tool").to_string();
        let input = &block["input"];
        match name.as_str() {
            "AskUserQuestion" | "ExitPlanMode" => return,
            "TodoWrite" => {
                let items = input["todos"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|t| PlanItem {
                                content: t["content"].as_str().unwrap_or("").to_string(),
                                status: match t["status"].as_str() {
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
                    Some(t) => format!("plan-{t}"),
                };
                self.out(&target, SessionEvent::Upsert { entry: Entry::Plan { id: plan_id, items } });
                return;
            }
            _ => {}
        }
        let (title, detail) = summarize_input(&name, input);
        let diffs = edits::edit_diffs(input);
        let path = input["file_path"].as_str().map(PathBuf::from);
        let call = ToolCall {
            id: id.clone(),
            kind: ToolKind::from_tool_name(&name),
            name,
            title,
            status: ToolStatus::Running,
            input: detail,
            output: None,
            diffs,
            locations: path.into_iter().collect(),
        };
        self.tools.insert(id, (call.clone(), target.clone()));
        self.out(&target, SessionEvent::Upsert { entry: Entry::Tool(call) });
    }

    fn on_user(&mut self, v: &Value) {
        if v["parent_tool_use_id"].is_null() {
            let uuid = v["uuid"].as_str().map(str::to_string);
            let content = &v["message"]["content"];
            let is_tool_result = content.as_array().is_some_and(|a| a.iter().any(|b| b["type"] == "tool_result"));
            if !is_tool_result {
                // The CLI replaying one of our prompts: learn its uuid for edit & rewind.
                let text = match content {
                    Value::String(s) => s.clone(),
                    Value::Array(a) => a.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
                    _ => String::new(),
                };
                if let (Some(u), Some(pos)) = (&uuid, self.pending_user.iter().position(|(_, t)| text.starts_with(t.trim()) || t.starts_with(text.trim())))
                    && let Some((entry_id, _)) = self.pending_user.remove(pos) {
                        self.em.emit(SessionEvent::UserRef { entry_id, agent_ref: u.clone(), prev_ref: self.last_uuid.clone() });
                    }
            }
            if uuid.is_some() {
                self.last_uuid = uuid;
            }
        }
        for block in v["message"]["content"].as_array().into_iter().flatten() {
            if block["type"] != "tool_result" {
                continue;
            }
            let id = block["tool_use_id"].as_str().unwrap_or("");
            let Some((mut call, target)) = self.tools.remove(id) else { continue };
            let text = match &block["content"] {
                Value::String(s) => s.clone(),
                Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
                _ => String::new(),
            };
            call.status = if block["is_error"].as_bool().unwrap_or(false) { ToolStatus::Failed } else { ToolStatus::Completed };
            call.output = Some(truncate(&text, 20_000));
            // The CLI reports exactly what changed; prefer that over our preview.
            if call.status == ToolStatus::Completed
                && !call.diffs.is_empty()
                && let Some(diffs) = edits::result_diffs(&v["tool_use_result"])
                && call.locations.first().is_none_or(|p| diffs.first().is_some_and(|d| &d.path == p))
            {
                call.diffs = diffs;
            }
            self.out(&target, SessionEvent::Upsert { entry: Entry::Tool(call) });
            // Images a tool returned (screenshots, a Read of a picture, MCP image results).
            if let Value::Array(parts) = &block["content"] {
                for (i, part) in parts.iter().enumerate() {
                    if let Some(entry) = image_entry(part, format!("{id}:img{i}")) {
                        self.out(&target, SessionEvent::Upsert { entry });
                    }
                }
            }
        }
    }

    fn on_result(&mut self, v: &Value) {
        let model_usage = v["modelUsage"].as_object();
        if let Some(mu) = model_usage {
            let key = self.model.clone().unwrap_or_default();
            let entry = mu.get(&key).or_else(|| mu.values().next());
            if let Some(w) = entry.and_then(|e| e["contextWindow"].as_u64()) {
                self.context_window = w;
            }
        }
        let u = &v["usage"];
        self.usage = ContextUsage {
            used_tokens: self.last_context,
            window_tokens: self.context_window,
            cost_usd: v["total_cost_usd"].as_f64(),
            input_tokens: u["input_tokens"].as_u64().unwrap_or(0)
                + u["cache_read_input_tokens"].as_u64().unwrap_or(0)
                + u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
            output_tokens: u["output_tokens"].as_u64().unwrap_or(0),
            credits: None,
        };
        self.em.emit(SessionEvent::Context { usage: self.usage });
        // The result only carries the window size when the model reports it; ask Claude too.
        self.control(json!({"subtype": "get_context_usage"}), Pending::ContextUsage);
        // Failed tools that never got results (e.g. interrupted).
        // Only the orchestrator's own turn ended; background subagents keep running.
        let own: Vec<String> = self.tools.iter().filter(|(_, (_, t))| t.is_none()).map(|(k, _)| k.clone()).collect();
        let pending: Vec<(ToolCall, Target)> = own.into_iter().filter_map(|k| self.tools.remove(&k)).collect();
        for (mut call, target) in pending {
            call.status = ToolStatus::Failed;
            self.out(&target, SessionEvent::Upsert { entry: Entry::Tool(call) });
        }
        if v["is_error"].as_bool().unwrap_or(false) {
            let msg = v["result"].as_str().or(v["error"].as_str()).unwrap_or("The turn ended with an error");
            self.em.notice(NoticeLevel::Error, msg.to_string());
        }
        self.em.emit(SessionEvent::Upsert {
            entry: Entry::TurnEnd {
                id: new_entry_id(),
                stop_reason: v["stop_reason"].as_str().or(v["subtype"].as_str()).unwrap_or("end_turn").to_string(),
                duration_ms: v["duration_ms"].as_u64().or(self.turn_started.map(|t| t.elapsed().as_millis() as u64)),
                cost_usd: self.usage.cost_usd,
            },
        });
        self.blocks.clear();
        self.set_working(false);
    }
}

fn opt(id: &str, label: &str, kind: PermissionOptionKind) -> PermissionOption {
    PermissionOption { id: id.into(), label: label.into(), kind }
}

pub(crate) fn parse_questions(input: &Value) -> Vec<Question> {
    input["questions"]
        .as_array()
        .map(|qs| {
            qs.iter()
                .map(|q| Question {
                    header: q["header"].as_str().map(str::to_string),
                    question: q["question"].as_str().unwrap_or("").to_string(),
                    multi_select: q["multiSelect"].as_bool().or(q["multi_select"].as_bool()).unwrap_or(false),
                    options: q["options"]
                        .as_array()
                        .map(|os| {
                            os.iter()
                                .map(|o| QuestionOption {
                                    label: o["label"].as_str().or(o.as_str()).unwrap_or("").to_string(),
                                    description: o["description"].as_str().map(str::to_string),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Find the first http(s) URL anywhere in a JSON value.
fn find_url(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if s.starts_with("https://") || s.starts_with("http://") => Some(s.clone()),
        Value::Object(o) => {
            // Prefer obviously named fields.
            for k in ["url", "session_url", "connect_url", "remote_url", "web_url"] {
                if let Some(u) = o.get(k).and_then(find_url) {
                    return Some(u);
                }
            }
            o.values().find_map(find_url)
        }
        Value::Array(a) => a.iter().find_map(find_url),
        _ => None,
    }
}

/// An [`Entry::Image`] from an Anthropic `image` content block (base64 source), saved to disk.
fn image_entry(block: &Value, id: String) -> Option<Entry> {
    if block["type"] != "image" || block["source"]["type"] != "base64" {
        return None;
    }
    let mime = block["source"]["media_type"].as_str().unwrap_or("image/png");
    match save_image_base64(block["source"]["data"].as_str()?, mime) {
        Ok(path) => Some(Entry::Image { id, path, mime: Some(mime.to_string()) }),
        Err(e) => {
            tracing::warn!("couldn't store agent image: {e}");
            None
        }
    }
}

fn image_block(path: &std::path::Path) -> Option<Value> {
    use base64_lite::encode;
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let media = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    };
    let bytes = std::fs::read(path).ok()?;
    Some(json!({"type": "image", "source": {"type": "base64", "media_type": media, "data": encode(&bytes)}}))
}

pub(crate) mod base64_lite {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    pub fn encode(b: &[u8]) -> String {
        let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
        for c in b.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            s.push(T[(n >> 18) as usize & 63] as char);
            s.push(T[(n >> 12) as usize & 63] as char);
            s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
            s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
        }
        s
    }
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… ({} more bytes)", &s[..end], s.len() - end)
}

#[allow(dead_code)]
fn _assert_send(_: CommandTx) {}

/// The `--model` value for a context choice: "1m" pins the 1M window with the `[1m]` suffix.
fn model_with_context(model: &str, context: Option<&str>) -> String {
    if context == Some("1m") && !model.ends_with(']') && model != "default" {
        format!("{model}[1m]")
    } else {
        model.to_string()
    }
}

/// Append raw protocol lines to `$MOTHR_TRACE` (debugging aid).
pub(crate) fn trace(line: &str) {
    if let Some(path) = std::env::var_os("MOTHR_TRACE") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

pub(crate) fn parse_traced(line: &str) -> serde_json::Result<Value> {
    trace(line);
    serde_json::from_str(line)
}
