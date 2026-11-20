//! One project's files: a folder view with its README, a file viewer (rendered Markdown with
//! an outline, highlighted code with line numbers, images), and a full-screen mode with a
//! tree of the whole project and a fuzzy "Go to file".

use super::outline::{self, Heading, Target};
use super::Back;
use crate::store::Store;
use crate::ui::kit::*;
use crate::ui::tokens::{self, sz};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Editor, EditorState, Input, InputEvent, InputState},
    text::{TextView, TextViewState, TextViewStyle},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mothr_core::*;
use mothr_hub::Hub;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

actions!(project_browser, [GoToFile, ToggleFullscreen, FinderNext, FinderPrev]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-p" } else { "ctrl-p" }, GoToFile, Some("ProjectBrowser")),
        KeyBinding::new("f11", ToggleFullscreen, Some("ProjectBrowser")),
        KeyBinding::new(if cfg!(target_os = "macos") { "cmd-shift-f" } else { "ctrl-shift-f" }, ToggleFullscreen, Some("ProjectBrowser")),
        KeyBinding::new("down", FinderNext, Some("FileFinder")),
        KeyBinding::new("up", FinderPrev, Some("FileFinder")),
    ]);
}

/// Image files the viewer shows as pictures.
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico"];

pub enum BrowserEvent {
    /// Back to the list of projects.
    Back,
    NewChat(ProjectRef),
    /// Full screen was entered or left.
    Changed,
}

impl EventEmitter<BrowserEvent> for ProjectBrowser {}

enum Loaded<T> {
    Loading,
    Ready(T),
    Failed(String),
}

/// Where the browser is: a folder, or an open file.
#[derive(Clone, PartialEq, Eq)]
enum Location {
    Dir(String),
    File(String),
}

/// A file on screen (the open file, or a folder's README).
struct Doc {
    path: String,
    content: Loaded<FileContent>,
    /// Rendered Markdown.
    markdown: Option<Entity<TextViewState>>,
    /// Highlighted source with line numbers (code, or Markdown as source).
    code: Option<Entity<EditorState>>,
    outline: Vec<Heading>,
    /// Markdown shown as source instead of rendered.
    source: bool,
    _load: Task<()>,
}

impl Doc {
    fn ready(&self) -> Option<&FileContent> {
        match &self.content {
            Loaded::Ready(c) => Some(c),
            _ => None,
        }
    }
}

