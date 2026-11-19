//! Run the remote server against an in-memory demo hub, to work on the web client:
//!
//! ```sh
//! cargo run -p mothr-remote --example demo            # prints the pairing URL
//! MOTHR_DEMO_TOKEN=devtoken-devtoken-devtoken cargo run -p mothr-remote --example demo
//! ```

use chrono::Utc;
use mothr_core::*;
use mothr_remote::{RemoteServer, generate_token};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

struct DemoHub {
    tx: broadcast::Sender<HubEvent>,
    sessions: Mutex<Vec<SessionState>>,
    usage: Vec<ProviderUsage>,
}

impl DemoHub {
    fn emit(self: &Arc<Self>, id: SessionId, event: SessionEvent) {
        if let Some(s) = self
            .sessions
            .lock()
            .unwrap()
            .iter_mut()
            .find(|s| s.meta.id == id)
        {
            s.apply(&event);
        }
        let _ = self.tx.send(HubEvent::Session { id, event });
    }

    fn with_meta(self: &Arc<Self>, id: SessionId, f: impl FnOnce(&mut SessionMeta)) {
        let meta = {
            let mut list = self.sessions.lock().unwrap();
            let Some(s) = list.iter_mut().find(|s| s.meta.id == id) else {
                return;
            };
            f(&mut s.meta);
            s.meta.clone()
        };
        let _ = self.tx.send(HubEvent::SessionMeta { meta });
    }

    /// After an answer: back to idle once nothing else is waiting.
    fn settle(self: &Arc<Self>, id: SessionId) {
        let waiting = self.session(id).is_some_and(|s| {
            s.entries.iter().any(|e| match e {
                Entry::Permission(p) => p.resolved.is_none(),
                Entry::Question(q) => q.answers.is_none(),
                _ => false,
            })
        });
        if !waiting {
            self.emit(id, SessionEvent::Status { status: SessionStatus::Idle });
        }
    }

