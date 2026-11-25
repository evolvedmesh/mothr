//! Skills: Mothr's skill library, shared by every agent. Installed skills are listed (turned
//! on or off, updated, removed); Discover searches the open skills index, browses popular skill
//! repos and finds skills other agents keep on this computer; anything else (a GitHub repo, a
//! git or SKILL.md link, a folder) goes in the box at the top.

use crate::store::Store;
use crate::ui::kit::*;
use crate::ui::tokens::{self, sz};
use gpui_kit::component::{
    Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mothr_core::*;
use mothr_hub::{POPULAR_SKILLS, SkillCatalog};
use std::collections::HashMap;
use std::time::Duration;

/// Rows a list shows before asking you to narrow it down.
const MAX_ROWS: usize = 150;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Installed,
    Discover,
}

/// Something fetched in the background.
enum Load<T> {
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}

pub struct SkillsPage {
    store: Entity<Store>,
    tab: Tab,
    /// "Add from anywhere": a repo, link or folder.
    add_input: Entity<InputState>,
    /// Discover's search box (skills.sh), and the filter for an installed list / browsed repo.
    search_input: Entity<InputState>,
    query: String,
    search: Load<Vec<SkillListing>>,
    /// The repo being browsed on Discover.
    browsing: Option<&'static SkillCatalog>,
    repos: HashMap<&'static str, Load<Vec<SkillListing>>>,
    /// Skills other agents keep on this computer.
    local: Load<Vec<SkillListing>>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    _search_task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl SkillsPage {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let add_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add owner/repo, a link or a folder"));
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search skills"));
        let subs = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe_in(&add_input, window, |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.add_typed(window, cx);
                }
            }),
            cx.subscribe_in(&search_input, window, |this, s, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.query = s.read(cx).value().trim().to_string();
                    if this.tab == Tab::Discover && this.browsing.is_none() {
                        this.queue_search(cx);
                    }
                    cx.notify();
                }
            }),
        ];
        Self {
            store,
            tab: Tab::Installed,
            add_input,
            search_input,
            query: String::new(),
            search: Load::Idle,
            browsing: None,
            repos: HashMap::new(),
            local: Load::Idle,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _search_task: None,
            _subs: subs,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_input.update(cx, |i, cx| i.focus(window, cx));
    }

    /// Open on Discover (with nothing installed yet there's little else to see).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.store.read(cx).skills.is_empty() && self.tab == Tab::Installed && self.query.is_empty() {
            self.set_tab(Tab::Discover, cx);
        }
    }

    /// Show Discover, browsing one of the popular repos if `repo` names it (dev hook).
    pub fn open_discover(&mut self, repo: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.set_tab(Tab::Discover, cx);
        if let Some(q) = repo.and_then(|r| r.strip_prefix("search:")) {
            self.search_input.update(cx, |i, cx| i.set_value(q.to_string(), window, cx));
            self.query = q.to_string();
            self.queue_search(cx);
            return;
        }
        if let Some(c) = repo.and_then(|r| POPULAR_SKILLS.iter().find(|c| c.repo.eq_ignore_ascii_case(r))) {
            self.browse(c, window, cx);
        }
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        if tab == Tab::Discover {
            if matches!(self.local, Load::Idle) {
                self.load_local(cx);
            }
            if self.browsing.is_none() && !self.query.is_empty() && matches!(self.search, Load::Idle) {
                self.queue_search(cx);
            }
        }
        cx.notify();
    }

    fn dispatch(&self, command: AppCommand, window: &mut Window, cx: &mut Context<Self>) {
        self.store.update(cx, |s, cx| {
            s.dispatch(command, window, cx);
        });
    }

    fn install(&self, source: SkillSource, window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch(AppCommand::InstallSkills { source }, window, cx);
    }

    /// Install whatever was typed or pasted into the add box.
    fn add_typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.add_input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        match SkillSource::parse(&text) {
            Ok(source) => {
                self.install(source, window, cx);
                self.add_input.update(cx, |i, cx| i.set_value("", window, cx));
            }
            Err(e) => {
                use gpui_kit::component::WindowExt as _;
                window.push_notification(gpui_kit::component::notification::Notification::warning(e), cx);
            }
        }
    }

    fn choose_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: true, prompt: Some("Add skills".into()) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    for path in paths {
                        this.install(SkillSource::Folder { path }, window, cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Search skills.sh a moment after typing stops (a newer search replaces this one).
    fn queue_search(&mut self, cx: &mut Context<Self>) {
        let q = self.query.clone();
        if q.chars().count() < 2 {
            self.search = Load::Idle;
            self._search_task = None;
            return;
        }
        self.search = Load::Loading;
        let hub = self.store.read(cx).hub.clone();
        self._search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let result = match hub.search_skills(q).await {
                Ok(r) => r.map_err(|e| e.to_string()),
                Err(_) => Err("Search stopped".to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.search = match result {
                    Ok(list) => Load::Ready(list),
                    Err(e) => Load::Failed(e),
                };
                cx.notify();
            });
        }));
    }

    fn browse(&mut self, catalog: &'static SkillCatalog, window: &mut Window, cx: &mut Context<Self>) {
        self.browsing = Some(catalog);
        self.search_input.update(cx, |i, cx| i.set_value("", window, cx));
        self.query.clear();
        self.scroll.set_offset(point(px(0.), px(0.)));
        if !matches!(self.repos.get(catalog.repo), Some(Load::Ready(_)) | Some(Load::Loading)) {
            self.repos.insert(catalog.repo, Load::Loading);
            let hub = self.store.read(cx).hub.clone();
            cx.spawn(async move |this, cx| {
                let result = match hub.browse_skills(catalog.repo.to_string()).await {
                    Ok(r) => r.map_err(|e| e.to_string()),
                    Err(_) => Err("Stopped".to_string()),
                };
                let _ = this.update(cx, |this, cx| {
                    this.repos.insert(
                        catalog.repo,
                        match result {
                            Ok(list) => Load::Ready(list),
                            Err(e) => Load::Failed(e),
                        },
                    );
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn stop_browsing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.browsing = None;
        self.search_input.update(cx, |i, cx| i.set_value("", window, cx));
        self.query.clear();
        self.search = Load::Idle;
        cx.notify();
    }

    fn load_local(&mut self, cx: &mut Context<Self>) {
        self.local = Load::Loading;
        let rx = self.store.read(cx).hub.local_agent_skills();
        cx.spawn(async move |this, cx| {
            let found = rx.await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.local = Load::Ready(found);
                cx.notify();
            });
        })
        .detach();
    }

    fn set_auto(&mut self, on: bool, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let mut s = store.hub.settings();
        s.skills_auto = on;
        store.hub.update_settings(s);
        cx.notify();
    }

    // ------------------------------------------------------------------ render

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let store = self.store.read(cx);
        let auto = store.hub.settings().skills_auto;
        let dir = store.hub.skills_dir();
        v_flex()
            .gap(sz(14.))
            .child(
                h_flex()
                    .items_end()
                    .gap(sz(12.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(sz(2.))
                            .child(div().text_size(sz(tokens::text::XXL)).font_weight(FontWeight::SEMIBOLD).child("Skills"))
                            .child(
                                div()
                                    .text_size(sz(tokens::text::SM))
                                    .text_color(pal.text_faint)
                                    .child("Every agent can use these. Type /name in a chat to run one."),
                            ),
                    )
                    .child(
                        Button::new("skills-open-folder")
                            .flex_none()
                            .small()
                            .ghost()
                            .icon(icon("folder-open"))
                            .tooltip(format!("Open the library folder ({})", tildify(&dir)))
                            .on_click(move |_, _, cx| {
                                let _ = std::fs::create_dir_all(&dir);
                                cx.open_url(&format!("file://{}", dir.display()));
                            }),
                    )
                    .child(
                        Button::new("skills-rescan")
                            .flex_none()
                            .small()
                            .ghost()
                            .icon(icon("refresh-cw"))
                            .tooltip("Re-read the library (after editing skills by hand)")
                            .on_click(cx.listener(|this, _, window, cx| this.dispatch(AppCommand::RescanSkills, window, cx))),
                    ),
            )
            .child(
                h_flex()
                    .gap(sz(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .rounded(sz(tokens::radius::LG))
                            .border_1()
                            .border_color(pal.border_strong)
                            .bg(pal.surface)
                            .px(sz(4.))
                            .py(sz(2.))
                            .child(Input::new(&self.add_input).appearance(false).prefix(icon("plus").text_color(pal.text_faint))),
                    )
                    .child(Button::new("skills-add").flex_none().label("Add").on_click(cx.listener(|this, _, window, cx| this.add_typed(window, cx))))
                    .child(
                        Button::new("skills-add-folder")
                            .flex_none()
                            .ghost()
                            .icon(icon("folder-open"))
                            .label("Folder…")
                            .tooltip("Add a skill folder, or a folder of skills")
                            .on_click(cx.listener(|this, _, window, cx| this.choose_folder(window, cx))),
                    ),
            )
            .child(
                h_flex()
                    .gap(sz(8.))
                    .items_center()
                    .child(Switch::new("skills-auto").checked(auto).on_change(cx.listener(|this, v: &bool, _, cx| this.set_auto(*v, cx))))
                    .child(
                        div()
                            .text_size(sz(tokens::text::SM))
                            .text_color(pal.text_muted)
                            .child("Let agents pick a skill on their own when a request fits (new chats)"),
                    ),
            )
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let count = self.store.read(cx).skills.len();
        let installed = if count == 0 { "Installed".to_string() } else { format!("Installed · {count}") };
        h_flex()
            .gap(sz(10.))
            .items_center()
            .child(
                segment_track(&pal)
                    .child(segment("skills-tab-installed", installed, self.tab == Tab::Installed, &pal).on_click(cx.listener(|this, _, _, cx| this.set_tab(Tab::Installed, cx))))
                    .child(segment("skills-tab-discover", "Discover", self.tab == Tab::Discover, &pal).on_click(cx.listener(|this, _, _, cx| this.set_tab(Tab::Discover, cx)))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .w(sz(280.))
                    .rounded(sz(tokens::radius::MD))
                    .border_1()
                    .border_color(pal.border_strong)
                    .bg(pal.surface)
                    .px(sz(2.))
                    .child(Input::new(&self.search_input).small().appearance(false).prefix(icon("search").small().text_color(pal.text_faint)).cleanable(true)),
            )
    }

    fn render_installed(&self, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let q = self.query.to_lowercase();
        let skills: Vec<Skill> = store
            .skills
            .iter()
            .filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q) || s.description.to_lowercase().contains(&q))
            .cloned()
            .collect();
        if store.skills.is_empty() {
            return v_flex()
                .py(sz(40.))
                .items_center()
                .gap(sz(12.))
                .child(empty_state("puzzle", "No skills yet", "Find some on Discover, or add a repo or folder above.", cx))
                .child(
                    Button::new("skills-go-discover")
                        .small()
                        .icon(icon("search"))
                        .label("Discover skills")
                        .on_click(cx.listener(|this, _, _, cx| this.set_tab(Tab::Discover, cx))),
                )
                .into_any_element();
        }
        if skills.is_empty() {
            return empty_state("search", "No matches", "Try a different word.", cx).py(sz(40.)).into_any_element();
        }
        let mut list = v_flex().gap(sz(6.));
        for s in &skills {
            list = list.child(self.render_skill(s, cx));
        }
        list.into_any_element()
    }

    fn render_skill(&self, s: &Skill, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let installing = s.source.as_ref().is_some_and(|src| self.store.read(cx).skills_installing.contains(src));
        let name = s.name.clone();
        let (n1, n2, n3) = (name.clone(), name.clone(), name.clone());
        let dir = s.dir.clone();
        let updatable = s.source.is_some();
        let web = s.source.as_ref().and_then(SkillSource::web_url);
        let origin = match &s.source {
            Some(src) => src.label(),
            None => "Added by hand".to_string(),
        };
        let enabled = s.enabled;
        let weak = cx.entity().downgrade();
        h_flex()
            .id(ElementId::Name(format!("skill-{name}").into()))
            .px(sz(14.))
            .py(sz(10.))
            .gap(sz(12.))
            .items_start()
            .rounded(sz(tokens::radius::LG))
            .border_1()
            .border_color(pal.border)
            .bg(pal.surface)
            .child(
                div()
                    .flex_none()
                    .mt(sz(2.))
                    .size(sz(28.))
                    .rounded(sz(tokens::radius::MD))
                    .bg(pal.surface_hover)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("puzzle").size(sz(14.)).text_color(if enabled { pal.text_muted } else { pal.text_faint })),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(sz(3.))
                    .child(
                        h_flex()
                            .gap(sz(8.))
                            .items_center()
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(sz(tokens::text::BASE))
                                    .font_family(tokens::FONT_MONO)
                                    .text_color(if enabled { pal.text } else { pal.text_faint })
                                    .child(format!("/{name}")),
                            )
                            .when_some(s.hint.clone(), |d, h| d.child(div().flex_none().text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child(h)))
                            .when(installing, |d| d.child(div().text_size(sz(tokens::text::XS)).text_color(pal.brand).child("Updating…")))
                            .when(!enabled, |d| d.child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child("Off"))),
                    )
                    .when(!s.description.is_empty(), |d| {
                        d.child(div().text_size(sz(tokens::text::SM)).text_color(pal.text_muted).line_clamp(2).child(s.description.clone()))
                    })
                    .child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).truncate().child(origin)),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap(sz(4.))
                    .items_center()
                    .child(
                        Switch::new(ElementId::Name(format!("skill-on-{name}").into()))
                            .checked(enabled)
                            .tooltip(if enabled { "Offered to agents" } else { "Not offered to agents" })
                            .on_change(cx.listener(move |this, v: &bool, window, cx| {
                                this.dispatch(AppCommand::SetSkillEnabled { name: n1.clone(), enabled: *v }, window, cx);
                            })),
                    )
                    .child(
                        Button::new(ElementId::Name(format!("skill-more-{name}").into()))
                            .ghost()
                            .small()
                            .icon(icon("ellipsis"))
                            .tooltip("More")
                            .dropdown_menu(move |menu, _, _| {
                                let (w1, w2) = (weak.clone(), weak.clone());
                                let (n2, n3) = (n2.clone(), n3.clone());
                                let dir = dir.clone();
                                let web = web.clone();
                                menu.item(PopupMenuItem::new("Update").icon(icon("refresh-cw")).disabled(!updatable || installing).on_click(move |_, window, cx| {
                                    if let Some(p) = w1.upgrade() {
                                        p.update(cx, |p, cx| p.dispatch(AppCommand::UpdateSkill { name: n2.clone() }, window, cx));
                                    }
                                }))
                                .item(PopupMenuItem::new("Open folder").icon(icon("folder-open")).on_click(move |_, _, cx| {
                                    cx.open_url(&format!("file://{}", dir.display()));
                                }))
                                .when_some(web, |m, url| {
                                    m.item(PopupMenuItem::new("Open source page").icon(icon("external-link")).on_click(move |_, _, cx| cx.open_url(&url)))
                                })
                                .separator()
                                .item(PopupMenuItem::new("Remove").icon(icon("trash")).on_click(move |_, window, cx| {
                                    if let Some(p) = w2.upgrade() {
                                        p.update(cx, |p, cx| p.dispatch(AppCommand::RemoveSkill { name: n3.clone() }, window, cx));
                                    }
                                }))
                            }),
                    ),
            )
    }

    fn render_discover(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(catalog) = self.browsing {
            return self.render_repo(catalog, cx);
        }
        if self.query.chars().count() >= 2 {
            return self.render_search(cx);
        }
        let _ = window;
        let pal = tokens::p(cx);
        let mut grid = h_flex().w_full().flex_wrap().items_start().gap(sz(10.));
        for c in POPULAR_SKILLS {
            grid = grid.child(
                v_flex()
                    .id(ElementId::Name(format!("skills-catalog-{}", c.repo).into()))
                    .w(sz(226.))
                    .p(sz(12.))
                    .gap(sz(4.))
                    .rounded(sz(tokens::radius::LG))
                    .border_1()
                    .border_color(pal.border)
                    .bg(pal.surface)
                    .cursor_pointer()
                    .hover(|d| d.bg(pal.surface_hover).border_color(pal.border_strong))
                    .on_click(cx.listener(move |this, _, window, cx| this.browse(c, window, cx)))
                    .child(
                        h_flex()
                            .gap(sz(6.))
                            .items_center()
                            .child(icon("github").size(sz(13.)).text_color(pal.text_muted))
                            .child(div().flex_1().min_w_0().truncate().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::MEDIUM).child(c.title))
                            .child(icon("chevron-right").size(sz(13.)).text_color(pal.text_faint)),
                    )
                    .child(div().text_size(sz(tokens::text::SM)).text_color(pal.text_muted).line_clamp(2).child(c.blurb))
                    .child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).font_family(tokens::FONT_MONO).child(c.repo)),
            );
        }
        let local = match &self.local {
            Load::Ready(list) if !list.is_empty() => {
                let mut col = v_flex().gap(sz(6.));
                for l in list.iter().take(MAX_ROWS) {
                    col = col.child(self.render_listing(l, true, cx));
                }
                Some(col.into_any_element())
            }
            Load::Ready(_) => Some(
                div()
                    .text_size(sz(tokens::text::SM))
                    .text_color(pal.text_faint)
                    .child("None found in ~/.claude/skills, ~/.agents/skills, ~/.codex/skills, ~/.copilot/skills or OpenCode's skill folder.")
                    .into_any_element(),
            ),
            Load::Loading => Some(loading_row("Looking…", cx).into_any_element()),
            _ => None,
        };
        v_flex()
            .gap(sz(22.))
            .child(
                v_flex()
                    .gap(sz(10.))
                    .child(section_title("Popular collections", Some("Browse a repo, or search the open skills index (skills.sh) above"), cx))
                    .child(grid),
            )
            .when_some(local, |d, l| {
                d.child(v_flex().gap(sz(10.)).child(section_title("On this computer", Some("Skills your other agents keep; adding one copies it into Mothr's library"), cx)).child(l))
            })
            .child(
                div()
                    .text_size(sz(tokens::text::XS))
                    .text_color(pal.text_faint)
                    .child("Skills are instructions (and sometimes scripts) your agents follow with their usual permissions. Add ones from sources you trust."),
            )
            .into_any_element()
    }

    fn render_search(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        match &self.search {
            Load::Idle | Load::Loading => loading_row("Searching skills.sh…", cx).into_any_element(),
            Load::Failed(e) => div().text_size(sz(tokens::text::SM)).text_color(pal.danger).child(e.clone()).into_any_element(),
            Load::Ready(list) if list.is_empty() => empty_state("search", "Nothing found", "Try another word, or paste a repo above.", cx).py(sz(30.)).into_any_element(),
            Load::Ready(list) => {
                let mut col = v_flex().gap(sz(6.)).child(section_title(&format!("{} results", list.len()), Some("from skills.sh, most installed first"), cx));
                let mut sorted: Vec<&SkillListing> = list.iter().collect();
                sorted.sort_by_key(|l| std::cmp::Reverse(l.installs.unwrap_or(0)));
                for l in sorted {
                    col = col.child(self.render_listing(l, true, cx));
                }
                col.into_any_element()
            }
        }
    }

    fn render_repo(&self, catalog: &'static SkillCatalog, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let q = self.query.to_lowercase();
        let back = Button::new("skills-back")
            .ghost()
            .small()
            .icon(icon("arrow-left"))
            .label("Collections")
            .on_click(cx.listener(|this, _, window, cx| this.stop_browsing(window, cx)));
        let source = SkillSource::github(catalog.repo);
        let installing_all = self.store.read(cx).installing(&source);
        let body: AnyElement = match self.repos.get(catalog.repo) {
            None | Some(Load::Idle) | Some(Load::Loading) => loading_row("Listing skills…", cx).into_any_element(),
            Some(Load::Failed(e)) => div().text_size(sz(tokens::text::SM)).text_color(pal.danger).child(e.clone()).into_any_element(),
            Some(Load::Ready(list)) => {
                let shown: Vec<&SkillListing> = list
                    .iter()
                    .filter(|l| q.is_empty() || l.name.to_lowercase().contains(&q) || l.description.as_deref().is_some_and(|d| d.to_lowercase().contains(&q)))
                    .collect();
                let total = list.len();
                let mut col = v_flex().gap(sz(6.));
                for l in shown.iter().take(MAX_ROWS) {
                    col = col.child(self.render_listing(l, false, cx));
                }
                if shown.len() > MAX_ROWS {
                    col = col.child(
                        div()
                            .py(sz(6.))
                            .text_size(sz(tokens::text::SM))
                            .text_color(pal.text_faint)
                            .child(format!("{} more — type in the search box to narrow them down", shown.len() - MAX_ROWS)),
                    );
                }
                if shown.is_empty() {
                    col = col.child(empty_state("search", "No matches", "Try a different word.", cx).py(sz(30.)));
                }
                v_flex()
                    .gap(sz(10.))
                    .child(
                        h_flex()
                            .gap(sz(8.))
                            .items_center()
                            .child(div().flex_1().text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child(format!("{total} skills")))
                            .when(total > 1 && total <= 60, |d| {
                                d.child(
                                    Button::new("skills-add-all")
                                        .small()
                                        .icon(icon("download"))
                                        .label(if installing_all { "Adding…".to_string() } else { format!("Add all {total}") })
                                        .disabled(installing_all)
                                        .on_click(cx.listener(move |this, _, window, cx| this.install(SkillSource::github(catalog.repo), window, cx))),
                                )
                            }),
                    )
                    .child(col)
                    .into_any_element()
            }
        };
        let repo_url = format!("https://github.com/{}", catalog.repo);
        v_flex()
            .gap(sz(14.))
            .child(
                h_flex()
                    .gap(sz(10.))
                    .items_center()
                    .child(back)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_size(sz(tokens::text::LG)).font_weight(FontWeight::SEMIBOLD).child(catalog.title))
                            .child(div().text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child(catalog.blurb)),
                    )
                    .child(
                        Button::new("skills-repo-link")
                            .ghost()
                            .small()
                            .icon(icon("external-link"))
                            .label(catalog.repo)
                            .on_click(move |_, _, cx| cx.open_url(&repo_url)),
                    ),
            )
            .child(body)
            .into_any_element()
    }

    /// A skill you could add.
    fn render_listing(&self, l: &SkillListing, show_origin: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let store = self.store.read(cx);
        let slug = skill_slug(&l.name);
        let installed = store.skills.iter().any(|s| s.name == slug);
        let installing = store.installing(&l.source);
        let source = l.source.clone();
        let key = l.source.key();
        let web = l.source.web_url().or_else(|| match &l.source {
            SkillSource::GitHub { repo, skill: Some(skill), .. } => Some(format!("https://skills.sh/{repo}/{skill}")),
            _ => None,
        });
        let meta = match l.installs {
            Some(n) if n > 0 => format!("{} · {} installs", l.origin, compact_count(n)),
            _ if show_origin => l.origin.clone(),
            _ => String::new(),
        };
        h_flex()
            .id(ElementId::Name(format!("listing-{key}").into()))
            .px(sz(14.))
            .py(sz(9.))
            .gap(sz(12.))
            .items_center()
            .rounded(sz(tokens::radius::LG))
            .border_1()
            .border_color(pal.border)
            .bg(pal.surface)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(sz(2.))
                    .child(div().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::MEDIUM).truncate().child(l.name.clone()))
                    .when_some(l.description.clone(), |d, desc| d.child(div().text_size(sz(tokens::text::SM)).text_color(pal.text_muted).line_clamp(2).child(desc)))
                    .when(!meta.is_empty(), |d| d.child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).truncate().child(meta))),
            )
            .when_some(web, |d, url| {
                d.child(
                    Button::new(ElementId::Name(format!("listing-web-{key}").into()))
                        .ghost()
                        .small()
                        .icon(icon("external-link"))
                        .tooltip("Open its page")
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
            })
            .child(if installing {
                Button::new(ElementId::Name(format!("listing-add-{key}").into())).small().label("Adding…").disabled(true).into_any_element()
            } else if installed {
                h_flex()
                    .flex_none()
                    .gap(sz(4.))
                    .px(sz(8.))
                    .items_center()
                    .text_size(sz(tokens::text::SM))
                    .text_color(pal.success)
                    .child(icon("check").size(sz(13.)))
                    .child("Added")
                    .into_any_element()
            } else {
                Button::new(ElementId::Name(format!("listing-add-{key}").into()))
                    .small()
                    .icon(icon("plus"))
                    .label("Add")
                    .on_click(cx.listener(move |this, _, window, cx| this.install(source.clone(), window, cx)))
                    .into_any_element()
            })
    }
}

