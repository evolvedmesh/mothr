//! Account limits & context usage: title-bar pills and the detail panel.

use super::kit::*;
use super::tokens;
use crate::store::Store;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    hover_card::HoverCard,
    progress::ProgressCircle,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::*;
use super::tokens::sz;
use gpui_kit::prelude::FluentBuilder as _;
use mothr_core::*;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// The title-bar limits button: one ring per agent account showing its most-used
/// limit. Hovering shows every account's details (like VS Code's Copilot status); clicking pins
/// the panel open.
pub fn limits_button(
    store: Entity<Store>,
    current: Option<SessionId>,
    pinned: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let pal = tokens::p(cx);
    let s = store.read(cx);
    let mut worst = UsageLevel::Ok;
    let mut any = false;
    let mut rings = h_flex().gap(sz(4.)).items_center();
    for u in s.usage.values() {
        let Some(w) = u.windows.iter().max_by(|a, b| a.used_fraction.total_cmp(&b.used_fraction)) else { continue };
        any = true;
        let agent = u.provider.agent();
        let level = w.level();
        worst = worst.max(level);
        let color = if level >= UsageLevel::Warn { level_color(level, cx) } else { agent_color(agent) };
        rings = rings.child(
            ProgressCircle::new(SharedString::from(format!("ring-{}", agent.id())))
                .value(w.used_fraction * 100.0)
                .color(color)
                .size(sz(20.))
                .child(agent_logo(agent).size(sz(9.)).text_color(color)),
        );
    }
    let alert = worst >= UsageLevel::Warn;
    let trigger = h_flex()
        .id("ai-limits")
        .gap(sz(6.))
        .h(sz(30.))
        .px(sz(8.))
        .items_center()
        .rounded(sz(tokens::radius::MD))
        .cursor_pointer()
        .when(alert, |d| d.bg(level_color(worst, cx).opacity(0.10)))
        .when(pinned, |d| d.bg(pal.surface_hover))
        .hover(|d| d.bg(pal.surface_hover))
        .on_click(on_click)
        // With no account limits reported yet, a quiet gauge keeps the card reachable.
        .when(!any, |d| d.child(icon("gauge").size(sz(15.)).text_color(pal.text_muted)))
        .child(rings);
    if pinned {
        return trigger.into_any_element();
    }
    HoverCard::new("ai-limits-card")
        .anchor(Anchor::TopRight)
        .appearance(false)
        .open_delay(std::time::Duration::from_millis(250))
        .close_delay(std::time::Duration::from_millis(200))
        .trigger(trigger)
        .content(move |_, _window, cx| {
            let store = store.clone();
            let st = store.read(cx);
            let meta = current.and_then(|id| st.session(id)).map(|s| s.meta.clone());
            let refresh = store.clone();
            div().mt(sz(6.)).child(panel(
                st,
                meta.as_ref(),
                move |_, window, cx| {
                    refresh.update(cx, |s, cx| {
                        s.dispatch(AppCommand::RefreshUsage, window, cx);
                    });
                },
                cx,
            ))
            .into_any_element()
        })
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
