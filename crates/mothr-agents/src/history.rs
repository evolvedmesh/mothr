//! Conversations agents keep on their own, so Mothr can list them and pick one up.
//!
//! - Claude Code keeps a JSONL transcript per session under `~/.claude/projects/<dir>/`.
//!   Mothr reads the list from there, and the history too: `--resume` doesn't replay it.
//! - ACP agents are asked with `session/list` (Copilot, Hermes, OpenCode, codex-acp) and
//!   replay the history themselves on `session/load`.
//! - Codex rollouts (`~/.codex/sessions`) are read directly when codex-acp can't list them.

use crate::claude::truncate;
use crate::{edits, summarize_input};
use chrono::{DateTime, Utc};
use mothr_core::{AgentKind, Entry, PastChat, PlanItem, PlanStatus, ToolCall, ToolKind, ToolStatus, Transport};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// At most this many past chats per agent, newest first.
const LIMIT: usize = 400;

/// The conversations `agent` keeps, newest first. Blocking (it may start the agent).
pub fn past_chats(transport: &Transport, agent: AgentKind) -> anyhow::Result<Vec<PastChat>> {
    let mut chats = match transport {
        Transport::ClaudeStreamJson { .. } => claude_chats(LIMIT),
        Transport::Acp { program, args, env } => match acp_list(program, args, env, agent)? {
            Some(chats) => chats,
            None if agent == AgentKind::Codex => codex_chats(LIMIT),
            None => anyhow::bail!("{} can't list its past chats", agent.display_name()),
        },
    };
    // A conversation can only be resumed in a folder that's still there.
    chats.retain(|c| c.cwd.is_dir());
    chats.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
    chats.truncate(LIMIT);
    Ok(chats)
}

/// A title for display: one line, no Mothr notes, never empty.
fn clean_title(raw: &str) -> String {
    let t = raw.trim();
    // Chats started from Mothr send their notes ahead of the prompt; agents that title a chat
    // after its first prompt then show the note.
    if t.is_empty() || t.starts_with("[Mothr]") {
        return "Untitled chat".into();
    }
    let line = t.lines().find(|l| !l.trim().is_empty()).unwrap_or(t).trim();
    let mut s: String = line.chars().take(120).collect();
    if line.chars().count() > 120 {
        s.push('…');
    }
    s
}

fn mtime(path: &Path) -> Option<DateTime<Utc>> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(DateTime::<Utc>::from(t))
}

/// Up to `n` bytes from the start and the end of a file (the whole file when it's small).
fn head_tail(path: &Path, n: u64) -> Option<(String, String)> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut head = vec![];
    (&mut f).take(n).read_to_end(&mut head).ok()?;
    let tail = if len > n {
        let mut tail = vec![];
        f.seek(SeekFrom::Start(len.saturating_sub(n).max(n))).ok()?;
        f.read_to_end(&mut tail).ok()?;
        tail
    } else {
        vec![]
    };
    Some((String::from_utf8_lossy(&head).into_owned(), String::from_utf8_lossy(&tail).into_owned()))
}

// ---------------------------------------------------------------------------------- Claude

fn claude_home() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".claude"))
}

