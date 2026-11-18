//! The client against the real remote server and an in-memory hub.

use mothr_client::{ChatStatus, ClientError, ConnectionState, Delivery, EntryView, MothrClient};
use mothr_core::*;
use mothr_remote::{RemoteServer, generate_token};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

struct FakeHub {
    tx: broadcast::Sender<HubEvent>,
    sessions: Mutex<Vec<SessionState>>,
    dispatched: Mutex<Vec<AppCommand>>,
}

impl FakeHub {
    fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(256);
        let mut meta = SessionMeta::new(AgentKind::ClaudeCode, SessionKind::Chat, PathBuf::from("/tmp/demo"), PermissionMode::Ask);
        meta.title = "Demo session".into();
        meta.status = SessionStatus::Idle;
        Arc::new(Self { tx, sessions: Mutex::new(vec![SessionState::new(meta)]), dispatched: Mutex::new(vec![]) })
    }
    fn id(&self) -> SessionId {
        self.sessions.lock().unwrap()[0].meta.id
    }
}

impl HubApi for FakeHub {
    fn snapshot(&self) -> HubSnapshot {
        HubSnapshot { sessions: self.sessions.lock().unwrap().clone(), ..Default::default() }
    }
    fn session(&self, id: SessionId) -> Option<SessionState> {
        self.sessions.lock().unwrap().iter().find(|s| s.meta.id == id).cloned()
    }
    fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.tx.subscribe()
    }
    fn dispatch(&self, command: AppCommand) -> anyhow::Result<Option<SessionId>> {
        if let AppCommand::Delete { .. } = command {
            anyhow::bail!("refused");
        }
        let new = matches!(command, AppCommand::NewSession { .. });
        self.dispatched.lock().unwrap().push(command);
        Ok(new.then(uuid::Uuid::new_v4))
    }
    fn search_projects(&self, query: &str, _limit: usize) -> Vec<ProjectHit> {
        vec![ProjectHit {
            project: ProjectRef::from_path(PathBuf::from(format!("/home/u/{query}"))),
            score: 1,
            name_indices: vec![],
            path_indices: vec![],
            display_path: format!("~/{query}"),
        }]
    }
}

fn wait_until(client: &MothrClient, what: &str, mut ok: impl FnMut(&MothrClient) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut rev = 0;
    while !ok(client) {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        rev = client.wait_for_change(rev, 500);
    }
}

#[test]
fn pairs_streams_and_sends_commands() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let hub = FakeHub::new();
    let token = generate_token();
    let settings = RemoteSettings { enabled: true, bind: "127.0.0.1".into(), port: 0, token: token.clone(), tunnel: TunnelKind::None };
    let server = rt.block_on(RemoteServer::start(hub.clone(), settings)).unwrap();
    let link = server.pairing_url().unwrap();

    let client = MothrClient::new(link.clone()).unwrap();
    client.connect();
    wait_until(&client, "snapshot", |c| c.connection().loaded);
    assert_eq!(client.connection().state, ConnectionState::Connected);
    let groups = client.chat_groups(String::new());
    assert_eq!(groups[0].chats[0].title, "Demo session");
    assert_eq!(groups[0].chats[0].status, ChatStatus::Ready);

    // Live events flow through the shared reducer.
    let id = hub.id();
    for d in ["Hi ", "there"] {
        let _ = hub.tx.send(HubEvent::Session { id, event: SessionEvent::AppendText { entry_id: "a".into(), thinking: false, delta: d.into() } });
    }
    wait_until(&client, "streamed text", |c| {
        c.chat(id.to_string()).is_some_and(|d| matches!(d.entries.last(), Some(EntryView::Assistant { text, .. }) if text == "Hi there"))
    });

    // Commands reach the hub as AppCommands.
    client.send(id.to_string(), "hello".into(), Delivery::Steer).unwrap();
    let new_id = client.new_chat("copilot".into(), String::new(), "plan".into(), None, Some("hi".into()), false).unwrap();
    assert!(uuid::Uuid::parse_str(&new_id).is_ok());
    let hits = client.search_projects("web".into()).unwrap();
    assert_eq!(hits[0].display_path, "~/web");
    {
        let d = hub.dispatched.lock().unwrap();
        assert!(matches!(&d[0], AppCommand::Session { command: SessionCommand::Prompt { delivery: mothr_core::Delivery::Steer, .. }, .. }));
        assert!(matches!(&d[1], AppCommand::NewSession { agent: AgentKind::Copilot, mode: PermissionMode::Plan, .. }));
    }
    assert!(matches!(client.send(id.to_string(), "  ".into(), Delivery::Normal), Err(ClientError::Invalid { .. })));

    // A wrong token is reported as unauthorized, not retried forever.
    let bad = MothrClient::new(link.replace(&token, "wrong-token-wrong-token")).unwrap();
    bad.connect();
    wait_until(&bad, "unauthorized", |c| c.connection().state == ConnectionState::Unauthorized);
    assert_eq!(bad.refresh_usage(), Err(ClientError::Unauthorized));

    drop(client);
    rt.block_on(server.stop());
}
