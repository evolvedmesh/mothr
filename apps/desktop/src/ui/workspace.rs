//! The root view: title bar, sidebar, routed main area and overlays.

use super::chat::{ChatEvent, ChatView};
use super::design::{DesignPage, DesignsEvent, DesignsPage};
use super::home::{Home, HomeEvent};
use super::kit::*;
use super::picker::{PickerEvent, ProjectPicker};
use super::projects::{Location, ProjectsEvent, ProjectsPage};
use super::settings::SettingsView;
use super::sidebar::{DraftRow, Sidebar, SidebarEvent};
use super::usage;
use super::{theme, tokens};
use crate::remote::RemoteController;
use crate::store::{Store, StoreEvent};
use crate::{FocusComposer, GoBack, GoForward, NewChat, NewDesign, OpenProjectPalette, OpenProjects, OpenSettings, ToggleSidebar};
use gpui_kit::component::{
    ActiveTheme as _, TitleBar, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
    v_flex,
};
use gpui_kit::*;
use super::tokens::sz;
use gpui_kit::prelude::FluentBuilder as _;
use mothr_core::*;
use mothr_hub::Hub;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
pub enum Route {
    Home,
    Session(SessionId),
    Designs,
    Projects,
    Settings,
}

/// A place in the app, as the back/forward history remembers it.
#[derive(Clone, PartialEq, Debug)]
enum Place {
    Home,
    Session(SessionId),
    Settings,
    /// The gallery, or one page of a design.
    Designs(Option<(PathBuf, DesignPage)>),
    /// The list, or a folder or file of one project.
    Projects(Option<(PathBuf, Location)>),
}

/// Back/forward through the places visited, like a browser.
#[derive(Default)]
struct History {
    back: Vec<Place>,
    forward: Vec<Place>,
    /// Where we are, as last recorded.
    here: Option<Place>,
    /// A sync is queued for the end of the current update.
    sync_queued: bool,
}

/// Who asked for the project picker.
#[derive(Clone, Copy, PartialEq)]
enum PickerFor {
    Chat,
    Design,
}

pub struct Workspace {
    store: Entity<Store>,
    remote: RemoteController,
    sidebar: Entity<Sidebar>,
    sidebar_open: bool,
    /// The new-chat box on screen.
    home: Entity<Home>,
    /// Drafts set aside while another new chat is started (shown in the sidebar).
    drafts: Vec<Entity<Home>>,
    next_draft: u64,
    designs: Entity<DesignsPage>,
    projects: Entity<ProjectsPage>,
    chats: HashMap<SessionId, Entity<ChatView>>,
    settings_view: Option<Entity<SettingsView>>,
    picker: Option<Entity<ProjectPicker>>,
    picker_for: PickerFor,
    usage_open: bool,
    route: Route,
    history: History,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
    _ticker: Task<()>,
}

