//! The Projects page: every indexed project with fuzzy search, and a browser for each one.

mod browser;
mod outline;

pub use browser::{BrowserEvent, Location, ProjectBrowser};

use crate::store::Store;
use crate::ui::kit::*;
use crate::ui::tokens::{self, sz};
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mothr_core::*;
use std::collections::HashMap;
use std::path::PathBuf;

actions!(projects, [SelectNext, SelectPrev, Confirm, Back]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, Some("ProjectsList")),
        KeyBinding::new("up", SelectPrev, Some("ProjectsList")),
        KeyBinding::new("ctrl-n", SelectNext, Some("ProjectsList")),
        KeyBinding::new("ctrl-p", SelectPrev, Some("ProjectsList")),
        KeyBinding::new("enter", Confirm, Some("ProjectsList")),
        KeyBinding::new("escape", Back, Some("ProjectBrowser")),
    ]);
    browser::init(cx);
}

/// Every project, however many are indexed.
const LIST_LIMIT: usize = 10_000;

pub enum ProjectsEvent {
    NewChatIn(ProjectRef),
}

impl EventEmitter<ProjectsEvent> for ProjectsPage {}

pub struct ProjectsPage {
    store: Entity<Store>,
    search: Entity<InputState>,
    query: String,
    hits: Vec<ProjectHit>,
    selected: usize,
    list_scroll: UniformListScrollHandle,
    /// What the last search saw: (indexed count, pinned paths). A change re-runs it.
    seen: (Option<usize>, Vec<PathBuf>),
    browsers: HashMap<PathBuf, Entity<ProjectBrowser>>,
    open: Option<Entity<ProjectBrowser>>,
    _subs: Vec<Subscription>,
}