/// Claude Code's sessions: one transcript per session, in a folder per project.
pub fn claude_chats(limit: usize) -> Vec<PastChat> {
    let root = claude_home().join("projects");
    let mut files: Vec<(PathBuf, SystemTime)> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|d| d.path().is_dir())
        .flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().flatten())
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
    // Read a few more than needed: some turn out to be empty.
    files.truncate(limit + limit / 2);
    let chunks: Vec<Vec<PathBuf>> = files.chunks(files.len().div_ceil(8).max(1)).map(|c| c.iter().map(|f| f.0.clone()).collect()).collect();
    let mut out: Vec<PastChat> = std::thread::scope(|s| {
        let handles: Vec<_> = chunks.into_iter().map(|c| s.spawn(move || c.iter().filter_map(|p| claude_chat(p)).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    out.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
    out.truncate(limit);
    out
}

/// What a Claude transcript says about itself, from its first and last lines.
fn claude_chat(path: &Path) -> Option<PastChat> {
    let id = path.file_stem()?.to_string_lossy().into_owned();
    let (head, tail) = head_tail(path, 96 * 1024)?;
    let mut cwd = None;
    let mut prompt = None;
    let mut title = None;
    for line in head.lines().chain(tail.lines()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if cwd.is_none() {
            cwd = v["cwd"].as_str().map(PathBuf::from);
        }
        match v["type"].as_str().unwrap_or("") {
            // The last title written wins; one the user set beats a generated one.
            "custom-title" => title = v["customTitle"].as_str().map(|t| (2, t.to_string())).or(title),
            "ai-title" if title.as_ref().is_none_or(|t| t.0 <= 1) => title = v["aiTitle"].as_str().map(|t| (1, t.to_string())).or(title),
            "summary" if title.is_none() => title = v["summary"].as_str().map(|t| (0, t.to_string())),
            "user" if prompt.is_none() && v["isSidechain"] != true && v["isMeta"] != true => prompt = user_text(&v["message"]["content"]),
            _ => {}
        }
    }
    let title = title.map(|t| t.1).or(prompt)?;
    Some(PastChat { agent: AgentKind::ClaudeCode, id, cwd: cwd?, title: clean_title(&title), updated_at: mtime(path), chat: None })
}

/// The text the user typed in a transcript message, if it's a typed message at all (not a
/// tool result, a command's output or a Mothr note).
fn user_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => {
            if blocks.iter().any(|b| b["type"] == "tool_result") {
                return None;
            }
            blocks.iter().filter_map(|b| b["text"].as_str()).filter(|t| !t.starts_with("[Mothr]")).collect::<Vec<_>>().join("\n")
        }
        _ => return None,
    };
    let text = text.trim();
    // `<command-name>`, `<local-command-stdout>`, `<system-reminder>` … aren't typed text.
    (!text.is_empty() && !text.starts_with('<')).then(|| text.to_string())
}

/// The transcript file of a Claude session.
fn claude_transcript_path(session_id: &str, cwd: &Path) -> Option<PathBuf> {
    let root = claude_home().join("projects");
    let file = format!("{session_id}.jsonl");
    // Claude names the folder after the path, every character but letters and digits a dash.
    let encoded: String = cwd.display().to_string().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let direct = root.join(encoded).join(&file);
    if direct.is_file() {
        return Some(direct);
    }
    std::fs::read_dir(&root).ok()?.flatten().map(|d| d.path().join(&file)).find(|p| p.is_file())
}

/// The history of a Claude session as Mothr transcript entries: the branch that ends at the
/// latest message (a rewound conversation keeps its abandoned branches in the file too).
pub fn claude_history(session_id: &str, cwd: &Path) -> Vec<Entry> {
    let Some(path) = claude_transcript_path(session_id, cwd) else { return vec![] };
    let Ok(file) = std::fs::File::open(&path) else { return vec![] };
    let lines: Vec<Value> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<Value>(&l).ok())
        .filter(|v| v["uuid"].is_string() && v["isSidechain"] != true)
        .collect();
    claude_entries(&lines)
}

