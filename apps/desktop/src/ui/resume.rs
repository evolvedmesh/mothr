//! Resume picker (Ctrl+Shift+R): the conversations every agent keeps on its own (started in its
//! CLI, another app, or a Mothr chat since deleted), to pick one up as a Mothr chat.

use super::kit::*;
use super::tokens::{self, sz};
use crate::store::Store;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    h_flex,
    input::{Input, InputEvent, InputState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mothr_core::*;
use std::collections::BTreeMap;

actions!(resume_picker, [SelectNext, SelectPrev, Confirm, Dismiss]);

pub enum ResumeEvent {
    Picked(PastChat),
    Dismiss,
}

impl EventEmitter<ResumeEvent> for ResumePicker {}

/// What one agent answered.
enum Listing {
    Loading,
    Done(Vec<PastChat>),
    Failed(String),
}

pub struct ResumePicker {
    input: Entity<InputState>,
    agents: Vec<AgentKind>,
    listings: BTreeMap<AgentKind, Listing>,
    /// Only this agent's chats; None = every agent.
    filter: Option<AgentKind>,
    hits: Vec<PastChat>,
    selected: usize,
    scroll: ScrollHandle,
    _subs: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, Some("ResumePicker")),
        KeyBinding::new("up", SelectPrev, Some("ResumePicker")),
        KeyBinding::new("ctrl-n", SelectNext, Some("ResumePicker")),
        KeyBinding::new("ctrl-p", SelectPrev, Some("ResumePicker")),
        KeyBinding::new("enter", Confirm, Some("ResumePicker")),
        KeyBinding::new("escape", Dismiss, Some("ResumePicker")),
    ]);
}

impl ResumePicker {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search past chats by title or repo…"));
        let subs = vec![cx.subscribe(&input, |this, _, ev: &InputEvent, cx| match ev {
            InputEvent::Change => this.refilter(cx),
            InputEvent::PressEnter { .. } => this.confirm(cx),
            _ => {}
        })];
        let agents = store.read(cx).available_agents();
        let mut listings = BTreeMap::new();
        let mut tasks = vec![];
        // Each agent answers on its own time (ACP agents have to start first), so each fills in
        // as soon as it has.
        for agent in agents.iter().copied() {
            listings.insert(agent, Listing::Loading);
            let hub = store.read(cx).hub.clone();
            tasks.push(cx.spawn(async move |this, cx| {
                let result = cx.background_spawn(async move { mothr_core::HubApi::past_chats(&*hub, agent) }).await;
                let _ = this.update(cx, |this, cx| {
                    this.listings.insert(
                        agent,
                        match result {
                            Ok(chats) => Listing::Done(chats),
                            Err(e) => Listing::Failed(e.to_string()),
                        },
                    );
                    this.refilter(cx);
                });
            }));
        }
        let mut this = Self { input, agents, listings, filter: None, hits: vec![], selected: 0, scroll: ScrollHandle::new(), _subs: subs, _tasks: tasks };
        this.refilter(cx);
        this
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.focus(window, cx));
    }

    fn set_filter(&mut self, filter: Option<AgentKind>, cx: &mut Context<Self>) {
        self.filter = filter;
        self.refilter(cx);
    }

    /// Every chat whose title or folder holds all the words typed, newest first.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        let q = self.input.read(cx).value().to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        let mut hits: Vec<PastChat> = self
            .listings
            .iter()
            .filter(|(a, _)| self.filter.is_none_or(|f| f == **a))
            .filter_map(|(_, l)| match l {
                Listing::Done(chats) => Some(chats),
                _ => None,
            })
            .flatten()
            .filter(|c| {
                let hay = format!("{} {} {}", c.title.to_lowercase(), tildify(&c.cwd).to_lowercase(), c.agent.short_name().to_lowercase());
                words.iter().all(|w| hay.contains(w))
            })
            .cloned()
            .collect();
        hits.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
        hits.truncate(300);
        self.hits = hits;
        self.selected = self.selected.min(self.hits.len().saturating_sub(1));
        cx.notify();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.hits.get(self.selected) {
            cx.emit(ResumeEvent::Picked(c.clone()));
        }
    }

    fn move_selection(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.hits.is_empty() {
            return;
        }
        let n = self.hits.len() as isize;
        self.selected = ((self.selected as isize + step).rem_euclid(n)) as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    fn render_filters(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let chip = |id: SharedString, label: String, active: bool, agent: Option<AgentKind>, loading: bool| {
            h_flex()
                .id(ElementId::Name(id))
                .flex_none()
                .gap(sz(6.))
                .px(sz(9.))
                .h(sz(26.))
                .items_center()
                .rounded(sz(tokens::radius::MD))
                .text_size(sz(tokens::text::SM))
                .cursor_pointer()
                .map(|d| if active { d.bg(pal.surface_hover).text_color(pal.text) } else { d.text_color(pal.text_muted).hover(|d| d.text_color(pal.text)) })
                .when_some(agent, |d, a| d.child(agent_avatar(a, 16.)))
                .child(label)
                .when(loading, |d| d.child(Spinner::new().xsmall()))
        };
        let mut row = h_flex().px_2().py(sz(6.)).gap(sz(2.)).flex_wrap().border_b_1().border_color(pal.border);
        row = row.child(chip("rf-all".into(), "All agents".into(), self.filter.is_none(), None, false).on_click(cx.listener(|this, _, _, cx| this.set_filter(None, cx))));
        for a in self.agents.iter().copied() {
            let loading = matches!(self.listings.get(&a), Some(Listing::Loading));
            row = row.child(
                chip(format!("rf-{}", a.id()).into(), a.short_name().to_string(), self.filter == Some(a), Some(a), loading)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_filter(Some(a), cx))),
            );
        }
        row
    }
}

