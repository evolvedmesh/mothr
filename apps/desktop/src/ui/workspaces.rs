//! Workspaces: groups of chats you switch between, like a browser's spaces. This is the
//! overview page (every workspace and the chats in it, where they are made, renamed, given an
//! icon, reordered and removed) plus the pieces the sidebar's switcher shares.

use crate::store::Store;
use crate::ui::kit::*;
use crate::ui::tokens::{self, sz};
use gpui_kit::component::{
    Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mothr_core::*;

/// Keyboard hints for stepping through workspaces and jumping to one.
pub const KEYS_NEXT: &str = if cfg!(target_os = "macos") { "⌘⌥→" } else { "Ctrl+Alt+→" };
pub const KEYS_PREV: &str = if cfg!(target_os = "macos") { "⌘⌥←" } else { "Ctrl+Alt+←" };
pub const KEYS_OVERVIEW: &str = if cfg!(target_os = "macos") { "⌘⇧W" } else { "Ctrl+Shift+W" };

/// The shortcut that jumps to the workspace at `index` ("Ctrl+3"), if it has one.
pub fn jump_keys(index: usize) -> Option<String> {
    let key = match index {
        0..=8 => (index + 1).to_string(),
        9 => "0".into(),
        _ => return None,
    };
    Some(if cfg!(target_os = "macos") { format!("⌘{key}") } else { format!("Ctrl+{key}") })
}

/// The icon a workspace is drawn with.
pub fn workspace_icon(w: &Workspace) -> &'static str {
    w.icon_name().unwrap_or("layers")
}

/// How a workspace's dot looks in the switcher.
pub struct Dot {
    pub id: WorkspaceId,
    pub index: usize,
    pub name: String,
    pub active: bool,
    /// A chat in it waits on you.
    pub needs: bool,
    /// A chat in it is working.
    pub working: bool,
}

impl Dot {
    /// Name and shortcut, for the dot's tooltip.
    pub fn tooltip(&self) -> String {
        let mut t = self.name.clone();
        if self.needs {
            t.push_str(" · needs you");
        } else if self.working {
            t.push_str(" · working");
        }
        match jump_keys(self.index) {
            Some(k) => format!("{t}  ({k})"),
            None => t,
        }
    }
}

/// One dot per workspace, in order.
pub fn dots(store: &Store) -> Vec<Dot> {
    let active = store.active_workspace_id();
    store
        .workspaces
        .iter()
        .enumerate()
        .map(|(index, w)| {
            let chats = store.sessions.iter().filter(|s| s.meta.kind != SessionKind::Design && store.workspace_of(&s.meta) == Some(w.id));
            let (mut needs, mut working) = (false, false);
            for s in chats {
                needs |= s.meta.status == SessionStatus::NeedsInput;
                working |= matches!(s.meta.status, SessionStatus::Working | SessionStatus::Starting);
            }
            Dot { id: w.id, index, name: w.name.clone(), active: active == Some(w.id), needs, working }
        })
        .collect()
}

/// A workspace's dot: a pill when it's the one on screen, amber when a chat in it waits on you.
pub fn dot_mark(d: &Dot, pal: &tokens::Palette, vertical: bool) -> Div {
    let color = if d.active {
        pal.brand
    } else if d.needs {
        pal.warning
    } else if d.working {
        pal.brand.opacity(0.7)
    } else {
        pal.text_faint.opacity(0.55)
    };
    let (long, short) = if d.active { (16., 6.) } else { (6., 6.) };
    let (w, h) = if vertical { (short, long) } else { (long, short) };
    div().w(sz(w)).h(sz(h)).rounded_full().bg(color)
}

pub enum WorkspacesEvent {
    OpenChat(SessionId),
    /// Switch to a workspace and show where you left it.
    Switch(WorkspaceId),
    /// Switch to a workspace and start a chat there.
    NewChatIn(WorkspaceId),
}

impl EventEmitter<WorkspacesEvent> for WorkspacesPage {}

/// How many chats a card lists before "N more".
const CARD_CHATS: usize = 6;

pub struct WorkspacesPage {
    store: Entity<Store>,
    /// The workspace being renamed, and its name field.
    editing: Option<(WorkspaceId, Entity<InputState>)>,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
    _edit_sub: Option<Subscription>,
}

impl WorkspacesPage {
    pub fn new(store: Entity<Store>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subs = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self { store, editing: None, focus: cx.focus_handle(), _subs: subs, _edit_sub: None }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.editing {
            Some((_, input)) => input.update(cx, |i, cx| i.focus(window, cx)),
            None => window.focus(&self.focus, cx),
        }
    }

