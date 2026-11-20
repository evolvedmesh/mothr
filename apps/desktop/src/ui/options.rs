//! The run-settings chip: agent, model, reasoning effort and context window behind one button,
//! shared by the new-chat box and the chat header.

use super::kit::*;
use super::tokens::{self, sz};
use gpui_kit::component::{
    h_flex,
    button::{Button, ButtonVariants as _},
    popover::Popover,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mothr_core::{AgentInstall, AgentKind, ChoiceInfo, ModelInfo, effort_label};
use std::rc::Rc;

/// What `model` can be set to. With no model chosen (the agent's default) the entry the agent
/// calls "default" is used, else the first model that has the setting.
pub fn settings_for(models: &[ModelInfo], model: Option<&str>) -> (Vec<String>, Vec<ChoiceInfo>) {
    let chosen = model.and_then(|m| models.iter().find(|x| x.id == m));
    let fallback = || models.iter().find(|x| x.id == "default");
    let efforts = chosen
        .or_else(fallback)
        .filter(|m| !m.efforts.is_empty())
        .or_else(|| if model.is_none() { models.iter().find(|m| !m.efforts.is_empty()) } else { None })
        .map(|m| m.efforts.clone())
        .unwrap_or_default();
    let contexts = chosen
        .or_else(fallback)
        .filter(|m| !m.contexts.is_empty())
        .or_else(|| if model.is_none() { models.iter().find(|m| !m.contexts.is_empty()) } else { None })
        .map(|m| m.contexts.clone())
        .unwrap_or_default();
    (efforts, contexts)
}


/// Applies one [`Pick`] to whatever owns the chip.
type OnPick = Rc<dyn Fn(Pick, &mut Window, &mut App)>;

/// Everything the run-settings chip shows and lets you change.
#[derive(Clone)]
pub struct RunSettings {
    /// Agents to choose from; empty when the agent is fixed (a running chat).
    pub agents: Vec<AgentInstall>,
    pub agent: Option<AgentKind>,
    pub models: Vec<ModelInfo>,
    pub model: Option<String>,
    pub efforts: Vec<String>,
    pub effort: Option<String>,
    pub contexts: Vec<ChoiceInfo>,
    pub context: Option<String>,
    /// New chats can go back to the agent's default; a running chat can't unset a value.
    pub allow_default: bool,
    /// Short on room: show only the icon and put the summary in the tooltip.
    pub compact: bool,
}

/// One change made in the run-settings popover. `None` means the agent's own default.
#[derive(Clone)]
pub enum Pick {
    Agent(AgentKind),
    Model(Option<String>),
    Effort(Option<String>),
    Context(Option<String>),
}

impl RunSettings {
    fn model_name(&self) -> Option<String> {
        let m = self.model.as_ref()?;
        Some(self.models.iter().find(|x| &x.id == m).map(|x| x.name.clone()).unwrap_or_else(|| m.clone()))
    }

    fn context_label(&self) -> Option<String> {
        let c = self.context.as_ref()?;
        Some(self.contexts.iter().find(|x| &x.id == c).map(|x| x.label.clone()).unwrap_or_else(|| c.clone()))
    }

    /// "Claude Code · Opus 5.5 · High", plus the context window when one is picked.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = vec![];
        if !self.agents.is_empty() {
            parts.push(self.agent.map(|k| k.display_name().to_string()).unwrap_or_else(|| "No agent".into()));
        }
        match self.model_name() {
            Some(m) => parts.push(m),
            None if self.agents.is_empty() => parts.push("Default model".into()),
            None => {}
        }
        if let Some(e) = &self.effort {
            parts.push(effort_label(e));
        }
        if let Some(c) = self.context_label() {
            parts.push(c);
        }
        parts.join(" · ")
    }
}

/// The chip and its popover. `set` gets each change; the popover stays open so several
/// settings can be changed in one go.
pub fn run_settings<T: 'static>(
    id: &'static str,
    s: RunSettings,
    weak: WeakEntity<T>,
    set: impl Fn(&mut T, Pick, &mut Window, &mut Context<T>) + Clone + 'static,
) -> impl IntoElement {
    let summary = s.summary();
    let compact = s.compact;
    let trigger = Button::new(id)
        .ghost()
        .map(|b| match (s.agents.is_empty(), s.agent) {
            (false, Some(k)) => b.icon(agent_logo(k)),
            _ => b.icon(icon("brain")),
        })
        .when(compact, |b| b.tooltip(summary.clone()))
        .when(!compact, |b| b.label(summary.clone()).dropdown_caret(true).tooltip("Agent, model and reasoning"));
    let s = Rc::new(s);
    let pick: OnPick = Rc::new(move |p, window, cx| {
        if let Some(e) = weak.upgrade() {
            e.update(cx, |t, cx| set(t, p, window, cx));
        }
    });
    Popover::new(SharedString::from(format!("{id}-popover")))
        .anchor(Anchor::TopRight)
        .trigger(trigger)
        .content(move |_, _, cx| panel(&s, &pick, cx))
}

