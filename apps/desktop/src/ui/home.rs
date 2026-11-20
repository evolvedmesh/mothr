//! The launcher: pick an agent, a project and a permission mode, then start chatting.

use super::chat::{file_chip, image_view, is_image, mime_for};
use super::kit::*;
use super::options::{Pick, RunSettings, run_settings, settings_for};
use super::{clip, tokens};
use crate::store::Store;
use std::path::PathBuf;
use gpui_kit::component::{
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    Sizable as _,
    v_flex,
};
use gpui_kit::*;
use super::tokens::sz;
use gpui_kit::prelude::FluentBuilder as _;
use mothr_core::*;

pub enum HomeEvent {
    Started(SessionId),
    PickProject,
    /// The text or attachments of the draft changed.
    Changed,
}

impl EventEmitter<HomeEvent> for Home {}

pub struct Home {
    /// Which draft this is (the workspace keeps several).
    pub id: u64,
    store: Entity<Store>,
    input: Entity<TextareaState>,
    attachments: Vec<PathBuf>,
    agent: Option<AgentKind>,
    project: Option<ProjectRef>,
    /// A general chat, not tied to any project folder.
    general: bool,
    /// Model for the next chat (None = the agent's own default).
    model: Option<String>,
    /// Reasoning effort / context window for the next chat (None = the agent's own default).
    effort: Option<String>,
    context: Option<String>,
    slash_sel: usize,
    _subs: Vec<Subscription>,
}

