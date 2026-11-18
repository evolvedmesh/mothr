//! Headless driver smoke test: `cargo run -p mothr-agents --example drive -- <agent_id> <cwd> "<prompt>" [model]`
//! `DRIVE_PERMISSION=allow|always|reject` picks how permission requests are answered (default allow).
use mothr_agents::*;
use mothr_core::*;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let kind = AgentKind::from_id(&args[1]).expect("agent id");
    let install = detect(kind, &Default::default());
    println!("install: {install:?}");
    let transport = install.transport.expect("not available");
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel();
    let meta = SessionMeta::new(kind, SessionKind::Chat, args[2].clone().into(), PermissionMode::Ask);
    // DRIVE_ATTACH=<image path> attaches an image to the prompt.
    let spec = SessionSpec {
        id: meta.id, agent: kind, kind: SessionKind::Chat, cwd: args[2].clone().into(), mode: PermissionMode::Ask,
        model: args.get(4).cloned(), resume: None, resume_at: None, system_prompt: None, title: "smoke".into(),
    };
    let mut state = SessionState::new(meta);
    let cmd = spawn(&transport, spec, out_tx).unwrap();
    cmd.send(SessionCommand::Prompt { text: args[3].clone(), attachments: std::env::var("DRIVE_ATTACH").ok().map(|p| vec![Attachment { path: p.into(), mime: Some("image/png".into()) }]).unwrap_or_default(), delivery: Default::default()}).unwrap();
    let mut prompted = true;
    while let Some(o) = out_rx.recv().await {
        match o {
            DriverOutput::Event(_, e) => {
                match &e {
                    SessionEvent::AppendText { .. } => {}
                    other => println!("EVENT {}", serde_json::to_string(other).unwrap().chars().take(300).collect::<String>()),
                }
                state.apply(&e);
                if let SessionEvent::Upsert { entry: Entry::Permission(p) } = &e {
                    for d in &p.diffs {
                        println!("DIFF line {:?} {:?}\n{}", d.start_line, d.stats(), unified_patch("file", &[d]));
                    }
                    let want = match std::env::var("DRIVE_PERMISSION").as_deref() {
                        Ok("reject") => PermissionOptionKind::RejectOnce,
                        Ok("always") => PermissionOptionKind::AllowAlways,
                        _ => PermissionOptionKind::AllowOnce,
                    };
                    let opt = p.options.iter().find(|o| o.kind == want).or(p.options.first()).unwrap();
                    cmd.send(SessionCommand::RespondPermission { entry_id: p.id.clone(), option_id: opt.id.clone() }).unwrap();
                }
                if let SessionEvent::Upsert { entry: Entry::Question(q) } = &e {
                    let answers = q.questions.iter().map(|qq| vec![qq.options.first().map(|o| o.label.clone()).unwrap_or("yes".into())]).collect();
                    cmd.send(SessionCommand::AnswerQuestion { entry_id: q.id.clone(), answers }).unwrap();
                }
                if matches!(e, SessionEvent::Status { status: SessionStatus::Idle }) && prompted && state.entries.iter().any(|e| matches!(e, Entry::TurnEnd{..})) {
                    prompted = false;
                    cmd.send(SessionCommand::Shutdown).unwrap();
                }
            }
            DriverOutput::SubEvent(_, tool, e) => println!("SUBAGENT {tool}: {}", serde_json::to_string(&e).unwrap().chars().take(200).collect::<String>()),
            DriverOutput::Usage(u) => println!("USAGE {u:?}"),
            DriverOutput::Exited(_, err) => { println!("EXITED {err:?}"); break; }
        }
    }
    println!("\n==== FINAL TRANSCRIPT ({} entries, status {:?}, ctx {:?})", state.entries.len(), state.meta.status, state.meta.context);
    for e in &state.entries { println!("- {}", serde_json::to_string(e).unwrap().chars().take(220).collect::<String>()); }
}