pub struct ProjectBrowser {
    store: Entity<Store>,
    project: ProjectRef,
    /// Folder listings by relative path ("" is the root).
    dirs: HashMap<String, Loaded<DirListing>>,
    /// Folders open in the tree.
    expanded: HashSet<String>,
    /// The folder shown when no file is open.
    dir: String,
    file: Option<Doc>,
    readme: Option<Doc>,
    fullscreen: bool,
    tree_open: bool,
    /// Places visited before the current one, for the mouse's back button.
    back_stack: Vec<Location>,
    /// Places stepped back from, for the mouse's forward button.
    forward_stack: Vec<Location>,
    finder: Entity<InputState>,
    finder_query: String,
    finder_hits: Vec<FileHit>,
    finder_selected: usize,
    _finder_task: Task<()>,
    content_scroll: ScrollHandle,
    tree_scroll: ScrollHandle,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl ProjectBrowser {
    pub fn new(store: Entity<Store>, project: ProjectRef, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let finder = cx.new(|cx| InputState::new(window, cx).placeholder("Go to file…"));
        let subs = vec![cx.subscribe_in(&finder, window, |this, input, ev: &InputEvent, window, cx| match ev {
            InputEvent::Change => {
                this.finder_query = input.read(cx).value().to_string();
                this.run_finder(cx);
            }
            InputEvent::PressEnter { .. } => {
                if let Some(h) = this.finder_hits.get(this.finder_selected) {
                    let path = h.path.clone();
                    this.clear_finder(window, cx);
                    this.open_file(path, window, cx);
                }
            }
            _ => {}
        })];
        let mut this = Self {
            store,
            project,
            dirs: HashMap::new(),
            expanded: HashSet::new(),
            dir: String::new(),
            file: None,
            readme: None,
            fullscreen: false,
            tree_open: true,
            back_stack: vec![],
            forward_stack: vec![],
            finder,
            finder_query: String::new(),
            finder_hits: vec![],
            finder_selected: 0,
            _finder_task: Task::ready(()),
            content_scroll: ScrollHandle::new(),
            tree_scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
            _subs: subs,
        };
        this.browse(String::new(), window, cx);
        this
    }

    /// Developer hook (`MOTHR_OPEN=project:…`).
    pub fn dev_open(&mut self, file: Option<String>, full: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = file {
            self.go_to(f, None, window, cx);
        }
        self.set_fullscreen(full, cx);
    }

    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn hub(&self, cx: &App) -> Arc<Hub> {
        self.store.read(cx).hub.clone()
    }

    fn root(&self) -> PathBuf {
        self.project.path.clone()
    }

    fn set_fullscreen(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.fullscreen != on {
            self.fullscreen = on;
            if on {
                // Show where the open file lives.
                let at = self.file.as_ref().map(|f| parent_path(&f.path).to_string()).unwrap_or_else(|| self.dir.clone());
                self.reveal_in_tree(&at, cx);
            }
            cx.emit(BrowserEvent::Changed);
            cx.notify();
        }
    }

    // ------------------------------------------------------------------ loading

    fn load_dir(&mut self, dir: &str, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.dirs.get(dir), Some(Loaded::Ready(_) | Loaded::Loading)) {
            return;
        }
        self.dirs.insert(dir.to_string(), Loaded::Loading);
        let (hub, root, dir) = (self.hub(cx), self.root(), dir.to_string());
        cx.spawn_in(window, async move |this, cx| {
            let d = dir.clone();
            let res = cx.background_spawn(async move { hub.list_dir(&root, &d) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let loaded = match res {
                    Ok(l) => Loaded::Ready(l),
                    Err(e) => Loaded::Failed(format!("{e:#}")),
                };
                this.dirs.insert(dir.clone(), loaded);
                if dir == this.dir && this.file.is_none() {
                    this.load_readme(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Re-read every loaded folder (files may have changed on disk).
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirs: Vec<String> = self.dirs.keys().cloned().collect();
        self.dirs.clear();
        self.readme = None;
        for d in dirs {
            self.load_dir(&d, window, cx);
        }
        if let Some(path) = self.file.as_ref().map(|f| f.path.clone()) {
            self.show_file(path, window, cx);
        }
    }

    fn load_doc(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>, readme: bool) -> Doc {
        let (hub, root, p, want) = (self.hub(cx), self.root(), path.clone(), path.clone());
        let task = cx.spawn_in(window, async move |this, cx| {
            let res = cx.background_spawn(async move { hub.read_file(&root, &p) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let doc = if readme { this.readme.as_mut() } else { this.file.as_mut() };
                let Some(doc) = doc.filter(|d| d.path == want) else { return };
                match res {
                    Ok(content) => {
                        if let Some(text) = &content.text {
                            if content.is_markdown() {
                                let text = text.clone();
                                doc.outline = outline::headings(&text);
                                doc.markdown = Some(cx.new(|cx| TextViewState::markdown(&text, cx).selectable(true)));
                            } else {
                                doc.code = Some(code_editor(&content.language, text, window, cx));
                            }
                        }
                        doc.content = Loaded::Ready(content);
                    }
                    Err(e) => doc.content = Loaded::Failed(format!("{e:#}")),
                }
                cx.notify();
            });
        });
        Doc { path, content: Loaded::Loading, markdown: None, code: None, outline: vec![], source: false, _load: task }
    }

    fn load_readme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Loaded::Ready(listing)) = self.dirs.get(&self.dir) else { return };
        let Some(path) = listing.readme.clone() else {
            self.readme = None;
            return;
        };
        if self.readme.as_ref().is_some_and(|r| r.path == path) {
            return;
        }
        self.readme = Some(self.load_doc(path, window, cx, true));
    }

    // ------------------------------------------------------------------ navigation

    fn location(&self) -> Location {
        match &self.file {
            Some(f) => Location::File(f.path.clone()),
            None => Location::Dir(self.dir.clone()),
        }
    }

    /// Remember where we are before going to `next`; a new place drops the forward history.
    fn record(&mut self, next: Location) {
        let here = self.location();
        if here != next {
            self.back_stack.push(here);
            self.forward_stack.clear();
        }
    }

    fn show(&mut self, loc: Location, window: &mut Window, cx: &mut Context<Self>) {
        match loc {
            Location::Dir(d) => self.show_dir(d, window, cx),
            Location::File(f) => self.show_file(f, window, cx),
        }
    }

    /// The mouse's back button: the previous place visited. False when there is none.
    pub fn history_back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(prev) = self.back_stack.pop() else { return false };
        self.forward_stack.push(self.location());
        self.show(prev, window, cx);
        true
    }

    /// The mouse's forward button: undo a step back. False when there is nowhere to go.
    pub fn history_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(next) = self.forward_stack.pop() else { return false };
        self.back_stack.push(self.location());
        self.show(next, window, cx);
        true
    }

    /// Show a folder (no file open).
    fn browse(&mut self, dir: String, window: &mut Window, cx: &mut Context<Self>) {
        self.record(Location::Dir(dir.clone()));
        self.show_dir(dir, window, cx);
    }

    fn show_dir(&mut self, dir: String, window: &mut Window, cx: &mut Context<Self>) {
        self.file = None;
        if self.dir != dir {
            self.readme = None;
        }
        self.dir = dir.clone();
        self.content_scroll.set_offset(point(px(0.), px(0.)));
        self.load_dir(&dir, window, cx);
        self.load_readme(window, cx);
        if self.fullscreen {
            self.reveal_in_tree(&dir, cx);
            self.expanded.insert(dir.clone());
        }
        // Folders the tree will show need their listings.
        let ancestors = ancestors(&dir);
        for a in ancestors {
            self.load_dir(&a, window, cx);
        }
        cx.notify();
    }

    fn open_file(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.record(Location::File(path.clone()));
        self.show_file(path, window, cx);
    }

    fn show_file(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let dir = parent_path(&path).to_string();
        self.dir = dir.clone();
        self.content_scroll.set_offset(point(px(0.), px(0.)));
        self.file = Some(self.load_doc(path, window, cx, false));
        for a in ancestors(&dir).into_iter().chain(std::iter::once(dir.clone())) {
            self.load_dir(&a, window, cx);
        }
        self.reveal_in_tree(&dir, cx);
        cx.notify();
    }

    /// Open every folder down to `dir` in the tree.
    fn reveal_in_tree(&mut self, dir: &str, _cx: &mut Context<Self>) {
        for a in ancestors(dir) {
            self.expanded.insert(a);
        }
        if !dir.is_empty() {
            self.expanded.insert(dir.to_string());
        }
    }

    /// Open whatever is at `path`: a folder or a file.
    fn go_to(&mut self, path: String, anchor: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let full = match resolve_inside(&self.root(), &path) {
            Some(p) => p,
            None => {
                self.toast(format!("“{path}” isn't in this project"), cx);
                return;
            }
        };
        if full.is_dir() {
            self.browse(path, window, cx);
        } else if full.is_file() {
            self.open_file(path.clone(), window, cx);
            if let Some(a) = anchor {
                self.reveal_anchor_later(path, a, window, cx);
            }
        } else {
            self.toast(format!("“{path}” doesn't exist"), cx);
        }
    }

    /// Once the linked document has loaded, scroll to the heading.
    fn reveal_anchor_later(&mut self, path: String, anchor: String, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            for _ in 0..40 {
                cx.background_executor().timer(std::time::Duration::from_millis(50)).await;
                let done = this
                    .update(cx, |this, cx| {
                        let ready = this.file.as_ref().is_some_and(|f| f.path == path && f.markdown.is_some());
                        if ready {
                            this.reveal_anchor(&anchor, cx);
                        }
                        ready || this.file.as_ref().is_none_or(|f| f.path != path)
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    fn reveal_anchor(&mut self, anchor: &str, cx: &mut Context<Self>) {
        let Some(doc) = self.file.as_ref().or(self.readme.as_ref()) else { return };
        if let Some(h) = doc.outline.iter().find(|h| h.slug == anchor.to_lowercase()).cloned() {
            self.reveal_heading(&h, cx);
        }
    }

    fn reveal_heading(&mut self, h: &Heading, cx: &mut Context<Self>) {
        let Some(doc) = self.file.as_mut().or(self.readme.as_mut()) else { return };
        doc.source = false;
        if let Some(md) = &doc.markdown {
            let range = h.range.clone();
            md.update(cx, |s, cx| {
                if let Some(r) = s.rendered_text().range_for_source(range) {
                    let _ = s.reveal_range(r.start..r.start, cx);
                }
            });
        }
        cx.notify();
    }

    fn follow_link(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        let from = self.file.as_ref().or(self.readme.as_ref()).map(|d| d.path.clone()).unwrap_or_default();
        match outline::resolve_link(url, &from) {
            Target::External(u) => cx.open_url(&u),
            Target::Anchor(a) => self.reveal_anchor(&a, cx),
            Target::Project { path, anchor } if path == from => {
                if let Some(a) = anchor {
                    self.reveal_anchor(&a, cx);
                }
            }
            Target::Project { path, anchor } => self.go_to(path, anchor, window, cx),
            Target::Outside => self.toast("That link points outside the project".into(), cx),
        }
    }

    /// One step back: file → its folder → parent folder → the project list.
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.finder_query.is_empty() {
            self.clear_finder(window, cx);
        } else if self.file.is_some() {
            let dir = self.dir.clone();
            self.browse(dir, window, cx);
        } else if self.fullscreen {
            self.set_fullscreen(false, cx);
        } else if !self.dir.is_empty() {
            let up = parent_path(&self.dir).to_string();
            self.browse(up, window, cx);
        } else {
            cx.emit(BrowserEvent::Back);
        }
        window.focus(&self.focus, cx);
    }

    fn toast(&self, text: String, cx: &mut Context<Self>) {
        self.store.update(cx, |_, cx| cx.emit(crate::store::StoreEvent::Toast(NoticeLevel::Info, text)));
    }

    // ------------------------------------------------------------------ finder

    fn run_finder(&mut self, cx: &mut Context<Self>) {
        let q = self.finder_query.clone();
        if q.trim().is_empty() {
            self.finder_hits.clear();
            self._finder_task = Task::ready(());
            cx.notify();
            return;
        }
        let (hub, root) = (self.hub(cx), self.root());
        self._finder_task = cx.spawn(async move |this, cx| {
            // Let fast typing settle before walking the project.
            cx.background_executor().timer(std::time::Duration::from_millis(60)).await;
            let hits = cx.background_spawn(async move { hub.find_files(&root, &q, 60).unwrap_or_default() }).await;
            let _ = this.update(cx, |this, cx| {
                this.finder_hits = hits;
                this.finder_selected = 0;
                cx.notify();
            });
        });
    }

    fn clear_finder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finder_query.clear();
        self.finder_hits.clear();
        self.finder.update(cx, |i, cx| i.set_value("", window, cx));
        cx.notify();
    }

    fn focus_finder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.fullscreen && self.file.is_some() {
            // In a file, the finder lives in the tree.
            self.set_fullscreen(true, cx);
        }
        self.finder.update(cx, |i, cx| i.focus(window, cx));
    }

    // ------------------------------------------------------------------ rendering

    fn render_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let p = self.project.clone();
        let tags: Vec<String> = p.tags.iter().filter(|t| *t != "git").cloned().collect();
        let pinned = p.pinned || self.store.read(cx).pinned.iter().any(|x| x.path == p.path);
        let (chat_p, pin_path) = (p.clone(), p.path.clone());
        h_flex()
            .px(sz(24.))
            .pt(sz(22.))
            .pb(sz(16.))
            .gap(sz(14.))
            .items_center()
            .child(
                Button::new("proj-back")
                    .ghost()
                    .icon(icon("arrow-left"))
                    .tooltip("All projects  (Esc)")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(BrowserEvent::Back))),
            )
            .child(
                div()
                    .size(sz(44.))
                    .flex_none()
                    .rounded(sz(tokens::radius::LG))
                    .bg(pal.surface)
                    .border_1()
                    .border_color(pal.border_strong)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("folder-git-2").size(sz(20.)).text_color(pal.text_muted)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(sz(3.))
                    .child(div().text_size(sz(tokens::text::XL)).font_weight(FontWeight::SEMIBOLD).truncate().child(p.name.clone()))
                    .child(
                        h_flex()
                            .gap(sz(10.))
                            .items_center()
                            .text_size(sz(tokens::text::SM))
                            .text_color(pal.text_faint)
                            .child(div().truncate().child(tildify(&p.path)))
                            .when_some(p.git_branch.clone(), |d, b| {
                                d.child(h_flex().flex_none().gap(sz(4.)).items_center().child(icon("git-branch").size(sz(12.))).child(b))
                            })
                            .children(tags.into_iter().take(4).map(|t| {
                                div().flex_none().text_size(sz(tokens::text::XS)).px(sz(6.)).py(sz(1.)).rounded(sz(tokens::radius::XS)).bg(pal.surface_hover).text_color(pal.text_muted).child(t)
                            })),
                    ),
            )
            .child(
                Button::new("proj-pin")
                    .ghost()
                    .icon(icon(if pinned { "pin-off" } else { "pin" }))
                    .tooltip(if pinned { "Unpin project" } else { "Pin project" })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let path = pin_path.clone();
                        this.project.pinned = !pinned;
                        this.store.update(cx, |s, cx| {
                            s.dispatch(AppCommand::PinProject { path, pinned: !pinned }, window, cx);
                        });
                    })),
            )
            .child(
                Button::new("proj-refresh")
                    .ghost()
                    .icon(icon("refresh-cw"))
                    .tooltip("Reload files")
                    .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
            )
            .child(
                Button::new("proj-full")
                    .ghost()
                    .icon(icon("maximize-2"))
                    .tooltip(shortcut_tip("Full screen with file tree", "F11"))
                    .on_click(cx.listener(|this, _, _, cx| this.set_fullscreen(true, cx))),
            )
            .child(
                Button::new("proj-chat")
                    .icon(icon("message-square-plus"))
                    .label("New chat here")
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(BrowserEvent::NewChat(chat_p.clone())))),
            )
    }

    /// project / src / ui — each part opens that folder.
    fn render_crumbs(&self, path: &str, is_file: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut row = h_flex().gap(sz(2.)).items_center().min_w_0().overflow_hidden().text_size(sz(tokens::text::BASE));
        let crumb = |id: String, label: String, target: String, last: bool, cx: &mut Context<Self>| {
            div()
                .id(ElementId::Name(id.into()))
                .flex_none()
                .px(sz(6.))
                .py(sz(3.))
                .rounded(sz(tokens::radius::SM))
                .text_color(if last { pal.text } else { pal.text_muted })
                .when(last, |d| d.font_weight(FontWeight::MEDIUM))
                .when(!last, |d| d.cursor_pointer().hover(|d| d.bg(pal.surface_hover).text_color(pal.text)))
                .when(!last, |d| d.on_click(cx.listener(move |this, _, window, cx| this.browse(target.clone(), window, cx))))
                .child(label)
        };
        row = row.child(crumb("crumb-root".into(), self.project.name.clone(), String::new(), parts.is_empty(), cx));
        let mut acc = String::new();
        for (i, part) in parts.iter().enumerate() {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(part);
            let last = i + 1 == parts.len();
            row = row.child(div().flex_none().text_color(pal.text_faint).child("/"));
            if last && is_file {
                row = row.child(
                    h_flex()
                        .flex_none()
                        .gap(sz(6.))
                        .px(sz(6.))
                        .items_center()
                        .child(file_icon(part, false, &pal))
                        .child(div().font_weight(FontWeight::MEDIUM).child(part.to_string())),
                );
            } else {
                row = row.child(crumb(format!("crumb-{acc}"), part.to_string(), acc.clone(), last, cx));
            }
        }
        row
    }

    fn render_entry_row(&self, e: &FileEntry, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let path = e.path.clone();
        let dir = e.dir;
        h_flex()
            .id(ElementId::Name(format!("entry-{}", e.path).into()))
            .h(sz(36.))
            .px(sz(14.))
            .gap(sz(10.))
            .items_center()
            .border_t_1()
            .border_color(pal.border)
            .cursor_pointer()
            .hover(|d| d.bg(pal.surface_hover.opacity(0.6)))
            .on_click(cx.listener(move |this, _, window, cx| {
                if dir {
                    this.browse(path.clone(), window, cx);
                } else {
                    this.open_file(path.clone(), window, cx);
                }
            }))
            .child(file_icon(&e.name, e.dir, &pal))
            .child(div().flex_1().min_w_0().truncate().text_size(sz(tokens::text::BASE)).text_color(pal.text.opacity(0.92)).child(e.name.clone()))
            .when(!e.dir, |d| d.child(div().flex_none().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child(format_size(e.size))))
            .when(e.dir, |d| d.child(icon("chevron-right").size(sz(13.)).text_color(pal.text_faint)))
    }

    /// The folder view: what's in it, then its README.
    fn render_folder(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let dir = self.dir.clone();
        let mut card = v_flex().rounded(sz(tokens::radius::LG)).border_1().border_color(pal.border_strong).bg(pal.surface).overflow_hidden();
        card = card.child(
            h_flex()
                .h(sz(42.))
                .px(sz(8.))
                .gap(sz(8.))
                .items_center()
                .bg(pal.chrome)
                .child(self.render_crumbs(&dir, false, cx))
                .child(div().flex_1())
                .when(!dir.is_empty(), |d| {
                    let up = parent_path(&dir).to_string();
                    d.child(
                        Button::new("folder-up")
                            .ghost()
                            .small()
                            .icon(icon("corner-left-up"))
                            .tooltip("Up one folder")
                            .on_click(cx.listener(move |this, _, window, cx| this.browse(up.clone(), window, cx))),
                    )
                }),
        );
        match self.dirs.get(&dir) {
            None | Some(Loaded::Loading) => {
                card = card.child(div().p(sz(18.)).text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child("Loading…"));
            }
            Some(Loaded::Failed(e)) => {
                card = card.child(div().p(sz(18.)).text_size(sz(tokens::text::SM)).text_color(pal.danger).child(e.clone()));
            }
            Some(Loaded::Ready(l)) => {
                let entries = l.entries.clone();
                let truncated = l.truncated;
                if entries.is_empty() {
                    card = card.child(div().p(sz(18.)).text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child("This folder is empty."));
                }
                for e in &entries {
                    card = card.child(self.render_entry_row(e, cx));
                }
                if truncated {
                    card = card.child(
                        div()
                            .px(sz(14.))
                            .py(sz(10.))
                            .border_t_1()
                            .border_color(pal.border)
                            .text_size(sz(tokens::text::XS))
                            .text_color(pal.text_faint)
                            .child(format!("Showing the first {MAX_DIR_ENTRIES} entries. Use Go to file to find the rest.")),
                    );
                }
            }
        }
        let readme = self.readme.as_ref().map(|r| r.path.clone());
        let readme_el = readme.map(|path| {
            let name = file_name(&path).to_string();
            let body = self.render_doc_body(true, false, cx);
            v_flex()
                .mt(sz(20.))
                .rounded(sz(tokens::radius::LG))
                .border_1()
                .border_color(pal.border_strong)
                .bg(pal.surface)
                .overflow_hidden()
                .child(
                    h_flex()
                        .id("readme-head")
                        .h(sz(40.))
                        .px(sz(14.))
                        .gap(sz(8.))
                        .items_center()
                        .bg(pal.chrome)
                        .border_b_1()
                        .border_color(pal.border)
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| this.open_file(path.clone(), window, cx)))
                        .child(icon("book-open").size(sz(14.)).text_color(pal.text_muted))
                        .child(div().flex_1().text_size(sz(tokens::text::SM)).font_weight(FontWeight::MEDIUM).child(name))
                        .child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child("Open")),
                )
                .child(div().px(sz(32.)).py(sz(24.)).child(body))
        });
        v_flex()
            .id("folder-scroll")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.content_scroll)
            .child(
                v_flex()
                    .w_full()
                    .max_w(sz(980.))
                    .mx_auto()
                    .px(sz(24.))
                    .pb(sz(40.))
                    .child(self.render_finder_box(cx))
                    .child(if self.finder_query.trim().is_empty() { card.into_any_element() } else { self.render_finder_hits(cx).into_any_element() })
                    .children(readme_el.filter(|_| self.finder_query.trim().is_empty())),
            )
            .into_any_element()
    }

    /// The body of the open file (or of the README when `readme`), without chrome.
    /// `scroll_own` lets a code editor fill and scroll its own area.
    fn render_doc_body(&mut self, readme: bool, scroll_own: bool, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let root = self.root();
        let weak = cx.entity().downgrade();
        let scroll = self.content_scroll.clone();
        let doc = if readme { self.readme.as_ref() } else { self.file.as_ref() };
        let Some(doc) = doc else { return div().into_any_element() };
        let notice = |text: String, color: Hsla| div().py(sz(24.)).text_size(sz(tokens::text::SM)).text_color(color).child(text).into_any_element();
        let content = match &doc.content {
            Loaded::Loading => return notice("Loading…".into(), pal.text_faint),
            Loaded::Failed(e) => return notice(e.clone(), pal.danger),
            Loaded::Ready(c) => c,
        };
        if content.text.is_none() {
            let full = root.join(&content.path);
            if is_image(&content.path) {
                return div()
                    .p(sz(24.))
                    .flex()
                    .justify_center()
                    .child(img(full).max_w_full().max_h(sz(720.)).object_fit(ObjectFit::Contain).rounded(sz(tokens::radius::MD)))
                    .into_any_element();
            }
            return v_flex()
                .py(sz(48.))
                .items_center()
                .gap(sz(8.))
                .child(icon("file-question").size(sz(28.)).text_color(pal.text_faint))
                .child(div().text_size(sz(tokens::text::BASE)).child("Binary file"))
                .child(div().text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child(format!("{} · not shown", format_size(content.size))))
                .into_any_element();
        }
        if let (Some(md), false) = (&doc.markdown, doc.source) {
            let doc_dir = root.join(parent_path(&doc.path));
            let link_weak = weak.clone();
            return TextView::new(md)
                .selectable(true)
                .style(reading_style(cx))
                .text_size(sz(tokens::text::READ))
                .on_link_click(move |url, _, window, cx| {
                    let url = url.to_string();
                    if let Some(b) = link_weak.upgrade() {
                        b.update(cx, |b, cx| b.follow_link(&url, window, cx));
                    }
                })
                .image_source(move |uri| image_for(uri, &doc_dir))
                .on_reveal(move |line, _, _| {
                    let view = scroll.bounds();
                    let off = scroll.offset();
                    scroll.set_offset(point(off.x, off.y - (line.origin.y - view.origin.y) + sz(16.)));
                })
                .into_any_element();
        }
        if let Some(code) = &doc.code {
            let lines = content.text.as_deref().map_or(0, |t| t.lines().count());
            let editor = Editor::new(code).readonly(true).bordered(false).appearance(false);
            return if scroll_own {
                div().size_full().child(editor.h_full()).into_any_element()
            } else {
                // Inline (a README.txt): tall enough for the text, the page scrolls.
                div().h(sz((lines.min(400) as f32 + 2.) * 20.)).child(editor.h_full()).into_any_element()
            };
        }
        if doc.markdown.is_some() && doc.source {
            // Markdown as source: the editor is made on first use.
            return notice("Loading…".into(), pal.text_faint);
        }
        notice("Nothing to show.".into(), pal.text_faint)
    }

    /// The open file: a bar with its path and options, then its content.
    fn render_file(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let Some(doc) = self.file.as_ref() else { return div().into_any_element() };
        let path = doc.path.clone();
        let content = doc.ready().cloned();
        let is_md = doc.markdown.is_some();
        let source = doc.source;
        let outline = if is_md && !source && self.fullscreen { doc.outline.clone() } else { vec![] };
        let meta = content.as_ref().map(|c| {
            let mut parts = vec![];
            if let Some(t) = &c.text {
                let n = t.lines().count();
                parts.push(format!("{n} line{}", if n == 1 { "" } else { "s" }));
            }
            parts.push(format_size(c.size));
            if c.language != "text" {
                parts.push(c.language.clone());
            }
            if c.truncated {
                parts.push(format!("first {} shown", format_size(MAX_FILE_BYTES)));
            }
            parts.join(" · ")
        });
        let full_path = self.root().join(&path);
        let bar = h_flex()
            .h(sz(44.))
            .px(sz(12.))
            .gap(sz(8.))
            .items_center()
            .border_b_1()
            .border_color(pal.border)
            .bg(pal.bg)
            .child(
                Button::new("file-close")
                    .ghost()
                    .small()
                    .icon(icon("arrow-left"))
                    .tooltip("Back to the folder  (Esc)")
                    .on_click(cx.listener(|this, _, window, cx| {
                        let dir = this.dir.clone();
                        this.browse(dir, window, cx);
                    })),
            )
            .child(div().flex_1().min_w_0().child(self.render_crumbs(&path, true, cx)))
            .when_some(meta, |d, m| d.child(div().flex_none().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child(m)))
            .when(is_md, |d| {
                d.child(
                    segment_track(&pal)
                        .flex_none()
                        .child(segment("md-preview", "Preview", !source, &pal).on_click(cx.listener(|this, _, window, cx| this.set_source(false, window, cx))))
                        .child(segment("md-source", "Source", source, &pal).on_click(cx.listener(|this, _, window, cx| this.set_source(true, window, cx)))),
                )
            })
            .child(
                Button::new("file-copy-path")
                    .ghost()
                    .small()
                    .icon(icon("copy"))
                    .tooltip("Copy path")
                    .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(full_path.display().to_string()))),
            )
            .child(
                Button::new("file-full")
                    .ghost()
                    .small()
                    .icon(icon(if self.fullscreen { "minimize-2" } else { "maximize-2" }))
                    .tooltip(shortcut_tip(if self.fullscreen { "Exit full screen" } else { "Full screen with file tree" }, "F11"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let on = !this.fullscreen;
                        this.set_fullscreen(on, cx);
                    })),
            );
        let fills = doc.code.is_some() && (!is_md || source);
        let body = self.render_doc_body(false, fills, cx);
        let main: AnyElement = if fills {
            div().flex_1().min_w_0().min_h_0().bg(pal.bg).child(body).into_any_element()
        } else {
            div()
                .id("file-scroll")
                .flex_1()
                .min_w_0()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&self.content_scroll)
                .child(v_flex().w_full().max_w(sz(tokens::layout::READ_MAX_W + 64.)).mx_auto().px(sz(32.)).pt(sz(28.)).pb(sz(80.)).child(body))
                .into_any_element()
        };
        v_flex()
            .size_full()
            .child(bar)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(main)
                    .when(outline.len() >= 2, |d| d.child(self.render_outline(&outline, cx))),
            )
            .into_any_element()
    }

    fn set_source(&mut self, source: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.file.as_mut() else { return };
        doc.source = source;
        if source
            && doc.code.is_none()
            && let Some(text) = doc.ready().and_then(|c| c.text.clone())
        {
            doc.code = Some(code_editor("markdown", &text, window, cx));
        }
        cx.notify();
    }

    /// "On this page": the document's headings, to jump between sections.
    fn render_outline(&self, outline: &[Heading], cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let min = outline.iter().map(|h| h.level).min().unwrap_or(1);
        let mut list = v_flex().gap(sz(1.));
        for (i, h) in outline.iter().enumerate() {
            let heading = h.clone();
            let depth = (h.level - min).min(3) as f32;
            list = list.child(
                div()
                    .id(ElementId::Name(format!("toc-{i}").into()))
                    .pl(sz(8. + depth * 12.))
                    .pr(sz(8.))
                    .py(sz(4.))
                    .rounded(sz(tokens::radius::SM))
                    .text_size(sz(tokens::text::SM))
                    .text_color(if h.level == min { pal.text.opacity(0.88) } else { pal.text_muted })
                    .truncate()
                    .cursor_pointer()
                    .hover(|d| d.bg(pal.surface_hover).text_color(pal.text))
                    .on_click(cx.listener(move |this, _, _, cx| this.reveal_heading(&heading, cx)))
                    .child(h.text.clone()),
            );
        }
        v_flex()
            .id("outline")
            .w(sz(220.))
            .flex_none()
            .h_full()
            .border_l_1()
            .border_color(pal.border)
            .px(sz(10.))
            .pt(sz(20.))
            .overflow_y_scroll()
            .child(div().px(sz(8.)).pb(sz(8.)).text_size(sz(tokens::text::XS)).font_weight(FontWeight::MEDIUM).text_color(pal.text_faint).child("On this page"))
            .child(list)
    }

    fn render_finder_box(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        div()
            .key_context("FileFinder")
            .on_action(cx.listener(|this, _: &FinderNext, _, cx| this.move_finder(1, cx)))
            .on_action(cx.listener(|this, _: &FinderPrev, _, cx| this.move_finder(-1, cx)))
            .my(sz(12.))
            .rounded(sz(tokens::radius::MD))
            .border_1()
            .border_color(pal.border)
            .bg(pal.surface_hover.opacity(0.6))
            .child(Input::new(&self.finder).small().appearance(false).prefix(icon("file-search").small().text_color(pal.text_faint)).cleanable(true))
    }

    fn move_finder(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.finder_hits.is_empty() {
            return;
        }
        let n = self.finder_hits.len() as isize;
        self.finder_selected = (self.finder_selected as isize + delta).rem_euclid(n) as usize;
        cx.notify();
    }

    fn render_finder_hits(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let mut list = v_flex().gap(sz(1.));
        if self.finder_hits.is_empty() {
            list = list.child(div().px(sz(10.)).py(sz(12.)).text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child("No files match."));
        }
        for (i, h) in self.finder_hits.iter().enumerate() {
            let path = h.path.clone();
            let name = file_name(&h.path).to_string();
            let folder = parent_path(&h.path).to_string();
            let offset = if folder.is_empty() { 0 } else { folder.chars().count() as u32 + 1 };
            let name_idx: Vec<u32> = h.indices.iter().filter(|&&x| x >= offset).map(|x| x - offset).collect();
            let folder_idx: Vec<u32> = h.indices.iter().copied().filter(|&x| x < offset).collect();
            let selected = i == self.finder_selected;
            list = list.child(
                h_flex()
                    .id(ElementId::Name(format!("hit-{}", h.path).into()))
                    .px(sz(10.))
                    .py(sz(6.))
                    .gap(sz(8.))
                    .items_center()
                    .rounded(sz(tokens::radius::SM))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(pal.surface_hover))
                    .hover(|d| d.bg(pal.surface_hover.opacity(0.7)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.clear_finder(window, cx);
                        this.open_file(path.clone(), window, cx);
                    }))
                    .child(file_icon(&name, false, &pal))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(sz(tokens::text::SM)).truncate().child(highlighted(&name, &name_idx, pal.text, pal.brand)))
                            .when(!folder.is_empty(), |d| d.child(div().text_size(sz(tokens::text::XS)).truncate().child(highlighted(&folder, &folder_idx, pal.text_faint, pal.brand)))),
                    ),
            );
        }
        list
    }

    /// Rows of the tree under `dir`, depth-first, as far as folders are open.
    fn tree_rows(&self, dir: &str, depth: usize, out: &mut Vec<(FileEntry, usize)>) {
        let Some(Loaded::Ready(l)) = self.dirs.get(dir) else { return };
        for e in &l.entries {
            out.push((e.clone(), depth));
            if e.dir && self.expanded.contains(&e.path) {
                self.tree_rows(&e.path, depth + 1, out);
            }
        }
    }

    fn toggle_dir(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.expanded.remove(&path) {
            self.expanded.insert(path.clone());
            self.load_dir(&path, window, cx);
        }
        cx.notify();
    }

    fn render_tree(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let mut rows = vec![];
        self.tree_rows("", 0, &mut rows);
        let current_file = self.file.as_ref().map(|f| f.path.clone());
        let current_dir = self.file.is_none().then(|| self.dir.clone());
        let loading = |path: &str| matches!(self.dirs.get(path), Some(Loaded::Loading));
        let mut list = v_flex().py(sz(4.));
        for (e, depth) in rows {
            let open = e.dir && self.expanded.contains(&e.path);
            let active = current_file.as_deref() == Some(e.path.as_str()) || current_dir.as_deref() == Some(e.path.as_str());
            let path = e.path.clone();
            let dir = e.dir;
            let busy = dir && open && loading(&e.path);
            list = list.child(
                h_flex()
                    .id(ElementId::Name(format!("tree-{}", e.path).into()))
                    .relative()
                    .h(sz(26.))
                    .pl(sz(8. + depth as f32 * 14.))
                    .pr(sz(8.))
                    .mx(sz(6.))
                    .gap(sz(6.))
                    .items_center()
                    .rounded(sz(tokens::radius::SM))
                    .cursor_pointer()
                    .when(active, |d| d.bg(pal.brand_soft))
                    .when(!active, |d| d.hover(|d| d.bg(pal.surface_hover)))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        if dir {
                            // A click toggles; a double click also shows the folder.
                            this.toggle_dir(path.clone(), window, cx);
                            if ev.click_count() > 1 {
                                this.expanded.insert(path.clone());
                                this.browse(path.clone(), window, cx);
                            }
                        } else {
                            this.open_file(path.clone(), window, cx);
                        }
                    }))
                    // Indent guides.
                    .children((0..depth).map(|d| div().absolute().top_0().bottom_0().left(sz(14. + d as f32 * 14.)).w(sz(1.)).bg(pal.border)))
                    .child(
                        div()
                            .w(sz(12.))
                            .flex_none()
                            .when(dir, |d| d.child(icon(if open { "chevron-down" } else { "chevron-right" }).size(sz(12.)).text_color(pal.text_faint))),
                    )
                    .child(if dir { icon(if open { "folder-open" } else { "folder" }).size(sz(14.)).text_color(pal.text_muted).into_any_element() } else { file_icon(&e.name, false, &pal).into_any_element() })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(sz(tokens::text::SM))
                            .text_color(if active { pal.text } else { pal.text.opacity(0.85) })
                            .when(active, |d| d.font_weight(FontWeight::MEDIUM))
                            .child(e.name.clone()),
                    )
                    .when(busy, |d| d.child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child("…"))),
            );
        }
        let searching = !self.finder_query.trim().is_empty();
        let name = self.project.name.clone();
        v_flex()
            .w(sz(300.))
            .flex_none()
            .h_full()
            .border_r_1()
            .border_color(pal.border)
            .bg(pal.chrome)
            .child(
                h_flex()
                    .h(sz(44.))
                    .pl(sz(8.))
                    .pr(sz(6.))
                    .gap(sz(6.))
                    .items_center()
                    .border_b_1()
                    .border_color(pal.border)
                    .child(
                        Button::new("tree-back")
                            .ghost()
                            .small()
                            .icon(icon("arrow-left"))
                            .tooltip("Leave full screen")
                            .on_click(cx.listener(|this, _, _, cx| this.set_fullscreen(false, cx))),
                    )
                    .child(
                        div()
                            .id("tree-root")
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(sz(tokens::text::BASE))
                            .font_weight(FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| this.browse(String::new(), window, cx)))
                            .child(name),
                    )
                    .child(
                        Button::new("tree-collapse")
                            .ghost()
                            .small()
                            .icon(icon("chevrons-down-up"))
                            .tooltip("Collapse folders")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.expanded.clear();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("tree-hide")
                            .ghost()
                            .small()
                            .icon(icon("panel-left-close"))
                            .tooltip("Hide file tree")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tree_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().px(sz(8.)).child(self.render_finder_box(cx)))
            .child(
                div()
                    .id("tree-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.tree_scroll)
                    .child(if searching { div().px(sz(6.)).child(self.render_finder_hits(cx)).into_any_element() } else { list.into_any_element() }),
            )
    }
}

