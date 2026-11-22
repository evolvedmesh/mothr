//! A tiny MCP server (streamable HTTP, JSON responses) that gives every ACP agent an interactive
//! multiple-choice question tool.
//!
//! Claude Code has `AskUserQuestion` built in, but ACP has no question primitive, and some agents
//! (Copilot) don't offer a question tool over ACP at all. Mothr hosts this server on loopback and
//! registers it with the agent in `session/new`; a `tools/call` becomes an [`Entry::Question`] card
//! and the call returns once the user answers.

use mothr_core::{Question, QuestionOption};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

pub(crate) const TOOL: &str = "ask_user_question";

/// One question round trip. `reply` gets one answer list per question, or `None` if cancelled.
/// One answer list per question; `None` if the question was dismissed.
pub(crate) type Answers = Option<Vec<Vec<String>>>;

pub(crate) struct AskRequest {
    pub questions: Vec<Question>,
    pub reply: oneshot::Sender<Answers>,
}

pub(crate) struct AskServer {
    /// The `mcpServers` entry to hand to the agent.
    pub config: Value,
    pub requests: mpsc::UnboundedReceiver<AskRequest>,
}

/// Start listening on a random loopback port. The URL carries a secret path so other local
/// processes can't pose as the agent.
/// `wait` is how long a tool call may stay open: MCP clients abandon calls after a fixed time
/// (Copilot ~3 min, others about a minute), so a slower answer goes back as a chat message.
pub(crate) async fn start(wait: Duration) -> Option<AskServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await.ok()?;
    let port = listener.local_addr().ok()?.port();
    let secret = uuid::Uuid::new_v4().simple().to_string();
    let (tx, requests) = mpsc::unbounded_channel();
    let path = format!("/{secret}");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { break };
            let (tx, path) = (tx.clone(), path.clone());
            tokio::spawn(async move {
                let _ = serve(stream, &path, tx, wait).await;
            });
        }
    });
    Some(AskServer {
        config: json!({"type": "http", "name": "mothr", "url": format!("http://127.0.0.1:{port}/{secret}"), "headers": []}),
        requests,
    })
}

async fn serve(stream: TcpStream, secret_path: &str, tx: mpsc::UnboundedSender<AskRequest>, wait: Duration) -> std::io::Result<()> {
    let mut rd = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if rd.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let mut parts = line.split_whitespace();
        let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
        let mut len = 0usize;
        let mut close = false;
        loop {
            let mut h = String::new();
            if rd.read_line(&mut h).await? == 0 {
                return Ok(());
            }
            let h = h.trim();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':')
                && k.eq_ignore_ascii_case("content-length")
            {
                len = v.trim().parse().unwrap_or(0);
            } else if let Some((k, v)) = h.split_once(':')
                && k.eq_ignore_ascii_case("connection")
            {
                close = v.trim().eq_ignore_ascii_case("close");
            }
        }
        let mut body = vec![0u8; len.min(1 << 20)];
        rd.read_exact(&mut body).await?;
        let out = if path != secret_path {
            http(404, "")
        } else if method != "POST" {
            http(405, "")
        } else {
            match serde_json::from_slice::<Value>(&body) {
                Ok(v) => match handle(&v, &tx, wait).await {
                    Some(r) => http(200, &r.to_string()),
                    None => http(202, ""),
                },
                Err(_) => http(400, ""),
            }
        };
        rd.get_mut().write_all(out.as_bytes()).await?;
        if close {
            return Ok(());
        }
    }
}

fn http(code: u16, body: &str) -> String {
    let reason = match code {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Method Not Allowed",
    };
    format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",
        body.len()
    )
}

/// Handle one JSON-RPC message. `None` means a notification (no reply body).
async fn handle(v: &Value, tx: &mpsc::UnboundedSender<AskRequest>, wait: Duration) -> Option<Value> {
    let id = v.get("id").filter(|i| !i.is_null())?.clone();
    let ok = |result: Value| Some(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    let err = |code: i64, msg: &str| Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}}));
    match v["method"].as_str().unwrap_or("") {
        "initialize" => ok(json!({
            "protocolVersion": v["params"]["protocolVersion"].as_str().unwrap_or("2025-03-26"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "mothr", "version": env!("CARGO_PKG_VERSION")},
        })),
        "ping" => ok(json!({})),
        "tools/list" => ok(json!({"tools": [tool_def()]})),
        "tools/call" => {
            if v["params"]["name"].as_str() != Some(TOOL) {
                return err(-32602, "unknown tool");
            }
            let questions = parse(&v["params"]["arguments"]);
            if questions.is_empty() {
                return ok(text("No questions were given. Pass at least one question with a few options.", true));
            }
            let (reply, rx) = oneshot::channel();
            if tx.send(AskRequest { questions: questions.clone(), reply }).is_err() {
                return ok(text("The chat is no longer open.", true));
            }
            match tokio::time::timeout(wait, rx).await {
                Ok(Ok(Some(answers))) => ok(text(&format_answers(&questions, &answers), false)),
                Ok(_) => ok(text("The user dismissed the question without answering.", true)),
                // Dropping `rx` tells the chat the call is gone; the answer will come as a message.
                Err(_) => ok(text(
                    "The user hasn't answered yet. The question stays open in their chat. Don't ask it again and don't guess: stop here and wait. Their answers will arrive as their next message.",
                    false,
                )),
            }
        }
        m if m.starts_with("notifications/") => None,
        _ => err(-32601, "method not found"),
    }
}