    /// Simulate an agent turn: stream thinking + markdown, run a tool, finish.
    fn fake_turn(self: &Arc<Self>, id: SessionId, prompt: String) {
        let hub = self.clone();
        std::thread::spawn(move || {
            let pause = |ms| std::thread::sleep(Duration::from_millis(ms));
            hub.emit(
                id,
                SessionEvent::Upsert {
                    entry: Entry::User {
                        id: new_entry_id(),
                        text: prompt.clone(),
                        attachments: vec![],
                        at: Utc::now(), agent_ref: None, prev_ref: None
                    },
                },
            );
            hub.emit(
                id,
                SessionEvent::Status {
                    status: SessionStatus::Working,
                },
            );
            let th = new_entry_id();
            for w in "Let me look at the project layout first.".split_inclusive(' ') {
                hub.emit(
                    id,
                    SessionEvent::AppendText {
                        entry_id: th.clone(),
                        thinking: true,
                        delta: w.into(),
                    },
                );
                pause(30);
            }
            hub.emit(id, SessionEvent::StreamEnd { entry_id: th });
            let tool_id = new_entry_id();
            let mut tool = ToolCall {
                id: tool_id,
                name: "Bash".into(),
                title: "ls -la".into(),
                kind: ToolKind::Execute,
                status: ToolStatus::Running,
                input: Some("ls -la".into()),
                output: None,
                diffs: vec![],
                locations: vec![],
            };
            hub.emit(
                id,
                SessionEvent::Upsert {
                    entry: Entry::Tool(tool.clone()),
                },
            );
            pause(600);
            tool.status = ToolStatus::Completed;
            tool.output = Some("Cargo.toml\nsrc\ntests\nREADME.md".into());
            hub.emit(
                id,
                SessionEvent::Upsert {
                    entry: Entry::Tool(tool),
                },
            );
            // "permission" / "question" in the prompt: stop and wait for the user, like a real
            // agent would ("later" waits a few seconds first, to test phone notifications).
            let lower = prompt.to_lowercase();
            if lower.contains("permission") || lower.contains("question") {
                pause(if lower.contains("later") { 8_000 } else { 300 });
                let entry = if lower.contains("permission") {
                    Entry::Permission(PermissionRequest {
                        id: new_entry_id(),
                        tool_call_id: None,
                        title: "Run `cargo publish`".into(),
                        detail: Some("cargo publish -p mothr-core".into()),
                        kind: ToolKind::Execute,
                        options: vec![
                            PermissionOption { id: "allow".into(), label: "Allow once".into(), kind: PermissionOptionKind::AllowOnce },
                            PermissionOption { id: "always".into(), label: "Always allow".into(), kind: PermissionOptionKind::AllowAlways },
                            PermissionOption { id: "deny".into(), label: "Deny".into(), kind: PermissionOptionKind::RejectOnce },
                        ],
                        resolved: None,
                        diffs: vec![],
                    })
                } else {
                    Entry::Question(QuestionRequest {
                        id: new_entry_id(),
                        questions: vec![Question {
                            header: Some("Database".into()),
                            question: "Which database should the cache use?".into(),
                            options: vec![
                                QuestionOption { label: "SQLite".into(), description: Some("One file, no server".into()) },
                                QuestionOption { label: "Postgres".into(), description: None },
                            ],
                            multi_select: false,
                        }],
                        answers: None,
                    })
                };
                hub.emit(id, SessionEvent::Upsert { entry });
                hub.emit(id, SessionEvent::Status { status: SessionStatus::NeedsInput });
                return;
            }
            let a = new_entry_id();
            let reply = format!(
                "You said: **{}**\n\nHere's a plan:\n\n1. Read `src/lib.rs`\n2. Write tests\n3. Ship it\n\n```rust\nfn main() {{\n    println!(\"hello <world>\");\n}}\n```\n",
                prompt.replace('*', "")
            );
            for chunk in reply.split_inclusive(' ') {
                hub.emit(
                    id,
                    SessionEvent::AppendText {
                        entry_id: a.clone(),
                        thinking: false,
                        delta: chunk.into(),
                    },
                );
                pause(25);
            }
            hub.emit(id, SessionEvent::StreamEnd { entry_id: a });
            hub.emit(
                id,
                SessionEvent::Upsert {
                    entry: Entry::TurnEnd {
                        id: new_entry_id(),
                        stop_reason: "end_turn".into(),
                        duration_ms: Some(4200),
                        cost_usd: Some(0.0123),
                    },
                },
            );
            hub.emit(
                id,
                SessionEvent::Context {
                    usage: ContextUsage {
                        used_tokens: 61_000,
                        window_tokens: 200_000,
                        cost_usd: Some(0.42),
                        input_tokens: 50_000,
                        output_tokens: 11_000,
                        credits: None,
                    },
                },
            );
            hub.emit(
                id,
                SessionEvent::Status {
                    status: SessionStatus::Idle,
                },
            );
        });
    }
}

impl HubApi for DemoHub {
    fn snapshot(&self) -> HubSnapshot {
        let agent = |kind, ok: bool| AgentInstall {
            kind,
            binary: None,
            version: None,
            transport: ok.then(|| Transport::Acp {
                program: "agent".into(),
                args: vec![],
                env: vec![],
            }),
            note: None,
        };
        let project = |p: &str| ProjectRef {
            pinned: p.ends_with("mothr"),
            git_branch: Some("main".into()),
            tags: vec!["rust".into()],
            ..ProjectRef::from_path(p.into())
        };
        HubSnapshot {
            sessions: self.sessions.lock().unwrap().clone(),
            agents: vec![
                agent(AgentKind::ClaudeCode, true),
                agent(AgentKind::Codex, true),
                agent(AgentKind::Copilot, true),
                agent(AgentKind::OpenCode, false),
                agent(AgentKind::Hermes, true),
            ],
            usage: self.usage.clone(),
            pinned_projects: vec![project("/home/me/projects/mothr")],
            recent_projects: vec![
                project("/home/me/projects/website"),
                project("/home/me/projects/api"),
            ],
            ..Default::default()
        }
    }

