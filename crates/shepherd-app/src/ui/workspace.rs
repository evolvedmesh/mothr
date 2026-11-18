//! The root view: title bar, sidebar, routed main area and overlays.

use super::chat::{ChatEvent, ChatView};
use super::home::{Home, HomeEvent};
use super::kit::*;
use super::picker::{PickerEvent, ProjectPicker};
use super::settings::SettingsView;
use super::sidebar::{Sidebar, SidebarEvent};
use super::usage;
use super::{theme, tokens};
use crate::remote::RemoteController;
use crate::store::{Store, StoreEvent};
use crate::{FocusComposer, NewChat, NewDesign, OpenProjectPalette, OpenSettings, ToggleSidebar};
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
use shepherd_core::*;
use shepherd_hub::Hub;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
pub enum Route {
    Home,
    Session(SessionId),
    Settings,
}

pub struct Workspace {
    store: Entity<Store>,
    remote: RemoteController,
    sidebar: Entity<Sidebar>,
    sidebar_open: bool,
    home: Entity<Home>,
    chats: HashMap<SessionId, Entity<ChatView>>,
    settings_view: Option<Entity<SettingsView>>,
    picker: Option<Entity<ProjectPicker>>,
    usage_open: bool,
    route: Route,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
    _ticker: Task<()>,
}

impl Workspace {
    pub fn new(hub: Arc<Hub>, remote: RemoteController, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = cx.new(|cx| Store::new(hub, cx));
        let sidebar = cx.new(|cx| Sidebar::new(store.clone(), window, cx));
        let home = cx.new(|cx| Home::new(store.clone(), window, cx));
        let mut subs = vec![];
        subs.push(cx.subscribe_in(&sidebar, window, |this, _, ev: &SidebarEvent, window, cx| match ev {
            SidebarEvent::Open(id) => this.open_session(*id, window, cx),
            SidebarEvent::NewChat => this.new_chat(SessionKind::Chat, None, window, cx),
            SidebarEvent::NewDesign => this.new_chat(SessionKind::Design, None, window, cx),
            SidebarEvent::NewGeneral => this.new_general_chat(window, cx),
            SidebarEvent::NewChatIn(p) => this.new_chat(SessionKind::Chat, Some(p.clone()), window, cx),
        }));
        subs.push(cx.subscribe_in(&home, window, |this, _, ev: &HomeEvent, window, cx| match ev {
            HomeEvent::Started(id) => this.open_session(*id, window, cx),
            HomeEvent::PickProject => this.open_picker(window, cx),
        }));
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
        let mut this = Self {
            store,
            remote,
            sidebar,
            sidebar_open: true,
            home,
            chats: HashMap::new(),
            settings_view: None,
            picker: None,
            usage_open: false,
            route: Route::Home,
            focus,
            _subs: subs,
            _ticker: ticker,
        };
        let home = this.home.clone();
        window.defer(cx, move |window, cx| home.update(cx, |h, cx| h.focus(window, cx)));
        this.dev_hooks(window, cx);
        this
    }

    /// Developer hooks for screenshots / smoke tests:
    /// `SHEPHERD_OPEN=settings|picker|latest|design` and
    /// `SHEPHERD_DEV_PROMPT="<agent_id>|<mode>|<model>|<chat|design>|<prompt>"` (project = cwd).
    fn dev_hooks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = std::env::var("SHEPHERD_OPEN").ok();
        let prompt = std::env::var("SHEPHERD_DEV_PROMPT").ok();
        if open.is_none() && prompt.is_none() {
            return;
        }
        let open_delay = std::env::var("SHEPHERD_OPEN_DELAY_MS").ok().and_then(|v| v.parse::<u64>().ok());
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

    fn navigate(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        self.route = route;
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
        self.sidebar.update(cx, |s, cx| s.set_active(active, cx));
        cx.notify();
    }

    fn open_session(&mut self, id: SessionId, window: &mut Window, cx: &mut Context<Self>) {
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

    fn new_chat(&mut self, kind: SessionKind, project: Option<ProjectRef>, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Home, window, cx);
        self.home.update(cx, |h, cx| {
            h.set_kind(kind, cx);
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
        if let Ok(text) = std::env::var("SHEPHERD_PREFILL") {
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
            Some("picker") => this.open_picker(window, cx),
            Some("usage") => {
                this.usage_open = true;
                cx.notify();
            }
            Some("subagent") => {
                let id = this.store.read(cx).sessions.iter().rev().find(|s| s.meta.parent.is_some()).map(|s| s.meta.id);
                if let Some(id) = id {
                    this.open_session(id, window, cx);
                }
            }
            Some(kind @ ("latest" | "design")) => {
                let want_design = kind == "design";
                let id = this
                    .store
                    .read(cx)
                    .sessions
                    .iter()
                    .rev()
                    .find(|s| s.meta.parent.is_none() && (!want_design || s.meta.kind == SessionKind::Design))
                    .map(|s| s.meta.id);
                if let Some(id) = id {
                    this.open_session(id, window, cx);
                }
            }
            _ => {}
        }
    }

    fn new_general_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(Route::Home, window, cx);
        self.home.update(cx, |h, cx| {
            h.set_general(cx);
            h.focus(window, cx);
        });
    }

    fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picker = cx.new(|cx| ProjectPicker::new(self.store.clone(), window, cx));
        self._subs.push(cx.subscribe_in(&picker, window, |this, _, ev: &PickerEvent, window, cx| {
            match ev {
                PickerEvent::Picked(p) => {
                    let p = p.clone();
                    this.picker = None;
                    this.new_chat(this.home.read(cx).kind(), Some(p), window, cx);
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
        let store = self.store.read(cx);
        let remote = self.remote.status();
        let pills = usage::title_pills(store, cx.listener(|this, _: &ClickEvent, _, cx| {
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
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_open = !this.sidebar_open;
                                cx.notify();
                            })),
                    )
                    .child(
                        h_flex()
                            .gap(sz(8.))
                            .items_center()
                            .child(logo_mark(18., &pal))
                            .child(div().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::SEMIBOLD).child("Shepherd")),
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
                                .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Shepherd Remote is on").build(window, cx)),
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
            Route::Settings => match &self.settings_view {
                Some(s) => s.clone().into_any_element(),
                None => div().into_any_element(),
            },
        };
        let theme = cx.theme().clone();
        let route_key: SharedString = match self.route {
            Route::Home => "home".into(),
            Route::Session(id) => id.to_string().into(),
            Route::Settings => "settings".into(),
        };
        let title_bar = self.render_title_bar(window, cx);

        v_flex()
            .id("workspace")
            .track_focus(&self.focus)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .on_action(cx.listener(|this, _: &NewChat, window, cx| this.new_chat(SessionKind::Chat, None, window, cx)))
            .on_action(cx.listener(|this, _: &NewDesign, window, cx| this.new_chat(SessionKind::Design, None, window, cx)))
            .on_action(cx.listener(|this, _: &crate::NewGeneralChat, window, cx| this.new_general_chat(window, cx)))
            .on_action(cx.listener(|this, _: &OpenProjectPalette, window, cx| this.open_picker(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.navigate(Route::Settings, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar_open = !this.sidebar_open;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusComposer, window, cx| match this.route {
                Route::Session(id) => {
                    if let Some(c) = this.chats.get(&id).cloned() {
                        c.update(cx, |c, cx| c.focus(window, cx));
                    }
                }
                _ => this.home.update(cx, |h, cx| h.focus(window, cx)),
            }))
            .child(title_bar)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .when(self.sidebar_open, |el| {
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
