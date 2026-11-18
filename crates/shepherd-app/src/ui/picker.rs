//! Fuzzy project finder (Ctrl+P): instant search over every project on disk.

use super::kit::*;
use super::tokens;
use crate::store::Store;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::*;
use super::tokens::sz;
use gpui_kit::prelude::FluentBuilder as _;
use shepherd_core::*;

actions!(picker, [SelectNext, SelectPrev, Confirm, Dismiss]);

pub enum PickerEvent {
    Picked(ProjectRef),
    /// Start a general chat that isn't tied to a project.
    General,
    Dismiss,
}

impl EventEmitter<PickerEvent> for ProjectPicker {}

pub struct ProjectPicker {
    store: Entity<Store>,
    input: Entity<InputState>,
    hits: Vec<ProjectHit>,
    selected: usize,
    scroll: ScrollHandle,
    _subs: Vec<Subscription>,
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, Some("ProjectPicker")),
        KeyBinding::new("up", SelectPrev, Some("ProjectPicker")),
        KeyBinding::new("ctrl-n", SelectNext, Some("ProjectPicker")),
        KeyBinding::new("ctrl-p", SelectPrev, Some("ProjectPicker")),
        KeyBinding::new("enter", Confirm, Some("ProjectPicker")),
        KeyBinding::new("escape", Dismiss, Some("ProjectPicker")),
    ]);
}

impl ProjectPicker {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search projects by name or path…"));
        let subs = vec![cx.subscribe(&input, |this, input, ev: &InputEvent, cx| match ev {
            InputEvent::Change => {
                let q = input.read(cx).value().to_string();
                this.search(&q, cx);
            }
            InputEvent::PressEnter { .. } => this.confirm(cx),
            _ => {}
        })];
        let mut this = Self { store, input, hits: vec![], selected: 0, scroll: ScrollHandle::new(), _subs: subs };
        this.search("", cx);
        this
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.focus(window, cx));
    }

    fn search(&mut self, q: &str, cx: &mut Context<Self>) {
        self.hits = self.store.read(cx).hub.search_projects(q, 60);
        self.selected = 0;
        self.scroll.scroll_to_item(0);
        cx.notify();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(h) = self.hits.get(self.selected) {
            cx.emit(PickerEvent::Picked(h.project.clone()));
        }
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Open project".into()) });
        let _ = window;
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| {
                        let idx = this.store.read(cx).hub.projects().clone();
                        let p = idx.add_manual(path.clone()).unwrap_or_else(|| ProjectRef::from_path(path));
                        cx.emit(PickerEvent::Picked(p));
                    });
                }
        })
        .detach();
    }
}

impl Render for ProjectPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let count = self.store.read(cx).project_count;
        let mut list = v_flex().gap(sz(1.)).p_1();
        list = list.child(
            h_flex()
                .id("hit-general")
                .px_3()
                .py_2()
                .gap_3()
                .rounded(sz(8.))
                .items_center()
                .cursor_pointer()
                .hover(|d| d.bg(theme.list_hover))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(PickerEvent::General)))
                .child(
                    div()
                        .size(sz(30.))
                        .rounded(sz(8.))
                        .bg(theme.primary.opacity(0.12))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon("globe").size(sz(15.)).text_color(theme.primary)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .child(div().text_sm().child("General chat"))
                        .child(div().text_xs().text_color(theme.muted_foreground).child("Not tied to a project — questions, quick tasks, files anywhere")),
                ),
        );
        for (i, h) in self.hits.iter().enumerate() {
            let selected = i == self.selected;
            let p = h.project.clone();
            let path = tildify(&p.path);
            let tags = p.tags.iter().filter(|t| *t != "git").take(3).cloned().collect::<Vec<_>>();
            list = list.child(
                h_flex()
                    .id(ElementId::Name(format!("hit-{}", p.path.display()).into()))
                    .px_3()
                    .py_2()
                    .gap_3()
                    .rounded(sz(8.))
                    .items_center()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.list_active))
                    .hover(|d| d.bg(theme.list_hover))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(PickerEvent::Picked(p.clone()))))
                    .child(
                        div()
                            .size(sz(30.))
                            .rounded(sz(8.))
                            .bg(theme.muted)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(if h.project.pinned { "pin" } else { "folder-git-2" }).size(sz(15.)).text_color(theme.muted_foreground)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().child(highlighted(&h.project.name, &h.name_indices, theme.foreground, theme.primary)))
                            .child(div().text_xs().child(highlighted(&path, &h.path_indices, theme.muted_foreground, theme.primary))),
                    )
                    .children(tags.into_iter().map(|t| {
                        div().text_xs().px(sz(6.)).py(sz(1.)).rounded(sz(4.)).bg(theme.muted).text_color(theme.muted_foreground).child(t)
                    }))
                    .when_some(h.project.git_branch.clone(), |d, b| {
                        d.child(
                            h_flex()
                                .gap_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(icon("git-branch").size(sz(12.)))
                                .child(b),
                        )
                    }),
            );
        }
        if self.hits.is_empty() {
            list = list.child(div().p_6().text_sm().text_color(theme.muted_foreground).text_center().child("No projects match. Use Browse… to open any folder."));
        }

        v_flex()
            .key_context("ProjectPicker")
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| {
                if !this.hits.is_empty() {
                    this.selected = (this.selected + 1) % this.hits.len();
                    this.scroll.scroll_to_item(this.selected);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| {
                if !this.hits.is_empty() {
                    this.selected = (this.selected + this.hits.len() - 1) % this.hits.len();
                    this.scroll.scroll_to_item(this.selected);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.confirm(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(PickerEvent::Dismiss)))
            .w(sz(640.))
            .max_h(sz(520.))
            .rounded(sz(tokens::radius::XL))
            .border_1()
            .border_color(tokens::p(cx).border_strong)
            .bg(tokens::p(cx).raised)
            .shadow(tokens::shadow_lg(&tokens::p(cx)))
            .overflow_hidden()
            .child(
                h_flex()
                    .p_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1().child(Input::new(&self.input).appearance(false).prefix(icon("search").small())))
                    .child(
                        Button::new("browse")
                            .ghost()
                            .small()
                            .icon(icon("folder-open"))
                            .label("Browse…")
                            .on_click(cx.listener(|this, _, window, cx| this.browse(window, cx))),
                    ),
            )
            .child(div().id("hits").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.scroll).child(list))
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_3()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(h_flex().gap_1().child(kbd_hint("↑↓", cx)).child("navigate"))
                    .child(h_flex().gap_1().child(kbd_hint("↵", cx)).child("start chat here"))
                    .child(h_flex().gap_1().child(kbd_hint("esc", cx)).child("close"))
                    .child(div().flex_1())
                    .child(match count {
                        Some(n) => format!("{n} projects indexed"),
                        None => "Indexing…".into(),
                    }),
            )
            .with_animation(
                "picker-in",
                Animation::new(std::time::Duration::from_millis(180)).with_easing(ease_out_quint()),
                |d, t| d.opacity(t).mt(sz((1.0 - t) * -10.0)),
            )
    }
}
