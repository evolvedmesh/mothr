//! The window's frame, which lives in the sidebar: its header (the Mothr mark, finding a project,
//! back/forward, collapsing it) doubles as the window's title bar, and its footer holds what
//! runs and what's left (live activity, account limits, Mothr Remote) next to light/dark and
//! Settings. Collapsed, the sidebar becomes a rail of the same controls.

use super::*;
use gpui_kit::component::{Disableable as _, Selectable as _, Sizable as _};

/// The open sidebar's width, and the collapsed rail's.
pub(super) const SIDEBAR_W: f32 = 286.;
pub(super) const RAIL_W: f32 = 52.;

/// Keyboard hint for the project finder.
const FINDER_KEYS: &str = if cfg!(target_os = "macos") { "⌘P" } else { "Ctrl+P" };

/// Chats working, or waiting on you, right now.
struct Activity {
    needs: Vec<(SessionId, String)>,
    working: Vec<(SessionId, String)>,
}

impl Activity {
    fn is_empty(&self) -> bool {
        self.needs.is_empty() && self.working.is_empty()
    }

    /// The chat to open: the first one waiting on you, else the first one working.
    fn first(&self) -> Option<SessionId> {
        self.needs.first().or(self.working.first()).map(|(id, _)| *id)
    }

    fn label(&self) -> String {
        let mut parts = vec![];
        if !self.needs.is_empty() {
            parts.push(format!("{} needs you", self.needs.len()));
        }
        if !self.working.is_empty() {
            parts.push(format!("{} working", self.working.len()));
        }
        parts.join(" · ")
    }

    fn tooltip(&self) -> String {
        self.needs
            .iter()
            .map(|(_, t)| format!("Needs you · {t}"))
            .chain(self.working.iter().map(|(_, t)| format!("Working · {t}")))
            .take(6)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Workspace {
    fn activity(&self, cx: &App) -> Activity {
        let store = self.store.read(cx);
        let top = || store.sessions.iter().filter(|s| s.meta.parent.is_none());
        Activity {
            needs: top().filter(|s| s.meta.status == SessionStatus::NeedsInput).map(|s| (s.meta.id, s.meta.title.clone())).collect(),
            working: top()
                .filter(|s| matches!(s.meta.status, SessionStatus::Working | SessionStatus::Starting))
                .map(|s| (s.meta.id, s.meta.title.clone()))
                .collect(),
        }
    }

    /// The open sidebar: header, the chat list, live activity and the footer.
    pub(super) fn render_sidebar_panel(&mut self, right: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let activity = self.activity(cx);
        v_flex()
            .h_full()
            .w(sz(SIDEBAR_W))
            .flex_none()
            .map(|d| if right { d.border_l_1() } else { d.border_r_1() })
            .border_color(theme.sidebar_border)
            .bg(theme.sidebar)
            .child(self.render_sidebar_header(right, cx))
            .child(div().flex_1().min_h_0().child(self.sidebar.clone()))
            .when(!activity.is_empty(), |d| d.child(self.render_activity_row(&activity, cx)))
            .child(self.render_sidebar_footer(right, cx))
            .with_animation(
                "sidebar-in",
                Animation::new(Duration::from_millis(180)).with_easing(ease_out_quint()),
                |d, t| d.opacity(t),
            )
            .into_any_element()
    }

    /// The header is the window's title bar: drag it to move the window, double-click it to
    /// maximize; the window's own buttons sit here when Mothr draws them.
    fn render_sidebar_header(&mut self, right: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let (back, fwd) = (!self.history.back.is_empty(), !self.history.forward.is_empty());
        TitleBar::new()
            .bg(pal.chrome)
            .border_b_0()
            .pr(sz(6.))
            .child(
                h_flex()
                    .gap(sz(8.))
                    .pl(sz(4.))
                    .items_center()
                    .child(logo_mark(18., &pal))
                    .child(div().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::SEMIBOLD).child("Mothr")),
            )
            .child(
                h_flex()
                    .gap(sz(1.))
                    .items_center()
                    // Clicks on the controls must not start a window move.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new("find-project")
                            .ghost()
                            .small()
                            .icon(icon("search"))
                            .tooltip(format!("Find a repo  ({FINDER_KEYS})"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_picker(PickerFor::Chat, window, cx))),
                    )
                    .child(
                        Button::new("go-back")
                            .ghost()
                            .small()
                            .icon(icon("chevron-left"))
                            .disabled(!back)
                            .tooltip("Back")
                            .on_click(cx.listener(|this, _, window, cx| this.go_history(NavigationDirection::Back, window, cx))),
                    )
                    .child(
                        Button::new("go-forward")
                            .ghost()
                            .small()
                            .icon(icon("chevron-right"))
                            .disabled(!fwd)
                            .tooltip("Forward")
                            .on_click(cx.listener(|this, _, window, cx| this.go_history(NavigationDirection::Forward, window, cx))),
                    )
                    .child(
                        Button::new("toggle-sidebar")
                            .ghost()
                            .small()
                            .icon(icon(if right { "panel-right" } else { "panel-left" }))
                            .tooltip("Collapse sidebar  (Ctrl+B)")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                    ),
            )
    }

