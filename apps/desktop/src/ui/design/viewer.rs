//! One design, browsed like a storybook: foundations, components and screens on the left, the
//! selected page in the middle, the agent's chat on the right.

use super::composer::{ComposerEvent, DesignComposer};
use super::export::ExportKind;
use super::*;
use crate::ui::chat::{ChatEvent, ChatView};
use crate::ui::kit::*;
use crate::ui::tokens::{self, sz};
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use mothr_design::{Device, design_root, load, watch};
use std::cell::Cell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

pub enum ViewerEvent {
    Back,
    /// Another page is on screen.
    Moved,
    OpenChat(SessionId),
}

impl EventEmitter<ViewerEvent> for DesignViewer {}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Page {
    Overview,
    Color,
    Type,
    Space,
    Elevation,
    Component(usize),
    Screen(usize),
}

/// What components are previewed on.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Stage {
    Design,
    Light,
    Dark,
}

pub struct DesignViewer {
    pub(super) project: PathBuf,
    pub(super) store: Entity<Store>,
    pub(super) doc: DesignDoc,
    pub(super) page: Page,
    pub(super) stage: Stage,
    /// Screen zoom; `None` fits the screen to the canvas.
    pub(super) zoom: Option<f32>,
    /// Size of the screen canvas at the last layout.
    pub(super) viewport: Rc<Cell<Size<Pixels>>>,
    /// Component variants whose source is showing.
    pub(super) sources: HashSet<(usize, usize)>,
    pub(super) scroll: ScrollHandle,
    filter: Option<Entity<InputState>>,
    query: String,
    pub(super) exporting: Option<ExportKind>,
    pub(super) export_open: bool,
    /// Developer hook `MOTHR_DESIGN_EXPORT=html|storybook`: export once the view is up.
    dev_export: Option<ExportKind>,
    chat_open: bool,
    chat: Option<(SessionId, Entity<ChatView>)>,
    composer: Option<Entity<DesignComposer>>,
    _watcher: Option<notify::RecommendedWatcher>,
    _task: Task<()>,
    _subs: Vec<Subscription>,
}

/// `MOTHR_DESIGN_PAGE=overview|color|type|space|elevation|component:<n>|screen:<n>`.
fn dev_page() -> Option<Page> {
    let v = std::env::var("MOTHR_DESIGN_PAGE").ok()?;
    let index = |p: &str| v.strip_prefix(p)?.parse().ok();
    Some(match v.as_str() {
        "overview" => Page::Overview,
        "color" => Page::Color,
        "type" => Page::Type,
        "space" => Page::Space,
        "elevation" => Page::Elevation,
        _ => index("component:").map(Page::Component).or(index("screen:").map(Page::Screen))?,
    })
}

impl DesignViewer {
    pub fn new(project: PathBuf, store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let doc = load(&project);
        let (watcher, task) = match watch(&project) {
            Ok((w, rx)) => {
                let task = cx.spawn(async move |this, cx| {
                    while rx.recv().await.is_ok() {
                        if this.update(cx, |this, cx| this.reload(cx)).is_err() {
                            break;
                        }
                    }
                });
                (Some(w), task)
            }
            Err(e) => {
                tracing::warn!("design watch failed: {e}");
                (None, Task::ready(()))
            }
        };
        let subs = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        let dev_export = std::env::var("MOTHR_DESIGN_EXPORT");
        let mut this = Self {
            project,
            store,
            doc,
            page: dev_page().unwrap_or(Page::Overview),
            stage: Stage::Design,
            zoom: None,
            viewport: Rc::new(Cell::new(Size::default())),
            sources: HashSet::new(),
            scroll: ScrollHandle::new(),
            filter: None,
            query: String::new(),
            exporting: None,
            export_open: dev_export.as_deref() == Ok("menu"),
            dev_export: match dev_export.as_deref() {
                Ok("html") => Some(ExportKind::Html),
                Ok("storybook") => Some(ExportKind::Storybook),
                _ => None,
            },
            chat_open: false,
            chat: None,
            composer: None,
            _watcher: watcher,
            _task: task,
            _subs: subs,
        };
        this.clamp_page();
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.doc = load(&self.project);
        self.clamp_page();
        cx.notify();
    }

