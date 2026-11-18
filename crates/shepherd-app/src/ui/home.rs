//! The launcher: pick an agent, a project and a permission mode, then start chatting.

use super::kit::*;
use super::tokens;
use crate::store::Store;
use gpui_kit::component::{
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    v_flex,
};
use gpui_kit::*;
use super::tokens::sz;
use gpui_kit::prelude::FluentBuilder as _;
use shepherd_core::*;

pub enum HomeEvent {
    Started(SessionId),
    PickProject,
}

impl EventEmitter<HomeEvent> for Home {}

pub struct Home {
    store: Entity<Store>,
    input: Entity<TextareaState>,
    agent: Option<AgentKind>,
    project: Option<ProjectRef>,
    /// A general chat, not tied to any project folder.
    general: bool,
    /// Model for the next chat (None = the agent's own default).
    model: Option<String>,
    slash_sel: usize,
    kind: SessionKind,
    _subs: Vec<Subscription>,
}

impl Home {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 12)
                .submit_on_enter(true)
                .placeholder("Describe what you want to build, fix or explore…")
        });
        let subs = vec![
            cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { shift: false, .. } => this.start(window, cx),
                InputEvent::Change => {
                    this.slash_sel = 0;
                    cx.notify();
                }
                _ => {}
            }),
            {
                let weak = cx.entity().downgrade();
                cx.intercept_keystrokes(move |ev, window, cx| {
                    if let Some(this) = weak.upgrade() {
                        this.update(cx, |this, cx| this.on_keystroke(ev, window, cx));
                    }
                })
            },
            cx.observe(&store, |this, store, cx| {
                // Default to the first available agent once detection finishes.
                if this.agent.is_none() {
                    let s = store.read(cx);
                    let settings = s.hub.settings();
                    let pref = settings.preferred_agent();
                    this.agent = pref.filter(|k| s.available_agents().contains(k)).or_else(|| s.available_agents().first().copied());
                    this.model = this.agent.and_then(|a| settings.preferred_model(a));
                }
                if this.project.is_none() {
                    this.project = store.read(cx).recent.first().cloned();
                }
                cx.notify();
            }),
        ];
        Self { store, input, agent: None, project: None, general: false, model: None, slash_sel: 0, kind: SessionKind::Chat, _subs: subs }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.focus(window, cx));
    }

    pub fn kind(&self) -> SessionKind {
        self.kind
    }

    pub fn set_kind(&mut self, kind: SessionKind, cx: &mut Context<Self>) {
        self.kind = kind;
        cx.notify();
    }

    pub fn set_project(&mut self, p: ProjectRef, cx: &mut Context<Self>) {
        self.project = Some(p);
        self.general = false;
        cx.notify();
    }

    /// Switch the launcher to a general chat (no project folder).
    pub fn set_general(&mut self, cx: &mut Context<Self>) {
        self.general = true;
        self.kind = SessionKind::Chat;
        cx.notify();
    }

    fn select_agent(&mut self, agent: AgentKind, cx: &mut Context<Self>) {
        if self.agent != Some(agent) {
            self.agent = Some(agent);
            self.model = self.store.read(cx).hub.settings().preferred_model(agent);
            self.store.read(cx).record_selection(agent, None);
        }
        cx.notify();
    }

    fn select_model(&mut self, model: Option<String>, cx: &mut Context<Self>) {
        if let Some(agent) = self.agent {
            self.store.read(cx).record_selection(agent, model.as_deref());
        }
        self.model = model;
        cx.notify();
    }

    fn model_menu_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(agent) = self.agent else { return div().into_any_element() };
        let models = self.store.read(cx).known_models(agent);
        let current = self.model.clone();
        let label = match &current {
            Some(m) => models.iter().find(|x| &x.id == m).map(|x| x.name.clone()).unwrap_or_else(|| m.clone()),
            None => "Default model".into(),
        };
        let weak = cx.entity().downgrade();
        Button::new("home-model")
            .ghost()
            .icon(icon("brain"))
            .label(label)
            .dropdown_caret(true)
            .tooltip(if models.is_empty() { "More models appear after your first chat with this agent" } else { "Model for new chats" })
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.label("Model").max_h(sz(420.)).scrollable(true);
                let w = weak.clone();
                menu = menu.item(PopupMenuItem::new("Agent default").checked(current.is_none()).on_click(move |_, _, cx| {
                    if let Some(h) = w.upgrade() {
                        h.update(cx, |h, cx| h.select_model(None, cx));
                    }
                }));
                for mi in &models {
                    let w = weak.clone();
                    let id = mi.id.clone();
                    let label = match &mi.description {
                        Some(d) => format!("{} — {}", mi.name, d),
                        None => mi.name.clone(),
                    };
                    menu = menu.item(PopupMenuItem::new(label).checked(current.as_deref() == Some(mi.id.as_str())).on_click(move |_, _, cx| {
                        if let Some(h) = w.upgrade() {
                            h.update(cx, |h, cx| h.select_model(Some(id.clone()), cx));
                        }
                    }));
                }
                menu
            })
            .into_any_element()
    }

    fn slash_matches(&self, cx: &App) -> Vec<CommandInfo> {
        let text = self.input.read(cx).value();
        let Some(query) = text.strip_prefix('/') else { return vec![] };
        if query.contains(char::is_whitespace) {
            return vec![];
        }
        let Some(agent) = self.agent else { return vec![] };
        rank_commands(&self.store.read(cx).known_commands(agent), query)
    }

    fn on_keystroke(&mut self, ev: &KeystrokeEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.input.read(cx).focus_handle(cx).is_focused(window) || ev.keystroke.modifiers.modified() {
            return;
        }
        let matches = self.slash_matches(cx);
        if matches.is_empty() {
            return;
        }
        match ev.keystroke.key.as_str() {
            "up" => self.slash_sel = (self.slash_sel + matches.len() - 1) % matches.len(),
            "down" => self.slash_sel = (self.slash_sel + 1) % matches.len(),
            "tab" | "enter" => {
                let value = format!("/{} ", matches[self.slash_sel.min(matches.len() - 1)].name);
                self.input.update(cx, |i, cx| i.set_value(value, window, cx));
            }
            "escape" => self.input.update(cx, |i, cx| i.set_value("", window, cx)),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// The app-wide default permission mode (persisted in settings).
    fn default_mode(&self, cx: &App) -> PermissionMode {
        self.store.read(cx).hub.settings().default_mode
    }

    fn set_default_mode(&mut self, mode: PermissionMode, cx: &mut Context<Self>) {
        let hub = self.store.read(cx).hub.clone();
        let mut s = hub.settings();
        s.default_mode = mode;
        hub.update_settings(s);
        cx.notify();
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        let Some(agent) = self.agent else {
            window.push_notification(Notification::warning("No agent is installed. Install Claude Code, Codex, Copilot, OpenCode or Hermes."), cx);
            return;
        };
        let project = match (&self.project, self.kind) {
            _ if self.general && self.kind == SessionKind::Chat => std::path::PathBuf::new(),
            (Some(p), _) => p.path.clone(),
            (None, SessionKind::Design) => {
                // Design without a project gets its own workspace folder.
                let slug: String = derive_title(if text.is_empty() { "design" } else { &text })
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == ' ')
                    .take(32)
                    .collect::<String>()
                    .trim()
                    .replace(' ', "-")
                    .to_lowercase();
                let dir = designs_dir().join(format!("{}-{}", slug, chrono::Local::now().format("%m%d-%H%M")));
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    window.push_notification(Notification::error(format!("Couldn't create design folder: {e}")), cx);
                    return;
                }
                dir
            }
            (None, SessionKind::Chat) => {
                cx.emit(HomeEvent::PickProject);
                return;
            }
        };
        if text.is_empty() && self.kind == SessionKind::Chat {
            return;
        }
        let prompt = if text.is_empty() { None } else { Some(text) };
        let mode = self.default_mode(cx).for_agent(agent);
        let cmd = AppCommand::NewSession { agent, kind: self.kind, project, mode, model: self.model.clone(), prompt };
        let id = self.store.update(cx, |s, cx| s.dispatch(cmd, window, cx));
        if let Some(id) = id {
            self.input.update(cx, |i, cx| i.set_value("", window, cx));
            cx.emit(HomeEvent::Started(id));
        }
    }

    fn agent_menu_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let weak = cx.entity().downgrade();
        let agents = self.store.read(cx).agents.clone();
        let current = self.agent;
        let label = current.map(|k| k.display_name()).unwrap_or("No agent");
        Button::new("agent-pick")
            .ghost()
            .when_some(current, |b, k| b.icon(icon(agent_icon(k))))
            .label(label)
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.label("Agent");
                for a in &agents {
                    let weak = weak.clone();
                    let kind = a.kind;
                    let note = if a.available() {
                        a.version.clone().map(|v| format!("  v{v}")).unwrap_or_default()
                    } else {
                        "  — not installed".into()
                    };
                    menu = menu.item(
                        PopupMenuItem::new(format!("{}{}", kind.display_name(), note))
                            .icon(icon(agent_icon(kind)))
                            .checked(current == Some(kind))
                            .disabled(!a.available())
                            .on_click(move |_, _, cx| {
                                if let Some(h) = weak.upgrade() {
                                    h.update(cx, |h, cx| {
                                        h.select_agent(kind, cx);
                                        cx.notify();
                                    });
                                }
                            }),
                    );
                }
                menu
            })
    }
}

