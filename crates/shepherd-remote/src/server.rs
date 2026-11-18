//! HTTP routes, authentication middleware and the WebSocket client loop.

use crate::RemoteStatus;
use crate::auth::{self, FAILURE_DELAY};
use axum::Router;
use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Json, Response};
use axum::routing::{get, post};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use shepherd_core::{AppCommand, HubApi, HubEvent, HubSnapshot, SessionId};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};

const INDEX_HTML: &str = include_str!("../assets/index.html");
const LOGIN_HTML: &str = include_str!("../assets/login.html");
const PING_EVERY: Duration = Duration::from_secs(20);
const ICON_SVG: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'><rect width='64' height='64' rx='14' fill='#16181d'/><path d='M20 44c0-9 5-16 12-16s12 7 12 16' fill='none' stroke='#D97757' stroke-width='5' stroke-linecap='round'/><circle cx='32' cy='21' r='6' fill='#e8e6e3'/><path d='M46 14v26' stroke='#e8e6e3' stroke-width='4' stroke-linecap='round'/><path d='M46 14c-5 0-7 4-7 7' fill='none' stroke='#e8e6e3' stroke-width='4' stroke-linecap='round'/></svg>";
const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ws: wss:; manifest-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

#[derive(Clone)]
pub struct AppState {
    pub hub: Arc<dyn HubApi>,
    pub token: Arc<str>,
    pub status: watch::Sender<RemoteStatus>,
    pub shutdown: watch::Receiver<bool>,
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/api/snapshot", get(snapshot))
        .route("/api/sessions/{id}", get(session))
        .route("/api/projects", get(projects))
        .route("/api/command", post(command))
        .route("/api/ws", get(ws_upgrade))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_auth));
    Router::new()
        .route("/", get(index))
        .route("/manifest.webmanifest", get(manifest))
        .route("/icon.svg", get(icon))
        .merge(api)
        .fallback(|| async { (StatusCode::NOT_FOUND, "not found") })
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

async fn security_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    res
}

async fn require_auth(State(st): State<AppState>, req: Request, next: Next) -> Response {
    if auth::authorize(req.headers(), req.uri(), &st.token).is_some() {
        return next.run(req).await;
    }
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    tracing::warn!(
        "remote: unauthorized {} {} from {peer:?}",
        req.method(),
        req.uri().path()
    );
    tokio::time::sleep(FAILURE_DELAY).await;
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        Json(json!({ "ok": false, "error": "unauthorized" })),
    )
        .into_response()
}

fn is_https(headers: &HeaderMap) -> bool {
    let fwd = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    fwd("x-forwarded-proto").starts_with("https") || fwd("forwarded").contains("proto=https")
}

async fn index(State(st): State<AppState>, headers: HeaderMap, uri: Uri) -> Response {
    if let Some(presented) = auth::query_token(&uri) {
        if auth::constant_time_eq(presented.as_bytes(), st.token.as_bytes()) {
            let cookie = auth::session_cookie(&st.token, is_https(&headers));
            return (
                StatusCode::SEE_OTHER,
                [
                    (header::LOCATION, "/".to_string()),
                    (header::SET_COOKIE, cookie),
                    (header::CACHE_CONTROL, "no-store".to_string()),
                ],
            )
                .into_response();
        }
        tokio::time::sleep(FAILURE_DELAY).await;
        return login_page("That pairing token is not valid.");
    }
    if auth::authorize(&headers, &uri, &st.token).is_some() {
        return (
            [
                (header::CACHE_CONTROL, "no-cache"),
                (header::CONTENT_SECURITY_POLICY, CSP),
            ],
            Html(INDEX_HTML),
        )
            .into_response();
    }
    if !auth::presented_tokens(&headers, &uri).is_empty() {
        tokio::time::sleep(FAILURE_DELAY).await;
        return login_page("Your saved pairing is no longer valid. Pair again.");
    }
    login_page("")
}

