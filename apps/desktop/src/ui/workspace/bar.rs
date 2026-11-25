//! The bar along the window's edge: where you are, what's running, your limits and the app's
//! controls. Its items, their order and its edge (top or bottom) are yours to choose in
//! Settings → Appearance, or by right-clicking the bar.

use super::*;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};

/// Keyboard hint for the project palette.
const PALETTE_KEYS: &str = if cfg!(target_os = "macos") { "⌘P" } else { "Ctrl P" };

impl Workspace {
    pub(super) fn render_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let a = tokens::appearance();
        let bottom = a.bar_position == BarPosition::Bottom;
        let mut start = h_flex().flex_1().min_w_0().gap(sz(6.)).items_center();
        let mut center = h_flex().flex_shrink(1.).min_w_0().items_center();
        let mut end = h_flex()
            .flex_1()
            .min_w_0()
            .justify_end()
            .gap(sz(4.))
            .px_2()
            .items_center()
            // Clicks on the controls must not start a window move.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        // Icon buttons next to each other share one island, so the controls read as a group.
        let pal = tokens::p(cx);
        let island = |buttons: Vec<AnyElement>| {
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
                .children(buttons)
                .into_any_element()
        };
        let mut pending: Vec<AnyElement> = vec![];
        for &item in &a.bar_items {
            let Some(el) = self.bar_item(item, bottom, cx) else { continue };
            match item.zone() {
                BarZone::Start => start = start.child(el),
                BarZone::Center => center = center.child(el),
                BarZone::End if is_icon_button(item) => pending.push(el),
                BarZone::End => {
                    if !pending.is_empty() {
                        end = end.child(island(std::mem::take(&mut pending)));
                    }
                    end = end.child(el);
                }
            }
        }
        if !pending.is_empty() {
            end = end.child(island(pending));
        }
        let weak = cx.entity().downgrade();
        let bar = TitleBar::new()
            .when(bottom, |t| t.border_b_0().border_t_1())
            .child(start)
            .child(center)
            .child(end);
        div()
            .id("bar")
            .flex_none()
            .child(bar)
            .context_menu(move |menu, _, _| bar_menu(menu, &tokens::appearance(), weak.clone()))
            .into_any_element()
    }

    /// One item of the bar, or nothing when it has nothing to show right now.
    fn bar_item(&mut self, item: BarItem, bottom: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pal = tokens::p(cx);
        let a = tokens::appearance();
        Some(match item {
            BarItem::SidebarToggle => Button::new("toggle-sidebar")
                .icon(icon(if a.sidebar_side == Side::Right { "panel-right" } else { "panel-left" }))
                .ghost()
                .tooltip("Toggle sidebar  (Ctrl+B)")
                .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)))
                .into_any_element(),
            BarItem::Brand => h_flex()
                .gap(sz(8.))
                .px(sz(4.))
                .items_center()
                .child(logo_mark(18., &pal))
                .child(div().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::SEMIBOLD).child("Mothr"))
                .into_any_element(),
            BarItem::Navigation => {
                let (back, fwd) = (!self.history.back.is_empty(), !self.history.forward.is_empty());
                h_flex()
                    .gap(sz(1.))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new("go-back")
                            .icon(icon("chevron-left"))
                            .ghost()
                            .small()
                            .disabled(!back)
                            .tooltip("Back")
                            .on_click(cx.listener(|this, _, window, cx| this.go_history(NavigationDirection::Back, window, cx))),
                    )
                    .child(
                        Button::new("go-forward")
                            .icon(icon("chevron-right"))
                            .ghost()
                            .small()
                            .disabled(!fwd)
                            .tooltip("Forward")
                            .on_click(cx.listener(|this, _, window, cx| this.go_history(NavigationDirection::Forward, window, cx))),
                    )
                    .into_any_element()
            }
            BarItem::Location => self.bar_location(cx),
            BarItem::Activity => self.bar_activity(cx)?,
            BarItem::NewChat => Button::new("bar-new-chat")
                .small()
                .icon(icon("square-pen"))
                .ghost()
                .tooltip("New chat  (Ctrl+N)")
                .on_click(cx.listener(|this, _, window, cx| this.new_chat(None, window, cx)))
                .into_any_element(),
            BarItem::Search => Button::new("bar-search")
                .small()
                .icon(icon("search"))
                .ghost()
                .tooltip(format!("Find a project  ({PALETTE_KEYS})"))
                .on_click(cx.listener(|this, _, window, cx| this.open_picker(PickerFor::Chat, window, cx)))
                .into_any_element(),
            BarItem::Usage => {
                let current = match self.route {
                    Route::Session(id) => Some(id),
                    _ => None,
                };
                usage::limits_button(
                    self.store.clone(),
                    current,
                    self.usage_open,
                    bottom,
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.usage_open = !this.usage_open;
                        cx.notify();
                    }),
                    cx,
                )
            }
            BarItem::Remote => {
                let remote = self.remote.status();
                if !remote.running {
                    return None;
                }
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
                    .child(icon("radio-tower").size(sz(12.)))
                    .child(if remote.connected_clients > 0 { remote.connected_clients.to_string() } else { "Remote".into() })
                    .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Mothr Remote is on").build(window, cx))
                    .into_any_element()
            }
            BarItem::Theme => Button::new("toggle-theme")
                .small()
                .icon(icon(if pal.dark { "sun" } else { "moon" }))
                .ghost()
                .tooltip(if pal.dark { "Light mode" } else { "Dark mode" })
                .on_click(cx.listener(|this, _, window, cx| this.toggle_theme(window, cx)))
                .into_any_element(),
            BarItem::Settings => Button::new("open-settings")
                .small()
                .icon(icon("settings"))
                .ghost()
                .tooltip("Settings  (Ctrl+,)")
                .on_click(cx.listener(|this, _, window, cx| this.navigate(Route::Settings, window, cx)))
                .into_any_element(),
        })
    }

    /// Where you are, as a quiet pill in the middle of the bar. Clicking it jumps to a project.
    fn bar_location(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let store = self.store.read(cx);
        let (lead, title, detail): (AnyElement, String, Option<String>) = match self.route {
            Route::Session(id) => match store.session(id) {
                Some(s) => (
                    status_dot(ElementId::Name(format!("bar-dot-{id}").into()), &s.meta.status, cx),
                    s.meta.title.clone(),
                    Some(s.meta.project_name()),
                ),
                None => (icon("message-square").size(sz(13.)).into_any_element(), "Chat".into(), None),
            },
            Route::Home => {
                let project = self.home.read(cx).project_name();
                (icon("square-pen").size(sz(13.)).into_any_element(), "New chat".into(), (!project.is_empty()).then_some(project))
            }
            Route::Designs => (icon("palette").size(sz(13.)).into_any_element(), "Designs".into(), None),
            Route::Projects => (icon("folder").size(sz(13.)).into_any_element(), "Projects".into(), None),
            Route::Settings => (icon("settings").size(sz(13.)).into_any_element(), "Settings".into(), None),
        };
        h_flex()
            .id("bar-location")
            .w(sz(400.))
            .min_w(sz(180.))
            .flex_shrink(1.)
            .h(sz(26.))
            .px(sz(10.))
            .gap(sz(8.))
            .items_center()
            .rounded(sz(tokens::radius::MD))
            .border_1()
            .border_color(pal.border)
            .bg(pal.surface_hover.opacity(0.55))
            .text_size(sz(tokens::text::SM))
            .text_color(pal.text_muted)
            .cursor_pointer()
            .hover(|d| d.bg(pal.surface_hover).border_color(pal.border_strong).text_color(pal.text))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.open_picker(PickerFor::Chat, window, cx)))
            .child(div().flex_none().text_color(pal.text_faint).child(lead))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(sz(6.))
                    .child(div().min_w_0().truncate().text_color(pal.text).font_weight(FontWeight::MEDIUM).child(title))
                    .when_some(detail, |d, detail| {
                        d.child(div().flex_none().text_color(pal.text_faint).child("·")).child(div().min_w_0().truncate().child(detail))
                    }),
            )
            .child(div().flex_none().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child(PALETTE_KEYS))
            .with_animation(
                ElementId::Name(format!("bar-loc-{}", self.route_key()).into()),
                Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint()),
                |d, t| d.opacity(0.5 + 0.5 * t),
            )
            .into_any_element()
    }

    /// Chats working or waiting on you right now; hidden when nothing is.
    fn bar_activity(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pal = tokens::p(cx);
        let store = self.store.read(cx);
        let top = || store.sessions.iter().filter(|s| s.meta.parent.is_none());
        let needs: Vec<&SessionState> = top().filter(|s| s.meta.status == SessionStatus::NeedsInput).collect();
        let working: Vec<&SessionState> = top().filter(|s| matches!(s.meta.status, SessionStatus::Working | SessionStatus::Starting)).collect();
        if needs.is_empty() && working.is_empty() {
            return None;
        }
        let target = needs.first().or(working.first()).map(|s| s.meta.id)?;
        let color = if needs.is_empty() { pal.brand } else { pal.warning };
        let mut parts = vec![];
        if !needs.is_empty() {
            parts.push(format!("{} needs you", needs.len()));
        }
        if !working.is_empty() {
            parts.push(format!("{} working", working.len()));
        }
        let tip: String = needs
            .iter()
            .map(|s| format!("Needs you · {}", s.meta.title))
            .chain(working.iter().map(|s| format!("Working · {}", s.meta.title)))
            .take(6)
            .collect::<Vec<_>>()
            .join("\n");
        Some(
            h_flex()
                .id("bar-activity")
                .gap(sz(7.))
                .px(sz(9.))
                .h(sz(24.))
                .items_center()
                .rounded_full()
                .bg(color.opacity(0.10))
                .text_size(sz(tokens::text::XS))
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .cursor_pointer()
                .hover(|d| d.bg(color.opacity(0.18)))
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
                .on_click(cx.listener(move |this, _, window, cx| this.open_session(target, window, cx)))
                .child(
                    div().size(sz(7.)).rounded_full().bg(color).with_animation(
                        "bar-activity-pulse",
                        Animation::new(Duration::from_millis(1400)).repeat().with_easing(pulsating_between(0.35, 1.0)),
                        |d, t| d.opacity(t),
                    ),
                )
                .child(parts.join(" · "))
                .with_animation(
                    "bar-activity-in",
                    Animation::new(Duration::from_millis(240)).with_easing(ease_out_quint()),
                    |d, t| d.opacity(t),
                )
                .into_any_element(),
        )
    }

    fn route_key(&self) -> String {
        match self.route {
            Route::Home => "home".into(),
            Route::Session(id) => id.to_string(),
            Route::Designs => "designs".into(),
            Route::Projects => "projects".into(),
            Route::Settings => "settings".into(),
        }
    }

    /// Change the appearance from the bar's menu.
    fn edit_appearance(&mut self, f: impl FnOnce(&mut Appearance), cx: &mut Context<Self>) {
        theme::edit_appearance(&self.store, f, cx);
        cx.notify();
    }
}