pub fn mode_menu<T: 'static>(
    id: &'static str,
    agent: Option<AgentKind>,
    mode: PermissionMode,
    weak: WeakEntity<T>,
    set: impl Fn(&mut T, PermissionMode, &mut Window, &mut Context<T>) + Clone + 'static,
) -> impl IntoElement {
    let modes: Vec<PermissionMode> = agent.map(|a| a.supported_modes().to_vec()).unwrap_or_else(|| PermissionMode::ALL.to_vec());
    let danger = mode.is_dangerous();
    Button::new(id)
        .ghost()
        .icon(icon(mode_icon(mode)))
        .label(mode.label())
        .dropdown_caret(true)
        .when(danger, |b| b.text_color(hex(0xEF4444)))
        .dropdown_menu(move |mut menu, _, _| {
            menu = menu.label("Permissions");
            for m in &modes {
                let m = *m;
                let weak = weak.clone();
                let set = set.clone();
                menu = menu.item(
                    PopupMenuItem::new(format!("{} — {}", m.label(), m.description()))
                        .icon(icon(mode_icon(m)))
                        .checked(m == mode)
                        .on_click(move |_, window, cx| {
                            if let Some(e) = weak.upgrade() {
                                e.update(cx, |t, cx| set(t, m, window, cx));
                            }
                        }),
                );
            }
            menu
        })
}