impl Home {
    pub fn new(id: u64, store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
                    cx.emit(HomeEvent::Changed);
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
            cx.observe(&store, |this, _, cx| {
                this.sync_defaults(cx);
                cx.notify();
            }),
        ];
        let mut this = Self { id, store, input, attachments: vec![], agent: None, project: None, general: false, model: None, effort: None, context: None, slash_sel: 0, _subs: subs };
        this.sync_defaults(cx);
        this
    }

    /// Fill in the agent (and the last project) once they are known.
    fn sync_defaults(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        if self.agent.is_none() {
            let s = store.read(cx);
            let settings = s.hub.settings();
            let pref = settings.preferred_agent();
            self.agent = pref.filter(|k| s.available_agents().contains(k)).or_else(|| s.available_agents().first().copied());
            self.model = self.agent.and_then(|a| settings.preferred_model(a));
            self.effort = self.agent.and_then(|a| settings.preferred_effort(a));
            self.context = self.agent.and_then(|a| settings.preferred_context(a));
        }
        self.normalize_options(cx);
        if self.project.is_none() && !self.general {
            self.project = store.read(cx).recent.first().cloned();
        }
    }

    /// There is something written (or attached) here.
    pub fn has_draft(&self, cx: &App) -> bool {
        !self.input.read(cx).value().trim().is_empty() || !self.attachments.is_empty()
    }

    /// First line of the draft, for the sidebar.
    pub fn preview(&self, cx: &App) -> String {
        let text = self.input.read(cx).value();
        let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
        if line.is_empty() {
            return match self.attachments.len() {
                0 => String::new(),
                1 => "1 attachment".into(),
                n => format!("{n} attachments"),
            };
        }
        let mut out: String = line.chars().take(60).collect();
        if line.chars().count() > 60 {
            out.push('…');
        }
        out
    }

    pub fn agent(&self) -> Option<AgentKind> {
        self.agent
    }

    pub fn project_name(&self) -> String {
        if self.general { "General".into() } else { self.project.as_ref().map(|p| p.name.clone()).unwrap_or_default() }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.focus(window, cx));
    }

    pub fn set_project(&mut self, p: ProjectRef, cx: &mut Context<Self>) {
        self.project = Some(p);
        self.general = false;
        cx.notify();
    }

    /// Switch the launcher to a general chat (no project folder).
    pub fn set_general(&mut self, cx: &mut Context<Self>) {
        self.general = true;
        cx.notify();
    }

    fn select_agent(&mut self, agent: AgentKind, cx: &mut Context<Self>) {
        if self.agent != Some(agent) {
            self.agent = Some(agent);
            let settings = self.store.read(cx).hub.settings();
            self.model = settings.preferred_model(agent);
            self.effort = settings.preferred_effort(agent);
            self.context = settings.preferred_context(agent);
            self.normalize_options(cx);
            self.store.read(cx).record_selection(agent, None);
        }
        cx.notify();
    }

    fn select_model(&mut self, model: Option<String>, cx: &mut Context<Self>) {
        self.model = model;
        self.normalize_options(cx);
        self.remember(cx);
        cx.notify();
    }

    fn select_effort(&mut self, effort: Option<String>, cx: &mut Context<Self>) {
        self.effort = effort;
        self.remember(cx);
        cx.notify();
    }

    fn select_context(&mut self, context: Option<String>, cx: &mut Context<Self>) {
        self.context = context;
        self.remember(cx);
        cx.notify();
    }

    /// Save the picks so the next new chat starts with them.
    fn remember(&self, cx: &App) {
        if let Some(agent) = self.agent {
            let store = self.store.read(cx);
            store.record_model(agent, self.model.as_deref());
            store.record_options(agent, self.effort.as_deref(), self.context.as_deref());
        }
    }

    /// Drop a reasoning / context pick the chosen model doesn't offer (once the list is known).
    fn normalize_options(&mut self, cx: &App) {
        let Some(agent) = self.agent else { return };
        let models = self.store.read(cx).known_models(agent);
        if models.is_empty() {
            return;
        }
        if self.model.as_ref().is_some_and(|m| !models.iter().any(|x| &x.id == m)) {
            self.model = None;
        }
        let (efforts, contexts) = settings_for(&models, self.model.as_deref());
        if self.effort.as_ref().is_some_and(|e| !efforts.contains(e)) {
            self.effort = None;
        }
        if self.context.as_ref().is_some_and(|c| !contexts.iter().any(|x| &x.id == c)) {
            self.context = None;
        }
    }

    /// Agent, model, reasoning and context behind one chip.
    fn run_settings_chip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let store = self.store.read(cx);
        let agents = store.agents.clone();
        let models = self.agent.map(|a| store.known_models(a)).unwrap_or_default();
        let (efforts, contexts) = settings_for(&models, self.model.as_deref());
        let s = RunSettings {
            agents,
            agent: self.agent,
            models,
            model: self.model.clone(),
            efforts,
            effort: self.effort.clone(),
            contexts,
            context: self.context.clone(),
            allow_default: true,
            compact: false,
        };
        run_settings("home-run", s, cx.entity().downgrade(), |h: &mut Home, pick, _, cx| match pick {
            Pick::Agent(k) => h.select_agent(k, cx),
            Pick::Model(m) => h.select_model(m, cx),
            Pick::Effort(e) => h.select_effort(e, cx),
            Pick::Context(c) => h.select_context(c, cx),
        })
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
        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        let k = &ev.keystroke;
        if k.key == "v" && k.modifiers.secondary(cx) && !k.modifiers.shift && !k.modifiers.alt {
            // A pasted screenshot or copied file becomes an attachment; text goes to the box.
            let pasted = clip::attachments_from_clipboard(cx);
            if !pasted.is_empty() {
                self.add_attachments(pasted, cx);
                cx.stop_propagation();
            }
            return;
        }
        if k.modifiers.modified() {
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

    fn add_attachments(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        for p in paths {
            if !self.attachments.contains(&p) {
                self.attachments.push(p);
            }
        }
        cx.emit(HomeEvent::Changed);
        cx.notify();
    }

    fn attach(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Attach".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update(cx, |this, cx| this.add_attachments(paths, cx));
            }
        })
        .detach();
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
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }
        let project = match &self.project {
            _ if self.general => std::path::PathBuf::new(),
            Some(p) => p.path.clone(),
            None => {
                cx.emit(HomeEvent::PickProject);
                return;
            }
        };
        let mode = self.default_mode(cx).for_agent(agent);
        self.remember(cx);
        // Attachments travel with the first message, so it goes as a prompt of its own.
        let attachments: Vec<Attachment> = self.attachments.iter().map(|path| Attachment { mime: mime_for(path), path: path.clone() }).collect();
        let prompt = if attachments.is_empty() { Some(text.clone()) } else { None };
        let cmd = AppCommand::NewSession {
            agent,
            kind: SessionKind::Chat,
            project,
            mode,
            model: self.model.clone(),
            effort: self.effort.clone(),
            context: self.context.clone(),
            prompt,
        };
        let id = self.store.update(cx, |s, cx| s.dispatch(cmd, window, cx));
        if let Some(id) = id {
            if !attachments.is_empty() {
                let command = SessionCommand::Prompt { text, attachments, delivery: Delivery::Normal };
                self.store.update(cx, |s, cx| {
                    s.dispatch(AppCommand::Session { id, command }, window, cx);
                });
            }
            self.attachments.clear();
            self.input.update(cx, |i, cx| i.set_value("", window, cx));
            cx.emit(HomeEvent::Started(id));
        }
    }
}

/// Button that picks one of the detected agents.
pub fn agent_menu<T: 'static>(
    id: &'static str,
    agents: Vec<AgentInstall>,
    current: Option<AgentKind>,
    weak: WeakEntity<T>,
    set: impl Fn(&mut T, AgentKind, &mut Context<T>) + Clone + 'static,
) -> impl IntoElement {
    Button::new(id)
        .ghost()
        .when_some(current, |b, k| b.icon(agent_logo(k)))
        .label(current.map(|k| k.display_name()).unwrap_or("No agent"))
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            menu = menu.label("Agent");
            for a in &agents {
                let weak = weak.clone();
                let set = set.clone();
                let kind = a.kind;
                let note = if a.available() { a.version.clone().map(|v| format!("  v{v}")).unwrap_or_default() } else { "  — not installed".into() };
                menu = menu.item(
                    PopupMenuItem::new(format!("{}{}", kind.display_name(), note))
                        .icon(agent_logo(kind))
                        .checked(current == Some(kind))
                        .disabled(!a.available())
                        .on_click(move |_, _, cx| {
                            if let Some(e) = weak.upgrade() {
                                e.update(cx, |t, cx| set(t, kind, cx));
                            }
                        }),
                );
            }
            menu
        })
}