impl Workspace {
    pub fn new(hub: Arc<Hub>, remote: RemoteController, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| Store::new(hub, cx));
        let sidebar = cx.new(|cx| Sidebar::new(store.clone(), window, cx));
        let home = cx.new(|cx| Home::new(0, store.clone(), window, cx));
        let designs = cx.new(|cx| DesignsPage::new(store.clone(), window, cx));
        let projects = cx.new(|cx| ProjectsPage::new(store.clone(), window, cx));
        let mut subs = vec![];
        subs.push(cx.subscribe_in(&sidebar, window, |this, _, ev: &SidebarEvent, window, cx| match ev {
            SidebarEvent::Open(id) => this.open_session(*id, window, cx),
            SidebarEvent::NewChat => this.new_chat(None, window, cx),
            SidebarEvent::OpenDesigns => this.open_designs(window, cx),
            SidebarEvent::OpenProjects => this.open_projects(window, cx),
            SidebarEvent::OpenProject(p) => this.open_project(p.clone(), window, cx),
            SidebarEvent::NewGeneral => this.new_general_chat(window, cx),
            SidebarEvent::NewChatIn(p) => this.new_chat(Some(p.clone()), window, cx),
            SidebarEvent::OpenDraft(id) => this.open_draft(*id, window, cx),
            SidebarEvent::DiscardDraft(id) => this.discard_draft(*id, window, cx),
        }));
        subs.push(cx.subscribe_in(&designs, window, |this, _, ev: &DesignsEvent, window, cx| match ev {
            DesignsEvent::OpenChat(id) => this.open_session(*id, window, cx),
            DesignsEvent::PickProject => this.open_picker(PickerFor::Design, window, cx),
        }));
        subs.push(cx.subscribe_in(&projects, window, |this, _, ev: &ProjectsEvent, window, cx| match ev {
            ProjectsEvent::NewChatIn(p) => this.new_chat(Some(p.clone()), window, cx),
        }));
        // Full screen in a project hides the sidebar; moving around in it is history.
        subs.push(cx.observe(&projects, |this, _, cx| {
            this.queue_history_sync(cx);
            cx.notify();
        }));
        subs.push(cx.observe(&designs, |this, _, cx| this.queue_history_sync(cx)));
        subs.push(Self::watch_home(&home, window, cx));
        subs.push(cx.subscribe_in(&store, window, |this, _, ev: &StoreEvent, window, cx| match ev {
            StoreEvent::Toast(level, text) => {
                let n = match level {
                    NoticeLevel::Info => Notification::info(text.clone()),
                    NoticeLevel::Warning => Notification::warning(text.clone()),
                    NoticeLevel::Error => Notification::error(text.clone()),
                };
                window.push_notification(n, cx);
            }
            StoreEvent::Removed(id) => {
                this.chats.remove(id);
                this.history.forget(&Place::Session(*id));
                if this.route == Route::Session(*id) {
                    this.navigate(Route::Home, window, cx);
                }
            }
            _ => {}
        }));
        // Refresh relative times and remote status once a second.
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        let focus = cx.focus_handle();
        let pref = store.read(cx).hub.settings().theme;
        theme::apply_preference(pref, window, cx);
        subs.push(cx.observe_window_appearance(window, |this, window, cx| {
            let pref = this.store.read(cx).hub.settings().theme;
            if pref == ThemePreference::System {
                theme::apply_preference(pref, window, cx);
            }
        }));
        let sidebar_open = store.read(cx).hub.settings().sidebar_open;
        let mut this = Self {
            store,
            remote,
            sidebar,
            sidebar_open,
            home,
            drafts: Vec::new(),
            next_draft: 1,
            designs,
            projects,
            chats: HashMap::new(),
            settings_view: None,
            picker: None,
            picker_for: PickerFor::Chat,
            usage_open: false,
            route: Route::Home,
            history: History { here: Some(Place::Home), ..Default::default() },
            focus,
            _subs: subs,
            _ticker: ticker,
        };
        let home = this.home.clone();
        window.defer(cx, move |window, cx| home.update(cx, |h, cx| h.focus(window, cx)));
        this.dev_hooks(window, cx);
        this
    }

    fn watch_home(home: &Entity<Home>, window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe_in(home, window, |this, home, ev: &HomeEvent, window, cx| {
            let active = *home == this.home;
            match ev {
                HomeEvent::Started(id) if active => {
                    this.open_session(*id, window, cx);
                }
                HomeEvent::PickProject if active => this.open_picker(PickerFor::Chat, window, cx),
                HomeEvent::Changed => this.refresh_drafts(cx),
                _ => {}
            }
        })
    }

    /// Tell the sidebar which drafts exist: the ones set aside, plus the box on screen if it has
    /// something in it.
    fn refresh_drafts(&mut self, cx: &mut Context<Self>) {
        let on_home = self.route == Route::Home;
        let mut rows = vec![];
        for (home, active) in self.drafts.iter().map(|h| (h, false)).chain(std::iter::once((&self.home, on_home))) {
            let h = home.read(cx);
            if h.has_draft(cx) {
                rows.push(DraftRow { id: h.id, preview: h.preview(cx), agent: h.agent(), project: h.project_name(), active: active || (home == &self.home && on_home) });
            }
        }
        self.sidebar.update(cx, |s, cx| s.set_drafts(rows, cx));
    }

    fn fresh_home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<Home> {
        let id = self.next_draft;
        self.next_draft += 1;
        let store = self.store.clone();
        let home = cx.new(|cx| Home::new(id, store, window, cx));
        let sub = Self::watch_home(&home, window, cx);
        self._subs.push(sub);
        home
    }

    /// Put the box on screen aside when it holds a draft; otherwise it can be reused.
    fn stash_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.home.read(cx).has_draft(cx) {
            let fresh = self.fresh_home(window, cx);
            let old = std::mem::replace(&mut self.home, fresh);
            self.drafts.push(old);
        }
    }

    fn open_draft(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.home.read(cx).id != id {
            let Some(i) = self.drafts.iter().position(|h| h.read(cx).id == id) else { return };
            let wanted = self.drafts.remove(i);
            self.stash_home(window, cx);
            self.home = wanted;
        }
        self.navigate(Route::Home, window, cx);
        self.home.update(cx, |h, cx| h.focus(window, cx));
    }

    fn discard_draft(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.home.read(cx).id == id {
            self.home = self.fresh_home(window, cx);
        } else {
            self.drafts.retain(|h| h.read(cx).id != id);
        }
        self.refresh_drafts(cx);
        cx.notify();
    }

    /// Developer hooks for screenshots / smoke tests:
    /// `MOTHR_OPEN=settings|picker|latest|changes|designs|design|projects|project:<path>[:<file>[:full]]` and
    /// `MOTHR_DEV_PROMPT="<agent_id>|<mode>|<model>|<chat|design>|<prompt>"` (project = cwd).
    fn dev_hooks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = std::env::var("MOTHR_OPEN").ok();
        let prompt = std::env::var("MOTHR_DEV_PROMPT").ok();
        if open.is_none() && prompt.is_none() {
            return;
        }
        let open_delay = std::env::var("MOTHR_OPEN_DELAY_MS").ok().and_then(|v| v.parse::<u64>().ok());
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(1500)).await;
            if let (Some(ms), Some(_)) = (open_delay, prompt.as_ref()) {
                // Start the dev prompt now, open the requested view later.
                let p = prompt.clone();
                let _ = this.update_in(cx, |this, window, cx| this.run_dev_prompt(p.as_deref(), window, cx));
                cx.background_executor().timer(Duration::from_millis(ms)).await;
                let _ = this.update_in(cx, |this, window, cx| this.run_dev_open(open.as_deref(), window, cx));
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.run_dev_prompt(prompt.as_deref(), window, cx);
                this.run_dev_open(open.as_deref(), window, cx);
            });
        })
        .detach();
    }

    /// Where we are now, in history terms.
    fn place(&self, cx: &App) -> Place {
        match self.route {
            Route::Home => Place::Home,
            Route::Session(id) => Place::Session(id),
            Route::Settings => Place::Settings,
            Route::Designs => Place::Designs(self.designs.read(cx).place(cx)),
            Route::Projects => Place::Projects(self.projects.read(cx).place(cx)),
        }
    }

    /// Record the place once the current update has settled, so a move that changes the
    /// route and then the page within it is one step, not two.
    fn queue_history_sync(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.history.sync_queued, true) {
            return;
        }
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            let _ = this.update(cx, |this, cx| {
                this.history.sync_queued = false;
                let place = this.place(cx);
                this.history.arrive(place);
            });
        });
    }

    /// The mouse's back/forward buttons (and Alt+←/→, Cmd+[/] on macOS).
    fn go_history(&mut self, dir: NavigationDirection, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.place(cx);
        let (from, to) = match dir {
            NavigationDirection::Back => (&mut self.history.back, &mut self.history.forward),
            NavigationDirection::Forward => (&mut self.history.forward, &mut self.history.back),
        };
        let Some(target) = from.pop() else { return };
        to.push(here);
        self.restore(target, window, cx);
        // Whatever we actually landed on is "here" now, so the step isn't recorded again.
        self.history.here = Some(self.place(cx));
    }

    fn restore(&mut self, place: Place, window: &mut Window, cx: &mut Context<Self>) {
        match place {
            Place::Home => {
                self.navigate(Route::Home, window, cx);
                self.home.update(cx, |h, cx| h.focus(window, cx));
            }
            Place::Session(id) => self.open_session(id, window, cx),
            Place::Settings => self.navigate(Route::Settings, window, cx),
            Place::Designs(p) => {
                self.navigate(Route::Designs, window, cx);
                self.designs.update(cx, |d, cx| d.restore(p, window, cx));
            }
            Place::Projects(p) => {
                self.navigate(Route::Projects, window, cx);
                self.projects.update(cx, |pr, cx| pr.restore(p, window, cx));
            }
        }
    }

    fn navigate(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        self.route = route;
        self.queue_history_sync(cx);
        self.picker = None;
        self.usage_open = false;
        if route == Route::Settings && self.settings_view.is_none() {
            let store = self.store.clone();
            let remote = self.remote.clone();
            self.settings_view = Some(cx.new(|cx| SettingsView::new(store, remote, window, cx)));
        }
        let active = match route {
            Route::Session(id) => Some(id),
            _ => None,
        };
        let designs = route == Route::Designs;
        let projects = route == Route::Projects;
        self.sidebar.update(cx, |s, cx| s.set_active(active, designs, projects, cx));
        self.refresh_drafts(cx);
        cx.notify();
    }

    /// The Design page, as it was left.
    fn open_designs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Designs, window, cx);
        self.designs.update(cx, |d, cx| d.refresh(cx));
    }

    /// The Projects page, as it was left.
    fn open_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Asking again while on it goes back to the list.
        if self.route == Route::Projects {
            self.projects.update(cx, |p, cx| p.show_list(window, cx));
        } else {
            self.navigate(Route::Projects, window, cx);
            self.projects.update(cx, |p, cx| p.focus(window, cx));
        }
    }

    /// One project's files.
    fn open_project(&mut self, project: ProjectRef, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Projects, window, cx);
        self.projects.update(cx, |p, cx| p.open_project(project, window, cx));
    }

    /// The Design gallery, ready to describe a new design.
    fn new_design(&mut self, project: Option<ProjectRef>, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Designs, window, cx);
        self.designs.update(cx, |d, cx| d.start_new(project, window, cx));
    }

    fn open_session(&mut self, id: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        // Design chats live next to their design, on the Design page.
        // (A session started a moment ago may not have reached the store yet.)
        let store = self.store.read(cx);
        let meta = store.session(id).map(|s| s.meta.clone()).or_else(|| store.hub.session(id).map(|s| s.meta));
        let design = meta.filter(|m| m.kind == SessionKind::Design).map(|m| m.project);
        if let Some(project) = design {
            self.navigate(Route::Designs, window, cx);
            self.designs.update(cx, |d, cx| d.open_project(project, Some(id), window, cx));
            self.store.update(cx, |s, cx| {
                s.dispatch(AppCommand::MarkRead { id }, window, cx);
            });
            return;
        }
        if !self.chats.contains_key(&id) {
            let store = self.store.clone();
            let view = cx.new(|cx| ChatView::new(id, store, window, cx));
            self._subs.push(cx.subscribe_in(&view, window, |this, _, ev: &ChatEvent, window, cx| match ev {
                ChatEvent::Open(id) => this.open_session(*id, window, cx),
            }));
            self.chats.insert(id, view);
        }
        self.navigate(Route::Session(id), window, cx);
        self.store.update(cx, |s, cx| {
            s.dispatch(AppCommand::MarkRead { id }, window, cx);
        });
        if let Some(chat) = self.chats.get(&id).cloned() {
            window.defer(cx, move |window, cx| chat.update(cx, |c, cx| c.focus(window, cx)));
        }
    }

    fn new_chat(&mut self, project: Option<ProjectRef>, window: &mut Window, cx: &mut Context<Self>) {
        // A new chat while writing one starts a second draft; an empty box is just reused.
        if self.route != Route::Home || self.home.read(cx).has_draft(cx) {
            self.stash_home(window, cx);
        }
        self.navigate(Route::Home, window, cx);
        self.home.update(cx, |h, cx| {
            if let Some(p) = project {
                h.set_project(p, cx);
            }
            h.focus(window, cx);
        });
    }

    fn run_dev_prompt(&mut self, prompt: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let this = self;
        if let Some(spec) = prompt {
            let parts: Vec<&str> = spec.splitn(5, '|').collect();
            if let [agent, mode, model, kind, text] = parts[..] {
                let cmd = AppCommand::NewSession {
                    agent: AgentKind::from_id(agent).unwrap_or(AgentKind::ClaudeCode),
                    kind: if kind == "design" { SessionKind::Design } else { SessionKind::Chat },
                    project: std::env::current_dir().unwrap_or_default(),
                    mode: PermissionMode::from_id(mode).unwrap_or_default(),
                    model: (!model.is_empty()).then(|| model.to_string()),
                    effort: None,
                    context: None,
                    prompt: Some(text.to_string()),
                };
                if let Some(id) = this.store.update(cx, |s, cx| s.dispatch(cmd, window, cx)) {
                    this.open_session(id, window, cx);
                }
            }
        }
    }

    fn run_dev_open(&mut self, open: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let this = self;
        if let Ok(text) = std::env::var("MOTHR_PREFILL") {
            let route = this.route;
            let chats = this.chats.clone();
            let text2 = text.clone();
            window.defer(cx, move |window, cx| {
                if let Route::Session(id) = route
                    && let Some(c) = chats.get(&id) {
                        c.update(cx, |c, cx| c.prefill(&text2, window, cx));
                    }
            });
        }
        match open {
            Some("settings") => this.navigate(Route::Settings, window, cx),
            Some("remote") => {
                this.navigate(Route::Settings, window, cx);
                if let Some(sv) = &this.settings_view {
                    sv.read(cx).scroll_to_bottom();
                }
            }
            Some("picker") => this.open_picker(PickerFor::Chat, window, cx),
            Some("usage") => {
                this.usage_open = true;
                cx.notify();
            }
            Some("designs") => this.open_designs(window, cx),
            Some("projects") => this.open_projects(window, cx),
            Some(p) if p.starts_with("project:") => {
                // MOTHR_OPEN=project:<path>[:<file>[:full]] opens a project (and a file) for screenshots.
                let mut parts = p["project:".len()..].splitn(3, ':');
                let path = PathBuf::from(parts.next().unwrap_or_default());
                let file = parts.next().map(str::to_string).filter(|f| !f.is_empty());
                let full = parts.next() == Some("full");
                let project = this.store.read(cx).hub.projects().get(&path).unwrap_or_else(|| ProjectRef::from_path(path));
                this.open_project(project, window, cx);
                this.projects.update(cx, |pp, cx| pp.dev_open(file, full, window, cx));
            }
            Some("changes") => {
                let id = this.store.read(cx).sessions.iter().rev().find(|s| s.meta.parent.is_none() && s.meta.kind == SessionKind::Chat).map(|s| s.meta.id);
                if let Some(id) = id {
                    this.open_session(id, window, cx);
                    if let Some(chat) = this.chats.get(&id) {
                        chat.update(cx, |c, cx| c.set_changes_open(true, cx));
                    }
                }
            }
            Some("design") => {
                this.navigate(Route::Designs, window, cx);
                this.designs.update(cx, |d, cx| d.open_latest_when_ready(cx));
            }
            Some("subagent") => {
                let id = this.store.read(cx).sessions.iter().rev().find(|s| s.meta.parent.is_some()).map(|s| s.meta.id);
                if let Some(id) = id {
                    this.open_session(id, window, cx);
                }
            }
            Some("latest") => {
                let id = this.store.read(cx).sessions.iter().rev().find(|s| s.meta.parent.is_none()).map(|s| s.meta.id);
                if let Some(id) = id {
                    this.open_session(id, window, cx);
                }
            }
            _ => {}
        }
    }

    fn new_general_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route != Route::Home || self.home.read(cx).has_draft(cx) {
            self.stash_home(window, cx);
        }
        self.navigate(Route::Home, window, cx);
        self.home.update(cx, |h, cx| {
            h.set_general(cx);
            h.focus(window, cx);
        });
    }

    fn open_picker(&mut self, purpose: PickerFor, window: &mut Window, cx: &mut Context<Self>) {
        self.picker_for = purpose;
        let picker = cx.new(|cx| ProjectPicker::new(self.store.clone(), window, cx));
        self._subs.push(cx.subscribe_in(&picker, window, |this, _, ev: &PickerEvent, window, cx| {
            match ev {
                PickerEvent::Picked(p) => {
                    let p = p.clone();
                    this.picker = None;
                    match this.picker_for {
                        PickerFor::Chat if this.route == Route::Home => {
                            // Chosen from the new-chat box itself: this draft gets the project.
                            this.home.update(cx, |h, cx| {
                                h.set_project(p, cx);
                                h.focus(window, cx);
                            });
                        }
                        PickerFor::Chat => this.new_chat(Some(p), window, cx),
                        PickerFor::Design => this.new_design(Some(p), window, cx),
                    }
                }
                PickerEvent::General => {
                    this.picker = None;
                    this.new_general_chat(window, cx);
                }
                PickerEvent::Dismiss => {
                    this.picker = None;
                    cx.notify();
                }
            }
        }));
        picker.update(cx, |p, cx| p.focus(window, cx));
        self.picker = Some(picker);
        cx.notify();
    }

    /// Show or hide the sidebar and remember the choice for next launch.
    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        let hub = self.store.read(cx).hub.clone();
        let mut s = hub.settings();
        s.sidebar_open = self.sidebar_open;
        hub.update_settings(s);
        cx.notify();
    }

    fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dark = tokens::p(cx).dark;
        let pref = if dark { ThemePreference::Light } else { ThemePreference::Dark };
        let hub = self.store.read(cx).hub.clone();
        let mut s = hub.settings();
        s.theme = pref;
        hub.update_settings(s);
        theme::apply_preference(pref, window, cx);
    }

    fn render_title_bar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let remote = self.remote.status();
        let current = match self.route {
            Route::Session(id) => Some(id),
            _ => None,
        };
        let pills = usage::limits_button(self.store.clone(), current, self.usage_open, cx.listener(|this, _: &ClickEvent, _, cx| {
            this.usage_open = !this.usage_open;
            cx.notify();
        }), cx);
        TitleBar::new()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("toggle-sidebar")
                            .icon(icon("panel-left"))
                            .ghost()
                            .tooltip("Toggle sidebar  (Ctrl+B)")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                    )
                    .child(
                        h_flex()
                            .gap(sz(8.))
                            .items_center()
                            .child(logo_mark(18., &pal))
                            .child(div().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::SEMIBOLD).child("Mothr")),
                    )
            )
            .child(
                h_flex()
                    .gap(sz(6.))
                    .px_2()
                    .items_center()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(pills)
                    .when(remote.running, |el| {
                        el.child(
                            h_flex()
                                .id("remote-indicator")
                                .gap(sz(6.))
                                .px(sz(8.))
                                .h(sz(24.))
                                .items_center()
                                .rounded_full()
                                .bg(pal.success.opacity(0.10))
                                .text_size(sz(tokens::text::XS))
                                .text_color(pal.success)
                                .cursor_pointer()
                                .hover(|d| d.bg(pal.success.opacity(0.18)))
                                .on_click(cx.listener(|this, _, window, cx| this.navigate(Route::Settings, window, cx)))
                                .child(div().size(sz(6.)).rounded_full().bg(pal.success))
                                .when(remote.connected_clients > 0, |d| d.child(remote.connected_clients.to_string()))
                                .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Mothr Remote is on").build(window, cx)),
                        )
                    })
                    .child(div().w(sz(1.)).h(sz(16.)).mx_1().bg(pal.border))
                    .child(
                        Button::new("toggle-theme")
                            .icon(icon(if pal.dark { "sun" } else { "moon" }))
                            .ghost()
                            .tooltip(if pal.dark { "Light mode" } else { "Dark mode" })
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_theme(window, cx))),
                    )
                    .child(
                        Button::new("open-settings")
                            .icon(icon("settings"))
                            .ghost()
                            .tooltip("Settings  (Ctrl+,)")
                            .on_click(cx.listener(|this, _, window, cx| this.navigate(Route::Settings, window, cx))),
                    ),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // gpui-kit components size in rems; keep them in step with the interface scale.
        let rem = sz(16.);
        if window.rem_size() != rem {
            window.set_rem_size(rem);
        }
        let main: AnyElement = match self.route {
            Route::Home => self.home.clone().into_any_element(),
            Route::Session(id) => match self.chats.get(&id) {
                Some(c) => c.clone().into_any_element(),
                None => self.home.clone().into_any_element(),
            },
            Route::Designs => self.designs.clone().into_any_element(),
            Route::Projects => self.projects.clone().into_any_element(),
            Route::Settings => match &self.settings_view {
                Some(s) => s.clone().into_any_element(),
                None => div().into_any_element(),
            },
        };
        let theme = cx.theme().clone();
        let route_key: SharedString = match self.route {
            Route::Home => "home".into(),
            Route::Session(id) => id.to_string().into(),
            Route::Designs => "designs".into(),
            Route::Projects => "projects".into(),
            Route::Settings => "settings".into(),
        };
        let title_bar = self.render_title_bar(window, cx);
        let project_fullscreen = self.route == Route::Projects && self.projects.read(cx).fullscreen(cx);

        v_flex()
            .id("workspace")
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .on_action(cx.listener(|this, _: &NewChat, window, cx| this.new_chat(None, window, cx)))
            .on_action(cx.listener(|this, _: &NewDesign, window, cx| this.new_design(None, window, cx)))
            .on_action(cx.listener(|this, _: &crate::NewGeneralChat, window, cx| this.new_general_chat(window, cx)))
            .on_action(cx.listener(|this, _: &OpenProjectPalette, window, cx| this.open_picker(PickerFor::Chat, window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.navigate(Route::Settings, window, cx)))
            .on_action(cx.listener(|this, _: &OpenProjects, window, cx| this.open_projects(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| this.go_history(NavigationDirection::Back, window, cx)))
            .on_action(cx.listener(|this, _: &GoForward, window, cx| this.go_history(NavigationDirection::Forward, window, cx)))
            // Capture phase, so chats, viewers and inputs can't swallow the side buttons.
            .capture_any_mouse_down(cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                if let MouseButton::Navigate(dir) = ev.button {
                    cx.stop_propagation();
                    this.go_history(dir, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FocusComposer, window, cx| match this.route {
                Route::Session(id) => {
                    if let Some(c) = this.chats.get(&id).cloned() {
                        c.update(cx, |c, cx| c.focus(window, cx));
                    }
                }
                Route::Designs => this.designs.update(cx, |d, cx| d.focus(window, cx)),
                Route::Projects => this.projects.update(cx, |p, cx| p.focus(window, cx)),
                _ => this.home.update(cx, |h, cx| h.focus(window, cx)),
            }))
            .child(title_bar)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .when(self.sidebar_open && !project_fullscreen, |el| {
                        el.child(
                            div()
                                .h_full()
                                .w(sz(286.))
                                .flex_none()
                                .border_r_1()
                                .border_color(theme.sidebar_border)
                                .bg(theme.sidebar)
                                .child(self.sidebar.clone())
                                .with_animation(
                                    "sidebar-in",
                                    Animation::new(Duration::from_millis(180)).with_easing(ease_out_quint()),
                                    |d, t| d.opacity(t),
                                ),
                        )
                    })
                    .child(
                        div()
                            .id(ElementId::Name(route_key.clone()))
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(main)
                            .with_animation(
                                ElementId::Name(format!("route-{route_key}").into()),
                                Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint()),
                                |d, t| d.opacity(0.4 + 0.6 * t),
                            ),
                    )
                    .when(self.usage_open, |el| {
                        let store = self.store.read(cx);
                        let current = match self.route {
                            Route::Session(id) => store.session(id).map(|s| s.meta.clone()),
                            _ => None,
                        };
                        el.child(
                            div()
                                .id("usage-backdrop")
                                .absolute()
                                .inset_0()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.usage_open = false;
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(sz(6.))
                                .right(sz(10.))
                                .occlude()
                                .child(usage::panel(
                                    store,
                                    current.as_ref(),
                                    cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.store.update(cx, |s, cx| {
                                            s.dispatch(AppCommand::RefreshUsage, window, cx);
                                        });
                                    }),
                                    cx,
                                )),
                        )
                    })
                    .when_some(self.picker.clone(), |el, picker| {
                        el.child(
                            div()
                                .id("picker-backdrop")
                                .absolute()
                                .inset_0()
                                .bg(gpui_kit::black().opacity(0.35))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.picker = None;
                                    cx.notify();
                                }))
                                .with_animation(
                                    "picker-fade",
                                    Animation::new(Duration::from_millis(140)),
                                    |d, t| d.opacity(t),
                                ),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(sz(64.))
                                .left_0()
                                .right_0()
                                .flex()
                                .justify_center()
                                .child(div().occlude().child(picker)),
                        )
                    }),
            )
    }
}