/// Items drawn as a bare icon button (grouped into an island at the end of the bar).
fn is_icon_button(item: BarItem) -> bool {
    matches!(item, BarItem::NewChat | BarItem::Search | BarItem::Theme | BarItem::Settings)
}

/// Right-click on the bar: show or hide its items and move it, without a trip to Settings.
fn bar_menu(mut menu: PopupMenu, a: &Appearance, ws: WeakEntity<Workspace>) -> PopupMenu {
    menu = menu.label("Show in the bar");
    for item in BarItem::ALL {
        let shown = a.shows(item);
        let ws = ws.clone();
        menu = menu.item(PopupMenuItem::new(item.label()).checked(shown).on_click(move |_, _, cx| {
            if let Some(ws) = ws.upgrade() {
                ws.update(cx, |w, cx| w.edit_appearance(|a| a.set_shown(item, !shown), cx));
            }
        }));
    }
    let bottom = a.bar_position == BarPosition::Bottom;
    let right = a.sidebar_side == Side::Right;
    let (ws1, ws2, ws3) = (ws.clone(), ws.clone(), ws);
    menu.separator()
        .item(PopupMenuItem::new(if bottom { "Move bar to the top" } else { "Move bar to the bottom" }).icon(icon(if bottom { "panel-top" } else { "panel-bottom" })).on_click(
            move |_, _, cx| {
                if let Some(ws) = ws1.upgrade() {
                    ws.update(cx, |w, cx| w.edit_appearance(|a| a.bar_position = if bottom { BarPosition::Top } else { BarPosition::Bottom }, cx));
                }
            },
        ))
        .item(PopupMenuItem::new(if right { "Sidebar on the left" } else { "Sidebar on the right" }).icon(icon(if right { "panel-left" } else { "panel-right" })).on_click(
            move |_, _, cx| {
                if let Some(ws) = ws2.upgrade() {
                    ws.update(cx, |w, cx| w.edit_appearance(|a| a.sidebar_side = if right { Side::Left } else { Side::Right }, cx));
                }
            },
        ))
        .separator()
        .item(PopupMenuItem::new("Customize…").icon(icon("sliders-horizontal")).on_click(move |_, window, cx| {
            if let Some(ws) = ws3.upgrade() {
                ws.update(cx, |w, cx| w.navigate(Route::Settings, window, cx));
            }
        }))
}