    /// Files can disappear while their page is open.
    fn clamp_page(&mut self) {
        let components = self.doc.system.as_ref().map_or(0, |s| s.components.len());
        let gone = match self.page {
            Page::Component(i) => i >= components,
            Page::Screen(i) => i >= self.doc.screens.len(),
            _ => false,
        };
        if gone {
            self.page = Page::Overview;
        }
    }

    pub(super) fn go(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        self.zoom = None;
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.emit(ViewerEvent::Moved);
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        match (&self.chat, &self.composer) {
            (Some((_, chat)), _) if self.chat_open => chat.update(cx, |c, cx| c.focus(window, cx)),
            (_, Some(composer)) if self.chat_open => composer.update(cx, |c, cx| c.focus(window, cx)),
            _ => {}
        }
    }

    pub fn show_chat(&mut self, id: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        self.attach_chat(id, window, cx);
        self.chat_open = true;
        self.focus(window, cx);
        cx.notify();
    }

    fn attach_chat(&mut self, id: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        if self.chat.as_ref().is_some_and(|(current, _)| *current == id) {
            return;
        }
        self.store.update(cx, |s, cx| {
            s.dispatch(AppCommand::MarkRead { id }, window, cx);
        });
        let store = self.store.clone();
        let view = cx.new(|cx| ChatView::new(id, store, window, cx));
        self._subs.push(cx.subscribe(&view, |_, _, ev: &ChatEvent, cx| match ev {
            ChatEvent::Open(id) => cx.emit(ViewerEvent::OpenChat(*id)),
            // Design chats work in their design's folder only.
            ChatEvent::AddProject(_) => {}
        }));
        self.chat = Some((id, view));
    }