pub fn mode_icon(m: PermissionMode) -> &'static str {
    match m {
        PermissionMode::Ask => "shield-check",
        PermissionMode::AcceptEdits => "file-pen",
        PermissionMode::Auto => "zap",
        PermissionMode::Plan => "map",
        PermissionMode::AllowAll => "shield-alert",
    }
}

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = tokens::p(cx);
        let agent_button = self.agent_menu_button(cx).into_any_element();
        let model_button = self.model_menu_button(cx);
        let store = self.store.read(cx);
        let no_agents = store.agents_scanned && store.available_agents().is_empty();
        let mut chips: Vec<ProjectRef> = store.pinned.clone();
        for r in &store.recent {
            if !chips.iter().any(|c| c.path == r.path) {
                chips.push(r.clone());
            }
        }
        if chips.len() < 5 {
            for h in store.hub.search_projects("", 10) {
                if chips.len() >= 6 {
                    break;
                }
                if !chips.iter().any(|c| c.path == h.project.path) {
                    chips.push(h.project);
                }
            }
        }
        chips.truncate(6);
        let design = self.kind == SessionKind::Design;
        let general = self.general && !design;
        let weak = cx.entity().downgrade();
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let default_mode = self.default_mode(cx);
        let effective_mode = self.agent.map(|a| default_mode.for_agent(a)).unwrap_or(default_mode);
        let project_label = match (&self.project, design) {
            _ if general => "General".to_string(),
            (Some(p), _) => p.name.clone(),
            (None, true) => "New design folder".into(),
            (None, false) => "Choose project".into(),
        };
        let send_enabled = self.agent.is_some();

        let segmented = h_flex()
            .p(sz(3.))
            .gap(sz(2.))
            .rounded(sz(tokens::radius::MD + 2.))
            .bg(pal.surface_hover)
            .children([(SessionKind::Chat, "Chat"), (SessionKind::Design, "Design")].map(|(k, label)| {
                let active = self.kind == k;
                div()
                    .id(label)
                    .px(sz(14.))
                    .h(sz(28.))
                    .flex()
                    .items_center()
                    .rounded(sz(tokens::radius::MD))
                    .text_size(sz(tokens::text::BASE))
                    .cursor_pointer()
                    .when(active, |d| d.bg(pal.surface).text_color(pal.text).font_weight(FontWeight::MEDIUM).shadow(tokens::shadow_sm(&pal)))
                    .when(!active, |d| d.text_color(pal.text_muted).hover(|d| d.text_color(pal.text)))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_kind(k, cx)))
                    .child(label)
            }));

        let matches = self.slash_matches(cx);
        let sel = self.slash_sel.min(matches.len().saturating_sub(1));
        let slash_menu = (!matches.is_empty()).then(|| {
            let mut list = v_flex().p(sz(4.)).gap(sz(1.));
            for (i, c) in matches.iter().enumerate() {
                let name = c.name.clone();
                list = list.child(
                    h_flex()
                        .id(ElementId::Name(format!("hslash-{}", c.name).into()))
                        .gap_3()
                        .px(sz(10.))
                        .py(sz(7.))
                        .rounded(sz(tokens::radius::MD))
                        .items_center()
                        .cursor_pointer()
                        .when(i == sel, |d| d.bg(pal.surface_hover))
                        .hover(|d| d.bg(pal.surface_hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let v = format!("/{name} ");
                            this.input.update(cx, |inp, cx| {
                                inp.set_value(v, window, cx);
                                inp.focus(window, cx);
                            });
                        }))
                        .child(div().flex_none().text_size(sz(tokens::text::BASE)).font_family(tokens::FONT_MONO).child(format!("/{}", c.name)))
                        .child(div().flex_1().min_w_0().truncate().text_size(sz(tokens::text::SM)).text_color(pal.text_muted).child(c.description.clone().unwrap_or_default())),
                );
            }
            div()
                .absolute()
                .top_full()
                .left_0()
                .right_0()
                .mt_2()
                .rounded(sz(tokens::radius::LG))
                .border_1()
                .border_color(pal.border_strong)
                .bg(pal.raised)
                .shadow(tokens::shadow_lg(&pal))
                .child(list)
        });
        let composer = v_flex()
            .relative()
            .w_full()
            .rounded(sz(tokens::radius::XL))
            .border_1()
            .border_color(if focused { pal.text_faint } else { pal.border_strong })
            .bg(pal.surface)
            .shadow(tokens::shadow_md(&pal))
            .when_some(slash_menu, |d, m| d.child(m))
            .child(div().px(sz(18.)).pt(sz(16.)).pb_1().text_size(sz(tokens::text::READ)).child(Textarea::new(&self.input).appearance(false)))
            .child(
                h_flex()
                    .px(sz(10.))
                    .pb(sz(10.))
                    .pt_1()
                    .gap_1()
                    .items_center()
                    .child(agent_button)
                    .child(model_button)
                    .child(
                        Button::new("project-pick")
                            .ghost()
                            .icon(icon(if general { "globe" } else { "folder" }))
                            .label(project_label)
                            .tooltip("Choose project  (Ctrl+P)")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(HomeEvent::PickProject))),
                    )
                    .child(mode_menu("home-mode", None, default_mode, weak.clone(), |h: &mut Home, m, _, cx| h.set_default_mode(m, cx)))
                    .when(effective_mode != default_mode, |d| {
                        d.child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child(format!("{} here", effective_mode.label())))
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("start")
                            .size(sz(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(sz(tokens::radius::LG))
                            .bg(if send_enabled { pal.text } else { pal.surface_hover })
                            .text_color(if send_enabled { pal.bg } else { pal.text_faint })
                            .when(send_enabled, |d| d.hover(|d| d.opacity(0.85)))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| this.start(window, cx)))
                            .child(icon("arrow-up").size(sz(18.))),
                    ),
            );

        let mut chip_row = h_flex().gap_2().flex_wrap().justify_center();
        if !design {
            chip_row = chip_row.child(project_chip("chip-general", "globe", "General", general, &pal).on_click(cx.listener(|this, _, _, cx| this.set_general(cx))));
        }
        for p in chips {
            let selected = !general && self.project.as_ref().is_some_and(|c| c.path == p.path);
            let p2 = p.clone();
            chip_row = chip_row.child(
                project_chip(ElementId::Name(format!("chip-{}", p.path.display()).into()), if p.pinned { "pin" } else { "folder" }, p.name.clone(), selected, &pal)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_project(p2.clone(), cx))),
            );
        }
        chip_row = chip_row.child(
            project_chip("chip-more", "search", "More…", false, &pal).on_click(cx.listener(|_, _, _, cx| cx.emit(HomeEvent::PickProject))),
        );

        div().id("home-scroll").size_full().overflow_y_scroll().child(
            v_flex()
                .w_full()
                .max_w(sz(720.))
                .mx_auto()
                .px_6()
                .pt(sz(120.))
                .pb_10()
                .gap(sz(20.))
                .items_center()
                .child(
                    div()
                        .text_size(sz(tokens::text::XXL + 4.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(pal.text)
                        .child(if design { "What are we designing?" } else { "What are we working on?" }),
                )
                .child(segmented)
                .child(div().w_full().mt_2().child(composer))
                .when(no_agents, |d| {
                    d.child(
                        div()
                            .text_size(sz(tokens::text::SM))
                            .text_color(pal.warning)
                            .child("No agents found. Install Claude Code, Codex, Copilot CLI, OpenCode or Hermes, then rescan in Settings."),
                    )
                })
                .child(chip_row),
        )
    }
}

fn project_chip(id: impl Into<ElementId>, icon_name: &'static str, label: impl Into<SharedString>, selected: bool, pal: &tokens::Palette) -> Stateful<Div> {
    let pal = *pal;
    h_flex()
        .id(id)
        .gap(sz(6.))
        .px(sz(11.))
        .h(sz(30.))
        .rounded_full()
        .items_center()
        .text_size(sz(tokens::text::SM))
        .cursor_pointer()
        .border_1()
        .border_color(if selected { pal.text_faint } else { pal.border })
        .bg(if selected { pal.surface_hover } else { gpui_kit::transparent_black() })
        .text_color(if selected { pal.text } else { pal.text_muted })
        .hover(move |d| d.text_color(pal.text).bg(pal.surface_hover))
        .child(icon(icon_name).size(sz(13.)))
        .child(label.into())
}