    fn session(&self, id: SessionId) -> Option<SessionState> {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.meta.id == id)
            .cloned()
    }

    fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.tx.subscribe()
    }

    fn dispatch(&self, command: AppCommand) -> anyhow::Result<Option<SessionId>> {
        // `dispatch` takes &self; re-wrap so helper threads can hold the hub.
        let this = HUB.get().expect("hub").clone();
        match command {
            AppCommand::NewSession {
                agent,
                kind,
                project,
                mode,
                prompt,
                ..
            } => {
                let state = SessionState::new(SessionMeta::new(agent, kind, project, mode));
                let id = state.meta.id;
                self.sessions.lock().unwrap().push(state.clone());
                let _ = self.tx.send(HubEvent::SessionCreated {
                    state: Box::new(state),
                });
                this.emit(
                    id,
                    SessionEvent::Ready {
                        agent_session_id: None,
                        model: Some("demo-model".into()),
                        models: vec![],
                        modes: vec![],
                    },
                );
                if let Some(p) = prompt {
                    this.fake_turn(id, p);
                }
                Ok(Some(id))
            }
            AppCommand::Session { id, command } => {
                match command {
                    SessionCommand::Prompt { text, .. } => this.fake_turn(id, text),
                    SessionCommand::Cancel => this.emit(
                        id,
                        SessionEvent::Status {
                            status: SessionStatus::Idle,
                        },
                    ),
                    SessionCommand::SetMode { mode } => this.emit(id, SessionEvent::Mode { mode }),
                    SessionCommand::RespondPermission {
                        entry_id,
                        option_id,
                    } => {
                        this.emit(
                            id,
                            SessionEvent::Resolved {
                                entry_id,
                                choice: Some(option_id),
                                answers: None,
                            },
                        );
                        this.settle(id);
                    }
                    SessionCommand::AnswerQuestion { entry_id, answers } => {
                        this.emit(
                            id,
                            SessionEvent::Resolved {
                                entry_id,
                                choice: None,
                                answers: Some(answers),
                            },
                        );
                        this.settle(id);
                    }
                    other => anyhow::bail!("demo hub ignores {other:?}"),
                }
                Ok(None)
            }
            AppCommand::PinSession { id, pinned } => Ok({
                this.with_meta(id, |m| m.pinned = pinned);
                None
            }),
            AppCommand::Rename { id, title } => Ok({
                this.with_meta(id, |m| m.title = title);
                None
            }),
            AppCommand::Archive { id, archived } => Ok({
                this.with_meta(id, |m| m.archived = archived);
                None
            }),
            AppCommand::MarkRead { id } => Ok({
                this.with_meta(id, |m| m.unread = false);
                None
            }),
            AppCommand::Resume { id } => Ok({
                this.emit(
                    id,
                    SessionEvent::Status {
                        status: SessionStatus::Idle,
                    },
                );
                None
            }),
            AppCommand::Delete { id } => {
                self.sessions.lock().unwrap().retain(|s| s.meta.id != id);
                let _ = self.tx.send(HubEvent::SessionRemoved { id });
                Ok(None)
            }
            other => {
                let _ = self.tx.send(HubEvent::Toast {
                    level: NoticeLevel::Info,
                    text: format!("demo: {other:?}"),
                });
                Ok(None)
            }
        }
    }

    fn search_projects(&self, query: &str, limit: usize) -> Vec<ProjectHit> {
        let q = query.to_lowercase();
        ["mothr", "website", "api", "shell-tools", "sheets"]
            .iter()
            .filter_map(|name| {
                // Subsequence match, reporting char indices into `name` and the full path.
                let path = format!("/home/me/projects/{name}");
                let indices = |hay: &str| {
                    let mut out = vec![];
                    let mut chars = q.chars().peekable();
                    for (i, c) in hay.chars().enumerate() {
                        if chars.peek() == Some(&c) {
                            out.push(i as u32);
                            chars.next();
                        }
                    }
                    (chars.peek().is_none()).then_some(out)
                };
                let ni = indices(name)?;
                Some(ProjectHit {
                    project: ProjectRef::from_path(PathBuf::from(&path)),
                    score: 100,
                    name_indices: ni,
                    path_indices: indices(&path).unwrap_or_default(),
                    display_path: path.clone(),
                })
            })
            .take(limit)
            .collect()
    }
}

