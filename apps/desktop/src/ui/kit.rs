//! Small shared visual building blocks: icons, agent avatars, status dots, meters.

use gpui_kit::component::{ActiveTheme as _, Icon, IconNamed, h_flex, v_flex};
use gpui_kit::*;
use super::tokens::sz;
use gpui_kit::prelude::FluentBuilder as _;
use mothr_core::*;
use std::time::Duration;

/// Any Lucide icon by name (the app embeds the full catalog).
#[derive(Clone, Copy)]
pub struct Lucide(pub &'static str);

impl IconNamed for Lucide {
    fn path(self) -> SharedString {
        format!("icons/{}.svg", self.0).into()
    }
}

pub fn icon(name: &'static str) -> Icon {
    Icon::new(Lucide(name))
}

/// Icon by runtime name (design canvas); falls back to a dot.
pub fn icon_dyn(name: &str) -> Svg {
    svg().path(SharedString::from(format!("icons/{}.svg", name.trim().to_ascii_lowercase().replace(' ', "-"))))
}

pub fn hex(c: u32) -> Hsla {
    rgb(c).into()
}

pub fn rgba_u32(c: u32) -> Hsla {
    rgba(c).into()
}

pub fn agent_color(kind: AgentKind) -> Hsla {
    hex(kind.accent())
}

/// An agent's real logo (assets/logos), drawn in the current text color unless tinted.
#[derive(Clone, Copy)]
pub struct AgentLogo(pub AgentKind);

impl IconNamed for AgentLogo {
    fn path(self) -> SharedString {
        format!("logos/{}.svg", self.0.id()).into()
    }
}

pub fn agent_logo(kind: AgentKind) -> Icon {
    Icon::new(AgentLogo(kind))
}

/// Rounded square avatar with the agent's logo on a soft tile of its brand color.
pub fn agent_avatar(kind: AgentKind, size: f32) -> Div {
    let c = agent_color(kind);
    div()
        .flex_none()
        .size(sz(size))
        .rounded(sz(size * 0.3))
        .bg(c.opacity(0.14))
        .border_1()
        .border_color(c.opacity(0.30))
        .flex()
        .items_center()
        .justify_center()
        .child(agent_logo(kind).size(sz(size * 0.6)).text_color(c))
}

pub fn status_color(status: &SessionStatus, cx: &App) -> Hsla {
    let t = cx.theme();
    match status {
        SessionStatus::Working | SessionStatus::Starting => t.primary,
        SessionStatus::NeedsInput => hex(0xF59E0B),
        SessionStatus::Error(_) => t.danger,
        SessionStatus::Idle => t.success,
        SessionStatus::Ended => t.muted_foreground.opacity(0.5),
    }
}

/// A small status dot; pulses while the agent works or waits on the user.
pub fn status_dot(id: impl Into<ElementId>, status: &SessionStatus, cx: &App) -> AnyElement {
    let color = status_color(status, cx);
    let dot = div().size(sz(7.)).rounded_full().bg(color);
    match status {
        SessionStatus::Working | SessionStatus::Starting | SessionStatus::NeedsInput => dot
            .with_animation(
                id,
                Animation::new(Duration::from_millis(1400)).repeat().with_easing(pulsating_between(0.35, 1.0)),
                |d, t| d.opacity(t),
            )
            .into_any_element(),
        _ => dot.into_any_element(),
    }
}

pub fn level_color(level: UsageLevel, cx: &App) -> Hsla {
    match level {
        UsageLevel::Ok => cx.theme().success,
        UsageLevel::Warn => hex(0xF59E0B),
        UsageLevel::Critical | UsageLevel::Exhausted => cx.theme().danger,
    }
}

/// Thin horizontal meter.
pub fn meter(fraction: f32, color: Hsla, cx: &App) -> Div {
    let f = fraction.clamp(0.0, 1.0);
    div()
        .h(sz(5.))
        .w_full()
        .rounded_full()
        .bg(cx.theme().muted)
        .overflow_hidden()
        .child(div().h_full().rounded_full().bg(color).w(relative(f.max(0.015))))
}

pub fn relative_time(t: chrono::DateTime<chrono::Utc>) -> String {
    let secs = (chrono::Utc::now() - t).num_seconds().max(0);
    match secs {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        86_400..=604_799 => format!("{}d", secs / 86_400),
        _ => t.with_timezone(&chrono::Local).format("%b %d").to_string(),
    }
}

/// A section label (small caps style).
pub fn section_label(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// Text with highlighted char indices (fuzzy matches).
pub fn highlighted(text: &str, indices: &[u32], base: Hsla, hi: Hsla) -> Div {
    let mut row = h_flex().min_w_0().overflow_hidden();
    let mut run = String::new();
    let mut run_hi = false;
    let flush = |row: Div, run: &mut String, is_hi: bool| -> Div {
        if run.is_empty() {
            return row;
        }
        let t = std::mem::take(run);
        row.child(
            div()
                .flex_none()
                .text_color(if is_hi { hi } else { base })
                .when(is_hi, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(t),
        )
    };
    for (i, ch) in text.chars().enumerate() {
        let h = indices.contains(&(i as u32));
        if h != run_hi {
            row = flush(row, &mut run, run_hi);
            run_hi = h;
        }
        run.push(ch);
    }
    flush(row, &mut run, run_hi)
}

/// Fade+slide-in wrapper for newly mounted content.
pub fn appear(id: impl Into<ElementId>, el: impl IntoElement + 'static) -> impl IntoElement {
    div().child(el).with_animation(
        id,
        Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
        |d, t| d.opacity(t).mt(sz((1.0 - t) * 6.0)),
    )
}

pub fn kbd_hint(text: &'static str, cx: &App) -> Div {
    div()
        .px(sz(5.))
        .py(sz(1.))
        .rounded(sz(4.))
        .border_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

pub fn empty_state(icon_name: &'static str, title: &'static str, body: impl Into<SharedString>, cx: &App) -> Div {
    v_flex()
        .items_center()
        .gap_2()
        .p_6()
        .child(
            div()
                .size(sz(44.))
                .rounded(sz(12.))
                .bg(cx.theme().muted)
                .flex()
                .items_center()
                .justify_center()
                .child(icon(icon_name).size(sz(22.)).text_color(cx.theme().muted_foreground)),
        )
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
        .child(div().text_sm().text_color(cx.theme().muted_foreground).text_center().child(body.into()))
}

/// Mothr's mark: a flat monogram tile.
pub fn logo_mark(size: f32, pal: &super::tokens::Palette) -> Div {
    div()
        .flex_none()
        .size(sz(size))
        .rounded(sz(size * 0.28))
        .bg(pal.text)
        .flex()
        .items_center()
        .justify_center()
        .text_color(pal.bg)
        .text_size(sz(size * 0.62))
        .font_weight(FontWeight::BOLD)
        .child("M")
}

/// The track behind a set of [`segment`]s.
pub fn segment_track(pal: &super::tokens::Palette) -> Div {
    h_flex().flex_none().p(sz(2.)).gap(sz(2.)).rounded(sz(super::tokens::radius::MD)).bg(pal.surface_hover)
}

/// One option of a segmented control.
pub fn segment(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool, pal: &super::tokens::Palette) -> Stateful<Div> {
    let pal = *pal;
    div()
        .id(id)
        .h(sz(24.))
        .px(sz(10.))
        .flex()
        .items_center()
        .rounded(sz(super::tokens::radius::SM))
        .text_size(sz(super::tokens::text::SM))
        .cursor_pointer()
        .when(active, |d| d.bg(pal.surface).text_color(pal.text).font_weight(FontWeight::MEDIUM).shadow(super::tokens::shadow_sm(&pal)))
        .when(!active, |d| d.text_color(pal.text_muted).hover(move |d| d.text_color(pal.text)))
        .child(label.into())
}

/// A softly pulsing status dot.
pub fn pulse_dot(id: &'static str, color: Hsla) -> impl IntoElement {
    div().flex_none().size(sz(7.)).rounded_full().bg(color).with_animation(
        id,
        Animation::new(std::time::Duration::from_millis(1400)).repeat().with_easing(pulsating_between(0.35, 1.0)),
        |d, t| d.opacity(t),
    )
}

/// The small "skill" tag on slash commands that come from Mothr's skill library.
pub fn skill_tag(pal: &super::tokens::Palette) -> Div {
    div()
        .flex_none()
        .px(sz(5.))
        .h(sz(16.))
        .flex()
        .items_center()
        .rounded(sz(super::tokens::radius::XS))
        .border_1()
        .border_color(pal.border_strong)
        .text_size(sz(10.5))
        .text_color(pal.text_faint)
        .child("skill")
}

/// Slash commands matching `query`: prefix matches first, then substring matches.
pub fn rank_commands(commands: &[CommandInfo], query: &str) -> Vec<CommandInfo> {
    let q = query.to_ascii_lowercase();
    let mut prefix: Vec<CommandInfo> = vec![];
    let mut contains: Vec<CommandInfo> = vec![];
    for c in commands {
        let n = c.name.to_ascii_lowercase();
        if n.starts_with(&q) {
            prefix.push(c.clone());
        } else if !q.is_empty() && (n.contains(&q) || c.description.as_deref().is_some_and(|d| d.to_ascii_lowercase().contains(&q))) {
            contains.push(c.clone());
        }
    }
    prefix.sort_by_key(|c| c.name.len());
    prefix.extend(contains);
    prefix.truncate(8);
    prefix
}
