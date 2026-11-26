//! Remote client for a running Mothr desktop.
//!
//! The mobile app (Kotlin Multiplatform, `apps/mobile`) uses this crate through UniFFI, so the
//! phone runs the exact same session model and reducer ([`mothr_core::SessionState::apply`]) as
//! the desktop app and the web client. The desktop's remote server (`mothr-remote`) streams a
//! snapshot plus [`mothr_core::HubEvent`]s over a WebSocket; commands go over HTTP.
//!
//! The Kotlin side pulls flat view records ([`ChatGroup`], [`ChatDetail`], …) and waits for
//! changes with [`MothrClient::wait_for_change`], which maps naturally onto a coroutine `Flow`.

uniffi::setup_scaffolding!();

mod update;
pub use update::{AppUpdate, check_app_update, download_app_update};

pub mod model;
mod pairing;
mod views;

pub use pairing::Pairing;
pub use views::*;

use futures::StreamExt;
use model::{ServerMsg, Store};
use mothr_core::{AgentKind, AppCommand, PermissionMode, SessionCommand, SessionId, SessionKind};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::{self, Message};

#[derive(Debug, Clone, PartialEq, thiserror::Error, uniffi::Error)]
pub enum ClientError {
    /// The pairing link can't be used.
    #[error("{reason}")]
    InvalidLink { reason: String },
    /// The desktop couldn't be reached.
    #[error("{reason}")]
    Network { reason: String },
    /// The desktop refused the command (e.g. the agent isn't installed).
    #[error("{reason}")]
    Rejected { reason: String },
    /// The pairing token was rejected.
    #[error("This phone isn't paired anymore. Pair again from Mothr → Settings → Remote.")]
    Unauthorized,
    /// A bad argument from the app (unknown id, empty prompt …).
    #[error("{reason}")]
    Invalid { reason: String },
}

fn invalid(message: impl Into<String>) -> ClientError {
    ClientError::Invalid { reason: message.into() }
}

/// Server reply to `/api/command`.
#[derive(Debug, serde::Deserialize)]
struct CommandReply {
    ok: bool,
    #[serde(default)]
    session_id: Option<SessionId>,
    #[serde(default)]
    error: Option<String>,
}

struct Conn {
    state: ConnectionState,
    message: Option<String>,
}

struct Shared {
    store: Mutex<Store>,
    conn: Mutex<Conn>,
    /// Bumped on every change; the UI waits on it.
    rev: watch::Sender<u64>,
}

impl Shared {
    fn bump(&self) {
        self.rev.send_modify(|r| *r += 1);
    }
    fn set_conn(&self, state: ConnectionState, message: Option<String>) {
        {
            let mut c = self.conn.lock();
            c.state = state;
            c.message = message;
        }
        self.bump();
    }
}

/// A connection to one paired Mothr desktop.
#[derive(uniffi::Object)]
pub struct MothrClient {
    pairing: Pairing,
    rt: tokio::runtime::Runtime,
    http: reqwest::Client,
    shared: Arc<Shared>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for MothrClient {
    fn drop(&mut self) {
        if let Some(t) = self.task.lock().take() {
            t.abort();
        }
    }
}

#[uniffi::export]
impl MothrClient {
    /// Create a client from the pairing link the desktop shows (`https://…/?token=…`).
    /// Doesn't touch the network; call [`MothrClient::connect`].
    #[uniffi::constructor]
    pub fn new(link: String) -> Result<Arc<Self>, ClientError> {
        let pairing = Pairing::parse(&link).map_err(|reason| ClientError::InvalidLink { reason })?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("mothr-client")
            .enable_all()
            .build()
            .map_err(|e| ClientError::Network { reason: e.to_string() })?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(8))
            .build()
            .map_err(|e| ClientError::Network { reason: e.to_string() })?;
        let (rev, _) = watch::channel(0);
        Ok(Arc::new(Self {
            pairing,
            rt,
            http,
            shared: Arc::new(Shared {
                store: Mutex::new(Store::default()),
                conn: Mutex::new(Conn { state: ConnectionState::Idle, message: None }),
                rev,
            }),
            task: Mutex::new(None),
        }))
    }

    /// The normalized pairing link (save it to reconnect later).
    pub fn link(&self) -> String {
        self.pairing.link()
    }

    /// The desktop's address, without the token.
    pub fn server(&self) -> String {
        self.pairing.base.clone()
    }