/// Permission-mode picker. In a composer it is a quiet chip (`chip`: no caret, and the mode's
/// description as its tooltip); in Settings it reads as a form control.
pub fn mode_menu<T: 'static>(
    id: &'static str,
    agent: Option<AgentKind>,
    mode: PermissionMode,
    chip: Option<String>,
    weak: WeakEntity<T>,
    set: impl Fn(&mut T, PermissionMode, &mut Window, &mut Context<T>) + Clone + 'static,
) -> impl IntoElement {
    let modes: Vec<PermissionMode> = agent.map(|a| a.supported_modes().to_vec()).unwrap_or_else(|| PermissionMode::ALL.to_vec());
    let danger = mode.is_dangerous();
    Button::new(id)
        .ghost()
        .icon(icon(mode_icon(mode)))
        .label(mode.label())
        .map(|b| match &chip {
            Some(tip) => b.tooltip(tip.clone()),
            None => b.dropdown_caret(true),
        })
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
        let run_chip = self.run_settings_chip(cx);
        let attachments = self.attachments.clone();
        let store = self.store.read(cx);
        let no_agents = store.agents_scanned && store.available_agents().is_empty();
        let general = self.general;
        let weak = cx.entity().downgrade();
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let default_mode = self.default_mode(cx);
        let effective_mode = self.agent.map(|a| default_mode.for_agent(a)).unwrap_or(default_mode);
        // An agent without the default mode runs in its nearest one; say so on hover.
        let mode_tip = if effective_mode != default_mode {
            format!("{} — {}. This agent runs it as {}.", default_mode.label(), default_mode.description(), effective_mode.label())
        } else {
            format!("{} — {}", default_mode.label(), default_mode.description())
        };
        let project_label = match &self.project {
            _ if general => "General".to_string(),
            Some(p) => p.name.clone(),
            None => "Choose project".into(),
        };
        let send_enabled = self.agent.is_some() && (self.has_draft(cx));

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
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.add_attachments(paths.paths().to_vec(), cx)))
            .when_some(slash_menu, |d, m| d.child(m))
            .when(!attachments.is_empty(), |d| {
                d.child(h_flex().px_3().pt_3().gap_1().flex_wrap().children(attachments.iter().enumerate().map(|(i, p)| {
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(if is_image(p) { image_view(p, format!("home-att-{i}"), 72., 56., cx).into_any_element() } else { file_chip(p, cx).into_any_element() })
                        .child(
                            Button::new(ElementId::Name(format!("home-rm-att-{i}").into()))
                                .ghost()
                                .small()
                                .icon(icon("x"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if i < this.attachments.len() {
                                        this.attachments.remove(i);
                                    }
                                    cx.emit(HomeEvent::Changed);
                                    cx.notify();
                                })),
                        )
                })))
            })
            .child(div().px(sz(18.)).pt(sz(16.)).pb_1().text_size(sz(tokens::text::READ)).child(Textarea::new(&self.input).appearance(false)))
            .child(
                h_flex()
                    .px(sz(10.))
                    .pb(sz(10.))
                    .pt_1()
                    .gap_1()
                    .items_end()
                    .child(
                        // Controls wrap when the composer is narrow; the send button stays put.
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .min_h(sz(36.))
                            .flex_wrap()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new("home-attach")
                                    .ghost()
                                    .icon(icon("paperclip"))
                                    .tooltip("Attach files or images (drop or paste them too)")
                                    .on_click(cx.listener(|this, _, _, cx| this.attach(cx))),
                            )
                            .child(mode_menu("home-mode", None, default_mode, Some(mode_tip), weak.clone(), |h: &mut Home, m, _, cx| h.set_default_mode(m, cx)))
                            .child(div().flex_1())
                            .child(run_chip),
                    )
                    .child(
                        div()
                            .id("start")
                            .flex_none()
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
                        .child("What are we working on?"),
                )
                .child(
                    v_flex().w_full().mt_2().gap(sz(6.)).child(composer).child(
                        // Where the chat runs sits under the box, not among the per-message controls.
                        h_flex().px(sz(6.)).child(
                            Button::new("project-pick")
                                .ghost()
                                .small()
                                .max_w(sz(260.))
                                .text_color(pal.text_muted)
                                .icon(icon(if general { "globe" } else { "folder" }))
                                .label(project_label)
                                .dropdown_caret(true)
                                .tooltip("Choose project  (Ctrl+P)")
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(HomeEvent::PickProject))),
                        ),
                    ),
                )
                .when(no_agents, |d| {
                    d.child(
                        div()
                            .text_size(sz(tokens::text::SM))
                            .text_color(pal.warning)
                            .child("No agents found. Install Claude Code, Codex, Copilot CLI, OpenCode or Hermes, then rescan in Settings."),
                    )
                }),
        )
    }
}