fn claude_entries(lines: &[Value]) -> Vec<Entry> {
    let by_uuid: HashMap<&str, usize> = lines.iter().enumerate().filter_map(|(i, v)| Some((v["uuid"].as_str()?, i))).collect();
    // Walk back from the newest message; compaction breaks `parentUuid` but keeps a logical link.
    let Some(leaf) = lines.iter().rposition(|v| matches!(v["type"].as_str(), Some("user" | "assistant"))) else { return vec![] };
    let mut chain = vec![];
    let mut seen = std::collections::HashSet::new();
    let mut at = Some(leaf);
    while let Some(i) = at {
        if !seen.insert(i) {
            break;
        }
        chain.push(i);
        let v = &lines[i];
        let parent = v["parentUuid"].as_str().or(v["logicalParentUuid"].as_str());
        at = parent.and_then(|p| by_uuid.get(p).copied());
    }
    chain.reverse();

    let mut entries: Vec<Entry> = vec![];
    let mut tools: HashMap<String, usize> = HashMap::new();
    let mut prev_uuid: Option<String> = None;
    for &i in &chain {
        let v = &lines[i];
        let uuid = v["uuid"].as_str().unwrap_or_default().to_string();
        let content = &v["message"]["content"];
        match v["type"].as_str().unwrap_or("") {
            "user" => {
                if v["isMeta"] != true
                    && v["isCompactSummary"] != true
                    && let Some(text) = user_text(content)
                {
                    let at = v["timestamp"].as_str().and_then(|t| t.parse::<DateTime<Utc>>().ok()).unwrap_or_else(Utc::now);
                    entries.push(Entry::User { id: uuid.clone(), text, attachments: vec![], at, agent_ref: Some(uuid.clone()), prev_ref: prev_uuid.clone() });
                }
                for block in content.as_array().into_iter().flatten().filter(|b| b["type"] == "tool_result") {
                    let Some(&ix) = block["tool_use_id"].as_str().and_then(|id| tools.get(id)) else { continue };
                    let Entry::Tool(call) = &mut entries[ix] else { continue };
                    let text = match &block["content"] {
                        Value::String(s) => s.clone(),
                        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
                        _ => String::new(),
                    };
                    call.status = if block["is_error"].as_bool().unwrap_or(false) { ToolStatus::Failed } else { ToolStatus::Completed };
                    call.output = Some(truncate(&text, 20_000));
                    if call.status == ToolStatus::Completed
                        && !call.diffs.is_empty()
                        && let Some(diffs) = edits::result_diffs(&v["toolUseResult"])
                        && call.locations.first().is_none_or(|p| diffs.first().is_some_and(|d| &d.path == p))
                    {
                        call.diffs = diffs;
                    }
                }
            }
            "assistant" => {
                for (bi, block) in content.as_array().into_iter().flatten().enumerate() {
                    let id = format!("{uuid}:{bi}");
                    match block["type"].as_str().unwrap_or("") {
                        "text" => {
                            let text = block["text"].as_str().unwrap_or("").to_string();
                            if !text.trim().is_empty() {
                                entries.push(Entry::Assistant { id, text, streaming: false });
                            }
                        }
                        "thinking" => {
                            let text = block["thinking"].as_str().unwrap_or("").to_string();
                            if !text.trim().is_empty() {
                                entries.push(Entry::Thinking { id, text, streaming: false });
                            }
                        }
                        "tool_use" => {
                            let tool_id = block["id"].as_str().unwrap_or(&id).to_string();
                            let name = block["name"].as_str().unwrap_or("tool").to_string();
                            let input = &block["input"];
                            if name == "TodoWrite" {
                                entries.push(Entry::Plan { id: format!("plan-{tool_id}"), items: plan_items(input) });
                                continue;
                            }
                            let (title, detail) = summarize_input(&name, input);
                            let path = input["file_path"].as_str().map(PathBuf::from);
                            tools.insert(tool_id.clone(), entries.len());
                            entries.push(Entry::Tool(ToolCall {
                                id: tool_id,
                                kind: ToolKind::from_tool_name(&name),
                                diffs: edits::edit_diffs(input),
                                name,
                                title,
                                // No result recorded: the session stopped before it came.
                                status: ToolStatus::Failed,
                                input: detail,
                                output: None,
                                locations: path.into_iter().collect(),
                            }));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if matches!(v["type"].as_str(), Some("user" | "assistant")) {
            prev_uuid = Some(uuid);
        }
    }
    entries
}

fn plan_items(input: &Value) -> Vec<PlanItem> {
    input["todos"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| PlanItem {
            content: t["content"].as_str().unwrap_or("").to_string(),
            status: match t["status"].as_str() {
                Some("completed") => PlanStatus::Completed,
                Some("in_progress") => PlanStatus::InProgress,
                _ => PlanStatus::Pending,
            },
        })
        .collect()
}

// ---------------------------------------------------------------------------------- ACP

/// Kills the agent when the listing is done (or abandoned).
struct Child(std::process::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Ask an ACP agent for its sessions. `None`: it can't list them.
fn acp_list(program: &Path, args: &[String], env: &[(String, String)], agent: AgentKind) -> anyhow::Result<Option<Vec<PastChat>>> {
    let mut child = Child(
        Command::new(program)
            .args(args)
            .envs(env.iter().cloned())
            .current_dir(dirs::home_dir().unwrap_or_else(std::env::temp_dir))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Couldn't start {}: {e}", agent.display_name()))?,
    );
    let mut stdin = child.0.stdin.take().expect("stdin");
    let stdout = child.0.stdout.take().expect("stdout");
    let (tx, rx) = std::sync::mpsc::channel::<Value>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&line)
                && tx.send(v).is_err()
            {
                break;
            }
        }
    });
    let mut next = 0u64;
    let mut call = |method: &str, params: Value, wait: Duration| -> anyhow::Result<Value> {
        next += 1;
        let id = next;
        writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        stdin.flush()?;
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let v = rx.recv_timeout(left).map_err(|_| anyhow::anyhow!("{} didn't answer", agent.display_name()))?;
            if v["id"].as_u64() != Some(id) || v.get("method").is_some() {
                continue;
            }
            if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
                anyhow::bail!("{}", e["message"].as_str().unwrap_or("error"));
            }
            return Ok(v["result"].clone());
        }
    };
    let init = call(
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
            "clientInfo": {"name": "mothr", "title": "Mothr", "version": env!("CARGO_PKG_VERSION")},
        }),
        Duration::from_secs(60),
    )?;
    let caps = &init["agentCapabilities"];
    let listable = caps["sessionCapabilities"]["list"].is_object();
    // Without `session/load` a listed session couldn't be opened anyway.
    if !listable || !caps["loadSession"].as_bool().unwrap_or(false) {
        return Ok(None);
    }
    let mut out = vec![];
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let mut params = json!({});
        if let Some(c) = &cursor {
            params["cursor"] = json!(c);
        }
        let page = call("session/list", params, Duration::from_secs(30))?;
        for s in page["sessions"].as_array().into_iter().flatten() {
            let (Some(id), Some(cwd)) = (s["sessionId"].as_str(), s["cwd"].as_str()) else { continue };
            out.push(PastChat {
                agent,
                id: id.to_string(),
                cwd: PathBuf::from(cwd),
                title: clean_title(s["title"].as_str().unwrap_or("")),
                updated_at: s["updatedAt"].as_str().and_then(|t| t.parse::<DateTime<Utc>>().ok()),
                chat: None,
            });
        }
        cursor = page["nextCursor"].as_str().map(str::to_string);
        if cursor.is_none() || out.len() >= LIMIT {
            break;
        }
    }
    Ok(Some(out))
}