    /// Open the live connection. Reconnects automatically until [`MothrClient::disconnect`].
    pub fn connect(&self) {
        let mut task = self.task.lock();
        if task.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        let url = self.pairing.ws_url();
        let shared = self.shared.clone();
        let unreachable = self.pairing.unreachable_message();
        *task = Some(self.rt.spawn(run_socket(url, unreachable, shared)));
    }

    pub fn disconnect(&self) {
        if let Some(t) = self.task.lock().take() {
            t.abort();
        }
        self.shared.set_conn(ConnectionState::Idle, None);
    }

    pub fn connection(&self) -> ConnectionInfo {
        let c = self.shared.conn.lock();
        let store = self.shared.store.lock();
        ConnectionInfo {
            state: c.state,
            server: self.pairing.base.clone(),
            computer: store.computer.clone(),
            message: c.message.clone(),
            loaded: store.loaded,
            needs_you: store.needs_you_count(),
        }
    }

    /// Increases on every change (data or connection).
    pub fn revision(&self) -> u64 {
        *self.shared.rev.borrow()
    }

    /// Block until the revision exceeds `after` or `timeout_ms` passes; returns the revision.
    /// Call from a background thread (e.g. `Dispatchers.IO`).
    pub fn wait_for_change(&self, after: u64, timeout_ms: u32) -> u64 {
        let mut rx = self.shared.rev.subscribe();
        self.rt.block_on(async {
            let _ = tokio::time::timeout(Duration::from_millis(timeout_ms as u64), rx.wait_for(|v| *v > after)).await;
        });
        *self.shared.rev.borrow()
    }

    // ---------------------------------------------------------------------------------------
    // Views

    /// Top-level chats in smart groups (Pinned, Needs you, Working, Today, …), filtered by `query`.
    pub fn chat_groups(&self, query: String) -> Vec<ChatGroup> {
        self.shared.store.lock().groups(&query)
    }

    pub fn chat(&self, id: String) -> Option<ChatDetail> {
        let id = SessionId::parse_str(&id).ok()?;
        self.shared.store.lock().detail(id)
    }

    /// Every agent Mothr knows, installed ones first.
    pub fn agents(&self) -> Vec<AgentView> {
        self.shared.store.lock().agents()
    }

    /// The reasoning levels and context sizes a new chat with `agent_id` on `model` (None: the
    /// agent's default) can pick. A running chat has them on [`ChatDetail`].
    pub fn model_settings(&self, agent_id: String, model: Option<String>) -> ModelSettings {
        let Some(agent) = AgentKind::from_id(&agent_id) else { return ModelSettings { efforts: vec![], contexts: vec![] } };
        self.shared.store.lock().model_settings(agent, model.as_deref())
    }

    /// Pinned then recent projects.
    pub fn projects(&self) -> Vec<ProjectView> {
        self.shared.store.lock().projects()
    }

    pub fn usage(&self) -> Vec<UsageView> {
        self.shared.store.lock().usage()
    }

    /// Permission requests and questions agents are blocked on, oldest first. The phone shows
    /// one notification per item and withdraws it when the item disappears.
    pub fn attention(&self) -> Vec<Attention> {
        self.shared.store.lock().attention()
    }

    /// Notifications from the desktop since the last call.
    pub fn take_toasts(&self) -> Vec<Toast> {
        std::mem::take(&mut self.shared.store.lock().toasts)
    }

    // ---------------------------------------------------------------------------------------
    // Commands (blocking; call from a background thread)

