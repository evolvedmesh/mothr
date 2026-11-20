//! Client-side state: the desktop hub's snapshot kept current by applying the same events
//! through the same reducer ([`SessionState::apply`]) the desktop and web client use, plus
//! flattened, UI-friendly views of it for the mobile app.

use crate::views::*;
use chrono::{DateTime, Local, Utc};
use mothr_core::{
    AgentInstall, AgentKind, Entry, HubEvent, HubSnapshot, ModelInfo, NoticeLevel, PermissionMode, PermissionOptionKind,
    PlanStatus, ProjectRef, ProviderUsage, SessionId, SessionKind, SessionState, SessionStatus, ToolKind,
    ToolStatus, UsageLevel,
};
use serde::Deserialize;
use std::collections::HashMap;

/// Messages the remote server pushes over the WebSocket.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    Snapshot {
        data: Box<HubSnapshot>,
        /// The desktop's name; older desktops don't send one.
        #[serde(default)]
        computer: Option<String>,
    },
    Event { data: Box<HubEvent> },
    Ack {
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
}

/// Longest tool output / diff shown on the phone; the desktop has the rest.
const MAX_OUTPUT: usize = 6_000;

#[derive(Debug, Default)]
pub struct Store {
    pub sessions: HashMap<SessionId, SessionState>,
    pub agents: Vec<AgentInstall>,
    pub usage: Vec<ProviderUsage>,
    pub pinned_projects: Vec<ProjectRef>,
    pub recent_projects: Vec<ProjectRef>,
    /// Models each agent offers, so a new chat can pick one before any session exists.
    pub agent_models: std::collections::BTreeMap<AgentKind, Vec<ModelInfo>>,
    pub toasts: Vec<Toast>,
    /// True once the first snapshot arrived.
    pub loaded: bool,
    /// The name the desktop goes by.
    pub computer: Option<String>,
}

impl Store {
    pub fn load(&mut self, snap: HubSnapshot) {
        self.sessions = snap.sessions.into_iter().map(|s| (s.meta.id, s)).collect();
        self.agents = snap.agents;
        self.usage = snap.usage;
        self.pinned_projects = snap.pinned_projects;
        self.recent_projects = snap.recent_projects;
        self.agent_models = snap.agent_models;
        self.loaded = true;
    }

    /// Apply one server message. Returns the session it touched, if any.
    pub fn apply_msg(&mut self, msg: ServerMsg) -> Option<SessionId> {
        match msg {
            ServerMsg::Snapshot { data, computer } => {
                self.load(*data);
                self.computer = computer;
                None
            }
            ServerMsg::Event { data } => self.apply_event(*data),
            ServerMsg::Ack { .. } => None,
        }
    }