static HUB: std::sync::OnceLock<Arc<DemoHub>> = std::sync::OnceLock::new();

fn demo_sessions() -> Vec<SessionState> {
    let mk = |agent, project: &str, title: &str, status| {
        let mut meta = SessionMeta::new(
            agent,
            SessionKind::Chat,
            project.into(),
            PermissionMode::Ask,
        );
        meta.title = title.into();
        meta.status = status;
        meta.model = Some("demo-model".into());
        SessionState::new(meta)
    };
    let mut a = mk(
        AgentKind::ClaudeCode,
        "/home/me/projects/mothr",
        "Wire up remote control",
        SessionStatus::NeedsInput,
    );
    a.meta.context = ContextUsage {
        used_tokens: 152_000,
        window_tokens: 200_000,
        cost_usd: Some(1.84),
        input_tokens: 0,
        output_tokens: 0,
        credits: None,
    };
    a.commands = ["compact|Summarize the conversation to free up context|", "review|Review the current changes|[focus]", "init|Write a CLAUDE.md for this project|", "model|Switch the model|<name>"]
        .iter()
        .map(|c| {
            let mut p = c.split('|');
            CommandInfo {
                name: p.next().unwrap_or_default().into(),
                description: p.next().map(String::from),
                hint: p.next().filter(|h| !h.is_empty()).map(String::from),
            }
        })
        .collect();
    a.entries = vec![
        Entry::User { id: "u1".into(), text: "Add a remote-control server.\nUse axum.".into(), attachments: vec![], at: Utc::now() , agent_ref: None, prev_ref: None},
        Entry::Thinking { id: "th".into(), text: "The user wants **axum**. Let me check the workspace.".into(), streaming: false },
        Entry::Plan { id: "plan".into(), items: vec![
            PlanItem { content: "Read the hub API".into(), status: PlanStatus::Completed },
            PlanItem { content: "Write the server".into(), status: PlanStatus::InProgress },
            PlanItem { content: "Write the web client".into(), status: PlanStatus::Pending },
        ] },
        Entry::Tool(ToolCall { id: "t1".into(), name: "Edit".into(), title: "Edit src/lib.rs".into(), kind: ToolKind::Edit, status: ToolStatus::Completed, input: Some("src/lib.rs".into()), output: None,
            diffs: vec![FileDiff { path: "src/lib.rs".into(), old_text: Some("fn a() {}\nfn b() {}\nfn c() {}".into()), new_text: "fn a() {}\nfn b() -> u8 { 1 }\nfn c() {}\nfn d() {}".into(), start_line: None }], locations: vec![] }),
        Entry::Tool(ToolCall { id: "t2".into(), name: "Bash".into(), title: "cargo test".into(), kind: ToolKind::Execute, status: ToolStatus::Failed, input: Some("cargo test -p mothr-remote".into()), output: Some("error[E0308]: mismatched types\n  --> src/lib.rs:2:20".into()), diffs: vec![], locations: vec![] }),
        Entry::Assistant { id: "a1".into(), text: "I'll fix the type error. See [axum docs](https://docs.rs/axum) and `Router::new()`.\n\n| step | state |\n|---|---|\n| build | ok |".into(), streaming: false },
        Entry::Notice { id: "n1".into(), level: NoticeLevel::Warning, text: "Context window is 76% full".into() },
        Entry::Permission(PermissionRequest { id: "p1".into(), tool_call_id: None, title: "Run `rm -rf target/`".into(), detail: Some("rm -rf target/".into()), kind: ToolKind::Execute, options: vec![
            PermissionOption { id: "allow".into(), label: "Allow once".into(), kind: PermissionOptionKind::AllowOnce },
            PermissionOption { id: "always".into(), label: "Always allow".into(), kind: PermissionOptionKind::AllowAlways },
            PermissionOption { id: "deny".into(), label: "Deny".into(), kind: PermissionOptionKind::RejectOnce },
        ], resolved: None, diffs: vec![] }),
        Entry::Question(QuestionRequest { id: "q1".into(), questions: vec![
            Question { header: Some("Port".into()), question: "Which port should the server use?".into(), options: vec![QuestionOption { label: "7878".into(), description: Some("Default".into()) }, QuestionOption { label: "8080".into(), description: None }], multi_select: false },
            Question { header: Some("Tunnels".into()), question: "Which tunnels should be supported?".into(), options: vec![QuestionOption { label: "Tailscale".into(), description: None }, QuestionOption { label: "Dev Tunnels".into(), description: None }], multi_select: true },
        ], answers: None }),
    ];
    let mut b = mk(
        AgentKind::Codex,
        "/home/me/projects/api",
        "Refactor auth middleware",
        SessionStatus::Working,
    );
    b.entries.push(Entry::Assistant {
        id: "a".into(),
        text: "Working on it".into(),
        streaming: true,
    });
    let mut c = mk(
        AgentKind::Copilot,
        "/home/me/projects/website",
        "Landing page copy",
        SessionStatus::Idle,
    );
    c.meta.unread = true;
    let mut d = mk(
        AgentKind::Hermes,
        "/home/me/projects/website",
        "Old experiment",
        SessionStatus::Ended,
    );
    d.meta.pinned = true;
    let e = mk(
        AgentKind::OpenCode,
        "/home/me/projects/mothr",
        "Fix flaky test",
        SessionStatus::Error("agent crashed".into()),
    );
    vec![a, b, c, d, e]
}