// ---------------------------------------------------------------------------------- Codex

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".codex"))
}

/// Codex rollouts, `sessions/YYYY/MM/DD/rollout-*.jsonl`.
pub fn codex_chats(limit: usize) -> Vec<PastChat> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<(PathBuf, SystemTime)>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && depth < 4 {
                walk(&p, depth + 1, out);
            } else if p.extension().is_some_and(|x| x == "jsonl")
                && let Ok(t) = e.metadata().and_then(|m| m.modified())
            {
                out.push((p, t));
            }
        }
    }
    let mut files = vec![];
    walk(&codex_home().join("sessions"), 0, &mut files);
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
    files.truncate(limit);
    files.iter().filter_map(|(p, _)| codex_chat(p)).collect()
}

fn codex_chat(path: &Path) -> Option<PastChat> {
    let (head, _) = head_tail(path, 64 * 1024)?;
    let mut lines = head.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok());
    let meta = lines.next()?;
    let payload = &meta["payload"];
    let id = payload["id"].as_str()?.to_string();
    let cwd = PathBuf::from(payload["cwd"].as_str()?);
    let prompt = lines.find_map(|v| {
        let p = &v["payload"];
        if v["type"] == "event_msg" && p["type"] == "user_message" {
            return p["message"].as_str().map(str::to_string).filter(|t| !t.trim().is_empty() && !t.starts_with('<'));
        }
        None
    });
    Some(PastChat { agent: AgentKind::Codex, id, cwd, title: clean_title(&prompt.unwrap_or_default()), updated_at: mtime(path), chat: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(v: Value) -> Value {
        v
    }

    #[test]
    fn claude_history_follows_the_latest_branch() {
        let lines = vec![
            line(json!({"type": "user", "uuid": "u1", "parentUuid": null, "timestamp": "2026-01-01T00:00:00Z", "message": {"content": "hello"}})),
            line(json!({"type": "assistant", "uuid": "a1", "parentUuid": "u1", "message": {"content": [{"type": "text", "text": "hi"}]}})),
            // An abandoned branch from an edit.
            line(json!({"type": "user", "uuid": "u2x", "parentUuid": "a1", "message": {"content": "old question"}})),
            line(json!({"type": "user", "uuid": "u2", "parentUuid": "a1", "message": {"content": [{"type": "text", "text": "[Mothr] note"}, {"type": "text", "text": "run ls"}]}})),
            line(json!({"type": "assistant", "uuid": "a2", "parentUuid": "u2", "message": {"content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "ls"}}]}})),
            line(json!({"type": "user", "uuid": "r1", "parentUuid": "a2", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": "a.txt"}]}})),
        ];
        let entries = claude_entries(&lines);
        let texts: Vec<String> = entries
            .iter()
            .map(|e| match e {
                Entry::User { text, prev_ref, .. } => format!("user:{text}:{}", prev_ref.clone().unwrap_or_default()),
                Entry::Assistant { text, .. } => format!("agent:{text}"),
                Entry::Tool(t) => format!("tool:{}:{:?}:{}", t.title, t.status, t.output.clone().unwrap_or_default()),
                _ => "other".into(),
            })
            .collect();
        assert_eq!(texts, vec!["user:hello:", "agent:hi", "user:run ls:a1", "tool:Run command:Completed:a.txt"]);
    }

    #[test]
    fn titles_are_one_clean_line() {
        assert_eq!(clean_title("  fix the build\nand more"), "fix the build");
        assert_eq!(clean_title("[Mothr] Skills: …"), "Untitled chat");
        assert_eq!(clean_title(""), "Untitled chat");
    }
}
