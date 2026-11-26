//! The prompt box that starts a design chat, on the gallery and in an idle viewer.

use crate::store::Store;
use crate::ui::home::{agent_menu, mode_icon};
use crate::ui::kit::*;
use crate::ui::tokens::{self, sz};
use gpui_kit::component::{
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    notification::Notification,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mothr_core::*;
use std::path::PathBuf;

pub enum ComposerEvent {
    Started { session: SessionId, project: PathBuf },
    PickProject,
}

impl EventEmitter<ComposerEvent> for DesignComposer {}

pub struct DesignComposer {
    store: Entity<Store>,
    input: Entity<TextareaState>,
    agent: Option<AgentKind>,
    /// Where the design goes; `None` creates a new folder for it.
    project: Option<ProjectRef>,
    /// The project is fixed (the composer belongs to an open design).
    locked: bool,
    _subs: Vec<Subscription>,
}

impl DesignComposer {
    pub fn new(store: Entity<Store>, project: Option<ProjectRef>, placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 8).submit_on_enter(true).placeholder(placeholder));
        let subs = vec![
            cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { shift: false, .. }) {
                    this.start(window, cx);
                }
            }),
            cx.observe(&store, |this, _, cx| {
                this.pick_default_agent(cx);
                cx.notify();
            }),
        ];
        let locked = project.is_some();
        let mut this = Self { store, input, agent: None, project, locked, _subs: subs };
        this.pick_default_agent(cx);
        this
    }

    fn pick_default_agent(&mut self, cx: &App) {
        if self.agent.is_none() {
            let store = self.store.read(cx);
            let available = store.available_agents();
            self.agent = store.hub.settings().preferred_agent().filter(|k| available.contains(k)).or(available.first().copied());
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.focus(window, cx));
    }

    pub fn set_project(&mut self, project: Option<ProjectRef>, cx: &mut Context<Self>) {
        self.project = project;
        cx.notify();
    }

    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_string();
        self.input.update(cx, |i, cx| {
            i.set_value(text, window, cx);
            i.focus(window, cx);
        });
    }

    /// A readable folder for a design that isn't part of a project.
    fn new_workspace(text: &str) -> std::io::Result<PathBuf> {
        let slug: String = derive_title(if text.is_empty() { "design" } else { text })
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ')
            .take(32)
            .collect::<String>()
            .trim()
            .replace(' ', "-")
            .to_lowercase();
        let dir = designs_dir().join(format!("{}-{}", slug, chrono::Local::now().format("%m%d-%H%M%S")));
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        let Some(agent) = self.agent else {
            window.push_notification(Notification::warning("No agent is installed. Install Claude Code, Codex, Copilot, OpenCode or Hermes."), cx);
            return;
        };
        let project = match &self.project {
            Some(p) => p.path.clone(),
            None => match Self::new_workspace(&text) {
                Ok(dir) => dir,
                Err(e) => {
                    window.push_notification(Notification::error(format!("Couldn't create a folder for the design: {e}")), cx);
                    return;
                }
            },
        };
        let settings = self.store.read(cx).hub.settings();
        let model = settings.preferred_model(agent);
        // Remembered reasoning / context only if the model offers them.
        let (efforts, contexts) = crate::ui::options::settings_for(&self.store.read(cx).known_models(agent), model.as_deref());
        let effort = settings.preferred_effort(agent).filter(|e| efforts.contains(e));
        let context = settings.preferred_context(agent).filter(|c| contexts.iter().any(|x| &x.id == c));
        let cmd = AppCommand::NewSession {
            agent,
            kind: SessionKind::Design,
            project: project.clone(),
            mode: settings.default_mode.for_agent(agent),
            model,
            effort,
            context,
            prompt: (!text.is_empty()).then_some(text),
            workspace: None,
            extra_projects: vec![],
        };
        if let Some(session) = self.store.update(cx, |s, cx| s.dispatch(cmd, window, cx)) {
            self.input.update(cx, |i, cx| i.set_value("", window, cx));
            cx.emit(ComposerEvent::Started { session, project });
        }
    }
}

impl Render for DesignComposer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = tokens::p(cx);
        let store = self.store.read(cx);
        let agents = store.agents.clone();
        let mode = self.agent.map(|a| store.hub.settings().default_mode.for_agent(a)).unwrap_or_default();
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let ready = self.agent.is_some();
        let weak = cx.entity().downgrade();
        let project_label = self.project.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "New folder".into());
        v_flex()
            .w_full()
            .rounded(sz(tokens::radius::XL))
            .border_1()
            .border_color(if focused { pal.text_faint } else { pal.border_strong })
            .bg(pal.surface)
            .shadow(tokens::shadow_md(&pal))
            .child(div().px(sz(16.)).pt(sz(14.)).pb_1().text_size(sz(tokens::text::READ)).child(Textarea::new(&self.input).appearance(false)))
            .child(
                h_flex()
                    .px(sz(8.))
                    .pb(sz(8.))
                    .gap_1()
                    .items_center()
                    .child(h_flex().flex_1().min_w_0().flex_wrap().gap_1().items_center().child(agent_menu("design-agent", agents, self.agent, weak, |c: &mut DesignComposer, k, cx| {
                        c.agent = Some(k);
                        c.store.read(cx).record_selection(k, None);
                        cx.notify();
                    }))
                    .when(!self.locked, |d| {
                        d.child(
                            Button::new("design-project")
                                .ghost()
                                .max_w(sz(180.))
                                .icon(icon(if self.project.is_some() { "folder" } else { "folder-plus" }))
                                .label(project_label)
                                .tooltip("Keep this design in a repo")
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::PickProject))),
                        )
                    })
                    .child(
                        h_flex()
                            .id("design-mode")
                            .gap(sz(6.))
                            .px(sz(6.))
                            .text_size(sz(tokens::text::XS))
                            .text_color(pal.text_faint)
                            .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Your default permission mode").build(window, cx))
                            .child(icon(mode_icon(mode)).size(sz(12.)))
                            .child(mode.label()),
                    ))
                    .child(
                        div()
                            .id("design-start")
                            .flex_none()
                            .size(sz(34.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(sz(tokens::radius::LG))
                            .bg(if ready { pal.text } else { pal.surface_hover })
                            .text_color(if ready { pal.bg } else { pal.text_faint })
                            .when(ready, |d| d.hover(|d| d.opacity(0.85)))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| this.start(window, cx)))
                            .child(icon("arrow-up").size(sz(17.))),
                    ),
            )
    }
}