    pub fn apply_event(&mut self, ev: HubEvent) -> Option<SessionId> {
        match ev {
            HubEvent::SessionCreated { state } => {
                let id = state.meta.id;
                self.sessions.insert(id, *state);
                Some(id)
            }
            HubEvent::Session { id, event } => {
                if let Some(s) = self.sessions.get_mut(&id) {
                    s.apply(&event);
                }
                Some(id)
            }
            HubEvent::SessionMeta { meta } => {
                let id = meta.id;
                match self.sessions.get_mut(&id) {
                    Some(s) => s.meta = meta,
                    None => {
                        self.sessions.insert(id, SessionState::new(meta));
                    }
                }
                Some(id)
            }
            HubEvent::SessionRemoved { id } => {
                self.sessions.remove(&id);
                Some(id)
            }
            HubEvent::Usage { usage } => {
                match self.usage.iter_mut().find(|u| u.provider == usage.provider) {
                    Some(u) => *u = usage,
                    None => self.usage.push(usage),
                }
                None
            }
            HubEvent::Agents { agents } => {
                self.agents = agents;
                None
            }
            HubEvent::Projects { pinned, recent } => {
                self.pinned_projects = pinned;
                self.recent_projects = recent;
                None
            }
            HubEvent::AgentModels { agent, models } => {
                self.agent_models.insert(agent, models);
                None
            }
            HubEvent::ProjectsIndexed { .. } => None,
            HubEvent::Toast { level, text } => {
                self.toasts.push(Toast { level: level_name(level).into(), text });
                if self.toasts.len() > 20 {
                    self.toasts.remove(0);
                }
                None
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Views

    fn summary(&self, s: &SessionState) -> ChatSummary {
        let m = &s.meta;
        let running = self
            .sessions
            .values()
            .filter(|c| c.meta.parent == Some(m.id) && matches!(c.meta.status, SessionStatus::Working | SessionStatus::NeedsInput | SessionStatus::Starting))
            .count();
        let total = self.sessions.values().filter(|c| c.meta.parent == Some(m.id)).count();
        ChatSummary {
            id: m.id.to_string(),
            title: m.title.clone(),
            agent: agent_ref(m.agent),
            project_name: Some(m.project_name()).filter(|n| !n.is_empty() && !m.general).unwrap_or_else(|| "General".into()),
            project_path: m.project.display().to_string(),
            general: m.general,
            design: m.kind == SessionKind::Design,
            status: chat_status(&m.status),
            status_label: m.status.label().into(),
            error: match &m.status {
                SessionStatus::Error(e) => Some(e.clone()),
                _ => None,
            },
            unread: m.unread,
            pinned: m.pinned,
            updated_ms: m.updated_at.timestamp_millis(),
            updated_label: relative_time(m.updated_at, Utc::now()),
            preview: preview(s),
            subagents_running: running as u32,
            subagents_total: total as u32,
            queued: m.queue.len() as u32,
            parent_id: m.parent.map(|p| p.to_string()),
        }
    }

    /// Top-level chats in smart groups: Pinned, Needs you, Working, then by recency.
    pub fn groups(&self, query: &str) -> Vec<ChatGroup> {
        let q = query.trim().to_lowercase();
        let mut list: Vec<&SessionState> = self
            .sessions
            .values()
            .filter(|s| !s.meta.archived && s.meta.parent.is_none())
            .filter(|s| {
                q.is_empty()
                    || s.meta.title.to_lowercase().contains(&q)
                    || s.meta.project_name().to_lowercase().contains(&q)
                    || s.meta.agent.display_name().to_lowercase().contains(&q)
            })
            .collect();
        list.sort_by_key(|s| std::cmp::Reverse(s.meta.updated_at));

        let now = Local::now();
        let order = ["Pinned", "Needs you", "Working", "Today", "Yesterday", "This week", "Older"];
        let mut buckets: Vec<Vec<ChatSummary>> = vec![Vec::new(); order.len()];
        for s in list {
            let m = &s.meta;
            let i = if m.pinned {
                0
            } else if m.status == SessionStatus::NeedsInput {
                1
            } else if matches!(m.status, SessionStatus::Working | SessionStatus::Starting) {
                2
            } else {
                let local = m.updated_at.with_timezone(&Local);
                let days = (now.date_naive() - local.date_naive()).num_days();
                match days {
                    ..=0 => 3,
                    1 => 4,
                    2..=6 => 5,
                    _ => 6,
                }
            };
            buckets[i].push(self.summary(s));
        }
        order
            .iter()
            .zip(buckets)
            .filter(|(_, c)| !c.is_empty())
            .map(|(t, chats)| ChatGroup { title: (*t).into(), chats })
            .collect()
    }

    pub fn needs_you_count(&self) -> u32 {
        self.sessions
            .values()
            .filter(|s| !s.meta.archived && s.meta.status == SessionStatus::NeedsInput)
            .count() as u32
    }

    /// Every unanswered permission request and question in chats that are waiting on the user,
    /// oldest first.
    pub fn attention(&self) -> Vec<Attention> {
        let mut out: Vec<Attention> = Vec::new();
        for s in self.sessions.values() {
            let m = &s.meta;
            if m.archived || m.status != SessionStatus::NeedsInput {
                continue;
            }
            let mut open_path = vec![m.id.to_string()];
            let mut parent = m.parent;
            while let Some(p) = parent.and_then(|p| self.sessions.get(&p)) {
                if open_path.len() > 8 {
                    break;
                }
                open_path.insert(0, p.meta.id.to_string());
                parent = p.meta.parent;
            }
            let agent = m.agent.display_name();
            for e in &s.entries {
                let (entry_id, kind, title, body, quick_replies) = match e {
                    Entry::Permission(p) if p.resolved.is_none() => {
                        let body = match p.detail.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                            Some(d) => format!("{}\n{}", p.title, clip(d, 300)),
                            None => p.title.clone(),
                        };
                        let pick = |kind: PermissionOptionKind, allow: bool| {
                            p.options.iter().find(|o| o.kind == kind).map(|o| QuickReply { id: o.id.clone(), label: o.label.clone(), allow })
                        };
                        let quick = [pick(PermissionOptionKind::AllowOnce, true), pick(PermissionOptionKind::RejectOnce, false)].into_iter().flatten().collect();
                        (p.id.clone(), AttentionKind::Permission, format!("{agent} needs permission"), body, quick)
                    }
                    Entry::Question(q) if q.answers.is_none() => {
                        let body = q
                            .questions
                            .iter()
                            .map(|x| x.question.trim().to_string())
                            .filter(|t| !t.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n");
                        let quick = match q.questions.as_slice() {
                            [only] if !only.multi_select && (1..=3).contains(&only.options.len()) => only
                                .options
                                .iter()
                                .map(|o| QuickReply { id: o.label.clone(), label: clip(&o.label, 24), allow: false })
                                .collect(),
                            _ => Vec::new(),
                        };
                        let title = if q.questions.len() > 1 { format!("{agent} has {} questions", q.questions.len()) } else { format!("{agent} has a question") };
                        (q.id.clone(), AttentionKind::Question, title, if body.is_empty() { "Question".into() } else { clip(&body, 400) }, quick)
                    }
                    _ => continue,
                };
                out.push(Attention {
                    key: format!("{}/{}", m.id, entry_id),
                    chat_id: m.id.to_string(),
                    entry_id: entry_id.to_string(),
                    open_path: open_path.clone(),
                    kind,
                    agent: agent_ref(m.agent),
                    chat_title: m.title.clone(),
                    project_name: if m.general { "General".into() } else { m.project_name() },
                    title,
                    body,
                    quick_replies,
                    updated_ms: m.updated_at.timestamp_millis(),
                });
            }
        }
        out.sort_by(|a, b| a.updated_ms.cmp(&b.updated_ms).then_with(|| a.key.cmp(&b.key)));
        out
    }

    pub fn detail(&self, id: SessionId) -> Option<ChatDetail> {
        let s = self.sessions.get(&id)?;
        let m = &s.meta;
        let ctx = m.context;
        let subagents = {
            let mut subs: Vec<&SessionState> = self.sessions.values().filter(|c| c.meta.parent == Some(id)).collect();
            subs.sort_by_key(|c| c.meta.created_at);
            subs.into_iter().map(|c| self.summary(c)).collect()
        };
        Some(ChatDetail {
            summary: self.summary(s),
            mode: mode_option(m.permission_mode),
            modes: m.agent.supported_modes().iter().map(|&p| mode_option(p)).collect(),
            model: m.model.clone(),
            models: model_options(if s.models.is_empty() { self.agent_models.get(&m.agent).map(Vec::as_slice).unwrap_or_default() } else { &s.models }),
            effort: m.effort.clone(),
            context_limit: m.context_limit.clone(),
            commands: s
                .commands
                .iter()
                .map(|c| CommandView { name: c.name.clone(), description: c.description.clone(), hint: c.hint.clone() })
                .collect(),
            queue: m.queue.iter().map(|q| QueuedView { id: q.id.clone(), text: q.text.clone() }).collect(),
            entries: s.entries.iter().map(entry_view).collect(),
            context_fraction: ctx.fraction(),
            context_label: if ctx.window_tokens > 0 {
                format!(
                    "{} / {}",
                    mothr_core::format_tokens(ctx.used_tokens.min(ctx.window_tokens)),
                    mothr_core::format_tokens(ctx.window_tokens)
                )
            } else {
                String::new()
            },
            cost_label: ctx.cost_usd.map(|c| format!("${c:.2}")),
            credits_label: ctx.credits.map(mothr_core::format_credits),
            busy: matches!(m.status, SessionStatus::Working | SessionStatus::NeedsInput | SessionStatus::Starting),
            live: m.status.is_live(),
            can_steer: m.agent.supports_steering(),
            subagents,
        })
    }

    pub fn agents(&self) -> Vec<AgentView> {
        let mut out: Vec<AgentView> = AgentKind::ALL
            .iter()
            .map(|&k| {
                let install = self.agents.iter().find(|a| a.kind == k);
                AgentView {
                    agent: agent_ref(k),
                    available: install.is_some_and(|a| a.available()),
                    version: install.and_then(|a| a.version.clone()),
                    note: install.and_then(|a| a.note.clone()),
                    modes: k.supported_modes().iter().map(|&p| mode_option(p)).collect(),
                    models: self.agent_models.get(&k).map(|m| model_options(m)).unwrap_or_default(),
                }
            })
            .collect();
        out.sort_by_key(|a| !a.available);
        out
    }

    pub fn projects(&self) -> Vec<ProjectView> {
        let mut seen = std::collections::HashSet::new();
        self.pinned_projects
            .iter()
            .chain(&self.recent_projects)
            .filter(|p| seen.insert(p.path.clone()))
            .map(project_view)
            .collect()
    }

    pub fn usage(&self) -> Vec<UsageView> {
        let mut out: Vec<UsageView> = self
            .usage
            .iter()
            .map(|u| {
                let windows: Vec<WindowView> = u
                    .windows
                    .iter()
                    .map(|w| WindowView {
                        label: w.label.clone(),
                        fraction: w.used_fraction,
                        level: level_id(w.level()).into(),
                        resets: w.resets_at.map(mothr_core::format_reset),
                        amount: match (w.used, w.limit) {
                            (Some(a), Some(b)) => Some(format!("{} / {}", trim_num(a), trim_num(b))),
                            _ => None,
                        },
                        detail: w.detail.clone(),
                    })
                    .collect();
                UsageView {
                    agent: agent_ref(u.provider.agent()),
                    provider_name: u.provider.display_name().into(),
                    plan: u.plan.clone(),
                    error: u.error.clone(),
                    worst_fraction: u.windows.iter().map(|w| w.used_fraction).fold(0.0, f32::max),
                    level: level_id(u.worst_level()).into(),
                    updated_label: relative_time(u.updated_at, Utc::now()),
                    windows,
                }
            })
            .collect();
        out.sort_by(|a, b| a.agent.id.cmp(&b.agent.id));
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Helpers

pub fn agent_ref(k: AgentKind) -> AgentRef {
    AgentRef {
        id: k.id().into(),
        name: k.display_name().into(),
        short_name: k.short_name().into(),
        color: k.accent(),
        monogram: k.monogram().into(),
    }
}

pub fn mode_option(p: PermissionMode) -> ModeOption {
    ModeOption { id: p.id().into(), label: p.label().into(), description: p.description().into(), dangerous: p.is_dangerous() }
}

fn project_view(p: &ProjectRef) -> ProjectView {
    ProjectView {
        path: p.path.display().to_string(),
        name: p.name.clone(),
        display_path: desktop_path(&p.path.display().to_string()),
        pinned: p.pinned,
        branch: p.git_branch.clone(),
        tags: p.tags.iter().filter(|t| *t != "git").cloned().collect(),
        name_indices: vec![],
    }
}

pub fn hit_view(h: &mothr_core::ProjectHit) -> ProjectView {
    let mut v = project_view(&h.project);
    if !h.display_path.is_empty() {
        v.display_path = h.display_path.clone();
    }
    v.name_indices = h.name_indices.clone();
    v
}

pub fn dir_view(l: mothr_core::DirListing) -> DirView {
    DirView {
        path: l.path,
        entries: l
            .entries
            .into_iter()
            .map(|e| FileEntryView {
                size_label: if e.dir { String::new() } else { mothr_core::format_size(e.size) },
                markdown: !e.dir && mothr_core::is_markdown_name(&e.name),
                name: e.name,
                path: e.path,
                dir: e.dir,
            })
            .collect(),
        truncated: l.truncated,
        readme: l.readme,
    }
}

pub fn file_view(f: mothr_core::FileContent) -> FileView {
    FileView {
        name: f.name().to_string(),
        size_label: mothr_core::format_size(f.size),
        markdown: f.is_markdown(),
        line_count: f.text.as_deref().map_or(0, |t| t.lines().count() as u32),
        path: f.path,
        language: f.language,
        text: f.text,
        truncated: f.truncated,
    }
}

/// A finder hit, with the match indices moved from the whole path onto the file name.
pub fn file_hit_view(h: mothr_core::FileHit) -> FileHitView {
    let folder = mothr_core::parent_path(&h.path).to_string();
    let name = mothr_core::file_name(&h.path).to_string();
    let offset = if folder.is_empty() { 0 } else { folder.chars().count() as u32 + 1 };
    FileHitView {
        name_indices: h.indices.iter().filter(|&&i| i >= offset).map(|i| i - offset).collect(),
        path: h.path,
        name,
        folder,
    }
}

fn chat_status(s: &SessionStatus) -> ChatStatus {
    match s {
        SessionStatus::Starting => ChatStatus::Starting,
        SessionStatus::Idle => ChatStatus::Ready,
        SessionStatus::Working => ChatStatus::Working,
        SessionStatus::NeedsInput => ChatStatus::NeedsYou,
        SessionStatus::Error(_) => ChatStatus::Error,
        SessionStatus::Ended => ChatStatus::Ended,
    }
}

fn level_name(l: NoticeLevel) -> &'static str {
    match l {
        NoticeLevel::Info => "info",
        NoticeLevel::Warning => "warning",
        NoticeLevel::Error => "error",
    }
}

fn level_id(l: UsageLevel) -> &'static str {
    match l {
        UsageLevel::Ok => "ok",
        UsageLevel::Warn => "warn",
        UsageLevel::Critical => "critical",
        UsageLevel::Exhausted => "exhausted",
    }
}

fn trim_num(n: f64) -> String {
    if n.fract() == 0.0 { format!("{n:.0}") } else { format!("{n:.1}") }
}

/// The latest line worth showing under a chat's title.
fn preview(s: &SessionState) -> String {
    for e in s.entries.iter().rev() {
        let text = match e {
            Entry::Permission(p) if p.resolved.is_none() => return format!("Waiting: {}", p.title),
            Entry::Question(q) if q.answers.is_none() => {
                return q.questions.first().map(|x| x.question.clone()).unwrap_or_else(|| "Question".into());
            }
            Entry::Assistant { text, .. } | Entry::User { text, .. } => text,
            Entry::Tool(t) => &t.title,
            _ => continue,
        };
        let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
        if !line.is_empty() {
            return clip(&line.replace(['#', '*', '`'], ""), 120);
        }
    }
    String::new()
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// Shorten a path on the *desktop* for display: `/home/me/x` → `~/x`. (The phone's own home
/// folder says nothing about the desktop's, so this goes by the usual home layouts.)
pub fn desktop_path(path: &str) -> String {
    for root in ["/home/", "/Users/"] {
        if let Some(rest) = path.strip_prefix(root) {
            return match rest.split_once('/') {
                Some((_, tail)) if !tail.is_empty() => format!("~/{tail}"),
                _ => "~".into(),
            };
        }
    }
    if let Some(i) = path.find(":\\Users\\") {
        let rest = &path[i + ":\\Users\\".len()..];
        return match rest.split_once('\\') {
            Some((_, tail)) if !tail.is_empty() => format!("~\\{tail}"),
            _ => "~".into(),
        };
    }
    path.to_string()
}

/// "now", "5m", "3h", "2d", or a date.
pub fn relative_time(t: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - t).num_seconds().max(0);
    match secs {
        0..60 => "now".into(),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => format!("{}h", secs / 3600),
        86_400..604_800 => format!("{}d", secs / 86_400),
        _ => t.with_timezone(&Local).format("%b %-d").to_string(),
    }
}

fn tool_kind(k: ToolKind) -> &'static str {
    match k {
        ToolKind::Read => "read",
        ToolKind::Edit => "edit",
        ToolKind::Delete => "delete",
        ToolKind::Move => "move",
        ToolKind::Search => "search",
        ToolKind::Execute => "execute",
        ToolKind::Think => "think",
        ToolKind::Fetch => "fetch",
        ToolKind::Task => "task",
        ToolKind::Other => "other",
    }
}

pub fn entry_view(e: &Entry) -> EntryView {
    match e {
        Entry::User { id, text, at, .. } => EntryView::User { id: id.clone(), text: text.clone(), at_ms: at.timestamp_millis() },
        Entry::Assistant { id, text, streaming } => EntryView::Assistant { id: id.clone(), text: text.clone(), streaming: *streaming },
        Entry::Thinking { id, text, streaming } => EntryView::Thinking { id: id.clone(), text: text.clone(), streaming: *streaming },
        Entry::Tool(t) => {
            let (mut added, mut removed) = (0u32, 0u32);
            let mut diff = String::new();
            for d in &t.diffs {
                let old = d.old_text.as_deref().unwrap_or("");
                let td = similar::TextDiff::from_lines(old, &d.new_text);
                if t.diffs.len() > 1 {
                    diff.push_str(&format!("── {}\n", d.path.display()));
                }
                for change in td.iter_all_changes() {
                    let sign = match change.tag() {
                        similar::ChangeTag::Insert => {
                            added += 1;
                            '+'
                        }
                        similar::ChangeTag::Delete => {
                            removed += 1;
                            '-'
                        }
                        similar::ChangeTag::Equal => continue,
                    };
                    if diff.len() < MAX_OUTPUT {
                        diff.push(sign);
                        diff.push_str(change.value().trim_end_matches('\n'));
                        diff.push('\n');
                    }
                }
            }
            EntryView::Tool {
                id: t.id.clone(),
                title: t.title.clone(),
                kind: tool_kind(t.kind).into(),
                status: match t.status {
                    ToolStatus::Pending => ToolState::Pending,
                    ToolStatus::Running => ToolState::Running,
                    ToolStatus::Completed => ToolState::Done,
                    ToolStatus::Failed => ToolState::Failed,
                },
                input: t.input.clone(),
                output: t.output.as_deref().filter(|o| !o.trim().is_empty()).map(|o| clip(o, MAX_OUTPUT)),
                added,
                removed,
                diff: (!diff.is_empty()).then_some(diff),
            }
        }
        Entry::Plan { id, items } => EntryView::Plan {
            id: id.clone(),
            items: items
                .iter()
                .map(|i| PlanItemView {
                    content: i.content.clone(),
                    status: match i.status {
                        PlanStatus::Pending => "pending",
                        PlanStatus::InProgress => "in_progress",
                        PlanStatus::Completed => "completed",
                    }
                    .into(),
                })
                .collect(),
        },
        Entry::Permission(p) => EntryView::Permission {
            id: p.id.clone(),
            title: p.title.clone(),
            detail: p.detail.clone(),
            options: p
                .options
                .iter()
                .map(|o| PermissionChoice {
                    id: o.id.clone(),
                    label: o.label.clone(),
                    allow: matches!(o.kind, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways),
                    always: matches!(o.kind, PermissionOptionKind::AllowAlways | PermissionOptionKind::RejectAlways),
                })
                .collect(),
            resolved: p.resolved.as_ref().map(|r| {
                p.options.iter().find(|o| &o.id == r).map(|o| o.label.clone()).unwrap_or_else(|| r.clone())
            }),
        },
        Entry::Question(q) => EntryView::Question {
            id: q.id.clone(),
            questions: q
                .questions
                .iter()
                .map(|x| QuestionView {
                    header: x.header.clone(),
                    question: x.question.clone(),
                    options: x
                        .options
                        .iter()
                        .map(|o| QuestionChoice { label: o.label.clone(), description: o.description.clone() })
                        .collect(),
                    multi_select: x.multi_select,
                })
                .collect(),
            answers: q.answers.clone(),
        },
        Entry::Notice { id, level, text } => EntryView::Notice { id: id.clone(), level: level_name(*level).into(), text: text.clone() },
        // The mobile app doesn't render pictures yet; say one was produced.
        Entry::Image { id, path, .. } => EntryView::Notice {
            id: id.clone(),
            level: level_name(NoticeLevel::Info).into(),
            text: format!("Image: {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        },
        Entry::TurnEnd { id, stop_reason, duration_ms, cost_usd } => {
            let mut parts = Vec::new();
            if let Some(ms) = duration_ms {
                let s = *ms as f64 / 1000.0;
                parts.push(if s >= 60.0 { format!("{}m {}s", (s / 60.0) as u64, (s % 60.0) as u64) } else { format!("{s:.1}s") });
            }
            if let Some(c) = cost_usd {
                parts.push(format!("${c:.3}"));
            }
            if stop_reason != "end_turn" && !stop_reason.is_empty() {
                parts.push(stop_reason.replace('_', " "));
            }
            EntryView::TurnEnd { id: id.clone(), summary: parts.join(" · ") }
        }
    }
}

fn model_options(models: &[ModelInfo]) -> Vec<ModelOption> {
    models
        .iter()
        .map(|m| ModelOption {
            id: m.id.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            efforts: m.efforts.iter().map(|e| ChoiceOption { id: e.clone(), label: mothr_core::effort_label(e) }).collect(),
            contexts: m.contexts.iter().map(|c| ChoiceOption { id: c.id.clone(), label: c.label.clone() }).collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mothr_core::{SessionEvent, SessionMeta};

    fn session(title: &str, status: SessionStatus) -> SessionState {
        let mut m = SessionMeta::new(AgentKind::ClaudeCode, SessionKind::Chat, "/home/u/proj".into(), PermissionMode::Ask);
        m.title = title.into();
        m.status = status;
        SessionState::new(m)
    }

    fn store(sessions: Vec<SessionState>) -> Store {
        let mut st = Store::default();
        st.load(HubSnapshot { sessions, ..Default::default() });
        st
    }

    #[test]
    fn server_messages_parse_and_stream_through_the_shared_reducer() {
        let s = session("Fix bug", SessionStatus::Working);
        let id = s.meta.id;
        let mut st = Store::default();
        let snap = serde_json::json!({ "type": "snapshot", "data": HubSnapshot { sessions: vec![s], ..Default::default() } });
        st.apply_msg(serde_json::from_value(snap).unwrap());
        for d in ["Hel", "lo **there**"] {
            let ev = HubEvent::Session { id, event: SessionEvent::AppendText { entry_id: "a1".into(), thinking: false, delta: d.into() } };
            let msg = serde_json::json!({ "type": "event", "data": ev });
            assert_eq!(st.apply_msg(serde_json::from_value(msg).unwrap()), Some(id));
        }
        let d = st.detail(id).unwrap();
        assert!(matches!(&d.entries[0], EntryView::Assistant { text, streaming: true, .. } if text == "Hello **there**"));
        assert_eq!(d.summary.preview, "Hello there");
        let ack: ServerMsg = serde_json::from_str(r#"{"type":"ack","id":3,"ok":true}"#).unwrap();
        assert_eq!(st.apply_msg(ack), None);
    }

    #[test]
    fn smart_groups_order_and_hide_subagents() {
        let pinned = {
            let mut s = session("Pinned one", SessionStatus::Idle);
            s.meta.pinned = true;
            s
        };
        let waiting = session("Needs approval", SessionStatus::NeedsInput);
        let working = session("Busy", SessionStatus::Working);
        let idle = session("Done", SessionStatus::Idle);
        let mut child = session("Sub", SessionStatus::Working);
        child.meta.parent = Some(working.meta.id);
        let st = store(vec![idle, child, working, waiting, pinned]);
        let groups = st.groups("");
        let titles: Vec<_> = groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["Pinned", "Needs you", "Working", "Today"]);
        let busy = &groups[2].chats[0];
        assert_eq!((busy.subagents_running, busy.subagents_total), (1, 1));
        assert_eq!(st.groups("approv").len(), 1);
        assert_eq!(st.needs_you_count(), 1);
    }

    #[test]
    fn attention_lists_open_requests_with_quick_replies() {
        use mothr_core::{PermissionOption, PermissionRequest, Question, QuestionOption, QuestionRequest};
        let mut parent = session("Orchestrator", SessionStatus::Working);
        parent.meta.agent = AgentKind::Codex;
        let mut child = session("Sub", SessionStatus::NeedsInput);
        child.meta.parent = Some(parent.meta.id);
        child.entries.push(Entry::Permission(PermissionRequest {
            id: "p1".into(),
            tool_call_id: None,
            title: "Run cargo test".into(),
            detail: Some("cargo test --workspace".into()),
            kind: ToolKind::Execute,
            options: vec![
                PermissionOption { id: "always".into(), label: "Always allow".into(), kind: PermissionOptionKind::AllowAlways },
                PermissionOption { id: "y".into(), label: "Allow".into(), kind: PermissionOptionKind::AllowOnce },
                PermissionOption { id: "n".into(), label: "Deny".into(), kind: PermissionOptionKind::RejectOnce },
            ],
            resolved: None,
            diffs: vec![],
        }));
        let mut asking = session("Pick a db", SessionStatus::NeedsInput);
        let option = |l: &str| QuestionOption { label: l.into(), description: None };
        asking.entries.push(Entry::Question(QuestionRequest {
            id: "q1".into(),
            questions: vec![Question { header: None, question: "Which database?".into(), options: vec![option("Postgres"), option("SQLite")], multi_select: false }],
            answers: None,
        }));
        let mut answered = session("Answered", SessionStatus::NeedsInput);
        answered.entries.push(Entry::Question(QuestionRequest {
            id: "q2".into(),
            questions: vec![Question { header: None, question: "Done?".into(), options: vec![option("Yes")], multi_select: false }],
            answers: Some(vec![vec!["Yes".into()]]),
        }));
        let (pid, cid, aid) = (parent.meta.id, child.meta.id, asking.meta.id);
        let st = store(vec![parent, child, asking, answered]);
        let mut items = st.attention();
        items.sort_by(|a, b| a.entry_id.cmp(&b.entry_id));
        assert_eq!(items.len(), 2);
        let perm = &items[0];
        assert_eq!(perm.kind, AttentionKind::Permission);
        assert_eq!(perm.key, format!("{cid}/p1"));
        assert_eq!(perm.open_path, vec![pid.to_string(), cid.to_string()]);
        assert_eq!(perm.title, "Claude Code needs permission");
        assert_eq!(perm.body, "Run cargo test\ncargo test --workspace");
        let ids: Vec<_> = perm.quick_replies.iter().map(|r| (r.id.as_str(), r.allow)).collect();
        assert_eq!(ids, [("y", true), ("n", false)]);
        let q = &items[1];
        assert_eq!((q.kind, q.chat_id.clone()), (AttentionKind::Question, aid.to_string()));
        assert_eq!(q.body, "Which database?");
        assert_eq!(q.quick_replies.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["Postgres", "SQLite"]);
    }

    #[test]
    fn tool_diffs_count_lines_and_permissions_expose_choices() {
        use mothr_core::{FileDiff, PermissionOption, PermissionRequest, ToolCall};
        let tool = Entry::Tool(ToolCall {
            id: "t".into(),
            name: "Edit".into(),
            title: "Edit lib.rs".into(),
            kind: ToolKind::Edit,
            status: ToolStatus::Completed,
            input: None,
            output: None,
            diffs: vec![FileDiff { path: "lib.rs".into(), old_text: Some("a\nb\n".into()), new_text: "a\nc\nd\n".into(), start_line: None }],
            locations: vec![],
        });
        match entry_view(&tool) {
            EntryView::Tool { added, removed, diff, .. } => {
                assert_eq!((added, removed), (2, 1));
                assert_eq!(diff.unwrap(), "-b\n+c\n+d\n");
            }
            other => panic!("{other:?}"),
        }
        let perm = Entry::Permission(PermissionRequest {
            id: "p".into(),
            tool_call_id: None,
            title: "Run ls".into(),
            detail: Some("ls -la".into()),
            kind: ToolKind::Execute,
            options: vec![
                PermissionOption { id: "y".into(), label: "Allow".into(), kind: PermissionOptionKind::AllowOnce },
                PermissionOption { id: "n".into(), label: "Deny".into(), kind: PermissionOptionKind::RejectOnce },
            ],
            resolved: Some("y".into()),
            diffs: vec![],
        });
        match entry_view(&perm) {
            EntryView::Permission { options, resolved, .. } => {
                assert!(options[0].allow && !options[1].allow);
                assert_eq!(resolved.as_deref(), Some("Allow"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn desktop_paths_shorten_by_home_layout() {
        assert_eq!(desktop_path("/home/me/projects/mothr"), "~/projects/mothr");
        assert_eq!(desktop_path("/Users/ann"), "~");
        assert_eq!(desktop_path("C:\\Users\\ann\\code\\x"), "~\\code\\x");
        assert_eq!(desktop_path("/srv/app"), "/srv/app");
    }

    #[test]
    fn relative_times() {
        let now = Utc::now();
        assert_eq!(relative_time(now, now), "now");
        assert_eq!(relative_time(now - chrono::Duration::minutes(5), now), "5m");
        assert_eq!(relative_time(now - chrono::Duration::hours(3), now), "3h");
        assert_eq!(relative_time(now - chrono::Duration::days(2), now), "2d");
    }
}