    fn dispatch(&self, command: AppCommand, window: &mut Window, cx: &mut Context<Self>) {
        self.store.update(cx, |s, cx| {
            s.dispatch(command, window, cx);
        });
    }

    /// Add a workspace and name it right away.
    pub fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.store.read(cx).workspaces.clone();
        if existing.len() >= MAX_WORKSPACES {
            return;
        }
        let w = Workspace::new(next_workspace_name(&existing));
        let id = w.id;
        self.dispatch(AppCommand::CreateWorkspace { workspace: w.clone() }, window, cx);
        self.start_rename_as(id, w.name, window, cx);
    }

    /// Put a workspace's name into an editable field.
    pub fn rename(&mut self, id: WorkspaceId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.store.read(cx).workspace(id).map(|w| w.name.clone()) else { return };
        self.start_rename_as(id, name, window, cx);
    }

    fn start_rename_as(&mut self, id: WorkspaceId, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Workspace name").default_value(name));
        self._edit_sub = Some(cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_rename(window, cx),
            _ => {}
        }));
        input.update(cx, |i, cx| {
            i.focus(window, cx);
            i.select_all(window, cx);
        });
        self.editing = Some((id, input));
        cx.notify();
    }

    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, input)) = self.editing.take() else { return };
        self._edit_sub = None;
        let name = input.read(cx).value().to_string();
        let current = self.store.read(cx).workspace(id).cloned();
        if let (Some(mut w), Some(name)) = (current, clean_workspace_name(&name))
            && w.name != name
        {
            w.name = name;
            self.dispatch(AppCommand::UpdateWorkspace { workspace: w }, window, cx);
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        self._edit_sub = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn render_card(&self, w: &Workspace, index: usize, count: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let store = self.store.read(cx);
        let active = store.active_workspace_id() == Some(w.id);
        let chats: Vec<SessionMeta> = store.workspace_chats(w.id).into_iter().map(|s| s.meta.clone()).collect();
        let others: Vec<Workspace> = store.workspaces.iter().filter(|x| x.id != w.id).cloned().collect();
        let needs = chats.iter().filter(|m| m.status == SessionStatus::NeedsInput).count();
        let working = chats.iter().filter(|m| matches!(m.status, SessionStatus::Working | SessionStatus::Starting)).count();
        let mut summary = match chats.len() {
            0 => "No chats yet".to_string(),
            1 => "1 chat".to_string(),
            n => format!("{n} chats"),
        };
        if needs > 0 {
            summary.push_str(&format!(" · {needs} need{} you", if needs == 1 { "s" } else { "" }));
        }
        if working > 0 {
            summary.push_str(&format!(" · {working} working"));
        }
        let id = w.id;
        let weak = cx.entity().downgrade();
        let editing = self.editing.as_ref().filter(|(e, _)| *e == id).map(|(_, i)| i.clone());
        let can_delete = count > 1;
        let fallback = others.first().map(|o| o.name.clone()).unwrap_or_default();
        let current_icon = w.icon_name();
        let w_icon = w.clone();
        let w_menu = w.clone();

        let icon_button = Button::new(ElementId::Name(format!("ws-icon-{id}").into()))
            .ghost()
            .icon(icon(workspace_icon(w)).text_color(if active { pal.brand } else { pal.text_muted }))
            .tooltip("Change icon")
            .dropdown_menu({
                let weak = weak.clone();
                move |mut menu, _, _| {
                    menu = menu.label("Icon");
                    let w = w_icon.clone();
                    let weak2 = weak.clone();
                    menu = menu.item(PopupMenuItem::new("Plain dot").icon(icon("layers")).checked(current_icon.is_none()).on_click(move |_, window, cx| {
                        let mut w = w.clone();
                        w.icon = None;
                        if let Some(p) = weak2.upgrade() {
                            p.update(cx, |p, cx| p.dispatch(AppCommand::UpdateWorkspace { workspace: w }, window, cx));
                        }
                    }));
                    for name in WORKSPACE_ICONS {
                        let mut w = w_icon.clone();
                        w.icon = Some(name.to_string());
                        let weak = weak.clone();
                        menu = menu.item(PopupMenuItem::new(icon_label(name)).icon(icon(name)).checked(current_icon == Some(*name)).on_click(move |_, window, cx| {
                            if let Some(p) = weak.upgrade() {
                                let w = w.clone();
                                p.update(cx, |p, cx| p.dispatch(AppCommand::UpdateWorkspace { workspace: w }, window, cx));
                            }
                        }));
                    }
                    menu
                }
            });

        let title: AnyElement = match editing {
            Some(input) => div()
                .flex_1()
                .min_w_0()
                .rounded(sz(tokens::radius::SM))
                .border_1()
                .border_color(pal.border_strong)
                .bg(pal.bg)
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    if ev.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.cancel_rename(window, cx);
                    }
                }))
                .child(Input::new(&input).small().appearance(false))
                .into_any_element(),
            None => v_flex()
                .id(ElementId::Name(format!("ws-name-{id}").into()))
                .flex_1()
                .min_w_0()
                .gap(sz(1.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                    if ev.click_count() > 1 {
                        this.rename(id, window, cx);
                    }
                }))
                .child(
                    h_flex()
                        .gap(sz(6.))
                        .items_center()
                        .child(div().min_w_0().truncate().text_size(sz(tokens::text::LG)).font_weight(FontWeight::SEMIBOLD).text_color(pal.text).child(w.name.clone()))
                        .when_some(jump_keys(index), |d, k| d.child(div().flex_none().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child(k))),
                )
                .child(div().text_size(sz(tokens::text::XS)).text_color(if needs > 0 { pal.warning } else { pal.text_faint }).child(summary))
                .into_any_element(),
        };

        let actions = Button::new(ElementId::Name(format!("ws-more-{id}").into()))
            .ghost()
            .small()
            .icon(icon("ellipsis"))
            .tooltip("More")
            .dropdown_menu(move |menu, _, _| {
                let (w1, w2, w3, w4) = (weak.clone(), weak.clone(), weak.clone(), weak.clone());
                let w = w_menu.clone();
                menu.item(PopupMenuItem::new("Rename").icon(icon("pencil")).on_click(move |_, window, cx| {
                    if let Some(p) = w1.upgrade() {
                        p.update(cx, |p, cx| p.rename(w.id, window, cx));
                    }
                }))
                .item(PopupMenuItem::new("Move left").icon(icon("arrow-left")).disabled(index == 0).on_click(move |_, window, cx| {
                    if let Some(p) = w2.upgrade() {
                        p.update(cx, |p, cx| p.dispatch(AppCommand::MoveWorkspace { id, index: index.saturating_sub(1) }, window, cx));
                    }
                }))
                .item(PopupMenuItem::new("Move right").icon(icon("arrow-right")).disabled(index + 1 >= count).on_click(move |_, window, cx| {
                    if let Some(p) = w3.upgrade() {
                        p.update(cx, |p, cx| p.dispatch(AppCommand::MoveWorkspace { id, index: index + 1 }, window, cx));
                    }
                }))
                .separator()
                .item(
                    PopupMenuItem::new(if can_delete { format!("Delete (chats move to {fallback})") } else { "Delete".to_string() })
                        .icon(icon("trash"))
                        .disabled(!can_delete)
                        .on_click(move |_, window, cx| {
                            if let Some(p) = w4.upgrade() {
                                p.update(cx, |p, cx| p.dispatch(AppCommand::DeleteWorkspace { id }, window, cx));
                            }
                        }),
                )
            });

        let header = h_flex()
            .gap(sz(8.))
            .items_center()
            .child(icon_button)
            .child(title)
            .child(if active {
                div()
                    .flex_none()
                    .px(sz(8.))
                    .h(sz(22.))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(pal.brand_soft)
                    .text_size(sz(tokens::text::XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(pal.brand)
                    .child("Open")
                    .into_any_element()
            } else {
                Button::new(ElementId::Name(format!("ws-switch-{id}").into()))
                    .small()
                    .label("Switch")
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(WorkspacesEvent::Switch(id))))
                    .into_any_element()
            })
            .child(actions);

        let mut list = v_flex().gap(sz(1.));
        for m in chats.iter().take(CARD_CHATS) {
            list = list.child(self.render_chat(m, &others, cx));
        }
        let more = chats.len().saturating_sub(CARD_CHATS);

        v_flex()
            .id(ElementId::Name(format!("ws-card-{id}").into()))
            .w(sz(340.))
            .flex_none()
            .p(sz(12.))
            .gap(sz(10.))
            .rounded(sz(tokens::radius::LG))
            .border_1()
            .border_color(if active { pal.brand.opacity(0.45) } else { pal.border_strong })
            .bg(pal.surface)
            .child(header)
            .child(div().h(sz(1.)).bg(pal.border))
            .when(chats.is_empty(), |d| {
                d.child(
                    v_flex()
                        .py(sz(10.))
                        .gap(sz(8.))
                        .items_center()
                        .child(div().text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child("Nothing here yet."))
                        .child(
                            Button::new(ElementId::Name(format!("ws-new-chat-{id}").into()))
                                .small()
                                .icon(icon("square-pen"))
                                .label("New chat here")
                                .on_click(cx.listener(move |_, _, _, cx| cx.emit(WorkspacesEvent::NewChatIn(id)))),
                        ),
                )
            })
            .when(!chats.is_empty(), |d| d.child(list))
            .when(more > 0, |d| {
                d.child(
                    div()
                        .id(ElementId::Name(format!("ws-more-chats-{id}").into()))
                        .px(sz(8.))
                        .text_size(sz(tokens::text::XS))
                        .text_color(pal.text_faint)
                        .cursor_pointer()
                        .hover(|d| d.text_color(pal.text))
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(WorkspacesEvent::Switch(id))))
                        .child(format!("{more} more — switch to see them all")),
                )
            })
    }

    fn render_chat(&self, m: &SessionMeta, others: &[Workspace], cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let id = m.id;
        let store = self.store.clone();
        let others = others.to_vec();
        h_flex()
            .id(ElementId::Name(format!("ws-chat-{id}").into()))
            .px(sz(8.))
            .py(sz(5.))
            .gap(sz(8.))
            .items_center()
            .rounded(sz(tokens::radius::SM))
            .cursor_pointer()
            .hover(|d| d.bg(pal.surface_hover.opacity(0.7)))
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(WorkspacesEvent::OpenChat(id))))
            .child(agent_avatar(m.agent, 20.))
            .child(div().flex_1().min_w_0().truncate().text_size(sz(tokens::text::SM)).text_color(if m.unread { pal.text } else { pal.text.opacity(0.88) }).child(m.title.clone()))
            .child(status_dot(ElementId::Name(format!("ws-chat-dot-{id}").into()), &m.status, cx))
            .child(div().flex_none().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child(relative_time(m.updated_at)))
            .context_menu(move |mut menu, _, _| {
                if others.is_empty() {
                    return menu.label("Add a workspace to move chats between them");
                }
                menu = menu.label("Move to");
                for w in &others {
                    let store = store.clone();
                    let to = w.id;
                    menu = menu.item(PopupMenuItem::new(w.name.clone()).icon(icon(workspace_icon(w))).on_click(move |_, window, cx| {
                        store.update(cx, |s, cx| {
                            s.dispatch(AppCommand::SetWorkspace { id, workspace: to }, window, cx);
                        });
                    }));
                }
                menu
            })
    }
}