fn demo_usage() -> Vec<ProviderUsage> {
    let win = |label: &str, f: f32| LimitWindow {
        kind: WindowKind::FiveHour,
        label: label.into(),
        used_fraction: f,
        resets_at: Some(Utc::now() + chrono::Duration::minutes(134)),
        used: None,
        limit: None,
        detail: None,
    };
    let mut claude = ProviderUsage::new(Provider::Anthropic, "demo");
    claude.plan = Some("Max 20x".into());
    claude.windows = vec![win("5h", 0.42), win("Weekly", 0.78)];
    let mut codex = ProviderUsage::new(Provider::OpenAi, "demo");
    codex.windows = vec![win("5h", 0.93)];
    let mut copilot = ProviderUsage::new(Provider::GitHubCopilot, "demo");
    copilot.windows = vec![LimitWindow {
        used: Some(212.0),
        limit: Some(300.0),
        ..win("AIC", 0.71)
    }];
    vec![claude, codex, copilot]
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (tx, _) = broadcast::channel(1024);
    let hub = Arc::new(DemoHub {
        tx,
        sessions: Mutex::new(demo_sessions()),
        usage: demo_usage(),
    });
    let _ = HUB.set(hub.clone());
    let token = std::env::var("MOTHR_DEMO_TOKEN").unwrap_or_else(|_| generate_token());
    let port = std::env::var("MOTHR_DEMO_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(7979);
    let settings = RemoteSettings {
        enabled: true,
        bind: "127.0.0.1".into(),
        port,
        token,
        tunnel: TunnelKind::None,
    };
    let server = RemoteServer::start(hub, settings).await?;
    println!(
        "Mothr remote demo: {}",
        server.pairing_url().unwrap_or_default()
    );
    tokio::signal::ctrl_c().await?;
    server.stop().await;
    Ok(())
}