impl ProjectsPage {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find a project by name or path…"));
        let subs = vec![
            cx.subscribe_in(&search, window, |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    this.query = input.read(cx).value().to_string();
                    this.run_search(cx);
                }
                InputEvent::PressEnter { .. } => this.confirm(window, cx),
                _ => {}
            }),
            cx.observe(&store, |this, store, cx| {
                let s = store.read(cx);
                let seen = (s.project_count, s.pinned.iter().map(|p| p.path.clone()).collect());
                if seen != this.seen {
                    this.run_search(cx);
                }
            }),
        ];
        let mut this = Self {
            store,
            search,
            query: String::new(),
            hits: vec![],
            selected: 0,
            list_scroll: UniformListScrollHandle::new(),
            seen: (None, vec![]),
            browsers: HashMap::new(),
            open: None,
            _subs: subs,
        };
        this.run_search(cx);
        this
    }

    /// Whether a project's files are shown full screen (the app hides its sidebar).
    pub fn fullscreen(&self, cx: &App) -> bool {
        self.open.as_ref().is_some_and(|b| b.read(cx).fullscreen())
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.open {
            Some(b) => b.update(cx, |b, cx| b.focus(window, cx)),
            None => self.search.update(cx, |i, cx| i.focus(window, cx)),
        }
    }

    /// Back to the list of projects.
    pub fn show_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = None;
        self.run_search(cx);
        self.search.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }

    /// Developer hook: open a file of the open project, optionally full screen.
    pub fn dev_open(&mut self, file: Option<String>, full: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(b) = &self.open {
            b.update(cx, |b, cx| b.dev_open(file, full, window, cx));
        }
    }

    fn run_search(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        self.seen = (store.project_count, store.pinned.iter().map(|p| p.path.clone()).collect());
        let previous = self.hits.get(self.selected).map(|h| h.project.path.clone());
        self.hits = store.hub.search_projects(&self.query, LIST_LIMIT);
        // Keep the selection on the same project when the list refreshes underneath it.
        self.selected = previous.and_then(|p| self.hits.iter().position(|h| h.project.path == p)).filter(|_| self.query.is_empty()).unwrap_or(0);
        self.list_scroll.scroll_to_item(self.selected, ScrollStrategy::Top);
        cx.notify();
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(h) = self.hits.get(self.selected) {
            let p = h.project.clone();
            self.open_project(p, window, cx);
        }
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.hits.is_empty() {
            return;
        }
        let n = self.hits.len() as isize;
        self.selected = ((self.selected as isize + delta).rem_euclid(n)) as usize;
        self.list_scroll.scroll_to_item(self.selected, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Show a project's files, where they were left.
    pub fn open_project(&mut self, project: ProjectRef, window: &mut Window, cx: &mut Context<Self>) {
        let browser = match self.browsers.get(&project.path) {
            Some(b) => b.clone(),
            None => {
                let store = self.store.clone();
                let b = cx.new(|cx| ProjectBrowser::new(store, project.clone(), window, cx));
                self._subs.push(cx.subscribe_in(&b, window, |this, _, ev: &BrowserEvent, window, cx| match ev {
                    BrowserEvent::Back => this.show_list(window, cx),
                    BrowserEvent::NewChat(p) => cx.emit(ProjectsEvent::NewChatIn(p.clone())),
                    BrowserEvent::Changed => cx.notify(),
                }));
                self.browsers.insert(project.path.clone(), b.clone());
                b
            }
        };
        browser.update(cx, |b, cx| b.focus(window, cx));
        self.open = Some(browser);
        cx.notify();
    }

    /// Where the page is, for the app's back/forward history: the list (`None`) or a place in
    /// one project.
    pub fn place(&self, cx: &App) -> Option<(PathBuf, Location)> {
        let b = self.open.as_ref()?.read(cx);
        Some((b.project_path(), b.location()))
    }

    /// Return to a place from the history.
    pub fn restore(&mut self, place: Option<(PathBuf, Location)>, window: &mut Window, cx: &mut Context<Self>) {
        let browser = place.as_ref().and_then(|(path, _)| self.browsers.get(path).cloned());
        match (browser, place) {
            (Some(b), Some((_, loc))) => {
                b.update(cx, |b, cx| {
                    b.show(loc, window, cx);
                    b.focus(window, cx);
                });
                self.open = Some(b);
                cx.notify();
            }
            _ => self.show_list(window, cx),
        }
    }

    fn add_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Add project".into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    let idx = this.store.read(cx).hub.projects().clone();
                    let p = idx.add_manual(path.clone()).unwrap_or_else(|| ProjectRef::from_path(path));
                    this.run_search(cx);
                    this.open_project(p, window, cx);
                });
            }
        })
        .detach();
    }

    fn render_row(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let h = &self.hits[i];
        let p = h.project.clone();
        let selected = i == self.selected;
        let pinned = p.pinned;
        let path = if h.display_path.is_empty() { tildify(&p.path) } else { h.display_path.clone() };
        let tags: Vec<String> = p.tags.iter().filter(|t| *t != "git").take(3).cloned().collect();
        let key = p.path.display().to_string();
        let (open_p, chat_p, pin_path) = (p.clone(), p.clone(), p.path.clone());
        let row = h_flex()
            .id(ElementId::Name(format!("proj-{key}").into()))
            .group("proj")
            .w_full()
            .h(sz(58.))
            .px(sz(12.))
            .gap(sz(12.))
            .items_center()
            .rounded(sz(tokens::radius::MD))
            .cursor_pointer()
            .when(selected, |d| d.bg(pal.surface_hover))
            .when(!selected, |d| d.hover(|d| d.bg(pal.surface_hover.opacity(0.6))))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = i;
                this.open_project(open_p.clone(), window, cx);
            }))
            .child(
                div()
                    .size(sz(34.))
                    .flex_none()
                    .rounded(sz(tokens::radius::MD))
                    .bg(pal.surface)
                    .border_1()
                    .border_color(pal.border)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(if pinned { "pin" } else { "folder-git-2" }).size(sz(16.)).text_color(if pinned { pal.text } else { pal.text_muted })),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(sz(2.))
                    .child(
                        div()
                            .text_size(sz(tokens::text::BASE))
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(highlighted(&p.name, &h.name_indices, pal.text, pal.brand)),
                    )
                    .child(div().text_size(sz(tokens::text::XS)).truncate().child(highlighted(&path, &h.path_indices, pal.text_faint, pal.brand))),
            )
            .children(tags.into_iter().map(|t| {
                div().flex_none().text_size(sz(tokens::text::XS)).px(sz(6.)).py(sz(1.)).rounded(sz(tokens::radius::XS)).bg(pal.surface_hover).text_color(pal.text_muted).child(t)
            }))
            .when_some(p.git_branch.clone(), |d, b| {
                d.child(
                    h_flex()
                        .flex_none()
                        .max_w(sz(160.))
                        .gap(sz(4.))
                        .items_center()
                        .text_size(sz(tokens::text::XS))
                        .text_color(pal.text_faint)
                        .child(icon("git-branch").size(sz(12.)))
                        .child(div().truncate().child(b)),
                )
            })
            .child(
                h_flex()
                    .flex_none()
                    .gap(sz(2.))
                    .invisible()
                    .group_hover("proj", |d| d.visible())
                    .when(selected, |d| d.visible())
                    .child(
                        Button::new(ElementId::Name(format!("proj-pin-{key}").into()))
                            .ghost()
                            .small()
                            .icon(icon(if pinned { "pin-off" } else { "pin" }))
                            .tooltip(if pinned { "Unpin" } else { "Pin" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                let path = pin_path.clone();
                                this.store.update(cx, |s, cx| {
                                    s.dispatch(AppCommand::PinProject { path, pinned: !pinned }, window, cx);
                                });
                            })),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("proj-chat-{key}").into()))
                            .ghost()
                            .small()
                            .icon(icon("message-square-plus"))
                            .tooltip("New chat here")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.emit(ProjectsEvent::NewChatIn(chat_p.clone()));
                            })),
                    ),
            )
            .child(icon("chevron-right").size(sz(14.)).text_color(pal.text_faint));
        div().w_full().px(sz(12.)).child(row).into_any_element()
    }

    fn render_list(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let count = self.store.read(cx).project_count;
        let n = self.hits.len();
        let searching = !self.query.trim().is_empty();
        let summary = match (count, searching) {
            (None, _) => "Indexing projects…".to_string(),
            (Some(_), true) => format!("{n} match{}", if n == 1 { "" } else { "es" }),
            (Some(c), false) => format!("{c} project{} indexed", if c == 1 { "" } else { "s" }),
        };
        let list: AnyElement = if n == 0 {
            div()
                .pt(sz(48.))
                .child(if searching {
                    empty_state("search-x", "No projects match", "Try fewer letters, or part of the path.", cx)
                } else {
                    empty_state("folder-search", "No projects yet", "Mothr looks in the folders set in Settings → Projects. Add one with Add folder.", cx)
                })
                .into_any_element()
        } else {
            uniform_list("projects-list", n, cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                range.map(|i| this.render_row(i, cx)).collect::<Vec<_>>()
            }))
            .track_scroll(&self.list_scroll)
            .flex_1()
            .pb(sz(12.))
            .into_any_element()
        };
        v_flex()
            .key_context("ProjectsList")
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.confirm(window, cx)))
            .size_full()
            .child(
                v_flex()
                    .w_full()
                    .max_w(sz(920.))
                    .mx_auto()
                    .flex_1()
                    .min_h_0()
                    .child(
                        v_flex()
                            .px(sz(24.))
                            .pt(sz(28.))
                            .pb(sz(14.))
                            .gap(sz(14.))
                            .child(
                                h_flex()
                                    .items_end()
                                    .gap(sz(12.))
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .gap(sz(2.))
                                            .child(div().text_size(sz(tokens::text::XXL)).font_weight(FontWeight::SEMIBOLD).child("Projects"))
                                            .child(div().text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child(summary)),
                                    )
                                    .child(
                                        Button::new("projects-rescan")
                                            .ghost()
                                            .small()
                                            .icon(icon("refresh-cw"))
                                            .tooltip("Look for projects again")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.store.update(cx, |s, cx| {
                                                    s.dispatch(AppCommand::RescanProjects, window, cx);
                                                });
                                            })),
                                    )
                                    .child(
                                        Button::new("projects-add")
                                            .small()
                                            .icon(icon("folder-plus"))
                                            .label("Add folder…")
                                            .on_click(cx.listener(|this, _, window, cx| this.add_folder(window, cx))),
                                    ),
                            )
                            .child(
                                div()
                                    .rounded(sz(tokens::radius::LG))
                                    .border_1()
                                    .border_color(pal.border_strong)
                                    .bg(pal.surface)
                                    .px(sz(4.))
                                    .py(sz(2.))
                                    .child(Input::new(&self.search).appearance(false).prefix(icon("search").text_color(pal.text_faint)).cleanable(true)),
                            )
                            .child(
                                h_flex()
                                    .gap(sz(14.))
                                    .text_size(sz(tokens::text::XS))
                                    .text_color(pal.text_faint)
                                    .child(h_flex().gap(sz(4.)).child(kbd_hint("↑↓", cx)).child("move"))
                                    .child(h_flex().gap(sz(4.)).child(kbd_hint("↵", cx)).child("open"))
                                    .child(div().child(if searching { "Best matches first" } else { "Pinned, then recent, then by activity" })),
                            ),
                    )
                    .child(div().flex_1().min_h_0().flex().flex_col().child(list)),
            )
    }
}

impl Render for ProjectsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.open.clone() {
            Some(b) => div().size_full().child(b).into_any_element(),
            None => self.render_list(cx).into_any_element(),
        }
    }
}
