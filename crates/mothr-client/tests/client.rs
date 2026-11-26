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
    fn list_dir(&self, _project: &std::path::Path, dir: &str) -> anyhow::Result<DirListing> {
        if dir == "missing" {
            anyhow::bail!("“missing” doesn't exist");
        }
        Ok(DirListing {
            path: dir.into(),
            entries: vec![
                FileEntry { name: "src".into(), path: "src".into(), dir: true, size: 0 },
                FileEntry { name: "README.md".into(), path: "README.md".into(), dir: false, size: 2048 },
            ],
            truncated: false,
            readme: Some("README.md".into()),
        })
    }
    fn read_file(&self, _project: &std::path::Path, file: &str) -> anyhow::Result<FileContent> {
        Ok(FileContent { path: file.into(), size: 12, language: language_for(file).into(), text: Some("# Hi\n\nThere\n".into()), truncated: false })
    }
    fn find_files(&self, _project: &std::path::Path, _query: &str, _limit: usize) -> anyhow::Result<Vec<FileHit>> {
        Ok(vec![FileHit { path: "src/ui/sidebar.rs".into(), indices: vec![7, 9, 10] }])
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
    // Images are served from the profile's images folder; keep it throwaway.
    let home = std::env::temp_dir().join(format!("mothr-client-test-{}", uuid::Uuid::new_v4()));
    // SAFETY: the only test in this binary, set before any thread reads the environment.
    unsafe { std::env::set_var("MOTHR_HOME", &home) };
    std::fs::create_dir_all(images_dir()).unwrap();
    std::fs::write(images_dir().join("shot.png"), b"\x89PNG fake").unwrap();
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
    assert!(client.connection().computer.is_some_and(|name| !name.is_empty()));
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

    // A picture the agent produced: the phone fetches it by name.
    let path = images_dir().join("shot.png");
    let _ = hub.tx.send(HubEvent::Session { id, event: SessionEvent::Upsert { entry: Entry::Image { id: "img".into(), path, mime: Some("image/png".into()) } } });
    wait_until(&client, "image entry", |c| {
        c.chat(id.to_string()).is_some_and(|d| matches!(d.entries.last(), Some(EntryView::Image { name: Some(n), .. }) if n == "shot.png"))
    });
    assert_eq!(client.image("shot.png".into()).unwrap(), b"\x89PNG fake");
    assert!(matches!(client.image("gone.png".into()), Err(ClientError::Rejected { .. })));

    // Commands reach the hub as AppCommands.
    client.send(id.to_string(), "hello".into(), Delivery::Steer).unwrap();
    let new_id = client.new_chat("copilot".into(), String::new(), "plan".into(), Some("claude-opus-5.5".into()), Some("high".into()), Some("long_context".into()), Some("hi".into()), false).unwrap();
    assert!(uuid::Uuid::parse_str(&new_id).is_ok());
    let hits = client.search_projects("web".into()).unwrap();
    assert_eq!(hits[0].display_path, "~/web");
    assert_eq!(client.find_projects("web".into(), 500).unwrap()[0].name, "web");
    // The mode changes mid-chat; the chat shows it once the agent has switched.
    client.set_mode(id.to_string(), "allow_all".into()).unwrap();
    let _ = hub.tx.send(HubEvent::Session { id, event: SessionEvent::Mode { mode: PermissionMode::AllowAll } });
    wait_until(&client, "mode switched", |c| c.chat(id.to_string()).is_some_and(|d| d.mode.id == "allow_all" && d.mode.dangerous));
    {
        let d = hub.dispatched.lock().unwrap();
        assert!(matches!(&d[0], AppCommand::Session { command: SessionCommand::Prompt { delivery: mothr_core::Delivery::Steer, .. }, .. }));
        assert!(matches!(
            &d[1],
            AppCommand::NewSession { agent: AgentKind::Copilot, mode: PermissionMode::Plan, model: Some(m), effort: Some(e), context: Some(c), .. }
                if m == "claude-opus-5.5" && e == "high" && c == "long_context"
        ));
        assert!(matches!(&d[2], AppCommand::Session { id: s, command: SessionCommand::SetMode { mode: PermissionMode::AllowAll } } if *s == id));
    }
    assert!(matches!(client.send(id.to_string(), "  ".into(), Delivery::Normal), Err(ClientError::Invalid { .. })));

    // Project files.
    let dir = client.list_dir("/home/u/web".into(), String::new()).unwrap();
    assert!(dir.entries[0].dir);
    assert!(dir.entries[1].markdown);
    assert_eq!(dir.entries[1].size_label, "2.0 KB");
    let file = client.read_file("/home/u/web".into(), "README.md".into()).unwrap();
    assert!(file.markdown);
    assert_eq!(file.name, "README.md");
    assert_eq!(file.line_count, 3);
    let found = client.find_files("/home/u/web".into(), "sb".into()).unwrap();
    assert_eq!((found[0].name.as_str(), found[0].folder.as_str()), ("sidebar.rs", "src/ui"));
    assert_eq!(found[0].name_indices, vec![0, 2, 3]);
    assert_eq!(client.list_dir("/home/u/web".into(), "missing".into()), Err(ClientError::Rejected { reason: "“missing” doesn't exist".into() }));

    // A wrong token is reported as unauthorized, not retried forever.
    let bad = MothrClient::new(link.replace(&token, "wrong-token-wrong-token")).unwrap();
    bad.connect();
    wait_until(&bad, "unauthorized", |c| c.connection().state == ConnectionState::Unauthorized);
    assert_eq!(bad.refresh_usage(), Err(ClientError::Unauthorized));

    drop(client);
    rt.block_on(server.stop());
    let _ = std::fs::remove_dir_all(home);
}