/// "12.3K".
fn compact_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}K", n as f64 / 1_000.0).replace(".0K", "K"),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0).replace(".0M", "M"),
    }
}

fn section_title(title: &str, note: Option<&str>, cx: &App) -> impl IntoElement + use<> {
    let pal = tokens::p(cx);
    h_flex()
        .gap(sz(8.))
        .items_baseline()
        .child(div().text_size(sz(tokens::text::SM)).font_weight(FontWeight::SEMIBOLD).text_color(pal.text).child(title.to_string()))
        .when_some(note.map(str::to_string), |d, n| d.child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child(n)))
}

fn loading_row(text: &'static str, cx: &App) -> impl IntoElement + use<> {
    let pal = tokens::p(cx);
    h_flex()
        .py(sz(10.))
        .gap(sz(8.))
        .items_center()
        .text_size(sz(tokens::text::SM))
        .text_color(pal.text_faint)
        .child(pulse_dot("skills-loading", pal.brand))
        .child(text)
}

impl Render for SkillsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.tab {
            Tab::Installed => self.render_installed(cx),
            Tab::Discover => self.render_discover(window, cx),
        };
        v_flex()
            .id("skills-page")
            .key_context("SkillsPage")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                v_flex()
                    .w_full()
                    .max_w(sz(920.))
                    .mx_auto()
                    .px(sz(24.))
                    .pt(sz(28.))
                    .pb(sz(40.))
                    .gap(sz(18.))
                    .child(self.render_header(cx))
                    .child(self.render_tabs(cx))
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compact_counts() {
        assert_eq!(super::compact_count(999), "999");
        assert_eq!(super::compact_count(12_345), "12.3K");
        assert_eq!(super::compact_count(1_000), "1K");
        assert_eq!(super::compact_count(2_500_000), "2.5M");
    }
}