impl Render for ProjectBrowser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = tokens::p(cx);
        let content = if self.file.is_some() { self.render_file(cx) } else { self.render_folder(cx) };
        let show_tree = self.fullscreen && self.tree_open;
        v_flex()
            .key_context("ProjectBrowser")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Back, window, cx| this.back(window, cx)))
            .on_action(cx.listener(|this, _: &GoToFile, window, cx| this.focus_finder(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleFullscreen, _, cx| {
                let on = !this.fullscreen;
                this.set_fullscreen(on, cx);
            }))
            .size_full()
            .bg(pal.bg)
            .when(!self.fullscreen && self.file.is_none(), |d| d.child(self.render_header(cx)))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .when(show_tree, |d| d.child(self.render_tree(cx)))
                    .when(self.fullscreen && !self.tree_open, |d| {
                        d.child(
                            v_flex().w(sz(40.)).flex_none().h_full().items_center().pt(sz(8.)).border_r_1().border_color(pal.border).bg(pal.chrome).child(
                                Button::new("tree-show")
                                    .ghost()
                                    .small()
                                    .icon(icon("panel-left-open"))
                                    .tooltip("Show file tree")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.tree_open = true;
                                        cx.notify();
                                    })),
                            ),
                        )
                    })
                    .child(div().flex_1().min_w_0().h_full().child(content)),
            )
    }
}