fn login_page(message: &str) -> Response {
    // `message` is always one of our own static strings: no escaping needed.
    let body = LOGIN_HTML.replace("{{MESSAGE}}", message);
    (
        StatusCode::UNAUTHORIZED,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, CSP),
        ],
        Html(body),
    )
        .into_response()
}

async fn manifest(State(st): State<AppState>, headers: HeaderMap, uri: Uri) -> Response {
    // Installed web apps on iOS do not share Safari's cookies, so authenticated clients get
    // a start URL that re-pairs on launch.
    let start_url = if auth::authorize(&headers, &uri, &st.token).is_some() {
        format!("/?token={}", st.token)
    } else {
        "/".into()
    };
    let icon = format!(
        "data:image/svg+xml,{}",
        ICON_SVG
            .replace('#', "%23")
            .replace('<', "%3C")
            .replace('>', "%3E")
    );
    let body = json!({
        "name": "Shepherd",
        "short_name": "Shepherd",
        "description": "Remote control for your coding agents",
        "start_url": start_url,
        "scope": "/",
        "display": "standalone",
        "background_color": "#0f1115",
        "theme_color": "#0f1115",
        "icons": [
            { "src": icon, "sizes": "any", "type": "image/svg+xml", "purpose": "any" },
            { "src": "/icon.svg", "sizes": "any", "type": "image/svg+xml", "purpose": "maskable" }
        ]
    });
    (
        [
            (header::CONTENT_TYPE, "application/manifest+json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body.to_string(),
    )
        .into_response()
}

async fn icon() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "max-age=86400"),
        ],
        ICON_SVG,
    )
        .into_response()
}

async fn snapshot(State(st): State<AppState>) -> Json<HubSnapshot> {
    Json(st.hub.snapshot())
}

async fn session(State(st): State<AppState>, Path(id): Path<SessionId>) -> Response {
    match st.hub.session(id) {
        Some(s) => Json(s).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "no such session" })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ProjectsQuery {
    #[serde(default)]
    q: String,
    limit: Option<usize>,
}

async fn projects(State(st): State<AppState>, Query(q): Query<ProjectsQuery>) -> Response {
    let limit = q.limit.unwrap_or(20).clamp(1, 200);
    let hub = st.hub.clone();
    match tokio::task::spawn_blocking(move || hub.search_projects(&q.q, limit)).await {
        Ok(hits) => Json(hits).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Serialize, Default)]
pub struct CommandReply {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

async fn dispatch(hub: &Arc<dyn HubApi>, cmd: AppCommand) -> CommandReply {
    let hub = hub.clone();
    // The hub API is synchronous; keep it off the async workers.
    match tokio::task::spawn_blocking(move || hub.dispatch(cmd)).await {
        Ok(Ok(session_id)) => CommandReply {
            ok: true,
            session_id,
            error: None,
        },
        Ok(Err(e)) => CommandReply {
            ok: false,
            session_id: None,
            error: Some(format!("{e:#}")),
        },
        Err(e) => CommandReply {
            ok: false,
            session_id: None,
            error: Some(format!("dispatch panicked: {e}")),
        },
    }
}

async fn command(State(st): State<AppState>, body: Bytes) -> Response {
    let cmd: AppCommand = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => {
            let reply = CommandReply {
                ok: false,
                session_id: None,
                error: Some(format!("invalid command: {e}")),
            };
            return (StatusCode::BAD_REQUEST, Json(reply)).into_response();
        }
    };
    let reply = dispatch(&st.hub, cmd).await;
    let code = if reply.ok {
        StatusCode::OK
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    };
    (code, Json(reply)).into_response()
}

// ---------------------------------------------------------------------------------------------
// WebSocket

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg<'a> {
    Snapshot { data: &'a HubSnapshot },
    Event { data: &'a HubEvent },
}

async fn ws_upgrade(State(st): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(4 << 20)
        .on_upgrade(move |socket| client_loop(st, socket))
}

/// Keeps `connected_clients` in the status accurate for the lifetime of a socket.
struct ClientGuard(watch::Sender<RemoteStatus>);

impl ClientGuard {
    fn new(status: &watch::Sender<RemoteStatus>) -> Self {
        status.send_modify(|s| s.connected_clients += 1);
        Self(status.clone())
    }
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0
            .send_modify(|s| s.connected_clients = s.connected_clients.saturating_sub(1));
    }
}

