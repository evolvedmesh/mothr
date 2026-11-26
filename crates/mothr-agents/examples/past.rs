//! List the conversations an agent keeps on its own, as Mothr's resume picker sees them.
//! `cargo run -p mothr-agents --example past -- claude_code [session-id]`
//! With a session id (Claude Code), prints the history Mothr would import.

use mothr_agents::{detect, history};
use mothr_core::{AgentKind, Entry};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let kind = AgentKind::from_id(args.get(1).map(String::as_str).unwrap_or("claude_code")).expect("agent id");
    let transport = detect(kind, &Default::default()).transport.expect("not available");
    let t = std::time::Instant::now();
    let chats = history::past_chats(&transport, kind).expect("list");
    println!("{} chats in {:?}", chats.len(), t.elapsed());
    for c in chats.iter().take(15) {
        println!("{}  {}  {}  {}", c.updated_at.map(|t| t.to_rfc3339()).unwrap_or_default(), c.id, c.cwd.display(), c.title);
    }
    if let Some(id) = args.get(2) {
        let chat = chats.iter().find(|c| &c.id == id).expect("no such chat");
        for e in history::claude_history(id, &chat.cwd) {
            let line = match &e {
                Entry::User { text, prev_ref, .. } => format!("USER ({prev_ref:?}) {text}"),
                Entry::Assistant { text, .. } => format!("AGENT {text}"),
                Entry::Thinking { .. } => "THINKING".into(),
                Entry::Tool(t) => format!("TOOL {} {:?} {}", t.title, t.status, t.input.clone().unwrap_or_default()),
                Entry::Plan { items, .. } => format!("PLAN {} items", items.len()),
                other => format!("{other:?}"),
            };
            println!("  {}", line.chars().take(140).collect::<String>().replace('\n', " "));
        }
    }
}