/// A read-only code view with line numbers and highlighting.
fn code_editor(language: &str, text: &str, window: &mut Window, cx: &mut Context<ProjectBrowser>) -> Entity<EditorState> {
    let (language, text) = (language.to_string(), text.to_string());
    cx.new(|cx| EditorState::new(window, cx).language(language).soft_wrap(false).default_value(text))
}

/// Markdown for reading: generous rhythm, framed code.
fn reading_style(cx: &App) -> TextViewStyle {
    let pal = tokens::p(cx);
    let code = StyleRefinement::default().bg(pal.code_bg).border_color(pal.border).rounded(sz(tokens::radius::MD));
    TextViewStyle { paragraph_gap: rems(1.0), highlight_theme: cx.theme().highlight_theme.clone(), code_block: code, ..Default::default() }
}

/// Where a Markdown image comes from: the web as is, relative paths from the document's folder.
fn image_for(uri: &SharedUri, doc_dir: &Path) -> ImageSource {
    let s = uri.as_ref();
    if s.contains("://") || s.starts_with("data:") {
        return ImageSource::Resource(Resource::Uri(uri.clone()));
    }
    let rel = s.split(['?', '#']).next().unwrap_or(s);
    ImageSource::Resource(Resource::Path(Arc::from(doc_dir.join(rel.trim_start_matches("./")).as_path())))
}