    /// Fuzzy-search the desktop's projects.
    pub fn search_projects(&self, query: String) -> Result<Vec<ProjectView>, ClientError> {
        let url = format!("{}/api/projects", self.pairing.base);
        let hits: Vec<mothr_core::ProjectHit> = self.rt.block_on(async {
            let res = self
                .http
                .get(url)
                .bearer_auth(&self.pairing.token)
                .query(&[("q", query.as_str()), ("limit", "30")])
                .send()
                .await
                .map_err(net)?;
            if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(ClientError::Unauthorized);
            }
            res.error_for_status().map_err(net)?.json().await.map_err(net)
        })?;
        Ok(hits.iter().map(model::hit_view).collect())
    }

    /// Every indexed project matching `query` (all of them, best first, when it's empty), up to
    /// `limit`. Name matches carry highlight indices.
    pub fn find_projects(&self, query: String, limit: u32) -> Result<Vec<ProjectView>, ClientError> {
        let limit = limit.clamp(1, 2000).to_string();
        let hits: Vec<mothr_core::ProjectHit> = self.get("/api/projects", &[("q", &query), ("limit", &limit)])?;
        Ok(hits.iter().map(model::hit_view).collect())
    }

    /// One folder of a project (`dir` relative to the project, `""` for its root).
    pub fn list_dir(&self, project_path: String, dir: String) -> Result<DirView, ClientError> {
        let listing: mothr_core::DirListing = self.get("/api/projects/files", &[("project", &project_path), ("path", &dir)])?;
        Ok(model::dir_view(listing))
    }

    /// One file of a project (`path` relative to the project).
    pub fn read_file(&self, project_path: String, path: String) -> Result<FileView, ClientError> {
        let file: mothr_core::FileContent = self.get("/api/projects/file", &[("project", &project_path), ("path", &path)])?;
        Ok(model::file_view(file))
    }

    /// The bytes of a picture in a chat, by the name an [`EntryView`] gives it (PNG, JPEG, GIF
    /// or WebP).
    pub fn image(&self, name: String) -> Result<Vec<u8>, ClientError> {
        let mut url = reqwest::Url::parse(&self.pairing.base).map_err(|e| invalid(e.to_string()))?;
        url.path_segments_mut().map_err(|_| invalid("Bad server address"))?.pop_if_empty().extend(["api", "images", &name]);
        self.rt.block_on(async {
            let res = self.http.get(url).bearer_auth(&self.pairing.token).send().await.map_err(net)?;
            match res.status() {
                reqwest::StatusCode::UNAUTHORIZED => Err(ClientError::Unauthorized),
                reqwest::StatusCode::NOT_FOUND => Err(ClientError::Rejected { reason: "That image is no longer on the computer".into() }),
                _ => Ok(res.error_for_status().map_err(net)?.bytes().await.map_err(net)?.to_vec()),
            }
        })
    }

    /// Fuzzy-find files of a project by name or path.
    pub fn find_files(&self, project_path: String, query: String) -> Result<Vec<FileHitView>, ClientError> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }
        let hits: Vec<mothr_core::FileHit> = self.get("/api/projects/find", &[("project", &project_path), ("q", &query), ("limit", "40")])?;
        Ok(hits.into_iter().map(model::file_hit_view).collect())
    }

    #[allow(clippy::too_many_arguments)]
    /// Start a chat. An empty `project_path` starts a general chat. Returns the new chat id.
    pub fn new_chat(
        &self,
        agent_id: String,
        project_path: String,
        mode_id: String,
        model: Option<String>,
        effort: Option<String>,
        context: Option<String>,
        prompt: Option<String>,
        design: bool,
    ) -> Result<String, ClientError> {
        let agent = AgentKind::from_id(&agent_id).ok_or_else(|| invalid(format!("Unknown agent “{agent_id}”")))?;
        let mode = PermissionMode::from_id(&mode_id).ok_or_else(|| invalid(format!("Unknown mode “{mode_id}”")))?;
        let reply = self.command(&AppCommand::NewSession {
            agent,
            kind: if design { SessionKind::Design } else { SessionKind::Chat },
            project: project_path.into(),
            mode: mode.for_agent(agent),
            model: model.filter(|m| !m.is_empty()),
            effort: effort.filter(|e| !e.is_empty()),
            context: context.filter(|c| !c.is_empty()),
            prompt: prompt.filter(|p| !p.trim().is_empty()),
            workspace: None,
            extra_projects: vec![],
        })?;
        reply.map(|id| id.to_string()).ok_or_else(|| ClientError::Rejected { reason: "Mothr didn't return the new chat".into() })
    }

    /// Send a message. `Normal` queues it if the agent is busy.
    pub fn send(&self, chat_id: String, text: String, delivery: Delivery) -> Result<(), ClientError> {
        if text.trim().is_empty() {
            return Err(invalid("Type a message first"));
        }
        let delivery = match delivery {
            Delivery::Normal => mothr_core::Delivery::Normal,
            Delivery::Steer => mothr_core::Delivery::Steer,
            Delivery::Interrupt => mothr_core::Delivery::Interrupt,
        };
        self.session(&chat_id, SessionCommand::Prompt { text, attachments: vec![], delivery })
    }

    pub fn respond_permission(&self, chat_id: String, entry_id: String, option_id: String) -> Result<(), ClientError> {
        self.session(&chat_id, SessionCommand::RespondPermission { entry_id, option_id })
    }

    /// One answer list per question (option labels or free text).
    pub fn answer_question(&self, chat_id: String, entry_id: String, answers: Vec<Vec<String>>) -> Result<(), ClientError> {
        self.session(&chat_id, SessionCommand::AnswerQuestion { entry_id, answers })
    }

    /// Stop the agent's current turn.
    pub fn interrupt(&self, chat_id: String) -> Result<(), ClientError> {
        self.session(&chat_id, SessionCommand::Cancel)
    }

    pub fn set_mode(&self, chat_id: String, mode_id: String) -> Result<(), ClientError> {
        let mode = PermissionMode::from_id(&mode_id).ok_or_else(|| invalid(format!("Unknown mode “{mode_id}”")))?;
        self.session(&chat_id, SessionCommand::SetMode { mode })
    }

    pub fn set_model(&self, chat_id: String, model_id: String) -> Result<(), ClientError> {
        self.session(&chat_id, SessionCommand::SetModel { model: model_id })
    }

    /// Change the reasoning effort ([`ChoiceOption::id`]).
    pub fn set_effort(&self, chat_id: String, effort: String) -> Result<(), ClientError> {
        self.session(&chat_id, SessionCommand::SetEffort { effort })
    }

    /// Change the context window ([`ChoiceOption::id`]). The desktop restarts the agent for it,
    /// so this is refused while a turn is running.
    pub fn set_context(&self, chat_id: String, context: String) -> Result<(), ClientError> {
        self.session(&chat_id, SessionCommand::SetContext { context })
    }

    /// Remove a queued message.
    pub fn dequeue(&self, chat_id: String, queue_id: String) -> Result<(), ClientError> {
        let id = parse_id(&chat_id)?;
        self.command(&AppCommand::Dequeue { id, queue_id }).map(|_| ())
    }

    /// Restart an ended chat, resuming the agent's conversation.
    pub fn resume(&self, chat_id: String) -> Result<(), ClientError> {
        let id = parse_id(&chat_id)?;
        self.command(&AppCommand::Resume { id }).map(|_| ())
    }

    pub fn mark_read(&self, chat_id: String) -> Result<(), ClientError> {
        let id = parse_id(&chat_id)?;
        self.command(&AppCommand::MarkRead { id }).map(|_| ())
    }

    pub fn pin_chat(&self, chat_id: String, pinned: bool) -> Result<(), ClientError> {
        let id = parse_id(&chat_id)?;
        self.command(&AppCommand::PinSession { id, pinned }).map(|_| ())
    }

    pub fn refresh_usage(&self) -> Result<(), ClientError> {
        self.command(&AppCommand::RefreshUsage).map(|_| ())
    }

    // ---------------------------------------------------------------------------------------
    // Terminal beside a chat (the user's shell on the computer, in the chat's primary repo)

    /// The chat's terminal screen, once it's open.
    pub fn terminal(&self, chat_id: String) -> Option<TerminalView> {
        let id = SessionId::parse_str(&chat_id).ok()?;
        self.shared.store.lock().terminal(id)
    }

    /// Start the chat's terminal (an open one is left as it is; an exited one starts again).
    pub fn open_terminal(&self, chat_id: String) -> Result<(), ClientError> {
        let id = parse_id(&chat_id)?;
        self.command(&AppCommand::TerminalOpen { id, cwd: None }).map(|_| ())
    }

    /// Type into the terminal: text, or the escape sequences keys send ("\u{1b}[A" for ↑).
    pub fn terminal_input(&self, chat_id: String, data: String) -> Result<(), ClientError> {
        let id = parse_id(&chat_id)?;
        self.command(&AppCommand::TerminalInput { id, data }).map(|_| ())
    }

    /// Fit the shell to the phone's screen (it follows whoever typed or resized last).
    pub fn resize_terminal(&self, chat_id: String, cols: u16, rows: u16) -> Result<(), ClientError> {
        let id = parse_id(&chat_id)?;
        self.command(&AppCommand::TerminalResize { id, cols, rows }).map(|_| ())
    }

    pub fn restart_terminal(&self, chat_id: String) -> Result<(), ClientError> {
        let id = parse_id(&chat_id)?;
        self.command(&AppCommand::TerminalRestart { id }).map(|_| ())
    }

    /// End the shell.
    pub fn close_terminal(&self, chat_id: String) -> Result<(), ClientError> {
        let id = parse_id(&chat_id)?;
        self.command(&AppCommand::TerminalClose { id }).map(|_| ())
    }
}