fn encode(msg: &ServerMsg<'_>) -> Message {
    Message::Text(serde_json::to_string(msg).unwrap_or_default().into())
}

async fn client_loop(st: AppState, socket: WebSocket) {
    let _guard = ClientGuard::new(&st.status);
    let (mut sink, mut stream) = socket.split();
    // Snapshot and subscribe atomically so no event is lost or applied twice.
    let (initial, mut events) = st.hub.snapshot_and_subscribe();
    if sink
        .send(encode(&ServerMsg::Snapshot {
            data: &initial,
        }))
        .await
        .is_err()
    {
        return;
    }
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Message>();
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
    let mut shutdown = st.shutdown.clone();

    loop {
        let outgoing = tokio::select! {
            ev = events.recv() => match ev {
                Ok(ev) => encode(&ServerMsg::Event { data: &ev }),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!("remote client lagged by {n} events; resending snapshot");
                    encode(&ServerMsg::Snapshot { data: &st.hub.snapshot() })
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            Some(reply) = reply_rx.recv() => reply,
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    handle_client_message(&st, text.as_str(), &reply_tx);
                    continue;
                }
                Some(Ok(Message::Binary(bytes))) => {
                    handle_client_message(&st, &String::from_utf8_lossy(&bytes), &reply_tx);
                    continue;
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => continue, // ping/pong are handled by the library
            },
            _ = ping.tick() => Message::Ping(Bytes::new()),
            _ = crate::stopped(&mut shutdown) => {
                let _ = sink
                    .send(Message::Close(Some(CloseFrame { code: 1001, reason: "server stopping".into() })))
                    .await;
                break;
            }
        };
        if sink.send(outgoing).await.is_err() {
            break;
        }
    }
}

/// Client messages: `{"id": n, "command": AppCommand}`, `{"id": n, "op": "snapshot"}` or a bare
/// AppCommand. Every message gets an `ack` (snapshot requests get a snapshot as well).
fn handle_client_message(st: &AppState, text: &str, reply: &mpsc::UnboundedSender<Message>) {
    let ack = |id: serde_json::Value, r: CommandReply| {
        let mut v = json!({ "type": "ack", "id": id, "ok": r.ok });
        if let Some(sid) = r.session_id {
            v["session_id"] = json!(sid);
        }
        if let Some(e) = r.error {
            v["error"] = json!(e);
        }
        Message::Text(v.to_string().into())
    };
    let fail = |id: serde_json::Value, e: String| {
        ack(
            id,
            CommandReply {
                ok: false,
                session_id: None,
                error: Some(e),
            },
        )
    };

    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            let _ = reply.send(fail(serde_json::Value::Null, format!("invalid JSON: {e}")));
            return;
        }
    };
    let (id, command) = if value.get("type").is_some() {
        (serde_json::Value::Null, value)
    } else {
        let id = value.get("id").cloned().unwrap_or(serde_json::Value::Null);
        if value.get("op").and_then(|o| o.as_str()) == Some("snapshot") {
            let _ = reply.send(encode(&ServerMsg::Snapshot {
                data: &st.hub.snapshot(),
            }));
            let _ = reply.send(ack(
                id,
                CommandReply {
                    ok: true,
                    ..Default::default()
                },
            ));
            return;
        }
        match value.get("command") {
            Some(c) => (id, c.clone()),
            None => {
                let _ = reply.send(fail(id, "expected {\"id\", \"command\"}".into()));
                return;
            }
        }
    };
    let cmd: AppCommand = match serde_json::from_value(command) {
        Ok(c) => c,
        Err(e) => {
            let _ = reply.send(fail(id, format!("invalid command: {e}")));
            return;
        }
    };
    let hub = st.hub.clone();
    let reply = reply.clone();
    tokio::spawn(async move {
        let r = dispatch(&hub, cmd).await;
        let _ = reply.send(ack(id, r));
    });
}
