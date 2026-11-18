//! Session navigator with smart grouping, search, and pinned projects.

use super::kit::*;
use super::tokens;
use crate::store::Store;
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::*;
use super::tokens::sz;
use gpui_kit::prelude::FluentBuilder as _;
use mothr_core::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GroupMode {
    Smart,
    Project,
    Agent,
    Date,
}

impl GroupMode {
    fn label(self) -> &'static str {
        match self {
            GroupMode::Smart => "Smart",
            GroupMode::Project => "Project",
            GroupMode::Agent => "Agent",
            GroupMode::Date => "Date",
        }
    }
}

pub enum SidebarEvent {
    Open(SessionId),
    NewChat,
    NewDesign,
    NewGeneral,
    NewChatIn(ProjectRef),
}

impl EventEmitter<SidebarEvent> for Sidebar {}

pub struct Sidebar {
    store: Entity<Store>,
    search: Entity<InputState>,
    query: String,
    mode: GroupMode,
    active: Option<SessionId>,
    collapsed: std::collections::HashSet<String>,
    /// Orchestrator chats whose subagent list is folded.
    folded: std::collections::HashSet<SessionId>,
    _subs: Vec<Subscription>,
}

impl Sidebar {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats"));
        let subs = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe(&search, |this, s, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.query = s.read(cx).value().to_string();
                    cx.notify();
                }
            }),
        ];
        Self { store, search, query: String::new(), mode: GroupMode::Smart, active: None, collapsed: Default::default(), folded: Default::default(), _subs: subs }
    }

    pub fn set_active(&mut self, id: Option<SessionId>, cx: &mut Context<Self>) {
        self.active = id;
        cx.notify();
    }

    fn groups(&self, sessions: &[SessionState]) -> Vec<(String, Vec<SessionMeta>)> {
        let q = self.query.to_lowercase();
        let mut metas: Vec<SessionMeta> = sessions
            .iter()
            .map(|s| s.meta.clone())
            .filter(|m| m.parent.is_none())
            .filter(|m| {
                q.is_empty()
                    || m.title.to_lowercase().contains(&q)
                    || m.project_name().to_lowercase().contains(&q)
                    || m.agent.display_name().to_lowercase().contains(&q)
            })
            .collect();
        metas.sort_by_key(|m| std::cmp::Reverse(m.updated_at));

        let mut out: Vec<(String, Vec<SessionMeta>)> = vec![];
        let push = |name: String, m: SessionMeta, out: &mut Vec<(String, Vec<SessionMeta>)>| {
            if let Some(g) = out.iter_mut().find(|(n, _)| *n == name) {
                g.1.push(m);
            } else {
                out.push((name, vec![m]));
            }
        };
        let date_bucket = |m: &SessionMeta| -> String {
            let days = (chrono::Local::now().date_naive() - m.updated_at.with_timezone(&chrono::Local).date_naive()).num_days();
            match days {
                0 => "Today".into(),
                1 => "Yesterday".into(),
                2..=6 => "This week".into(),
                7..=30 => "This month".into(),
                _ => "Older".into(),
            }
        };
        match self.mode {
            GroupMode::Smart => {
                // Pinned → Needs you → Working → custom groups → by recency.
                for m in metas {
                    let g = if m.pinned {
                        "Pinned".to_string()
                    } else if m.status == SessionStatus::NeedsInput {
                        "Needs you".to_string()
                    } else if matches!(m.status, SessionStatus::Working | SessionStatus::Starting) {
                        "Working".to_string()
                    } else if let Some(g) = &m.group {
                        g.clone()
                    } else {
                        date_bucket(&m)
                    };
                    push(g, m, &mut out);
                }
                let rank = |n: &str| match n {
                    "Pinned" => 0,
                    "Needs you" => 1,
                    "Working" => 2,
                    "Today" => 4,
                    "Yesterday" => 5,
                    "This week" => 6,
                    "This month" => 7,
                    "Older" => 8,
                    _ => 3,
                };
                out.sort_by_key(|(n, _)| rank(n));
            }
            GroupMode::Project => {
                let mut map: BTreeMap<String, Vec<SessionMeta>> = BTreeMap::new();
                for m in metas {
                    map.entry(m.project_name()).or_default().push(m);
                }
                let mut v: Vec<_> = map.into_iter().collect();
                v.sort_by(|a, b| b.1[0].updated_at.cmp(&a.1[0].updated_at));
                out = v;
            }
            GroupMode::Agent => {
                for m in metas {
                    push(m.agent.display_name().to_string(), m, &mut out);
                }
            }
            GroupMode::Date => {
                for m in metas {
                    push(date_bucket(&m), m, &mut out);
                }
            }
        }
        out
    }

    fn render_row(&self, m: &SessionMeta, counts: (usize, usize, usize), cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let (sub_running, sub_total, sub_needs) = counts;
        let orchestrator = sub_total > 0;
        let folded = self.folded.contains(&m.id);
        let id = m.id;
        let active = self.active == Some(id);
        let store = self.store.clone();
        let pinned = m.pinned;
        let title: SharedString = m.title.clone().into();
        let design = m.kind == SessionKind::Design;
        let needs = m.status == SessionStatus::NeedsInput || sub_needs > 0;
        let working = matches!(m.status, SessionStatus::Working | SessionStatus::Starting) || sub_running > 0;
        let sub: SharedString = if sub_needs > 0 {
            format!("{sub_needs} subagent{} waiting for you", if sub_needs == 1 { "" } else { "s" }).into()
        } else if needs {
            "Waiting for you".into()
        } else if sub_running > 0 {
            format!("Orchestrating {sub_running} subagent{}", if sub_running == 1 { "" } else { "s" }).into()
        } else if working {
            "Working…".into()
        } else {
            format!("{} · {}", m.project_name(), relative_time(m.updated_at)).into()
        };
        h_flex()
            .id(ElementId::Name(format!("row-{id}").into()))
            .relative()
            .mx_2()
            .pl(sz(10.))
            .pr_2()
            .py(sz(7.))
            .gap(sz(10.))
            .rounded(sz(tokens::radius::MD))
            .cursor_pointer()
            .items_center()
            .when(active, |d| d.bg(pal.surface_hover))
            .when(!active, |d| d.hover(|d| d.bg(pal.surface_hover.opacity(0.6))))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(SidebarEvent::Open(id))))
            .when(active, |d| {
                d.child(div().absolute().left(sz(0.)).top(sz(9.)).bottom(sz(9.)).w(sz(2.5)).rounded_full().bg(pal.brand))
            })
            .child(
                div()
                    .relative()
                    .child(agent_avatar(m.agent, 26.))
                    .when(design, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(sz(-4.))
                                .left(sz(-4.))
                                .size(sz(14.))
                                .rounded_full()
                                .bg(pal.chrome)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon("palette").size(sz(9.)).text_color(pal.brand)),
                        )
                    })
                    .child(
                        div()
                            .absolute()
                            .bottom(sz(-2.))
                            .right(sz(-2.))
                            .p(sz(2.))
                            .rounded_full()
                            .bg(pal.chrome)
                            .child(status_dot(ElementId::Name(format!("dot-{id}").into()), &m.status, cx)),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(sz(1.))
                    .child(
                        div()
                            .text_size(sz(tokens::text::BASE))
                            .truncate()
                            .text_color(if active || m.unread { pal.text } else { pal.text.opacity(0.88) })
                            .when(m.unread || active, |d| d.font_weight(FontWeight::MEDIUM))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(sz(tokens::text::XS))
                            .truncate()
                            .text_color(if needs { pal.warning } else if working { pal.brand } else { pal.text_faint })
                            .child(sub),
                    ),
            )
            .when(orchestrator, |d| {
                let id2 = id;
                d.child(
                    h_flex()
                        .id(ElementId::Name(format!("orch-{id}").into()))
                        .gap(sz(4.))
                        .px(sz(6.))
                        .h(sz(20.))
                        .items_center()
                        .rounded_full()
                        .bg(if sub_running > 0 { pal.brand_soft } else { pal.surface_hover })
                        .text_size(sz(tokens::text::XS))
                        .text_color(if sub_running > 0 { pal.brand } else { pal.text_faint })
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(format!("Orchestrator · {sub_running} of {sub_total} subagents running")).build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if !this.folded.remove(&id2) {
                                this.folded.insert(id2);
                            }
                            cx.notify();
                        }))
                        .child(icon("network").size(sz(11.)))
                        .child(if sub_running > 0 { format!("{sub_running}/{sub_total}") } else { sub_total.to_string() })
                        .child(icon(if folded { "chevron-right" } else { "chevron-down" }).size(sz(10.))),
                )
            })
            .when(m.unread && !active, |d| d.child(div().size(sz(6.)).rounded_full().bg(pal.brand)))
            .when(pinned, |d| d.child(icon("pin").size(sz(11.)).text_color(pal.text_faint)))
            .context_menu(move |menu, _, _| {
                let s1 = store.clone();
                let s2 = store.clone();
                let s3 = store.clone();
                menu.item(
                    PopupMenuItem::new(if pinned { "Unpin" } else { "Pin" })
                        .icon(icon(if pinned { "pin-off" } else { "pin" }))
                        .on_click(move |_, window, cx| {
                            s1.update(cx, |s, cx| {
                                s.dispatch(AppCommand::PinSession { id, pinned: !pinned }, window, cx);
                            });
                        }),
                )
                .item(PopupMenuItem::new("Archive").icon(icon("archive")).on_click(move |_, window, cx| {
                    s2.update(cx, |s, cx| {
                        s.dispatch(AppCommand::Archive { id, archived: true }, window, cx);
                    });
                }))
                .separator()
                .item(PopupMenuItem::new("Delete").icon(icon("delete")).on_click(move |_, window, cx| {
                    s3.update(cx, |s, cx| {
                        s.dispatch(AppCommand::Delete { id }, window, cx);
                    });
                }))
            })
    }
}

