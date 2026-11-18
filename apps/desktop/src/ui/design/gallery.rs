//! The gallery of design systems, and the page that hosts the viewer.

use super::canvas::Renderer;
use super::composer::{ComposerEvent, DesignComposer};
use super::viewer::{DesignViewer, ViewerEvent};
use super::*;
use crate::store::StoreEvent;
use crate::ui::kit::*;
use crate::ui::tokens::{self, sz};
use gpui_kit::component::{h_flex, menu::{ContextMenuExt as _, PopupMenuItem}, spinner::Spinner, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use mothr_design::{FoundDesign, design_root, discover, workspaces_in};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

const IDEAS: [&str; 3] = [
    "A calm, premium design system for a personal finance app",
    "A playful mobile kit for a neighbourhood coffee shop",
    "A dense, keyboard-first dashboard for support teams",
];

pub enum DesignsEvent {
    /// Open a regular chat (a design agent's subagent).
    OpenChat(SessionId),
    PickProject,
}

impl EventEmitter<DesignsEvent> for DesignsPage {}

pub struct DesignsPage {
    store: Entity<Store>,
    composer: Entity<DesignComposer>,
    designs: Vec<FoundDesign>,
    scanned: bool,
    /// Rescan once the current scan ends.
    rescan: bool,
    scanning: bool,
    open_latest: bool,
    viewers: HashMap<PathBuf, Entity<DesignViewer>>,
    open: Option<Entity<DesignViewer>>,
    _scan: Task<()>,
    _debounce: Task<()>,
    _subs: Vec<Subscription>,
}

impl DesignsPage {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| DesignComposer::new(store.clone(), None, "Describe a product, its audience and how it should feel…", window, cx));
        let subs = vec![
            cx.subscribe_in(&composer, window, |this, _, ev: &ComposerEvent, window, cx| match ev {
                ComposerEvent::Started { session, project } => {
                    this.composer.update(cx, |c, cx| c.set_project(None, cx));
                    this.open_project(project.clone(), Some(*session), window, cx);
                }
                ComposerEvent::PickProject => cx.emit(DesignsEvent::PickProject),
            }),
            cx.subscribe(&store, |this, _, ev: &StoreEvent, cx| match ev {
                // A finished tool call may have written design files.
                StoreEvent::Session(id, SessionEvent::Upsert { entry: Entry::Tool(t) })
                    if t.status == ToolStatus::Completed && this.is_design_session(*id, cx) =>
                {
                    this.refresh_soon(cx)
                }
                StoreEvent::Created(_) | StoreEvent::Removed(_) => this.refresh_soon(cx),
                _ => {}
            }),
            cx.observe(&store, |_, _, cx| cx.notify()),
        ];
        let mut this = Self {
            store,
            composer,
            designs: vec![],
            scanned: false,
            rescan: false,
            scanning: false,
            open_latest: false,
            viewers: HashMap::new(),
            open: None,
            _scan: Task::ready(()),
            _debounce: Task::ready(()),
            _subs: subs,
        };
        this.refresh(cx);
        this
    }

    fn is_design_session(&self, id: SessionId, cx: &App) -> bool {
        self.store.read(cx).session(id).is_some_and(|s| s.meta.kind == SessionKind::Design)
    }

    fn refresh_soon(&mut self, cx: &mut Context<Self>) {
        self._debounce = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            let _ = this.update(cx, |this, cx| this.refresh(cx));
        });
    }

    /// Look for designs: those with a chat, the app's design folder, and known projects.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.scanning {
            self.rescan = true;
            return;
        }
        self.scanning = true;
        let store = self.store.read(cx);
        let mut candidates: Vec<PathBuf> = store.sessions.iter().filter(|s| s.meta.kind == SessionKind::Design).map(|s| s.meta.project.clone()).collect();
        candidates.extend(store.pinned.iter().chain(&store.recent).map(|p| p.path.clone()));
        self._scan = cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move {
                    candidates.extend(workspaces_in(&designs_dir()));
                    discover(candidates)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.designs = found;
                this.scanned = true;
                this.scanning = false;
                if std::mem::take(&mut this.open_latest)
                    && let Some(project) = this.designs.first().map(|d| d.project.clone())
                {
                    this.show_viewer(project, cx);
                }
                if std::mem::take(&mut this.rescan) {
                    this.refresh(cx);
                }
                cx.notify();
            });
        });
    }

    fn viewer_for(&mut self, project: PathBuf, cx: &mut Context<Self>) -> Entity<DesignViewer> {
        if let Some(v) = self.viewers.get(&project) {
            return v.clone();
        }
        let store = self.store.clone();
        let viewer = cx.new(|cx| DesignViewer::new(project.clone(), store, cx));
        self._subs.push(cx.subscribe(&viewer, |this, _, ev: &ViewerEvent, cx| match ev {
            ViewerEvent::Back => {
                this.open = None;
                this.refresh(cx);
                cx.notify();
            }
            ViewerEvent::OpenChat(id) => cx.emit(DesignsEvent::OpenChat(*id)),
        }));
        self.viewers.insert(project, viewer.clone());
        viewer
    }

    fn show_viewer(&mut self, project: PathBuf, cx: &mut Context<Self>) {
        self.open = Some(self.viewer_for(project, cx));
        cx.notify();
    }

    /// Open a design; with a `session`, its chat opens alongside.
    pub fn open_project(&mut self, project: PathBuf, session: Option<SessionId>, window: &mut Window, cx: &mut Context<Self>) {
        let viewer = self.viewer_for(project, cx);
        if let Some(id) = session {
            viewer.update(cx, |v, cx| v.show_chat(id, window, cx));
        }
        self.open = Some(viewer);
        self.refresh(cx);
        cx.notify();
    }

    /// Developer hook: open the newest design as soon as the scan finishes.
    pub fn open_latest_when_ready(&mut self, cx: &mut Context<Self>) {
        self.open_latest = true;
        self.refresh(cx);
    }

    /// Back to the gallery with the prompt focused.
    pub fn start_new(&mut self, project: Option<ProjectRef>, window: &mut Window, cx: &mut Context<Self>) {
        self.open = None;
        self.composer.update(cx, |c, cx| {
            if project.is_some() {
                c.set_project(project, cx);
            }
            c.focus(window, cx);
        });
        self.refresh(cx);
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.open {
            Some(v) => v.update(cx, |v, cx| v.focus(window, cx)),
            None => self.composer.update(cx, |c, cx| c.focus(window, cx)),
        }
    }

    fn render_card(&self, index: usize, d: &FoundDesign, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let store = self.store.read(cx);
        let name = design_name(&d.doc, &d.project, store);
        let session = latest_session(store, &d.project).map(|s| s.meta.status.clone());
        let project = d.project.clone();
        let folder = design_root(&d.project);
        let working = matches!(session, Some(SessionStatus::Working | SessionStatus::Starting));
        let needs = session == Some(SessionStatus::NeedsInput);
        let when = d.modified.map(|m| relative_time(chrono::DateTime::<chrono::Utc>::from(m))).unwrap_or_default();
        let place = d.project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let in_workspace = d.project.starts_with(designs_dir());
        let detail = if needs {
            "Needs you".to_string()
        } else if working {
            "Agent is working…".to_string()
        } else if in_workspace {
            format!("Edited {when}")
        } else {
            format!("{place} · {when}")
        };
        let detail_color = if needs { pal.warning } else if working { pal.brand } else { pal.text_faint };
        let counts = counts_label(&d.doc);
        v_flex()
            .id(ElementId::Name(format!("design-card-{index}").into()))
            .flex_1()
            .min_w(sz(280.))
            .max_w(sz(440.))
            .rounded(sz(tokens::radius::LG))
            .border_1()
            .border_color(pal.border)
            .bg(pal.surface)
            .overflow_hidden()
            .cursor_pointer()
            .hover(|s| s.border_color(pal.border_strong).shadow(tokens::shadow_md(&pal)))
            .on_click(cx.listener(move |this, _, _, cx| this.show_viewer(project.clone(), cx)))
            .context_menu(move |menu, _, _| {
                let folder = folder.clone();
                menu.item(PopupMenuItem::new("Show design files").icon(icon("folder-open")).on_click(move |_, _, cx| cx.reveal_path(&folder)))
            })
            .child(preview(&d.doc, 176., Corners { top: tokens::radius::LG - 1., bottom: 0. }, cx))
            .child(
                v_flex()
                    .px(sz(14.))
                    .py(sz(12.))
                    .gap(sz(3.))
                    .border_t_1()
                    .border_color(pal.border)
                    .child(
                        h_flex()
                            .gap(sz(8.))
                            .items_center()
                            .child(div().flex_1().min_w_0().truncate().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::MEDIUM).child(name))
                            .when_some(session.filter(|_| working || needs), |d, s| d.child(status_dot(ElementId::Name(format!("card-dot-{index}").into()), &s, cx))),
                    )
                    .child(
                        h_flex()
                            .gap(sz(8.))
                            .text_size(sz(tokens::text::XS))
                            .child(div().flex_none().text_color(detail_color).child(detail))
                            .child(div().flex_1().min_w_0().truncate().text_right().text_color(pal.text_faint).child(counts)),
                    ),
            )
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        v_flex().pt(sz(8.)).gap(sz(10.)).child(div().text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child("Try one of these")).children(IDEAS.iter().enumerate().map(|(i, idea)| {
            h_flex()
                .id(ElementId::Name(format!("idea-{i}").into()))
                .h(sz(38.))
                .px(sz(12.))
                .gap(sz(10.))
                .items_center()
                .rounded(sz(tokens::radius::MD))
                .border_1()
                .border_color(pal.border)
                .text_size(sz(tokens::text::BASE))
                .text_color(pal.text_muted)
                .cursor_pointer()
                .hover(|d| d.bg(pal.surface_hover).text_color(pal.text))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.composer.update(cx, |c, cx| c.set_text(idea, window, cx));
                }))
                .child(icon("arrow-up-right").size(sz(14.)).text_color(pal.text_faint))
                .child(*idea)
        }))
    }
}