impl Render for ResumePicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = tokens::p(cx);
        let theme = cx.theme().clone();
        let loading: Vec<AgentKind> = self.listings.iter().filter(|(a, l)| matches!(l, Listing::Loading) && self.filter.is_none_or(|f| f == **a)).map(|(a, _)| *a).collect();
        let failed: Vec<(AgentKind, String)> = self
            .listings
            .iter()
            .filter(|(a, _)| self.filter.is_none_or(|f| f == **a))
            .filter_map(|(a, l)| match l {
                Listing::Failed(e) => Some((*a, e.clone())),
                _ => None,
            })
            .collect();
        let mut list = v_flex().gap(sz(1.)).p_1();
        for (i, c) in self.hits.iter().enumerate() {
            let selected = i == self.selected;
            let pick = c.clone();
            let place = if dirs_home().as_deref() == Some(c.cwd.as_path()) { "General".to_string() } else { tildify(&c.cwd) };
            list = list.child(
                h_flex()
                    .id(ElementId::Name(format!("past-{}-{}", c.agent.id(), c.id).into()))
                    .px_3()
                    .py_2()
                    .gap_3()
                    .rounded(sz(8.))
                    .items_center()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.list_active))
                    .hover(|d| d.bg(theme.list_hover))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(ResumeEvent::Picked(pick.clone()))))
                    .child(agent_avatar(c.agent, 26.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().truncate().child(c.title.clone()))
                            .child(div().text_xs().truncate().text_color(theme.muted_foreground).child(place)),
                    )
                    .when(c.chat.is_some(), |d| {
                        d.child(div().flex_none().text_xs().px(sz(6.)).py(sz(1.)).rounded(sz(4.)).bg(theme.muted).text_color(theme.muted_foreground).child("in Mothr"))
                    })
                    .when_some(c.updated_at, |d, t| d.child(div().flex_none().text_xs().text_color(pal.text_faint).child(relative_time(t)))),
            );
        }
        if self.hits.is_empty() {
            let text = if !loading.is_empty() {
                "Looking for past chats…".to_string()
            } else if self.input.read(cx).value().is_empty() {
                "No past chats found.".to_string()
            } else {
                "No past chats match.".to_string()
            };
            list = list.child(div().p_6().text_sm().text_color(theme.muted_foreground).text_center().child(text));
        }
        let status = if !loading.is_empty() {
            format!("Asking {}…", loading.iter().map(|a| a.short_name()).collect::<Vec<_>>().join(", "))
        } else {
            format!("{} chat{}", self.hits.len(), if self.hits.len() == 1 { "" } else { "s" })
        };

        v_flex()
            .key_context("ResumePicker")
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &Confirm, _, cx| this.confirm(cx)))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(ResumeEvent::Dismiss)))
            .w(sz(680.))
            .max_h(sz(560.))
            .rounded(sz(tokens::radius::XL))
            .border_1()
            .border_color(pal.border_strong)
            .bg(pal.raised)
            .shadow(tokens::shadow_lg(&pal))
            .overflow_hidden()
            .child(
                h_flex()
                    .p_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1().child(Input::new(&self.input).appearance(false).prefix(icon("search").small()))),
            )
            .child(self.render_filters(cx))
            .child(div().id("past-hits").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.scroll).child(list))
            .children(failed.into_iter().map(|(a, e)| {
                h_flex()
                    .px_3()
                    .py(sz(6.))
                    .gap_2()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(pal.text_muted)
                    .child(icon("info").size(sz(12.)))
                    .child(div().truncate().child(format!("{}: {e}", a.display_name())))
            }))
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
                    .child(h_flex().gap_1().child(kbd_hint("↵", cx)).child("resume"))
                    .child(h_flex().gap_1().child(kbd_hint("esc", cx)).child("close"))
                    .child(div().flex_1())
                    .child(status),
            )
            .with_animation(
                "resume-in",
                Animation::new(std::time::Duration::from_millis(180)).with_easing(ease_out_quint()),
                |d, t| d.opacity(t).mt(sz((1.0 - t) * -10.0)),
            )
    }
}

fn dirs_home() -> Option<std::path::PathBuf> {
    Some(mothr_core::config::home_dir())
}
