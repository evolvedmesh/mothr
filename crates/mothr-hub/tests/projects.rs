//! A chat can work across more projects than its primary one; they can be changed later.
//! Runs against a throwaway `MOTHR_HOME`.

use mothr_core::*;
use mothr_hub::Hub;

#[test]
fn chats_take_more_projects() {
    let home = std::env::temp_dir().join(format!("mothr-projects-test-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
    let (primary, other, third) = (home.join("app"), home.join("lib"), home.join("docs"));
    for d in [&primary, &other, &third] {
        std::fs::create_dir_all(d).unwrap();
    }
    // SAFETY: this test binary runs this one test, so nothing else reads the environment.
    unsafe { std::env::set_var("MOTHR_HOME", &home) };
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();

    let chat = SessionMeta::new(AgentKind::ClaudeCode, SessionKind::Chat, primary.clone(), PermissionMode::Ask);
    let mut general = SessionMeta::new(AgentKind::ClaudeCode, SessionKind::Chat, home.clone(), PermissionMode::Ask);
    general.general = true;
    let (id, general_id) = (chat.id, general.id);
    std::fs::create_dir_all(sessions_dir()).unwrap();
    for m in [chat, general] {
        std::fs::write(sessions_dir().join(format!("{}.json", m.id)), serde_json::to_vec(&SessionState::new(m)).unwrap()).unwrap();
    }

    let settings = Settings { project_roots: vec![home.join("none")], usage_poll_secs: 3600, ..Default::default() };
    let hub = Hub::start(rt.handle().clone(), settings);
    let extra = |hub: &Hub| hub.session(id).unwrap().meta.extra_projects;

    // The primary and repeats are dropped; order is kept.
    let cmd = |projects: Vec<std::path::PathBuf>| AppCommand::SetExtraProjects { id, projects };
    hub.dispatch(cmd(vec![third.clone(), primary.clone(), other.clone(), third.clone()])).unwrap();
    assert_eq!(extra(&hub), vec![third.clone(), other.clone()]);
    // Their files can be browsed like the primary project's.
    std::fs::write(other.join("README.md"), "hi").unwrap();
    assert!(hub.read_file(&other, "README.md").is_ok());
    hub.dispatch(cmd(vec![other.clone()])).unwrap();
    assert_eq!(extra(&hub), vec![other.clone()]);
    assert!(hub.read_file(&third, "README.md").is_err(), "a removed project can't be browsed");

    // General chats aren't in a project, so they can't add one.
    assert!(hub.dispatch(AppCommand::SetExtraProjects { id: general_id, projects: vec![other.clone()] }).is_err());

    hub.shutdown();
    drop(hub);
    rt.shutdown_background();
    let _ = std::fs::remove_dir_all(&home);
}