    /// Chats working or waiting on you, above the footer while there are any. Opens the first.
    fn render_activity_row(&self, activity: &Activity, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let color = if activity.needs.is_empty() { pal.brand } else { pal.warning };
        let tip = activity.tooltip();
        let target = activity.first();
        div().px_3().pb_2().child(
            h_flex()
                .id("sidebar-activity")
                .w_full()
                .h(sz(30.))
                .px(sz(10.))
                .gap(sz(8.))
                .items_center()
                .rounded(sz(tokens::radius::MD))
                .bg(color.opacity(0.10))
                .text_size(sz(tokens::text::SM))
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .cursor_pointer()
                .hover(|d| d.bg(color.opacity(0.17)))
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
                .when_some(target, |d, id| d.on_click(cx.listener(move |this, _, window, cx| this.open_session(id, window, cx))))
                .child(pulse_dot("sidebar-activity-dot", color))
                .child(div().flex_1().min_w_0().truncate().child(activity.label()))
                .child(icon("chevron-right").size(sz(13.)))
                .with_animation(
                    "sidebar-activity-in",
                    Animation::new(Duration::from_millis(240)).with_easing(ease_out_quint()),
                    |d, t| d.opacity(t).mt(sz((1.0 - t) * 4.0)),
                ),
        )
    }

    /// Limits, Mothr Remote, light/dark and Settings, along the sidebar's bottom edge.
    fn render_sidebar_footer(&mut self, right: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let usage = self.usage_button(right, cx);
        h_flex()
            .flex_none()
            .h(sz(46.))
            .px_3()
            .gap(sz(6.))
            .items_center()
            .border_t_1()
            .border_color(pal.border)
            .child(usage)
            .child(div().flex_1())
            .children(self.remote_indicator(false, cx))
            .child(
                h_flex()
                    .flex_none()
                    .h(sz(28.))
                    .px(sz(2.))
                    .gap(sz(1.))
                    .items_center()
                    .rounded(sz(tokens::radius::MD + 2.))
                    .border_1()
                    .border_color(pal.border)
                    .bg(pal.surface_hover.opacity(0.35))
                    .child(self.theme_button(cx))
                    .child(self.settings_button(cx)),
            )
    }

