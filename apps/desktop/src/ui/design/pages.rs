//! The pages of the design viewer.

use super::canvas::{Renderer, canvas_bg};
use super::gallery::{Corners, preview};
use super::viewer::{DesignViewer, Page, Stage};
use super::*;
use crate::ui::kit::*;
use crate::ui::tokens::{self, Palette, sz};
use gpui_kit::component::{
    Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use mothr_design::{Component, Screen};

const MIN_ZOOM: f32 = 0.15;
const MAX_ZOOM: f32 = 2.0;

fn copy(text: String, what: &str, window: &mut Window, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    window.push_notification(Notification::success(format!("Copied {what}")).autohide(true), cx);
}

fn page_header(eyebrow: &'static str, title: String, detail: Option<String>, pal: &Palette) -> Div {
    v_flex()
        .gap(sz(6.))
        .child(div().text_size(sz(tokens::text::XS)).font_weight(FontWeight::MEDIUM).text_color(pal.text_faint).child(eyebrow))
        .child(div().text_size(sz(tokens::text::XXL)).font_weight(FontWeight::SEMIBOLD).line_height(relative(1.2)).child(title))
        .when_some(detail, |d, t| d.child(div().max_w(sz(620.)).text_size(sz(tokens::text::READ)).line_height(relative(1.55)).text_color(pal.text_muted).child(t)))
}

fn section_title(title: &'static str, pal: &Palette) -> Div {
    div().text_size(sz(tokens::text::LG)).font_weight(FontWeight::SEMIBOLD).text_color(pal.text).child(title)
}

fn meta(text: impl Into<SharedString>, pal: &Palette) -> Div {
    div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child(text.into())
}

fn mono(text: impl Into<SharedString>, pal: &Palette) -> Div {
    div().font_family(tokens::FONT_MONO).text_size(sz(tokens::text::XS)).text_color(pal.text_muted).child(text.into())
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

impl DesignViewer {
    pub(super) fn render_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if !self.doc.has_content() {
            return self.render_waiting(cx);
        }
        if let Page::Screen(i) = self.page {
            return self.render_screen(i, cx);
        }
        let pal = tokens::p(cx);
        let content = match self.page {
            Page::Overview => self.render_overview(cx),
            Page::Color => self.render_color(cx),
            Page::Type => self.render_type(cx),
            Page::Space => self.render_space(cx),
            Page::Elevation => self.render_elevation(cx),
            Page::Component(i) => self.render_component(i, cx),
            Page::Screen(_) => unreachable!("handled above"),
        };
        let key = format!("{:?}", self.page);
        div()
            .id("design-page")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .bg(pal.bg)
            .child(
                div().w_full().max_w(sz(1040.)).mx_auto().px(sz(36.)).pt(sz(32.)).pb(sz(56.)).child(appear(ElementId::Name(format!("page-{key}").into()), content)),
            )
            .into_any_element()
    }

    /// A new design, before the agent has saved anything.
    fn render_waiting(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let working = latest_session(self.store.read(cx), &self.project).is_some_and(|s| s.meta.status == SessionStatus::Working);
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(sz(10.))
            .child(if working { Spinner::new().into_any_element() } else { icon("palette").size(sz(26.)).text_color(pal.text_faint).into_any_element() })
            .child(div().text_size(sz(tokens::text::LG)).font_weight(FontWeight::SEMIBOLD).child(if working { "Designing…" } else { "Nothing here yet" }))
            .child(div().max_w(sz(380.)).text_center().text_size(sz(tokens::text::BASE)).text_color(pal.text_muted).child(if working {
                "Tokens, components and screens appear here as the agent saves them."
            } else {
                "Open the chat and describe what this design should be."
            }))
            .into_any_element()
    }

    fn stat_tile(&self, id: &'static str, value: usize, label: &'static str, page: Page, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        v_flex()
            .id(id)
            .flex_1()
            .min_w(sz(120.))
            .px(sz(14.))
            .py(sz(12.))
            .gap(sz(2.))
            .rounded(sz(tokens::radius::LG))
            .border_1()
            .border_color(pal.border)
            .cursor_pointer()
            .hover(|d| d.bg(pal.surface_hover))
            .on_click(cx.listener(move |this, _, _, cx| this.go(page, cx)))
            .child(div().text_size(sz(tokens::text::XL)).font_weight(FontWeight::SEMIBOLD).child(value.to_string()))
            .child(meta(label, &pal))
    }

    fn render_overview(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let doc = &self.doc;
        let t = doc.tokens();
        let components: Vec<Component> = doc.system.as_ref().map(|s| s.components.clone()).unwrap_or_default();
        let mut col = v_flex().gap(sz(28.)).child(page_header("Overview", self.name(cx), doc.description().map(str::to_string), &pal));
        col = col.child(div().rounded(sz(tokens::radius::LG)).overflow_hidden().border_1().border_color(pal.border).child(preview(doc, 208., Corners { top: tokens::radius::LG - 1., bottom: tokens::radius::LG - 1. }, cx)));
        let colors: Vec<u32> = t.colors.iter().filter_map(|(_, v)| doc.color(v)).collect();
        if !colors.is_empty() {
            col = col.child(
                h_flex()
                    .id("palette-strip")
                    .h(sz(36.))
                    .rounded(sz(tokens::radius::MD))
                    .overflow_hidden()
                    .border_1()
                    .border_color(pal.border)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.go(Page::Color, cx)))
                    .children(colors.iter().enumerate().map(|(i, c)| {
                        let r = sz(tokens::radius::MD - 1.);
                        div().flex_1().h_full().bg(hsla(*c)).when(i == 0, |d| d.rounded_l(r)).when(i + 1 == colors.len(), |d| d.rounded_r(r))
                    })),
            );
        }
        let mut stats = h_flex().flex_wrap().gap(sz(10.));
        if !t.colors.is_empty() {
            stats = stats.child(self.stat_tile("stat-color", t.colors.len(), "Colors", Page::Color, cx));
        }
        if !t.typography.is_empty() {
            stats = stats.child(self.stat_tile("stat-type", t.typography.len(), "Type styles", Page::Type, cx));
        }
        if let Some(first) = (!components.is_empty()).then_some(Page::Component(0)) {
            stats = stats.child(self.stat_tile("stat-components", components.len(), "Components", first, cx));
        }
        if !doc.screens.is_empty() {
            stats = stats.child(self.stat_tile("stat-screens", doc.screens.len(), "Screens", Page::Screen(0), cx));
        }
        col = col.child(stats);
        if !components.is_empty() {
            let r = Renderer::new(doc, 0.75);
            let stage = canvas_bg(doc, pal.surface);
            let mut grid = h_flex().flex_wrap().gap(sz(12.)).items_start();
            for (i, c) in components.iter().enumerate() {
                let Some(v) = c.variants().into_iter().next() else { continue };
                let count = c.variants().len();
                grid = grid.child(
                    v_flex()
                        .id(ElementId::Name(format!("tile-{i}").into()))
                        .flex_1()
                        .min_w(sz(240.))
                        .max_w(sz(420.))
                        .rounded(sz(tokens::radius::LG))
                        .border_1()
                        .border_color(pal.border)
                        .overflow_hidden()
                        .cursor_pointer()
                        .hover(|d| d.border_color(pal.border_strong))
                        .on_click(cx.listener(move |this, _, _, cx| this.go(Page::Component(i), cx)))
                        .child(div().h(sz(116.)).p(sz(12.)).flex().items_center().justify_center().overflow_hidden().rounded_t(sz(tokens::radius::LG - 1.)).bg(stage).child(div().max_w_full().child(r.node(&v.node, None))))
                        .child(
                            h_flex()
                                .px(sz(12.))
                                .py(sz(9.))
                                .gap(sz(8.))
                                .border_t_1()
                                .border_color(pal.border)
                                .child(div().flex_1().min_w_0().truncate().text_size(sz(tokens::text::SM)).child(c.name.clone()))
                                .child(meta(plural(count, "variant"), &pal)),
                        ),
                );
            }
            col = col.child(v_flex().gap(sz(12.)).child(section_title("Components", &pal)).child(grid));
        }
        if !doc.screens.is_empty() {
            let mut row = h_flex().flex_wrap().gap(sz(16.)).items_start();
            for (i, s) in doc.screens.iter().enumerate() {
                row = row.child(self.screen_thumb(i, s, cx));
            }
            col = col.child(v_flex().gap(sz(12.)).child(section_title("Screens", &pal)).child(row));
        }
        col.into_any_element()
    }

    fn screen_thumb(&self, i: usize, s: &Screen, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let (w, h) = s.size();
        let scale = (220.0 / h).min(300.0 / w);
        let bg = s.background.as_deref().and_then(|b| self.doc.color(b)).map(hsla).unwrap_or(pal.surface);
        let r = Renderer::new(&self.doc, scale);
        v_flex()
            .id(ElementId::Name(format!("thumb-{i}").into()))
            .flex_none()
            .gap(sz(8.))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.go(Page::Screen(i), cx)))
            .child(
                div()
                    .w(sz(w * scale))
                    .h(sz(h * scale))
                    .rounded(sz(tokens::radius::MD))
                    .overflow_hidden()
                    .border_1()
                    .border_color(pal.border)
                    .bg(bg)
                    .hover(|d| d.border_color(pal.border_strong))
                    .child(div().size_full().flex().flex_col().child(r.node(&s.root, None))),
            )
            .child(div().text_size(sz(tokens::text::SM)).child(s.name.clone()))
    }

    fn render_color(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let t = self.doc.tokens();
        let mut grid = h_flex().flex_wrap().gap(sz(12.)).items_start();
        for (i, (name, raw)) in t.colors.iter().enumerate() {
            let Some(c) = self.doc.color(raw) else { continue };
            let hex = hex_label(c);
            let on = readable_on(c);
            let ratio = contrast(c, on);
            let alias = raw.trim().strip_prefix('$').map(|a| format!("→ {a}"));
            let value = hex.clone();
            let label = name.clone();
            grid = grid.child(
                v_flex()
                    .id(ElementId::Name(format!("swatch-{i}").into()))
                    .flex_1()
                    .min_w(sz(150.))
                    .max_w(sz(230.))
                    .rounded(sz(tokens::radius::LG))
                    .border_1()
                    .border_color(pal.border)
                    .overflow_hidden()
                    .cursor_pointer()
                    .hover(|d| d.border_color(pal.border_strong))
                    .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Copy value").build(window, cx))
                    .on_click(move |_, window, cx| copy(value.clone(), &label, window, cx))
                    .child(
                        h_flex()
                            .h(sz(88.))
                            .p(sz(10.))
                            .items_end()
                            .justify_between()
                            .rounded_t(sz(tokens::radius::LG - 1.))
                            .bg(hsla(c))
                            .text_color(hsla(on))
                            .child(div().text_size(sz(tokens::text::LG)).font_weight(FontWeight::MEDIUM).child("Aa"))
                            .child(div().text_size(sz(tokens::text::XS)).opacity(0.75).child(format!("{ratio:.1}:1"))),
                    )
                    .child(
                        v_flex()
                            .px(sz(10.))
                            .py(sz(8.))
                            .gap(sz(1.))
                            .child(div().truncate().text_size(sz(tokens::text::SM)).font_weight(FontWeight::MEDIUM).child(name.clone()))
                            .child(h_flex().gap(sz(6.)).child(mono(hex, &pal)).when_some(alias, |d, a| d.child(meta(a, &pal)))),
                    ),
            );
        }
        v_flex().gap(sz(24.)).child(page_header("Foundations", "Color".into(), Some("Click a swatch to copy its value. The ratio is the contrast of the sample text.".into()), &pal)).child(grid).into_any_element()
    }

    fn render_type(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let t = self.doc.tokens();
        let mut list = v_flex().rounded(sz(tokens::radius::LG)).border_1().border_color(pal.border).overflow_hidden();
        for (i, (name, ts)) in t.typography.iter().enumerate() {
            let size = ts.size.unwrap_or(16.0);
            let weight = ts.weight.unwrap_or(400.0);
            let mut spec = format!("{size:.0} px · {weight:.0}");
            if let Some(l) = ts.line_height {
                spec.push_str(&format!(" · {l}"));
            }
            list = list.child(
                v_flex()
                    .px(sz(18.))
                    .py(sz(16.))
                    .gap(sz(8.))
                    .when(i > 0, |d| d.border_t_1().border_color(pal.border))
                    .child(
                        h_flex()
                            .gap(sz(10.))
                            .child(div().text_size(sz(tokens::text::SM)).font_weight(FontWeight::MEDIUM).child(name.clone()))
                            .child(mono(spec, &pal))
                            .when_some(ts.family.clone(), |d, f| d.child(meta(f, &pal))),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(sz(size))
                            .font_weight(FontWeight(weight))
                            .when_some(ts.line_height, |d, l| if l <= 4.0 { d.line_height(relative(l)) } else { d.line_height(sz(l)) })
                            .when_some(ts.family.clone(), |d, f| d.font_family(f))
                            .when(ts.italic.unwrap_or(false), |d| d.italic())
                            .child("The quick brown fox jumps over the lazy dog"),
                    ),
            );
        }
        v_flex().gap(sz(24.)).child(page_header("Foundations", "Typography".into(), None, &pal)).child(list).into_any_element()
    }

    fn render_space(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let t = self.doc.tokens();
        let mut col = v_flex().gap(sz(32.)).child(page_header("Foundations", "Space & radius".into(), None, &pal));
        if !t.spacing.is_empty() {
            let mut rows = v_flex().gap(sz(10.));
            for (n, v) in t.spacing.iter() {
                rows = rows.child(
                    h_flex()
                        .gap(sz(14.))
                        .items_center()
                        .child(div().w(sz(72.)).truncate().text_size(sz(tokens::text::SM)).child(n.clone()))
                        .child(div().w(sz(44.)).child(mono(format!("{}", v.0), &pal)))
                        .child(div().h(sz(14.)).w(sz(v.0.clamp(1.0, 640.0))).rounded(sz(2.)).bg(pal.brand.opacity(0.7))),
                );
            }
            col = col.child(v_flex().gap(sz(14.)).child(section_title("Spacing", &pal)).child(rows));
        }
        if !t.radii.is_empty() {
            let mut row = h_flex().flex_wrap().gap(sz(14.));
            for (n, v) in t.radii.iter() {
                row = row.child(
                    v_flex()
                        .gap(sz(8.))
                        .items_center()
                        .child(div().size(sz(72.)).rounded(sz(v.0.min(36.0))).border_1().border_color(pal.brand.opacity(0.7)).bg(pal.brand_soft))
                        .child(div().text_size(sz(tokens::text::SM)).child(n.clone()))
                        .child(mono(format!("{}", v.0), &pal)),
                );
            }
            col = col.child(v_flex().gap(sz(14.)).child(section_title("Radius", &pal)).child(row));
        }
        col.into_any_element()
    }

    fn render_elevation(&self, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let t = self.doc.tokens();
        let r = Renderer::new(&self.doc, 1.0);
        let stage = canvas_bg(&self.doc, pal.surface);
        let raised = self.doc.color("$surface").map(hsla).unwrap_or(pal.raised);
        let ink = hsla(ink_of(&self.doc, surface_of(&self.doc).unwrap_or(rgba_of(pal.surface))));
        let mut row = h_flex().flex_wrap().gap(sz(32.)).p(sz(40.)).rounded(sz(tokens::radius::LG)).border_1().border_color(pal.border).bg(stage);
        for (n, s) in t.shadows.iter() {
            row = row.child(
                v_flex()
                    .gap(sz(12.))
                    .items_center()
                    .child(r.apply_shadow(div().size(sz(112.)).rounded(sz(tokens::radius::LG)).bg(raised), &format!("${n}")))
                    .child(div().text_size(sz(tokens::text::SM)).text_color(ink).child(n.clone()))
                    .child(div().font_family(tokens::FONT_MONO).text_size(sz(tokens::text::XS)).opacity(0.6).text_color(ink).child(format!("{} {} {}", s.x.unwrap_or(0.), s.y.unwrap_or(0.), s.blur.unwrap_or(0.)))),
            );
        }
        v_flex().gap(sz(24.)).child(page_header("Foundations", "Elevation".into(), None, &pal)).child(row).into_any_element()
    }

    fn stage_color(&self, pal: &Palette) -> Hsla {
        match self.stage {
            Stage::Design => canvas_bg(&self.doc, pal.surface),
            Stage::Light => Palette::light().bg,
            Stage::Dark => Palette::dark().bg,
        }
    }

    fn render_component(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let Some(c) = self.doc.system.as_ref().and_then(|s| s.components.get(index)).cloned() else { return div().into_any_element() };
        let r = Renderer::new(&self.doc, 1.0);
        let stage = self.stage_color(&pal);
        let stage_button = |id: &'static str, label: &'static str, value: Stage, cx: &mut Context<Self>| {
            segment(id, label, self.stage == value, &pal).on_click(cx.listener(move |this, _, _, cx| {
                this.stage = value;
                cx.notify();
            }))
        };
        let controls = segment_track(&pal)
            .child(stage_button("stage-design", "Design", Stage::Design, cx))
            .child(stage_button("stage-light", "Light", Stage::Light, cx))
            .child(stage_button("stage-dark", "Dark", Stage::Dark, cx));
        let usage = serde_json::json!({ "type": "ref", "ref": c.name }).to_string();
        let mut col = v_flex().gap(sz(20.)).child(
            h_flex()
                .gap(sz(16.))
                .items_end()
                .child(div().flex_1().min_w_0().child(page_header("Component", c.name.clone(), c.description.clone(), &pal)))
                .child(controls),
        );
        for (vi, v) in c.variants().iter().enumerate() {
            let open = self.sources.contains(&(index, vi));
            let source = serde_json::to_string_pretty(&v.node).unwrap_or_default();
            let copied = source.clone();
            col = col.child(
                v_flex()
                    .rounded(sz(tokens::radius::LG))
                    .border_1()
                    .border_color(pal.border)
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .h(sz(38.))
                            .pl(sz(14.))
                            .pr(sz(6.))
                            .gap(sz(4.))
                            .items_center()
                            .border_b_1()
                            .border_color(pal.border)
                            .child(div().flex_1().min_w_0().truncate().text_size(sz(tokens::text::SM)).font_weight(FontWeight::MEDIUM).child(v.name.clone()))
                            .child(
                                Button::new(ElementId::Name(format!("src-{vi}").into()))
                                    .ghost()
                                    .xsmall()
                                    .icon(icon("code"))
                                    .label("Source")
                                    .selected(open)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !this.sources.remove(&(index, vi)) {
                                            this.sources.insert((index, vi));
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .min_h(sz(148.))
                            .p(sz(32.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(!open, |d| d.rounded_b(sz(tokens::radius::LG - 1.)))
                            .bg(stage)
                            .child(v_flex().w_full().max_w(sz(620.)).items_center().child(r.node(&v.node, None))),
                    )
                    .when(open, |d| {
                        d.child(
                            div()
                                .relative()
                                .border_t_1()
                                .border_color(pal.border)
                                .bg(pal.code_bg)
                                .child(
                                    div()
                                        .id(ElementId::Name(format!("src-body-{vi}").into()))
                                        .max_h(sz(300.))
                                        .overflow_y_scroll()
                                        .p(sz(14.))
                                        .font_family(tokens::FONT_MONO)
                                        .text_size(sz(tokens::text::SM))
                                        .text_color(pal.text_muted)
                                        .child(source),
                                )
                                .child(
                                    div().absolute().top(sz(6.)).right(sz(6.)).child(
                                        Button::new(ElementId::Name(format!("src-copy-{vi}").into()))
                                            .ghost()
                                            .xsmall()
                                            .icon(icon("copy"))
                                            .tooltip("Copy source")
                                            .on_click(move |_, window, cx| copy(copied.clone(), "source", window, cx)),
                                    ),
                                ),
                        )
                    }),
            );
        }
        col = col.child(
            h_flex()
                .id("usage")
                .px(sz(14.))
                .h(sz(38.))
                .gap(sz(10.))
                .items_center()
                .rounded(sz(tokens::radius::MD))
                .bg(pal.code_bg)
                .cursor_pointer()
                .on_click({
                    let usage = usage.clone();
                    move |_, window, cx| copy(usage.clone(), "usage", window, cx)
                })
                .child(meta("Use in a screen", &pal))
                .child(div().flex_1().min_w_0().truncate().child(mono(usage, &pal)))
                .child(icon("copy").size(sz(13.)).text_color(pal.text_faint)),
        );
        col.into_any_element()
    }

    /// One screen on a canvas, fitted to the space available.
    fn render_screen(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let Some(s) = self.doc.screens.get(index).cloned() else { return div().into_any_element() };
        let (w, h) = s.size();
        let viewport = self.viewport.get();
        let ui = tokens::scale();
        let fit = if viewport.width > px(0.) {
            let avail_w = (f32::from(viewport.width) - 64.0).max(80.0);
            let avail_h = (f32::from(viewport.height) - 64.0).max(80.0);
            (avail_w / (w * ui)).min(avail_h / (h * ui)).clamp(MIN_ZOOM, 1.0)
        } else {
            0.4
        };
        let zoom = self.zoom.unwrap_or(fit);
        let total = self.doc.screens.len();
        let bg = s.background.as_deref().and_then(|b| self.doc.color(b)).map(hsla).unwrap_or(gpui_kit::white());
        let r = Renderer::new(&self.doc, zoom);
        let mobile = matches!(s.device, mothr_design::Device::Mobile);
        let cell = self.viewport.clone();
        let entity = cx.entity_id();
        let step = |id: &'static str, ic: &'static str, tip: &'static str, enabled: bool, target: usize, cx: &mut Context<Self>| {
            Button::new(id).ghost().xsmall().icon(icon(ic)).tooltip(tip).when(!enabled, |b| b.opacity(0.35)).on_click(cx.listener(move |this, _, _, cx| {
                if enabled {
                    this.go(Page::Screen(target), cx);
                }
            }))
        };
        let zoom_by = move |delta: f32| move |this: &mut DesignViewer, _: &ClickEvent, _: &mut Window, cx: &mut Context<DesignViewer>| {
            this.zoom = Some((zoom + delta).clamp(MIN_ZOOM, MAX_ZOOM));
            cx.notify();
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_none()
                    .h(sz(40.))
                    .px(sz(12.))
                    .gap(sz(6.))
                    .items_center()
                    .border_b_1()
                    .border_color(pal.border)
                    .child(step("screen-prev", "chevron-left", "Previous screen", index > 0, index.saturating_sub(1), cx))
                    .child(step("screen-next", "chevron-right", "Next screen", index + 1 < total, index + 1, cx))
                    .child(div().min_w_0().truncate().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::MEDIUM).child(s.name.clone()))
                    .child(meta(format!("{:.0} × {:.0}", w, h), &pal))
                    .child(div().flex_1())
                    .child(Button::new("zoom-out").ghost().xsmall().icon(icon("minus")).tooltip("Zoom out").on_click(cx.listener(zoom_by(-0.1))))
                    .child(div().w(sz(40.)).text_center().text_size(sz(tokens::text::XS)).text_color(pal.text_muted).child(format!("{:.0}%", zoom * 100.0)))
                    .child(Button::new("zoom-in").ghost().xsmall().icon(icon("plus")).tooltip("Zoom in").on_click(cx.listener(zoom_by(0.1))))
                    .child(Button::new("zoom-fit").ghost().xsmall().label("Fit").selected(self.zoom.is_none()).on_click(cx.listener(|this, _, _, cx| {
                        this.zoom = None;
                        cx.notify();
                    }))),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .bg(pal.chrome)
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                if cell.replace(bounds.size) != bounds.size {
                                    cx.defer(move |cx| cx.notify(entity));
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(
                        div().id("screen-canvas").absolute().inset_0().overflow_scroll().child(
                            div().min_w_full().min_h_full().p(sz(28.)).flex().items_center().justify_center().child(
                                div()
                                    .flex_none()
                                    .w(sz(w * zoom))
                                    .h(sz(h * zoom))
                                    .rounded(sz(if mobile { 36.0 * zoom } else { 10.0 }))
                                    .overflow_hidden()
                                    .bg(bg)
                                    .shadow(tokens::shadow_lg(&pal))
                                    .border_1()
                                    .border_color(pal.border_strong)
                                    .child(div().size_full().flex().flex_col().child(r.node(&s.root, None))),
                            ),
                        ),
                    ),
            )
            .into_any_element()
    }
}