fn is_image(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, e)| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The folders above `dir`, outermost first, starting with the root ("").
fn ancestors(dir: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut acc = String::new();
    let parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(part);
        out.push(acc.clone());
    }
    out
}

/// `rel` inside `root`, if it exists and stays inside.
fn resolve_inside(root: &Path, rel: &str) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let p = root.join(rel).canonicalize().ok()?;
    p.starts_with(&root).then_some(p)
}

fn shortcut_tip(label: &str, key: &str) -> SharedString {
    format!("{label}  ({key})").into()
}

/// A file's icon, by kind.
fn file_icon(name: &str, dir: bool, pal: &tokens::Palette) -> Icon {
    if dir {
        return icon("folder").size(sz(14.)).text_color(pal.text_muted);
    }
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let name = match ext {
        _ if is_readme(&lower) => "book-open",
        "md" | "mdx" | "markdown" | "txt" => "file-text",
        "json" | "toml" | "yaml" | "yml" | "ini" | "cfg" | "conf" | "lock" => "file-cog",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp" => "file-image",
        "sh" | "bash" | "zsh" | "fish" => "file-terminal",
        "zip" | "tar" | "gz" | "xz" | "7z" => "file-archive",
        "" if lower.starts_with('.') => "file-cog",
        _ if language_for(&lower) != "text" => "file-code",
        _ => "file",
    };
    icon(name).size(sz(14.)).text_color(pal.text_faint)
}

#[cfg(test)]
mod tests {
    use super::ancestors;

    #[test]
    fn ancestors_start_at_the_root() {
        assert_eq!(ancestors(""), [""]);
        assert_eq!(ancestors("src"), [""]);
        assert_eq!(ancestors("src/ui/design"), ["", "src", "src/ui"]);
    }
}