impl Sidebar {
    fn render_child(&self, m: &SessionMeta, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let id = m.id;
        let active = self.active == Some(id);
        let needs = m.status == SessionStatus::NeedsInput;
        let info = m.subagent.clone().unwrap_or_default();
        let detail = if needs {
            "Needs you".to_string()
        } else if m.status.is_live() {
            match &info.last_tool {
                Some(t) => format!("{} · {} tools · {}", t, info.tool_uses, format_tokens(info.tokens)),
                None => "Starting…".to_string(),
            }
        } else {
            format!("{} · {} tools", m.status.label(), info.tool_uses)
        };
        h_flex()
            .id(ElementId::Name(format!("child-{id}").into()))
            .relative()
            .ml(sz(30.))
            .mr_2()
            .pl(sz(12.))
            .pr_2()
            .py(sz(5.))
            .gap(sz(8.))
            .rounded(sz(tokens::radius::SM))
            .items_center()
            .cursor_pointer()
            .when(active, |d| d.bg(pal.surface_hover))
            .when(!active, |d| d.hover(|d| d.bg(pal.surface_hover.opacity(0.6))))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(SidebarEvent::Open(id))))
            // Tree guide.
            .child(div().absolute().left(sz(0.)).top(sz(0.)).bottom(sz(0.)).w(sz(1.)).bg(pal.border_strong))
            .child(div().absolute().left(sz(0.)).top(sz(14.)).w(sz(8.)).h(sz(1.)).bg(pal.border_strong))
            .child(status_dot(ElementId::Name(format!("cdot-{id}").into()), &m.status, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_size(sz(tokens::text::SM)).truncate().text_color(if active { pal.text } else { pal.text_muted }).child(m.title.clone()))
                    .child(
                        div()
                            .text_size(sz(11.))
                            .truncate()
                            .text_color(if needs { pal.warning } else { pal.text_faint })
                            .child(detail),
                    ),
            )
            .when_some(info.agent_type.clone(), |d, t| {
                d.child(div().text_size(sz(10.5)).px(sz(5.)).rounded(sz(4.)).bg(pal.surface_hover).text_color(pal.text_faint).child(t))
            })
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = tokens::p(cx);
        let store = self.store.read(cx);
        let groups = self.groups(&store.sessions);
        let pinned_projects = store.pinned.clone();
        let total = store.sessions.len();
        let mode = self.mode;
        let weak = cx.entity().downgrade();

        let mut list = v_flex().gap(sz(1.)).pb_3();
        if groups.is_empty() {
            list = list.child(empty_state(
                "message-square-plus",
                if total == 0 { "No chats yet" } else { "No matches" },
                if total == 0 { "Start a chat with any installed agent." } else { "Try a different search." },
                cx,
            ));
        }
        for (name, metas) in groups {
            let collapsed = self.collapsed.contains(&name);
            let key = name.clone();
            let accent = name == "Needs you";
            list = list.child(
                h_flex()
                    .id(ElementId::Name(format!("group-{name}").into()))
                    .group("grp")
                    .px(sz(18.))
                    .pt(sz(14.))
                    .pb(sz(5.))
                    .gap(sz(6.))
                    .items_center()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.collapsed.remove(&key) {
                            this.collapsed.insert(key.clone());
                        }
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(sz(tokens::text::XS))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if accent { pal.warning } else { pal.text_faint })
                            .child(name.clone()),
                    )
                    .child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint.opacity(0.7)).child(metas.len().to_string()))
                    .child(div().flex_1())
                    .child(
                        div()
                            .invisible()
                            .group_hover("grp", |d| d.visible())
                            .child(icon(if collapsed { "chevron-right" } else { "chevron-down" }).size(sz(12.)).text_color(pal.text_faint)),
                    ),
            );
            if !collapsed {
                for m in &metas {
                    let (counts, kids) = {
                        let st = self.store.read(cx);
                        let kids: Vec<SessionMeta> = st.children(m.id).into_iter().map(|s| s.meta.clone()).collect();
                        (st.subagent_counts(m.id), kids)
                    };
                    list = list.child(self.render_row(m, counts, cx));
                    if !kids.is_empty() && !self.folded.contains(&m.id) {
                        for k in &kids {
                            list = list.child(self.render_child(k, cx));
                        }
                    }
                }
            }
        }

        v_flex()
            .size_full()
            .child(
                v_flex()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .gap(sz(10.))
                    .child(
                        h_flex()
                            .gap(sz(6.))
                            .child(
                                h_flex()
                                    .id("sb-new-chat")
                                    .flex_1()
                                    .h(sz(34.))
                                    .px_3()
                                    .gap_2()
                                    .items_center()
                                    .rounded(sz(tokens::radius::MD))
                                    .bg(pal.surface)
                                    .border_1()
                                    .border_color(pal.border_strong)
                                    .text_color(pal.text)
                                    .shadow(tokens::shadow_sm(&pal))
                                    .cursor_pointer()
                                    .hover(|d| d.bg(pal.surface_hover))
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::NewChat)))
                                    .child(icon("square-pen").size(sz(15.)))
                                    .child(div().flex_1().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::MEDIUM).child("New chat"))
                                    .child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child(SHORTCUT_NEW)),
                            )
                            .child(
                                div()
                                    .id("sb-new-general")
                                    .size(sz(34.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(sz(tokens::radius::MD))
                                    .border_1()
                                    .border_color(pal.border_strong)
                                    .cursor_pointer()
                                    .hover(|d| d.bg(pal.surface_hover))
                                    .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("General chat — no project  (Ctrl+Shift+G)").build(window, cx))
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::NewGeneral)))
                                    .child(icon("globe").size(sz(15.)).text_color(pal.text_muted)),
                            )
                            .child(
                                div()
                                    .id("sb-new-design")
                                    .size(sz(34.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(sz(tokens::radius::MD))
                                    .border_1()
                                    .border_color(pal.border_strong)
                                    .cursor_pointer()
                                    .hover(|d| d.bg(pal.surface_hover))
                                    .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("New design  (Ctrl+Shift+N)").build(window, cx))
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::NewDesign)))
                                    .child(icon("palette").size(sz(15.)).text_color(pal.text_muted)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap(sz(4.))
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .rounded(sz(tokens::radius::MD))
                                    .bg(pal.surface_hover.opacity(0.7))
                                    .child(Input::new(&self.search).small().appearance(false).prefix(icon("search").small().text_color(pal.text_faint)).cleanable(true)),
                            )
                            .child(
                                Button::new("group-mode")
                                    .ghost()
                                    .small()
                                    .icon(icon("list-filter"))
                                    .tooltip(format!("Group by: {}", mode.label()))
                                    .dropdown_menu(move |menu, _, _| {
                                        let mut menu = menu.label("Group chats by");
                                        for m in [GroupMode::Smart, GroupMode::Project, GroupMode::Agent, GroupMode::Date] {
                                            let weak = weak.clone();
                                            menu = menu.item(PopupMenuItem::new(m.label()).checked(m == mode).on_click(move |_, _, cx| {
                                                if let Some(sb) = weak.upgrade() {
                                                    sb.update(cx, |this, cx| {
                                                        this.mode = m;
                                                        cx.notify();
                                                    });
                                                }
                                            }));
                                        }
                                        menu
                                    }),
                            ),
                    )
            )
            .child(div().id("session-list").flex_1().min_h_0().overflow_y_scroll().child(list))
            .when(!pinned_projects.is_empty(), |d| {
                let mut projects = v_flex().gap(sz(1.)).px_2().pb_1();
                for p in pinned_projects.iter().take(6) {
                    let p2 = p.clone();
                    projects = projects.child(
                        h_flex()
                            .id(ElementId::Name(format!("pp-{}", p.path.display()).into()))
                            .group("pp")
                            .px(sz(10.))
                            .py(sz(5.))
                            .gap_2()
                            .rounded(sz(tokens::radius::SM))
                            .items_center()
                            .cursor_pointer()
                            .hover(|d| d.bg(pal.surface_hover.opacity(0.6)))
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(SidebarEvent::NewChatIn(p2.clone()))))
                            .child(icon("folder-git-2").size(sz(14.)).text_color(pal.text_faint))
                            .child(div().flex_1().text_size(sz(tokens::text::SM)).text_color(pal.text_muted).truncate().child(p.name.clone()))
                            .child(div().invisible().group_hover("pp", |d| d.visible()).child(icon("plus").size(sz(12.)).text_color(pal.text_faint))),
                    );
                }
                d.child(
                    v_flex()
                        .pt_2()
                        .child(div().px(sz(18.)).pb_1().text_size(sz(tokens::text::XS)).font_weight(FontWeight::MEDIUM).text_color(pal.text_faint).child("Pinned projects"))
                        .child(projects),
                )
            })
    }
}

#[cfg(target_os = "macos")]
const SHORTCUT_NEW: &str = "⌘N";
#[cfg(not(target_os = "macos"))]
const SHORTCUT_NEW: &str = "Ctrl N";