fn text(t: &str, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": t}], "isError": is_error})
}

pub(crate) fn format_answers(questions: &[Question], answers: &[Vec<String>]) -> String {
    if answers.iter().all(|a| a.is_empty()) {
        return "The user skipped the question without answering.".into();
    }
    let mut out = String::from("The user answered:\n");
    for (i, q) in questions.iter().enumerate() {
        let a = answers.get(i).filter(|a| !a.is_empty()).map(|a| a.join(", ")).unwrap_or_else(|| "(no answer)".into());
        out.push_str(&format!("- {}: {a}\n", q.question));
    }
    out
}

fn tool_def() -> Value {
    json!({
        "name": TOOL,
        "description": "Ask the user one or more interactive multiple-choice questions and wait for their answers. \
Use this whenever you need a decision, confirmation or preference from the user instead of writing the question in your reply. \
Each question has a short header, the question text and 2-4 options; the user can also type their own answer.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "description": "Questions to ask, shown one at a time.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "header": {"type": "string", "description": "Very short label (a few words)."},
                            "question": {"type": "string", "description": "The full question."},
                            "multiSelect": {"type": "boolean", "description": "Allow choosing several options."},
                            "options": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {"type": "string"},
                                        "description": {"type": "string"}
                                    },
                                    "required": ["label"]
                                }
                            }
                        },
                        "required": ["question", "options"]
                    }
                }
            },
            "required": ["questions"]
        }
    })
}

fn parse(args: &Value) -> Vec<Question> {
    crate::claude::parse_questions(args).into_iter().filter(|q| !q.question.trim().is_empty()).map(|mut q| {
        q.options.retain(|o: &QuestionOption| !o.label.trim().is_empty());
        q
    }).collect()
}

/// The answers as a chat message, for when the tool call has already given up waiting.
pub(crate) fn answers_message(questions: &[Question], answers: &[Vec<String>]) -> String {
    if answers.iter().all(|a| a.is_empty()) {
        return "I'm skipping those questions.".into();
    }
    let mut out = String::from("Here are my answers to your questions:\n");
    for (i, q) in questions.iter().enumerate() {
        let a = answers.get(i).filter(|a| !a.is_empty()).map(|a| a.join(", ")).unwrap_or_else(|| "(skipped)".into());
        out.push_str(&format!("- {}: {a}\n", q.question));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tool_call_round_trips_through_http() {
        let mut srv = start(Duration::from_secs(30)).await.unwrap();
        let url = srv.config["url"].as_str().unwrap().to_string();
        let (addr, path) = url.trim_start_matches("http://").split_once('/').map(|(a, p)| (a.to_string(), format!("/{p}"))).unwrap();
        let post = |path: String, body: Value| {
            let addr = addr.clone();
            async move {
                let mut s = TcpStream::connect(addr).await.unwrap();
                let b = body.to_string();
                s.write_all(format!("POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}", b.len()).as_bytes()).await.unwrap();
                let mut out = String::new();
                s.read_to_string(&mut out).await.unwrap();
                out
            }
        };
        assert!(post("/wrong".into(), json!({"jsonrpc":"2.0","id":1,"method":"ping"})).await.starts_with("HTTP/1.1 404"));
        let list = post(path.clone(), json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).await;
        assert!(list.contains(TOOL));
        let call = tokio::spawn(post(
            path,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":TOOL,"arguments":{"questions":[{"question":"Color?","options":[{"label":"Red"},{"label":"Blue"}]}]}}}),
        ));
        let req = srv.requests.recv().await.unwrap();
        assert_eq!(req.questions[0].options.len(), 2);
        req.reply.send(Some(vec![vec!["Blue".into()]])).unwrap();
        let resp = call.await.unwrap();
        assert!(resp.contains("Color?: Blue"), "{resp}");
    }
}
