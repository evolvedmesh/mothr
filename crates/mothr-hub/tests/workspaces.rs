//! Workspaces: creating, renaming, reordering and removing them, moving chats between them,
//! and keeping them across restarts. Runs against a throwaway `MOTHR_HOME`.

use mothr_core::*;
use mothr_hub::Hub;
use std::sync::Arc;

fn start(rt: &tokio::runtime::Runtime, home: &std::path::Path) -> Arc<Hub> {
    let settings = Settings { project_roots: vec![home.to_path_buf()], usage_poll_secs: 3600, ..Default::default() };
    Hub::start(rt.handle().clone(), settings)
}

#[test]
fn workspaces_hold_chats_and_survive_restarts() {
    let home = std::env::temp_dir().join(format!("mothr-ws-test-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: this test binary runs this one test, so nothing else reads the environment.
    unsafe { std::env::set_var("MOTHR_HOME", &home) };
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();

    // A chat saved by a version without workspaces.
    let old = SessionMeta::new(AgentKind::ClaudeCode, SessionKind::Chat, home.clone(), PermissionMode::Ask);
    let old_id = old.id;
    std::fs::create_dir_all(sessions_dir()).unwrap();
    std::fs::write(sessions_dir().join(format!("{old_id}.json")), serde_json::to_vec(&SessionState::new(old)).unwrap()).unwrap();

    let hub = start(&rt, &home);
    let snap = hub.snapshot();
    assert_eq!(snap.workspaces.len(), 1, "a first workspace is made");
    let main = snap.workspaces[0].clone();
    assert_eq!(main.name, DEFAULT_WORKSPACE_NAME);
    assert_eq!(snap.active_workspace, Some(main.id));
    let chat = |hub: &Hub| hub.session(old_id).unwrap().meta.workspace;
    assert_eq!(chat(&hub), Some(main.id), "older chats join it");

    // Make more, up to the limit; a blank name gets a numbered one.
    let mut second = Workspace::new("  ");
    second.icon = Some("rocket".into());
    hub.dispatch(AppCommand::CreateWorkspace { workspace: second.clone() }).unwrap();
    let ws = hub.snapshot().workspaces;
    assert_eq!(ws[1].name, "Workspace 2");
    assert_eq!(ws[1].icon.as_deref(), Some("rocket"));
    for i in 3..=MAX_WORKSPACES {
        hub.dispatch(AppCommand::CreateWorkspace { workspace: Workspace::new(format!("W{i}")) }).unwrap();
    }
    assert!(hub.dispatch(AppCommand::CreateWorkspace { workspace: Workspace::new("one too many") }).is_err());

    // Rename (not to nothing), reorder, select.
    let mut renamed = hub.snapshot().workspaces[1].clone();
    renamed.name = "Side projects".into();
    hub.dispatch(AppCommand::UpdateWorkspace { workspace: renamed.clone() }).unwrap();
    renamed.name = "   ".into();
    assert!(hub.dispatch(AppCommand::UpdateWorkspace { workspace: renamed.clone() }).is_err());
    hub.dispatch(AppCommand::MoveWorkspace { id: second.id, index: 0 }).unwrap();
    assert_eq!(hub.snapshot().workspaces[0].name, "Side projects");
    hub.dispatch(AppCommand::SelectWorkspace { id: second.id }).unwrap();
    assert_eq!(hub.snapshot().active_workspace, Some(second.id));

    // Move the chat, then remove its workspace: it lands in the first one left.
    hub.dispatch(AppCommand::SetWorkspace { id: old_id, workspace: main.id }).unwrap();
    hub.dispatch(AppCommand::SetWorkspace { id: old_id, workspace: second.id }).unwrap();
    assert_eq!(chat(&hub), Some(second.id));
    hub.dispatch(AppCommand::DeleteWorkspace { id: second.id }).unwrap();
    let snap = hub.snapshot();
    assert_eq!(snap.workspaces.len(), MAX_WORKSPACES - 1);
    assert_eq!(snap.workspaces[0].id, main.id);
    assert_eq!(chat(&hub), Some(main.id));
    assert_eq!(snap.active_workspace, Some(main.id), "removing the open workspace opens the first");

    // The last one can't go.
    for w in snap.workspaces.iter().skip(1) {
        hub.dispatch(AppCommand::DeleteWorkspace { id: w.id }).unwrap();
    }
    assert!(hub.dispatch(AppCommand::DeleteWorkspace { id: main.id }).is_err());

    // Kept across a restart.
    hub.dispatch(AppCommand::CreateWorkspace { workspace: Workspace::new("Later") }).unwrap();
    hub.shutdown();
    drop(hub);
    let hub = start(&rt, &home);
    let names: Vec<String> = hub.snapshot().workspaces.into_iter().map(|w| w.name).collect();
    assert_eq!(names, vec![DEFAULT_WORKSPACE_NAME.to_string(), "Later".to_string()]);
    assert_eq!(chat(&hub), Some(main.id));
    hub.shutdown();
    drop(hub);
    rt.shutdown_background();
    let _ = std::fs::remove_dir_all(&home);
}