impl History {
    /// Note the place we're at; a new place pushes the old one and drops the forward steps.
    fn arrive(&mut self, place: Place) {
        match self.here.replace(place.clone()) {
            Some(prev) if prev != place => {
                self.back.push(prev);
                self.forward.clear();
            }
            _ => {}
        }
    }

    /// Drop a place that no longer exists (a deleted chat).
    fn forget(&mut self, place: &Place) {
        for stack in [&mut self.back, &mut self.forward] {
            stack.retain(|p| p != place);
            stack.dedup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{History, Place};

    #[test]
    fn history_records_new_places_and_drops_forward_steps() {
        let mut h = History { here: Some(Place::Home), ..Default::default() };
        h.arrive(Place::Home);
        assert!(h.back.is_empty(), "staying put isn't a step");
        h.arrive(Place::Settings);
        h.arrive(Place::Projects(None));
        assert_eq!(h.back, vec![Place::Home, Place::Settings]);
        h.forward.push(Place::Designs(None));
        h.arrive(Place::Home);
        assert!(h.forward.is_empty(), "a new place drops the forward steps");
        assert_eq!(h.back, vec![Place::Home, Place::Settings, Place::Projects(None)]);
    }

    #[test]
    fn forgetting_a_place_collapses_repeats() {
        let mut h = History { back: vec![Place::Home, Place::Settings, Place::Home], ..Default::default() };
        h.forget(&Place::Settings);
        assert_eq!(h.back, vec![Place::Home]);
    }
}