/// `24 colors · 7 components · 2 screens`, leaving out what's missing.
fn counts_label(doc: &DesignDoc) -> String {
    let n = |count: usize, one: &str| (count > 0).then(|| format!("{count} {one}{}", if count == 1 { "" } else { "s" }));
    let parts: Vec<String> = [
        n(doc.system.as_ref().map_or(0, |s| s.tokens.colors.len()), "color"),
        n(doc.system.as_ref().map_or(0, |s| s.components.len()), "component"),
        n(doc.screens.len(), "screen"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() { "Empty".into() } else { parts.join(" · ") }
}

/// Corner radii of a preview. Parents clip to a rectangle, so it rounds its own fill.
#[derive(Clone, Copy)]
pub(super) struct Corners {
    pub top: f32,
    pub bottom: f32,
}

/// A design's calling card, drawn entirely from its own tokens and first screen.
pub(super) fn preview(doc: &DesignDoc, height: f32, corners: Corners, cx: &App) -> Div {
    let pal = tokens::p(cx);
    let round = move |d: Div| d.rounded_t(sz(corners.top)).rounded_b(sz(corners.bottom));
    if !doc.has_content() {
        return round(div())
            .h(sz(height))
            .flex()
            .items_center()
            .justify_center()
            .bg(pal.chrome)
            .child(icon("palette").size(sz(26.)).text_color(pal.border_strong));
    }
    let surface = surface_of(doc).unwrap_or(rgba_of(pal.surface));
    let ink = ink_of(doc, surface);
    let tokens = doc.tokens();
    let display = tokens.typography.iter().max_by(|a, b| a.1.size.unwrap_or(0.).total_cmp(&b.1.size.unwrap_or(0.))).map(|(_, t)| t.clone());
    let swatches: Vec<u32> = tokens.colors.iter().filter_map(|(_, v)| doc.color(v)).filter(|c| *c != surface && contrast(*c, surface) > 1.15).take(6).collect();
    let mut chips = h_flex().gap(sz(5.));
    for c in swatches {
        chips = chips.child(div().size(sz(16.)).rounded_full().bg(hsla(c)).border_1().border_color(hsla(ink).opacity(0.14)));
    }
    let sample = doc.system.as_ref().and_then(|s| s.components.iter().find_map(|c| c.variants().into_iter().next()));
    let r = Renderer::new(doc, 0.72);
    let left = v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .justify_between()
        .child(
            div()
                .text_size(sz(44.))
                .line_height(relative(1.0))
                .text_color(hsla(ink))
                .font_weight(FontWeight(display.as_ref().and_then(|t| t.weight).unwrap_or(600.0)))
                .when_some(display.and_then(|t| t.family), |d, f| d.font_family(f))
                .child("Aa"),
        )
        .when_some(sample, |d, v| d.child(h_flex().max_w_full().overflow_hidden().child(r.node(&v.node, None))))
        .child(chips);
    let mut row = h_flex().size_full().p(sz(16.)).gap(sz(16.)).child(left);
    if let Some(screen) = doc.screens.first() {
        let (w, h) = screen.size();
        let frame_h = height + 10.0;
        let scale = (frame_h / h).min(170.0 / w);
        let bg = screen.background.as_deref().and_then(|b| doc.color(b)).unwrap_or(surface);
        let shot = Renderer::new(doc, scale);
        row = row.child(
            div()
                .flex_none()
                .mt(sz(6.))
                .w(sz(w * scale))
                .h(sz(h * scale))
                .rounded(sz(8.))
                .overflow_hidden()
                .border_1()
                .border_color(hsla(ink).opacity(0.14))
                .bg(hsla(bg))
                .child(div().size_full().flex().flex_col().child(shot.node(&screen.root, None))),
        );
    }
    round(div()).h(sz(height)).overflow_hidden().bg(hsla(surface)).child(row)
}

impl Render for DesignsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(viewer) = &self.open {
            return viewer.clone().into_any_element();
        }
        let pal = tokens::p(cx);
        let designs = self.designs.clone();
        let mut grid = h_flex().flex_wrap().gap(sz(16.)).items_start();
        for (i, d) in designs.iter().enumerate() {
            grid = grid.child(self.render_card(i, d, cx));
        }
        let scanned = self.scanned;
        div()
            .id("designs-scroll")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .w_full()
                    .max_w(sz(1120.))
                    .mx_auto()
                    .px(sz(32.))
                    .pt(sz(40.))
                    .pb(sz(48.))
                    .gap(sz(28.))
                    .child(
                        v_flex()
                            .gap(sz(6.))
                            .child(div().text_size(sz(tokens::text::XXL)).font_weight(FontWeight::SEMIBOLD).child("Design"))
                            .child(div().text_size(sz(tokens::text::BASE)).text_color(pal.text_muted).child("Design systems and screens your agents have made.")),
                    )
                    .child(div().max_w(sz(720.)).child(self.composer.clone()))
                    .map(|d| {
                        if !designs.is_empty() {
                            d.child(
                                v_flex()
                                    .gap(sz(12.))
                                    .child(
                                        h_flex()
                                            .gap(sz(6.))
                                            .text_size(sz(tokens::text::SM))
                                            .child(div().font_weight(FontWeight::MEDIUM).child("Your design systems"))
                                            .child(div().text_color(pal.text_faint).child(designs.len().to_string())),
                                    )
                                    .child(grid),
                            )
                        } else if scanned {
                            d.child(div().max_w(sz(720.)).child(self.render_empty(cx)))
                        } else {
                            d.child(h_flex().justify_center().pt(sz(24.)).child(Spinner::new()))
                        }
                    }),
            )
            .into_any_element()
    }
}
