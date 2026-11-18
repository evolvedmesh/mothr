//! Account limits & context usage: title-bar pills and the detail panel.

use super::kit::*;
use super::tokens;
use crate::store::Store;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::*;
use super::tokens::sz;
use gpui_kit::prelude::FluentBuilder as _;
use shepherd_core::*;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Compact pills: one per provider with limits, showing its most-used window.
pub fn title_pills(store: &Store, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let mut row = h_flex().gap(sz(6.)).items_center();
    let mut any = false;
    for u in store.usage.values() {
        let Some(w) = u.windows.iter().max_by(|a, b| a.used_fraction.total_cmp(&b.used_fraction)) else { continue };
        any = true;
        let color = level_color(w.level(), cx);
        row = row.child(
            h_flex()
                .gap(sz(5.))
                .items_center()
                .px(sz(7.))
                .py(sz(3.))
                .rounded(sz(6.))
                .bg(if w.level() >= UsageLevel::Warn { color.opacity(0.12) } else { theme.muted.opacity(0.6) })
                .child(div().size(sz(6.)).rounded_full().bg(agent_color(u.provider.agent())))
                .child(div().text_xs().text_color(theme.muted_foreground).child(format!("{} {}", w.label, pct(w.used_fraction))))
                .child(mini_gauge(w.used_fraction, color, cx)),
        );
    }
    if !any {
        row = row.child(
            h_flex()
                .gap_1()
                .px(sz(7.))
                .py(sz(3.))
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(icon("gauge").size(sz(13.)))
                .child("Usage"),
        );
    }
    div()
        .id("usage-pills")
        .cursor_pointer()
        .rounded(sz(7.))
        .hover(|d| d.bg(theme.list_hover))
        .on_click(on_click)
        .child(row)
        .into_any_element()
}

fn pct(f: f32) -> String {
    format!("{:.0}%", (f * 100.0).max(0.0))
}

/// The usage detail panel.
pub fn panel(store: &Store, current: Option<&SessionMeta>, on_refresh: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let mut body = v_flex().gap_4();

    if let Some(m) = current {
        let c = m.context;
        body = body.child(
            v_flex()
                .gap_2()
                .child(section_label("THIS CHAT", cx))
                .child(context_row(&c, cx))
                .when_some(c.cost_usd, |d, cost| {
                    d.child(div().text_xs().text_color(theme.muted_foreground).child(format!("Session cost ${cost:.2}")))
                }),
        );
    }

    let mut providers = v_flex().gap_4();
    let mut any = false;
    for u in store.usage.values() {
        if u.windows.is_empty() && u.error.is_none() {
            continue;
        }
        any = true;
        let mut block = v_flex().gap_2().child(
            h_flex()
                .gap_2()
                .items_center()
                .child(agent_avatar(u.provider.agent(), 20.))
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(u.provider.display_name()))
                .when_some(u.plan.clone(), |d, plan| {
                    d.child(
                        div()
                            .text_xs()
                            .px(sz(6.))
                            .py(sz(1.))
                            .rounded(sz(4.))
                            .bg(theme.muted)
                            .text_color(theme.muted_foreground)
                            .child(plan),
                    )
                })
                .child(div().flex_1())
                .child(div().text_xs().text_color(theme.muted_foreground.opacity(0.7)).child(relative_time(u.updated_at))),
        );
        for w in &u.windows {
            let color = level_color(w.level(), cx);
            let right = match (w.used, w.limit) {
                (Some(used), Some(limit)) => format!("{used:.0} / {limit:.0}"),
                _ => pct(w.used_fraction),
            };
            let reset = w.resets_at.map(|r| format!("Resets {}", format_reset(r)));
            block = block.child(
                v_flex()
                    .gap(sz(5.))
                    .child(
                        h_flex()
                            .text_xs()
                            .child(div().flex_1().font_weight(FontWeight::MEDIUM).child(w.label.clone()))
                            .child(div().text_color(if w.level() >= UsageLevel::Warn { color } else { theme.muted_foreground }).child(right)),
                    )
                    .child(meter(w.used_fraction, color, cx))
                    .child(
                        h_flex()
                            .text_xs()
                            .text_color(theme.muted_foreground.opacity(0.8))
                            .child(div().flex_1().child(reset.unwrap_or_default()))
                            .when_some(w.detail.clone(), |d, det| d.child(det)),
                    ),
            );
        }
        if let Some(err) = &u.error {
            block = block.child(div().text_xs().text_color(theme.muted_foreground).child(err.clone()));
        }
        providers = providers.child(block);
    }
    if !any {
        providers = providers.child(div().text_sm().text_color(theme.muted_foreground).child("No account limits reported yet. They appear once an agent is signed in."));
    }

    v_flex()
        .w(sz(340.))
        .max_h(sz(560.))
        .p_4()
        .gap_4()
        .rounded(sz(tokens::radius::XL))
        .border_1()
        .border_color(tokens::p(cx).border_strong)
        .bg(tokens::p(cx).raised)
        .shadow(tokens::shadow_lg(&tokens::p(cx)))
        .child(
            h_flex()
                .items_center()
                .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child("Usage & limits"))
                .child(Button::new("refresh-usage").ghost().small().icon(icon("refresh-cw")).tooltip("Refresh").on_click(on_refresh)),
        )
        .child(div().id("usage-scroll").overflow_y_scroll().child(body.child(v_flex().gap_2().child(section_label("ACCOUNTS", cx)).child(providers))))
        .with_animation(
            "usage-panel",
            Animation::new(std::time::Duration::from_millis(160)).with_easing(ease_out_quint()),
            |d, t| d.opacity(t).mt(sz((1.0 - t) * -6.0)),
        )
}

pub fn context_row(c: &ContextUsage, cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let f = c.fraction();
    let color = level_color(UsageLevel::from_fraction(f), cx);
    v_flex()
        .gap(sz(5.))
        .child(
            h_flex()
                .text_xs()
                .child(div().flex_1().font_weight(FontWeight::MEDIUM).child("Context"))
                .child(div().text_color(theme.muted_foreground).child(if c.window_tokens > 0 {
                    format!("{} / {}  {}", format_tokens(c.used_tokens), format_tokens(c.window_tokens), pct(f))
                } else if c.used_tokens > 0 {
                    format!("{} tokens", format_tokens(c.used_tokens))
                } else {
                    "—".into()
                })),
        )
        .child(meter(f, color, cx))
}

#[allow(dead_code)]
fn _unused(_: ClickHandler) {}