impl MothrClient {
    /// GET a JSON API route. A refusal (`{"ok": false, "error": …}`) becomes `Rejected`.
    fn get<T: serde::de::DeserializeOwned>(&self, route: &str, query: &[(&str, &str)]) -> Result<T, ClientError> {
        let url = format!("{}{route}", self.pairing.base);
        self.rt.block_on(async {
            let res = self.http.get(url).bearer_auth(&self.pairing.token).query(query).send().await.map_err(net)?;
            let status = res.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(ClientError::Unauthorized);
            }
            if status.is_client_error() {
                let reason = res
                    .json::<serde_json::Value>()
                    .await
                    .ok()
                    .and_then(|v| v.get("error")?.as_str().map(str::to_string))
                    .unwrap_or_else(|| format!("Mothr refused that ({status})"));
                return Err(ClientError::Rejected { reason });
            }
            res.error_for_status().map_err(net)?.json().await.map_err(net)
        })
    }

    fn session(&self, chat_id: &str, command: SessionCommand) -> Result<(), ClientError> {
        let id = parse_id(chat_id)?;
        self.command(&AppCommand::Session { id, command }).map(|_| ())
    }

    fn command(&self, cmd: &AppCommand) -> Result<Option<SessionId>, ClientError> {
        let url = format!("{}/api/command", self.pairing.base);
        self.rt.block_on(async {
            let res = self.http.post(url).bearer_auth(&self.pairing.token).json(cmd).send().await.map_err(net)?;
            if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(ClientError::Unauthorized);
            }
            let reply: CommandReply = res.json().await.map_err(net)?;
            if reply.ok {
                Ok(reply.session_id)
            } else {
                Err(ClientError::Rejected { reason: reply.error.unwrap_or_else(|| "Mothr refused that".into()) })
            }
        })
    }
}