/// "flask-conical" → "Flask conical".
fn icon_label(name: &str) -> String {
    let words = name.split('-').filter(|w| w.parse::<u32>().is_err()).collect::<Vec<_>>().join(" ");
    let mut c = words.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

impl Render for WorkspacesPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = tokens::p(cx);
        let workspaces = self.store.read(cx).workspaces.clone();
        let count = workspaces.len();
        let full = count >= MAX_WORKSPACES;
        let mut grid = h_flex().flex_wrap().gap(sz(14.)).items_start();
        for (i, w) in workspaces.iter().enumerate() {
            grid = grid.child(self.render_card(w, i, count, cx));
        }
        v_flex()
            .id("workspaces-page")
            .key_context("WorkspacesPage")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .max_w(sz(1100.))
                    .mx_auto()
                    .px(sz(24.))
                    .pt(sz(28.))
                    .pb(sz(32.))
                    .gap(sz(18.))
                    .child(
                        h_flex()
                            .items_end()
                            .gap(sz(12.))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(sz(2.))
                                    .child(div().text_size(sz(tokens::text::XXL)).font_weight(FontWeight::SEMIBOLD).child("Workspaces"))
                                    .child(div().text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child(format!(
                                        "{count} of {MAX_WORKSPACES} · each keeps its own chats. Right-click a chat to move it."
                                    ))),
                            )
                            .child(
                                Button::new("ws-create")
                                    .flex_none()
                                    .small()
                                    .icon(icon("plus"))
                                    .label("New workspace")
                                    .disabled(full)
                                    .tooltip(if full { format!("You can have up to {MAX_WORKSPACES} workspaces") } else { "Add a workspace".into() })
                                    .on_click(cx.listener(|this, _, window, cx| this.create(window, cx))),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap(sz(14.))
                            .flex_wrap()
                            .text_size(sz(tokens::text::XS))
                            .text_color(pal.text_faint)
                            .child(h_flex().gap(sz(4.)).child(kbd_hint(KEYS_PREV, cx)).child(kbd_hint(KEYS_NEXT, cx)).child("previous / next"))
                            .child(h_flex().gap(sz(4.)).child(kbd_hint(if cfg!(target_os = "macos") { "⌘1 … ⌘0" } else { "Ctrl+1 … 0" }, cx)).child("jump to one"))
                            .child(h_flex().gap(sz(4.)).child(kbd_hint(KEYS_OVERVIEW, cx)).child("this overview")),
                    )
                    .child(grid),
            )
    }
}