    fn toggle_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat_open = !self.chat_open;
        if self.chat_open {
            match latest_session(self.store.read(cx), &self.project).map(|s| s.meta.id) {
                Some(id) => self.attach_chat(id, window, cx),
                None => self.ensure_composer(window, cx),
            }
            self.focus(window, cx);
        }
        cx.notify();
    }

    fn ensure_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_some() {
            return;
        }
        let project = ProjectRef::from_path(self.project.clone());
        let store = self.store.clone();
        let composer = cx.new(|cx| DesignComposer::new(store, Some(project), "What should change?", window, cx));
        self._subs.push(cx.subscribe_in(&composer, window, |this, _, ev: &ComposerEvent, window, cx| {
            if let ComposerEvent::Started { session, .. } = ev {
                this.show_chat(*session, window, cx);
            }
        }));
        self.composer = Some(composer);
    }

    fn ensure_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(f) = &self.filter {
            return f.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        self._subs.push(cx.subscribe(&input, |this, s, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.query = s.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        }));
        self.filter = Some(input.clone());
        input
    }

    pub(super) fn name(&self, cx: &App) -> String {
        design_name(&self.doc, &self.project, self.store.read(cx))
    }

    fn nav_row(&self, id: String, ic: &'static str, label: String, count: Option<usize>, page: Page, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let active = self.page == page;
        h_flex()
            .id(ElementId::Name(id.into()))
            .relative()
            .h(sz(30.))
            .px(sz(10.))
            .gap(sz(9.))
            .items_center()
            .rounded(sz(tokens::radius::MD))
            .cursor_pointer()
            .text_size(sz(tokens::text::BASE))
            .when(active, |d| d.bg(pal.surface_hover).text_color(pal.text).font_weight(FontWeight::MEDIUM))
            .when(!active, |d| d.text_color(pal.text_muted).hover(|d| d.bg(pal.surface_hover.opacity(0.6)).text_color(pal.text)))
            .on_click(cx.listener(move |this, _, _, cx| this.go(page, cx)))
            .when(active, |d| d.child(div().absolute().left(sz(0.)).top(sz(8.)).bottom(sz(8.)).w(sz(2.5)).rounded_full().bg(pal.brand)))
            .child(icon(ic).flex_none().size(sz(14.)).text_color(if active { pal.text } else { pal.text_faint }))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .when_some(count, |d, n| d.child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child(n.to_string())))
    }

    fn render_nav(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let filter = self.ensure_filter(window, cx);
        let q = self.query.clone();
        let hit = |s: &str| q.is_empty() || s.to_lowercase().contains(&q);
        let heading = |text: &'static str| div().px(sz(10.)).pt(sz(16.)).pb(sz(4.)).text_size(sz(tokens::text::XS)).font_weight(FontWeight::MEDIUM).text_color(pal.text_faint).child(text);
        let tokens = self.doc.tokens();
        let mut list = v_flex().px(sz(8.)).pb(sz(16.)).gap(sz(1.));
        if q.is_empty() {
            list = list.child(self.nav_row("nav-overview".into(), "layout-dashboard", "Overview".into(), None, Page::Overview, cx));
        }
        let space = tokens.spacing.len() + tokens.radii.len();
        let foundations = [
            ("nav-color", "swatch-book", "Color", tokens.colors.len(), Page::Color),
            ("nav-type", "type", "Typography", tokens.typography.len(), Page::Type),
            ("nav-space", "ruler", "Space & radius", space, Page::Space),
            ("nav-elevation", "layers", "Elevation", tokens.shadows.len(), Page::Elevation),
        ];
        let shown: Vec<_> = foundations.into_iter().filter(|f| f.3 > 0 && hit(f.2)).collect();
        if !shown.is_empty() {
            list = list.child(heading("Foundations"));
            for (id, ic, label, n, page) in shown {
                list = list.child(self.nav_row(id.into(), ic, label.into(), Some(n), page, cx));
            }
        }
        let components: Vec<(usize, String)> = self.doc.system.iter().flat_map(|s| s.components.iter().map(|c| c.name.clone()).enumerate()).filter(|(_, n)| hit(n)).collect();
        if !components.is_empty() {
            list = list.child(heading("Components"));
            for (i, name) in components {
                list = list.child(self.nav_row(format!("nav-c-{i}"), "component", name, None, Page::Component(i), cx));
            }
        }
        let screens: Vec<(usize, String, Device)> = self.doc.screens.iter().enumerate().map(|(i, s)| (i, s.name.clone(), s.device)).filter(|(_, n, _)| hit(n)).collect();
        if !screens.is_empty() {
            list = list.child(heading("Screens"));
            for (i, name, device) in screens {
                let ic = match device {
                    Device::Desktop => "monitor",
                    Device::Tablet => "tablet",
                    Device::Mobile => "smartphone",
                    Device::Watch => "watch",
                };
                list = list.child(self.nav_row(format!("nav-s-{i}"), ic, name, None, Page::Screen(i), cx));
            }
        }
        v_flex()
            .flex_none()
            .w(sz(228.))
            .h_full()
            .bg(pal.chrome)
            .border_r_1()
            .border_color(pal.border)
            .child(
                div().p(sz(8.)).child(
                    div()
                        .rounded(sz(tokens::radius::MD))
                        .bg(pal.surface_hover.opacity(0.7))
                        .child(Input::new(&filter).small().appearance(false).prefix(icon("search").small().text_color(pal.text_faint)).cleanable(true)),
                ),
            )
            .child(div().id("design-nav").flex_1().min_h_0().overflow_y_scroll().child(list))
    }

    fn render_top_bar(&self, session: Option<&SessionMeta>, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let name = self.name(cx);
        let can_export = !self.doc.is_empty();
        let chat_open = self.chat_open;
        let live = session.map(|m| m.status.clone()).filter(|s| matches!(s, SessionStatus::Working | SessionStatus::Starting | SessionStatus::NeedsInput));
        h_flex()
            .flex_none()
            .h(sz(48.))
            .pl(sz(8.))
            .pr(sz(12.))
            .gap(sz(6.))
            .items_center()
            .border_b_1()
            .border_color(pal.border)
            .child(
                h_flex()
                    .id("design-back")
                    .h(sz(30.))
                    .px(sz(8.))
                    .gap(sz(6.))
                    .items_center()
                    .rounded(sz(tokens::radius::MD))
                    .text_size(sz(tokens::text::BASE))
                    .text_color(pal.text_muted)
                    .cursor_pointer()
                    .hover(|d| d.bg(pal.surface_hover).text_color(pal.text))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ViewerEvent::Back)))
                    .child(icon("arrow-left").size(sz(15.)))
                    .child("Design"),
            )
            .child(div().text_size(sz(tokens::text::BASE)).text_color(pal.text_faint).child("/"))
            .child(div().min_w_0().truncate().px(sz(4.)).text_size(sz(tokens::text::BASE)).font_weight(FontWeight::MEDIUM).child(name))
            .when_some(live, |d, status| {
                let needs = status == SessionStatus::NeedsInput;
                d.child(
                    h_flex()
                        .id("viewer-live")
                        .flex_none()
                        .gap(sz(6.))
                        .px(sz(8.))
                        .h(sz(22.))
                        .items_center()
                        .rounded_full()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if !chat_open {
                                this.toggle_chat(window, cx);
                            }
                        }))
                        .bg(if needs { pal.warning.opacity(0.10) } else { pal.brand_soft })
                        .text_size(sz(tokens::text::XS))
                        .text_color(if needs { pal.warning } else { pal.brand })
                        .child(status_dot("viewer-status", &status, cx))
                        .child(if needs { "Needs you" } else { "Updating" }),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("design-chat")
                    .ghost()
                    .small()
                    .icon(icon("message-square"))
                    .label("Chat")
                    .selected(chat_open)
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_chat(window, cx))),
            )
            .child(
                Button::new("design-export")
                    .outline()
                    .small()
                    .icon(icon("share"))
                    .label("Export")
                    .loading(self.exporting.is_some())
                    .disabled(!can_export || self.exporting.is_some())
                    .tooltip(if can_export { "Export this design" } else { "Nothing to export yet" })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.export_open = !this.export_open;
                        cx.notify();
                    })),
            )
    }

    fn render_export_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let row = |id: &'static str, ic: &'static str, title: &'static str, detail: &'static str, cx: &mut Context<Self>| {
            h_flex()
                .id(id)
                .px(sz(10.))
                .py(sz(9.))
                .gap(sz(12.))
                .items_center()
                .rounded(sz(tokens::radius::MD))
                .cursor_pointer()
                .hover(|d| d.bg(pal.surface_hover))
                .child(
                    div()
                        .flex_none()
                        .size(sz(34.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(sz(tokens::radius::MD))
                        .border_1()
                        .border_color(pal.border)
                        .bg(pal.surface)
                        .child(icon(ic).size(sz(16.)).text_color(pal.text_muted)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(sz(1.))
                        .child(div().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::MEDIUM).child(title))
                        .child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_muted).child(detail)),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.export_open = false;
                    match id {
                        "export-html" => this.export_html(window, cx),
                        "export-storybook" => this.export_storybook(window, cx),
                        _ => cx.reveal_path(&design_root(&this.project)),
                    }
                    cx.notify();
                }))
        };
        v_flex()
            .id("export-menu")
            .occlude()
            .w(sz(340.))
            .p(sz(6.))
            .gap(sz(2.))
            .rounded(sz(tokens::radius::LG))
            .border_1()
            .border_color(pal.border_strong)
            .bg(pal.raised)
            .shadow(tokens::shadow_lg(&pal))
            .child(row("export-html", "file-code", "HTML", "One file with every screen and the system. Works offline.", cx))
            .child(row("export-storybook", "book-open", "Storybook", "A project with a story per variant, built to a static site.", cx))
            .child(div().h(sz(1.)).mx(sz(8.)).my(sz(4.)).bg(pal.border))
            .child(row("export-files", "folder-open", "Show design files", "The JSON your agent writes.", cx))
    }

    fn render_chat_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        // The chat may have been deleted while it was open.
        if self.chat.as_ref().is_some_and(|(id, _)| self.store.read(cx).session(*id).is_none()) {
            self.chat = None;
        }
        let latest = latest_session(self.store.read(cx), &self.project).map(|s| s.meta.id);
        if let Some(id) = latest.filter(|_| self.chat.is_none()) {
            self.attach_chat(id, window, cx);
        }
        if self.chat.is_none() {
            self.ensure_composer(window, cx);
        }
        div().flex_none().w(sz(420.)).h_full().border_l_1().border_color(pal.border).bg(pal.bg).map(|d| match (&self.chat, &self.composer) {
            (Some((_, chat)), _) => d.child(chat.clone()),
            (None, Some(composer)) => d.child(
                v_flex()
                    .size_full()
                    .justify_center()
                    .p(sz(20.))
                    .gap(sz(12.))
                    .child(div().text_size(sz(tokens::text::LG)).font_weight(FontWeight::SEMIBOLD).child("Continue this design"))
                    .child(div().text_size(sz(tokens::text::BASE)).text_color(pal.text_muted).child("An agent will work in this design's folder. Changes show up here as it saves."))
                    .child(composer.clone()),
            ),
            _ => d.child(div().size_full().flex().items_center().justify_center().child(Spinner::new())),
        })
    }
}