fn panel(s: &RunSettings, pick: &OnPick, cx: &App) -> AnyElement {
    let pal = tokens::p(cx);
    let with_agents = !s.agents.is_empty();

    let agents = with_agents.then(|| {
        let mut col = v_flex().w(sz(176.)).flex_none().gap(sz(1.)).child(section("Agent", &pal));
        for a in s.agents.iter().filter(|a| a.available()) {
            let kind = a.kind;
            let p = pick.clone();
            col = col.child(
                row(SharedString::from(format!("rs-agent-{}", kind.display_name())), Some(agent_logo(kind)), kind.display_name(), s.agent == Some(kind), &pal)
                    .on_click(move |_, window, cx| p(Pick::Agent(kind), window, cx)),
            );
        }
        col
    });

    let mut models = v_flex().id("rs-models").flex_1().min_w_0().max_h(sz(264.)).overflow_y_scroll().gap(sz(1.));
    if s.allow_default {
        let p = pick.clone();
        models = models.child(row("rs-model-default", None, "Agent default", s.model.is_none(), &pal).on_click(move |_, window, cx| p(Pick::Model(None), window, cx)));
    }
    for m in &s.models {
        let p = pick.clone();
        let id = m.id.clone();
        let tip = m.description.clone();
        models = models.child(
            row(SharedString::from(format!("rs-model-{}", m.id)), None, m.name.clone(), s.model.as_deref() == Some(m.id.as_str()), &pal)
                .when_some(tip, |r, tip| r.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)))
                .on_click(move |_, window, cx| p(Pick::Model(Some(id.clone())), window, cx)),
        );
    }
    if s.models.is_empty() {
        models = models.child(div().px(sz(8.)).py(sz(6.)).text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child("Loading models…"));
    }
    let models = v_flex().flex_1().min_w_0().gap(sz(1.)).child(section("Model", &pal)).child(models);

    let top = h_flex()
        .items_start()
        .gap(sz(8.))
        .when_some(agents, |d, a| d.child(a).child(div().w(px(1.)).self_stretch().bg(pal.border)))
        .child(models);

    let mut knobs = v_flex().gap(sz(8.));
    if !s.efforts.is_empty() {
        let mut opts: Vec<(Option<String>, String)> = vec![];
        if s.allow_default {
            opts.push((None, "Default".into()));
        }
        opts.extend(s.efforts.iter().map(|e| (Some(e.clone()), short_effort(e))));
        knobs = knobs.child(knob("Reasoning", "rs-effort", opts, s.effort.clone(), pick, Pick::Effort, &pal));
    }
    if !s.contexts.is_empty() {
        let mut opts: Vec<(Option<String>, String)> = vec![];
        if s.allow_default {
            opts.push((None, "Default".into()));
        }
        opts.extend(s.contexts.iter().map(|c| (Some(c.id.clone()), c.label.clone())));
        knobs = knobs.child(knob("Context", "rs-context", opts, s.context.clone(), pick, Pick::Context, &pal));
    }
    let has_knobs = !s.efforts.is_empty() || !s.contexts.is_empty();

    v_flex()
        .w(sz(if with_agents { 468. } else { 360. }))
        .gap(sz(10.))
        .child(top)
        .when(has_knobs, |d| d.child(div().h(px(1.)).bg(pal.border)).child(knobs))
        .into_any_element()
}

/// Small heading over a column of the popover.
fn section(label: &'static str, pal: &tokens::Palette) -> Div {
    div().px(sz(8.)).pb(sz(4.)).text_size(sz(tokens::text::XS)).font_weight(FontWeight::MEDIUM).text_color(pal.text_faint).child(label)
}

/// One choice in a column: a check marks the current one.
fn row(id: impl Into<ElementId>, lead: Option<gpui_kit::component::Icon>, label: impl Into<SharedString>, selected: bool, pal: &tokens::Palette) -> Stateful<Div> {
    let pal = *pal;
    h_flex()
        .id(id)
        .flex_none()
        .h(sz(30.))
        .px(sz(8.))
        .gap(sz(8.))
        .items_center()
        .rounded(sz(tokens::radius::MD))
        .text_size(sz(tokens::text::BASE))
        .cursor_pointer()
        .hover(move |d| d.bg(pal.surface_hover))
        .when(selected, |d| d.text_color(pal.text).font_weight(FontWeight::MEDIUM))
        .when(!selected, |d| d.text_color(pal.text_muted))
        .when_some(lead, |d, i| d.child(i.size(sz(15.))))
        .child(div().flex_1().min_w_0().truncate().child(label.into()))
        .child(div().flex_none().w(sz(14.)).when(selected, |d| d.child(icon("check").size(sz(14.)).text_color(pal.brand))))
}

/// A labelled segmented control for one setting.
fn knob(
    label: &'static str,
    id: &'static str,
    opts: Vec<(Option<String>, String)>,
    current: Option<String>,
    pick: &OnPick,
    wrap: fn(Option<String>) -> Pick,
    pal: &tokens::Palette,
) -> Div {
    let mut track = segment_track(pal);
    for (i, (value, text)) in opts.into_iter().enumerate() {
        let p = pick.clone();
        let active = value == current;
        track = track.child(segment(SharedString::from(format!("{id}-{i}")), text, active, pal).on_click(move |_, window, cx| p(wrap(value.clone()), window, cx)));
    }
    h_flex()
        .px(sz(8.))
        .gap(sz(12.))
        .items_center()
        .child(div().w(sz(72.)).flex_none().text_size(sz(tokens::text::SM)).text_color(pal.text_muted).child(label))
        .child(track)
}

/// Effort names short enough for a segment.
fn short_effort(e: &str) -> String {
    match e {
        "xhigh" => "X-high".into(),
        "medium" => "Med".into(),
        "minimal" => "Min".into(),
        other => effort_label(other),
    }
}