    /// The collapsed sidebar: the same controls as icons, top and bottom.
    pub(super) fn render_rail(&mut self, right: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let pal = tokens::p(cx);
        let activity = self.activity(cx);
        let dots = super::super::workspaces::dots(self.store.read(cx));
        let usage_open = self.usage_open;
        // macOS draws its window buttons over the top-left corner.
        let top_gap = if cfg!(target_os = "macos") && !right { 30. } else { 8. };
        v_flex()
            .id("sidebar-rail")
            .h_full()
            .w(sz(RAIL_W))
            .flex_none()
            .pt(sz(top_gap))
            .pb(sz(8.))
            .gap(sz(4.))
            .items_center()
            .map(|d| if right { d.border_l_1() } else { d.border_r_1() })
            .border_color(theme.sidebar_border)
            .bg(theme.sidebar)
            .child(
                Button::new("rail-expand")
                    .ghost()
                    .icon(icon(if right { "panel-right" } else { "panel-left" }))
                    .tooltip("Show sidebar  (Ctrl+B)")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(
                Button::new("rail-new-chat")
                    .ghost()
                    .icon(icon("square-pen"))
                    .tooltip("New chat  (Ctrl+N)")
                    .on_click(cx.listener(|this, _, window, cx| this.new_chat(None, window, cx))),
            )
            .child(
                Button::new("rail-find")
                    .ghost()
                    .icon(icon("search"))
                    .tooltip(format!("Find a repo  ({FINDER_KEYS})"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_picker(PickerFor::Chat, window, cx))),
            )
            // The empty middle moves the window, as the header would.
            .child(
                div()
                    .id("rail-drag")
                    .flex_1()
                    .w_full()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move()),
            )
            .when(dots.len() > 1, |d| {
                // The workspaces, top to bottom; the open one is a longer pill.
                let mut col = v_flex().pb(sz(6.)).gap(sz(1.)).items_center();
                for dot in &dots {
                    let id = dot.id;
                    let tip = dot.tooltip();
                    col = col.child(
                        div()
                            .id(ElementId::Name(format!("rail-ws-{id}").into()))
                            .w(sz(22.))
                            .min_h(sz(14.))
                            .py(sz(3.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(sz(tokens::radius::SM))
                            .cursor_pointer()
                            .hover(|x| x.bg(pal.surface_hover))
                            .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
                            .on_click(cx.listener(move |this, _, window, cx| this.switch_workspace(id, window, cx)))
                            .child(super::super::workspaces::dot_mark(dot, &pal, true)),
                    );
                }
                d.child(col)
            })
            .when(!activity.is_empty(), |d| {
                let color = if activity.needs.is_empty() { pal.brand } else { pal.warning };
                let tip = format!("{}\n{}", activity.label(), activity.tooltip());
                let count = activity.needs.len() + activity.working.len();
                let target = activity.first();
                d.child(
                    h_flex()
                        .id("rail-activity")
                        .size(sz(34.))
                        .gap(sz(3.))
                        .justify_center()
                        .items_center()
                        .rounded(sz(tokens::radius::MD))
                        .bg(color.opacity(0.10))
                        .text_size(sz(tokens::text::XS))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color)
                        .cursor_pointer()
                        .hover(|d| d.bg(color.opacity(0.18)))
                        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
                        .when_some(target, |d, id| d.on_click(cx.listener(move |this, _, window, cx| this.open_session(id, window, cx))))
                        .child(pulse_dot("rail-activity-dot", color))
                        .child(count.to_string()),
                )
            })
            .child(
                Button::new("rail-usage")
                    .ghost()
                    .icon(icon("gauge"))
                    .selected(usage_open)
                    .tooltip("Usage & limits")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.usage_open = !this.usage_open;
                        cx.notify();
                    })),
            )
            .children(self.remote_indicator(true, cx))
            .child(self.theme_button(cx))
            .child(self.settings_button(cx))
            .into_any_element()
    }

    /// Account limits: one ring per account; hover for the card, click to pin it.
    fn usage_button(&mut self, right: bool, cx: &mut Context<Self>) -> AnyElement {
        let current = match self.route {
            Route::Session(id) => Some(id),
            _ => None,
        };
        // The card opens upwards from the footer, over the chat rather than off the window.
        let anchor = if right { Anchor::BottomRight } else { Anchor::BottomLeft };
        usage::limits_button(
            self.store.clone(),
            current,
            self.usage_open,
            anchor,
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.usage_open = !this.usage_open;
                cx.notify();
            }),
            cx,
        )
    }

    /// Shown while Mothr Remote is on; opens its settings.
    fn remote_indicator(&self, compact: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let remote = self.remote.status();
        if !remote.running {
            return None;
        }
        let pal = tokens::p(cx);
        let label = if remote.connected_clients > 0 { remote.connected_clients.to_string() } else { "Remote".into() };
        Some(
            h_flex()
                .id(if compact { "rail-remote" } else { "remote-indicator" })
                .gap(sz(5.))
                .px(sz(8.))
                .h(sz(26.))
                .justify_center()
                .items_center()
                .rounded_full()
                .bg(pal.success.opacity(0.10))
                .text_size(sz(tokens::text::XS))
                .text_color(pal.success)
                .cursor_pointer()
                .hover(|d| d.bg(pal.success.opacity(0.18)))
                .on_click(cx.listener(|this, _, window, cx| this.navigate(Route::Settings, window, cx)))
                .child(icon("radio-tower").size(sz(12.)))
                .when(!compact || remote.connected_clients > 0, |d| d.child(label))
                .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Mothr Remote is on").build(window, cx))
                .into_any_element(),
        )
    }

    fn theme_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let dark = tokens::p(cx).dark;
        Button::new("toggle-theme")
            .ghost()
            .small()
            .icon(icon(if dark { "sun" } else { "moon" }))
            .tooltip(if dark { "Light mode" } else { "Dark mode" })
            .on_click(cx.listener(|this, _, window, cx| this.toggle_theme(window, cx)))
    }

    fn settings_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        Button::new("open-settings")
            .ghost()
            .small()
            .icon(icon("settings"))
            .tooltip("Settings  (Ctrl+,)")
            .on_click(cx.listener(|this, _, window, cx| this.navigate(Route::Settings, window, cx)))
    }
}
