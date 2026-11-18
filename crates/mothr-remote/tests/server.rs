//! End-to-end tests of the remote server against an in-memory hub.

use futures::{SinkExt, StreamExt};
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, COOKIE, LOCATION, SET_COOKIE};
use serde_json::{Value, json};
use mothr_core::*;
use mothr_remote::{RemoteServer, generate_token};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

struct FakeHub {
    tx: broadcast::Sender<HubEvent>,
    sessions: Mutex<Vec<SessionState>>,
    dispatched: Mutex<Vec<AppCommand>>,
}

impl FakeHub {
    fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(256);
        let mut meta = SessionMeta::new(
            AgentKind::ClaudeCode,
            SessionKind::Chat,
            PathBuf::from("/tmp/demo"),
            PermissionMode::Ask,
        );
        meta.title = "Demo session".into();
        let mut state = SessionState::new(meta);
        state.apply(&SessionEvent::Upsert {
            entry: Entry::User {
                id: "u1".into(),
                text: "hello".into(),
                attachments: vec![],
                at: chrono::Utc::now(), agent_ref: None, prev_ref: None
            },
        });
        Arc::new(Self {
            tx,
            sessions: Mutex::new(vec![state]),
            dispatched: Mutex::new(vec![]),
        })
    }
    fn first_id(&self) -> SessionId {
        self.sessions.lock().unwrap()[0].meta.id
    }
}