impl Render for DesignViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = tokens::p(cx);
        if let Some(kind) = self.dev_export.take() {
            cx.defer_in(window, move |this, window, cx| match kind {
                ExportKind::Html => this.export_html(window, cx),
                ExportKind::Storybook => this.export_storybook(window, cx),
            });
        }
        let session = latest_session(self.store.read(cx), &self.project).map(|s| s.meta.clone());
        let top = self.render_top_bar(session.as_ref(), cx);
        // Beside an open chat a narrow window keeps the canvas and gives up the page list.
        let show_nav = self.doc.has_content() && (!self.chat_open || window.viewport_size().width > sz(1240.));
        let nav = show_nav.then(|| self.render_nav(window, cx));
        let chat = self.chat_open.then(|| self.render_chat_panel(window, cx));
        let errors = self.doc.errors.clone();
        let body = self.render_page(cx);
        let export_menu = self.export_open.then(|| self.render_export_menu(cx));
        v_flex()
            .relative()
            .size_full()
            .bg(pal.bg)
            .child(top)
            .when(!errors.is_empty(), |d| {
                d.child(
                    v_flex()
                        .flex_none()
                        .px(sz(16.))
                        .py(sz(8.))
                        .gap(sz(2.))
                        .bg(pal.danger.opacity(0.08))
                        .border_b_1()
                        .border_color(pal.danger.opacity(0.25))
                        .text_size(sz(tokens::text::XS))
                        .text_color(pal.danger)
                        .children(errors.into_iter().map(|e| div().child(e))),
                )
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .when_some(nav, |d, n| d.child(n))
                    .child(div().flex_1().min_w_0().h_full().child(body))
                    .when_some(chat, |d, c| d.child(c)),
            )
            .when_some(export_menu, |d, menu| {
                d.child(
                    div().id("export-backdrop").absolute().inset_0().on_click(cx.listener(|this, _, _, cx| {
                        this.export_open = false;
                        cx.notify();
                    })),
                )
                .child(div().absolute().top(sz(44.)).right(sz(12.)).child(menu))
            })
    }
}