fn parse_id(id: &str) -> Result<SessionId, ClientError> {
    SessionId::parse_str(id).map_err(|_| invalid(format!("Unknown chat “{id}”")))
}

fn net(e: reqwest::Error) -> ClientError {
    let message = if e.is_timeout() {
        "Mothr didn't answer in time".to_string()
    } else if e.is_connect() {
        "Can't reach Mothr. Is the desktop app running with Remote on?".to_string()
    } else {
        e.to_string()
    };
    ClientError::Network { reason: message }
}

/// The server pings every 20 s; silence for longer than this means the link is dead
/// (common when a phone switches networks).
const SILENCE_LIMIT: Duration = Duration::from_secs(50);
/// An address with no route (Tailscale off on the phone) can otherwise hang for minutes.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

async fn run_socket(url: String, unreachable: &'static str, shared: Arc<Shared>) {
    let mut backoff = Duration::from_millis(500);
    // Retries keep the last message up, so the reason stays readable between attempts.
    shared.set_conn(ConnectionState::Connecting, None);
    loop {
        match tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(&url)).await {
            Ok(Ok((ws, _))) => {
                shared.set_conn(ConnectionState::Connected, None);
                backoff = Duration::from_millis(500);
                let (_sink, mut stream) = ws.split();
                let reason = loop {
                    match tokio::time::timeout(SILENCE_LIMIT, stream.next()).await {
                        Err(_) => break "No response from Mothr".to_string(),
                        Ok(None) => break "Connection closed".to_string(),
                        Ok(Some(Err(e))) => break e.to_string(),
                        Ok(Some(Ok(Message::Close(frame)))) => {
                            break frame.map(|f| f.reason.to_string()).filter(|r| !r.is_empty()).unwrap_or_else(|| "Mothr closed the connection".into());
                        }
                        Ok(Some(Ok(Message::Text(text)))) => handle_text(&shared, text.as_str()),
                        Ok(Some(Ok(Message::Binary(b)))) => handle_text(&shared, &String::from_utf8_lossy(&b)),
                        Ok(Some(Ok(_))) => {}
                    }
                };
                shared.set_conn(ConnectionState::Reconnecting, Some(reason));
            }
            Ok(Err(tungstenite::Error::Http(res))) if res.status().as_u16() == 401 => {
                shared.set_conn(ConnectionState::Unauthorized, Some(ClientError::Unauthorized.to_string()));
                return;
            }
            Ok(Err(e)) => {
                let msg = match &e {
                    tungstenite::Error::Io(_) => unreachable.to_string(),
                    tungstenite::Error::Http(res) if res.status().is_server_error() => unreachable.to_string(),
                    other => other.to_string(),
                };
                shared.set_conn(ConnectionState::Reconnecting, Some(msg));
            }
            Err(_) => shared.set_conn(ConnectionState::Reconnecting, Some(unreachable.to_string())),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(10));
    }
}

fn handle_text(shared: &Shared, text: &str) {
    match serde_json::from_str::<ServerMsg>(text) {
        Ok(msg) => {
            shared.store.lock().apply_msg(msg);
            shared.bump();
        }
        Err(e) => tracing::debug!("ignoring server message: {e}"),
    }
}