impl HubApi for FakeHub {
    fn snapshot(&self) -> HubSnapshot {
        HubSnapshot {
            sessions: self.sessions.lock().unwrap().clone(),
            agents: vec![AgentInstall {
                kind: AgentKind::ClaudeCode,
                binary: Some("/usr/bin/claude".into()),
                version: Some("2.0".into()),
                transport: Some(Transport::ClaudeStreamJson {
                    program: "/usr/bin/claude".into(),
                }),
                note: None,
            }],
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
        let is_new = matches!(command, AppCommand::NewSession { .. });
        if let AppCommand::Delete { .. } = command {
            anyhow::bail!("delete refused by fake hub");
        }
        self.dispatched.lock().unwrap().push(command);
        Ok(is_new.then(uuid::Uuid::new_v4))
    }
    fn search_projects(&self, query: &str, limit: usize) -> Vec<ProjectHit> {
        (0..limit.min(2))
            .map(|i| ProjectHit {
                project: ProjectRef::from_path(PathBuf::from(format!("/home/u/{query}{i}"))),
                score: 100 - i as u32,
                name_indices: vec![0],
                path_indices: vec![],
                display_path: format!("~/{query}{i}"),
            })
            .collect()
    }
}

async fn start(hub: Arc<FakeHub>, port: u16) -> (RemoteServer, String, String) {
    let token = generate_token();
    let settings = RemoteSettings {
        enabled: true,
        bind: "127.0.0.1".into(),
        port,
        token: token.clone(),
        tunnel: TunnelKind::None,
    };
    let server = RemoteServer::start(hub, settings)
        .await
        .expect("server starts");
    let base = server.status().local_url.expect("local url");
    (server, base, token)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_auth_and_api() {
    let hub = FakeHub::new();
    let (server, base, token) = start(hub.clone(), 0).await;
    assert!(base.starts_with("http://127.0.0.1:"), "{base}");
    let status = server.status();
    assert!(status.running);
    assert_eq!(status.tunnel, TunnelKind::None);
    assert_eq!(
        server.pairing_url().unwrap(),
        format!("{base}?token={token}")
    );
    let http = client();

    // Unauthorized API access, with the brute-force delay.
    let t0 = std::time::Instant::now();
    let res = http
        .get(format!("{base}api/snapshot"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert!(t0.elapsed() >= Duration::from_millis(450));
    let res = http
        .get(format!("{base}api/snapshot"))
        .header(AUTHORIZATION, "Bearer wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Bearer and query token.
    let res = http
        .get(format!("{base}api/snapshot"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let snap: HubSnapshot = res.json().await.unwrap();
    assert_eq!(snap.sessions.len(), 1);
    assert_eq!(snap.sessions[0].meta.title, "Demo session");
    let res = http
        .get(format!("{base}api/snapshot?token={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Cookie pairing flow.
    let res = http.get(&base).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert!(res.text().await.unwrap().contains("Pair with Mothr"));
    let res = http.get(format!("{base}?token=nope")).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = http
        .get(format!("{base}?token={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(res.headers()[LOCATION], "/");
    let cookie = res.headers()[SET_COOKIE].to_str().unwrap().to_string();
    assert!(
        cookie.starts_with(&format!("mothr_token={token};")),
        "{cookie}"
    );
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"),
        "{cookie}"
    );
    let cookie_header = format!("other=1; mothr_token={token}");
    let res = http
        .get(&base)
        .header(COOKIE, &cookie_header)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers().contains_key("content-security-policy"));
    let html = res.text().await.unwrap();
    assert!(html.contains("<script>") && html.contains("/api/ws"));
    let res = http
        .get(format!("{base}api/snapshot"))
        .header(COOKIE, &cookie_header)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Manifest: public, token start_url only when authenticated.
    let m: Value = http
        .get(format!("{base}manifest.webmanifest"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(m["start_url"], "/");
    assert_eq!(m["display"], "standalone");
    let m: Value = http
        .get(format!("{base}manifest.webmanifest"))
        .header(COOKIE, &cookie_header)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(m["start_url"], format!("/?token={token}"));

    // Sessions and project search.
    let id = hub.first_id();
    let res = http
        .get(format!("{base}api/sessions/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let s: SessionState = res.json().await.unwrap();
    assert_eq!(s.entries.len(), 1);
    let res = http
        .get(format!("{base}api/sessions/{}", uuid::Uuid::new_v4()))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let hits: Vec<ProjectHit> = http
        .get(format!("{base}api/projects?q=shep&limit=5"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].project.name, "shep0");

    // Commands.
    let cmd = AppCommand::NewSession {
        agent: AgentKind::Codex,
        kind: SessionKind::Chat,
        project: "/home/u/x".into(),
        mode: PermissionMode::AcceptEdits,
        model: None,
        prompt: Some("hi".into()),
    };
    let res = http
        .post(format!("{base}api/command"))
        .bearer_auth(&token)
        .json(&cmd)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let reply: Value = res.json().await.unwrap();
    assert_eq!(reply["ok"], true);
    assert!(reply["session_id"].is_string());
    assert_eq!(hub.dispatched.lock().unwrap().last(), Some(&cmd));
    let res = http
        .post(format!("{base}api/command"))
        .bearer_auth(&token)
        .json(&AppCommand::Delete { id })
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let reply: Value = res.json().await.unwrap();
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().unwrap().contains("refused"));
    let res = http
        .post(format!("{base}api/command"))
        .bearer_auth(&token)
        .body("{\"type\":\"bogus\"}")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let res = http
        .post(format!("{base}api/command"))
        .json(&cmd)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    server.stop().await;
    assert!(
        http.get(format!("{base}api/snapshot"))
            .bearer_auth(&token)
            .send()
            .await
            .is_err()
    );
}

async fn next_json<S>(ws: &mut S) -> Value
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("message in time")
            .unwrap()
            .unwrap();
        if let Message::Text(t) = msg {
            return serde_json::from_str(t.as_str()).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_snapshot_events_and_commands() {
    let hub = FakeHub::new();
    let (server, base, token) = start(hub.clone(), 0).await;
    let ws_base = base.replacen("http://", "ws://", 1);
    let mut status = server.subscribe_status();

    // No token: the upgrade is refused.
    assert!(
        tokio_tungstenite::connect_async(format!("{ws_base}api/ws"))
            .await
            .is_err()
    );

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("{ws_base}api/ws?token={token}"))
        .await
        .unwrap();
    let first = next_json(&mut ws).await;
    assert_eq!(first["type"], "snapshot");
    assert_eq!(
        first["data"]["sessions"][0]["meta"]["title"],
        "Demo session"
    );
    tokio::time::timeout(
        Duration::from_secs(5),
        status.wait_for(|s| s.connected_clients == 1),
    )
    .await
    .unwrap()
    .unwrap();

    // Hub broadcasts are forwarded verbatim.
    let id = hub.first_id();
    let event = HubEvent::Session {
        id,
        event: SessionEvent::AppendText {
            entry_id: "a1".into(),
            thinking: false,
            delta: "Hel".into(),
        },
    };
    hub.tx.send(event.clone()).unwrap();
    let msg = next_json(&mut ws).await;
    assert_eq!(msg["type"], "event");
    let got: HubEvent = serde_json::from_value(msg["data"].clone()).unwrap();
    assert_eq!(got, event);

    // Commands from the client are dispatched and acked with the client's id.
    let cmd = AppCommand::Session {
        id,
        command: SessionCommand::Prompt {
            text: "go".into(),
            attachments: vec![], delivery: Default::default()
        },
    };
    ws.send(Message::Text(
        json!({ "id": 7, "command": cmd }).to_string().into(),
    ))
    .await
    .unwrap();
    let ack = next_json(&mut ws).await;
    assert_eq!(ack, json!({ "type": "ack", "id": 7, "ok": true }));
    assert_eq!(hub.dispatched.lock().unwrap().last(), Some(&cmd));

    ws.send(Message::Text(
        json!({ "id": 8, "command": { "type": "nope" } })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let ack = next_json(&mut ws).await;
    assert_eq!(ack["id"], 8);
    assert_eq!(ack["ok"], false);

    ws.send(Message::Text(
        json!({ "id": 9, "op": "snapshot" }).to_string().into(),
    ))
    .await
    .unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "snapshot");
    assert_eq!(
        next_json(&mut ws).await,
        json!({ "type": "ack", "id": 9, "ok": true })
    );

    // Lagging clients get a fresh snapshot instead of a gap.
    for i in 0..300 {
        let _ = hub.tx.send(HubEvent::ProjectsIndexed { count: i });
    }
    let mut saw_snapshot = false;
    for _ in 0..300 {
        if next_json(&mut ws).await["type"] == "snapshot" {
            saw_snapshot = true;
            break;
        }
    }
    assert!(saw_snapshot, "lag should trigger a snapshot");

    ws.close(None).await.unwrap();
    drop(ws);
    tokio::time::timeout(
        Duration::from_secs(5),
        status.wait_for(|s| s.connected_clients == 0),
    )
    .await
    .unwrap()
    .unwrap();

    // A connected client is closed when the server stops.
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("{ws_base}api/ws?token={token}"))
        .await
        .unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "snapshot");
    server.stop().await;
    let end = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(m)) = ws.next().await {
            if let Message::Close(_) = m {
                return true;
            }
        }
        true
    })
    .await;
    assert!(end.unwrap());
    assert!(!status.borrow().running);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn falls_back_to_next_port() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    if port > 65000 {
        return;
    }
    let (server, base, _) = start(FakeHub::new(), port).await;
    let bound = server.local_addr().port();
    assert!(bound > port && bound <= port + 10, "{bound} vs {port}");
    assert!(base.ends_with(&format!(":{bound}/")));
    server.stop().await;
}

#[tokio::test]
async fn rejects_missing_token() {
    let settings = RemoteSettings {
        token: String::new(),
        port: 0,
        ..Default::default()
    };
    assert!(RemoteServer::start(FakeHub::new(), settings).await.is_err());
}
