//! Renders design nodes natively. `crates/mothr-design/src/export.rs` is the HTML port of this
//! renderer; keep the two in step.

use crate::ui::kit::*;
use crate::ui::tokens::sz;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mothr_design::{DesignDoc, Dim, Length as DLength, Node, NodeType, Style as DStyle, TokenScale};

/// The surface a design's components are meant to sit on.
pub fn canvas_bg(doc: &DesignDoc, fallback: Hsla) -> Hsla {
    doc.color("$bg")
        .or_else(|| doc.color("$background"))
        .or_else(|| doc.color("$surface"))
        .map(rgba_u32)
        .unwrap_or(fallback)
}

/// Converts design nodes into GPUI elements at a given zoom scale.
pub struct Renderer<'a> {
    doc: &'a DesignDoc,
    scale: f32,
    /// Whether the current instance's `text` override was already consumed.
    text_used: std::cell::Cell<bool>,
    /// Direction of the container currently laying out children.
    parent_row: std::cell::Cell<bool>,
}

impl<'a> Renderer<'a> {
    pub fn new(doc: &'a DesignDoc, scale: f32) -> Self {
        Self { doc, scale, text_used: Default::default(), parent_row: Default::default() }
    }

    fn s(&self, v: f32) -> Pixels {
        sz(v * self.scale)
    }

    fn color(&self, v: &Option<String>) -> Option<Hsla> {
        v.as_deref().and_then(|c| self.doc.color(c)).map(rgba_u32)
    }

    fn len(&self, l: &DLength, scale: TokenScale) -> f32 {
        self.doc.length(l, scale)
    }

    pub fn apply_shadow(&self, d: Div, name: &str) -> Div {
        let Some(sh) = self.doc.shadow(name) else { return d.shadow_md() };
        let color = self.color(&sh.color).unwrap_or(gpui_kit::black().opacity(0.25));
        d.shadow(vec![BoxShadow {
            color,
            offset: point(self.s(sh.x.unwrap_or(0.0)), self.s(sh.y.unwrap_or(4.0))),
            blur_radius: self.s(sh.blur.unwrap_or(12.0)),
            spread_radius: self.s(sh.spread.unwrap_or(0.0)),
            inset: false,
        }])
    }

    /// Apply box-level style (layout, spacing, fill, border, radius, shadow, text).
    fn style(&self, mut d: Div, st: &DStyle, default_dir_row: bool) -> Div {
        let row = st.direction.as_deref().map(|x| x.starts_with("row") || x == "horizontal").unwrap_or(default_dir_row);
        d = d.flex();
        // Mockups behave like scrolling screens: in a column, children keep their natural
        // height and the artboard clips, instead of flexbox squashing them to fit.
        if !self.parent_row.get() && !matches!(st.height, Dim::Fill) && st.grow.is_none() {
            d = d.flex_shrink_0();
        }
        d = if row { d.flex_row() } else { d.flex_col() };
        // "fill" stretches along the cross axis and grows along the parent's main axis only.
        let in_row = self.parent_row.get();
        d = match st.width {
            Dim::Px(v) => d.w(self.s(v)).flex_none(),
            Dim::Fill if in_row => d.w_full().flex_grow(1.).flex_basis(sz(0.)),
            Dim::Fill => d.w_full(),
            Dim::Percent(p) => d.w(relative(p / 100.0)),
            Dim::Auto => d,
        };
        d = match st.height {
            Dim::Px(v) => d.h(self.s(v)),
            Dim::Fill if !in_row => d.h_full().flex_grow(1.).flex_basis(sz(0.)),
            Dim::Fill => d.h_full(),
            Dim::Percent(p) => d.h(relative(p / 100.0)),
            Dim::Auto => d,
        };
        if let Some(v) = st.min_width {
            d = d.min_w(self.s(v));
        }
        if let Some(v) = st.max_width {
            d = d.max_w(self.s(v));
        }
        if let Some(v) = st.min_height {
            d = d.min_h(self.s(v));
        }
        if let Some(g) = st.grow {
            d = if g == 0.0 { d.flex_none() } else { d.flex_grow(g) };
        }
        if let Some(p) = &st.padding {
            d = d
                .pt(self.s(self.len(&p.top, TokenScale::Spacing)))
                .pr(self.s(self.len(&p.right, TokenScale::Spacing)))
                .pb(self.s(self.len(&p.bottom, TokenScale::Spacing)))
                .pl(self.s(self.len(&p.left, TokenScale::Spacing)));
        }
        if let Some(m) = &st.margin {
            d = d
                .mt(self.s(self.len(&m.top, TokenScale::Spacing)))
                .mr(self.s(self.len(&m.right, TokenScale::Spacing)))
                .mb(self.s(self.len(&m.bottom, TokenScale::Spacing)))
                .ml(self.s(self.len(&m.left, TokenScale::Spacing)));
        }
        if let Some(g) = &st.gap {
            d = d.gap(self.s(self.len(g, TokenScale::Spacing)));
        }
        d = match st.align.as_deref() {
            Some("center") => d.items_center(),
            Some("end" | "flex-end" | "bottom" | "right") => d.items_end(),
            Some("start" | "flex-start" | "top" | "left") => d.items_start(),
            Some("baseline") => d.items_baseline(),
            _ => d,
        };
        d = match st.justify.as_deref() {
            Some("center") => d.justify_center(),
            Some("end" | "flex-end") => d.justify_end(),
            Some("between" | "space-between") => d.justify_between(),
            Some("around" | "space-around" | "evenly" | "space-evenly") => d.justify_around(),
            _ => d,
        };
        if st.wrap.unwrap_or(false) {
            d = d.flex_wrap();
        }
        if let Some(grad) = st.gradient.as_ref().filter(|g| g.len() >= 2) {
            let a = self.doc.color(&grad[0]).map(rgba_u32);
            let b = self.doc.color(&grad[1]).map(rgba_u32);
            if let (Some(a), Some(b)) = (a, b) {
                d = d.bg(linear_gradient(180., linear_color_stop(a, 0.), linear_color_stop(b, 1.)));
            }
        } else if let Some(bg) = self.color(&st.bg) {
            d = d.bg(bg);
        }
        if let Some(r) = &st.radius {
            d = d.rounded(self.s(self.len(r, TokenScale::Radius)));
        }
        if let Some(b) = &st.border {
            let w = self.s(b.width.unwrap_or(1.0));
            d.style().border_widths = EdgesRefinement {
                top: Some(w.into()),
                right: Some(w.into()),
                bottom: Some(w.into()),
                left: Some(w.into()),
            };
            d = d.border_color(self.color(&b.color).unwrap_or(gpui_kit::black().opacity(0.15)));
        }
        if let Some(sh) = &st.shadow {
            d = self.apply_shadow(d, sh);
        }
        if let Some(o) = st.opacity {
            d = d.opacity(o.clamp(0.0, 1.0));
        }
        if st.overflow.as_deref() == Some("hidden") {
            d = d.overflow_hidden();
        }
        self.text_style(d, st)
    }

    fn text_style(&self, mut d: Div, st: &DStyle) -> Div {
        let ts = st.font.as_deref().and_then(|f| self.doc.type_style(f)).cloned().unwrap_or_default();
        if let Some(fg) = self.color(&st.fg) {
            d = d.text_color(fg);
        }
        let size = st.size.or(ts.size);
        if let Some(sz) = size {
            d = d.text_size(self.s(sz));
        } else {
            d = d.text_size(self.s(15.0));
        }
        if let Some(w) = st.weight.or(ts.weight) {
            d = d.font_weight(FontWeight(w));
        }
        if let Some(lh) = ts.line_height {
            d = if lh <= 4.0 { d.line_height(relative(lh)) } else { d.line_height(self.s(lh)) };
        }
        if let Some(f) = ts.family.clone() {
            d = d.font_family(f);
        }
        if st.italic.or(ts.italic).unwrap_or(false) {
            d = d.italic();
        }
        d = match st.text_align.as_deref() {
            Some("center") => d.text_center(),
            Some("right" | "end") => d.text_right(),
            _ => d,
        };
        d
    }

    pub fn node(&self, n: &Node, overrides: Option<&std::collections::BTreeMap<String, String>>) -> AnyElement {
        self.node_inner(n, overrides, 0)
    }

    fn node_inner(&self, n: &Node, ov: Option<&std::collections::BTreeMap<String, String>>, depth: usize) -> AnyElement {
        if depth > 48 {
            return div().into_any_element();
        }
        let text = |n: &Node| -> Option<String> {
            if let Some(ov) = ov {
                if let Some(name) = &n.name
                    && let Some(t) = ov.get(name) {
                        return Some(t.clone());
                    }
                if let Some(t) = ov.get("text").or(ov.get("label"))
                    && n.text.is_some() && !self.text_used.replace(true) {
                        return Some(t.clone());
                    }
            }
            n.text.clone()
        };
        let st = &n.style;
        let is_row = match n.kind {
            NodeType::Row => true,
            NodeType::Box | NodeType::Unknown => st.direction.as_deref().is_some_and(|x| x.starts_with("row") || x == "horizontal"),
            _ => false,
        };
        let children = |d: Div| -> Div {
            n.children.iter().fold(d, |d, c| {
                let saved = self.parent_row.replace(is_row);
                let el = self.node_inner(c, ov, depth + 1);
                self.parent_row.set(saved);
                if n.kind == NodeType::Stack {
                    d.child(
                        div()
                            .absolute()
                            .left(self.s(c.style.x.unwrap_or(0.0)))
                            .top(self.s(c.style.y.unwrap_or(0.0)))
                            .child(el),
                    )
                } else {
                    d.child(el)
                }
            })
        };
        match n.kind {
            NodeType::Box | NodeType::Column | NodeType::Unknown => children(self.style(div(), st, false)).into_any_element(),
            NodeType::Row => children(self.style(div(), st, true)).into_any_element(),
            NodeType::Stack => children(self.style(div().relative(), st, false)).into_any_element(),
            NodeType::Text => self.style(div(), st, true).child(text(n).unwrap_or_default()).into_any_element(),
            NodeType::Icon => {
                let size = st.size.unwrap_or(20.0);
                let color = self.color(&st.fg).unwrap_or(gpui_kit::black().opacity(0.7));
                icon_dyn(n.icon.as_deref().unwrap_or("circle"))
                    .flex_shrink_0()
                    .size(self.s(size))
                    .text_color(color)
                    .into_any_element()
            }
            NodeType::Image => {
                let mut d = self.style(div(), st, false).items_center().justify_center().overflow_hidden();
                if st.bg.is_none() && st.gradient.is_none() {
                    d = d.bg(gpui_kit::black().opacity(0.08));
                }
                if matches!(st.height, Dim::Auto) {
                    d = d.h(self.s(160.0));
                }
                if matches!(st.width, Dim::Auto) {
                    d = d.w_full();
                }
                d.child(icon_dyn("image").size(self.s(28.0)).text_color(gpui_kit::black().opacity(0.3)))
                    .into_any_element()
            }
            NodeType::Spacer => div().flex_1().into_any_element(),
            NodeType::Divider => {
                let c = self.color(&st.bg).or(self.color(&st.fg)).unwrap_or(gpui_kit::black().opacity(0.1));
                div().w_full().h(self.s(1.0).max(sz(1.))).bg(c).into_any_element()
            }
            NodeType::Input => {
                let mut d = self.style(div(), st, true).items_center();
                if st.padding.is_none() {
                    d = d.px(self.s(14.0));
                }
                if matches!(st.height, Dim::Auto) {
                    d = d.h(self.s(44.0));
                }
                if st.radius.is_none() {
                    d = d.rounded(self.s(10.0));
                }
                if st.border.is_none() {
                    d = d.border_1().border_color(gpui_kit::black().opacity(0.15));
                }
                let value = text(n);
                let shown = value.clone().or(n.placeholder.clone()).unwrap_or_default();
                d.gap(self.s(8.0))
                    .when_some(n.icon.clone(), |d, ic| d.child(icon_dyn(&ic).size(self.s(16.0)).text_color(gpui_kit::black().opacity(0.45))))
                    .child(div().when(value.is_none(), |d| d.opacity(0.5)).child(shown))
                    .into_any_element()
            }
            NodeType::Button => {
                let mut d = self.style(div(), st, true).flex_shrink_0().items_center().justify_center().gap(self.s(8.0));
                if st.padding.is_none() {
                    d = d.py(self.s(10.0)).px(self.s(18.0));
                }
                if st.radius.is_none() {
                    d = d.rounded(self.s(10.0));
                }
                if st.bg.is_none() && st.gradient.is_none() && st.border.is_none() {
                    let primary = self.doc.color("$primary").map(rgba_u32).unwrap_or(hex(0x4F46E5));
                    d = d.bg(primary).text_color(gpui_kit::white());
                }
                if st.weight.is_none() {
                    d = d.font_weight(FontWeight::SEMIBOLD);
                }
                d.when_some(n.icon.clone(), |d, ic| d.child(icon_dyn(&ic).size(self.s(16.0)).text_color(self.color(&st.fg).unwrap_or(gpui_kit::white()))))
                    .when_some(text(n), |d, t| d.child(t))
                    .into_any_element()
            }
            NodeType::Switch => {
                let on = n.checked.unwrap_or(false);
                let primary = self.color(&st.bg).or(self.doc.color("$primary").map(rgba_u32)).unwrap_or(hex(0x4F46E5));
                div()
                    .flex()
                    .flex_none()
                    .flex_shrink_0()
                    .w(self.s(40.0))
                    .h(self.s(24.0))
                    .rounded_full()
                    .p(self.s(3.0))
                    .bg(if on { primary } else { gpui_kit::black().opacity(0.18) })
                    .when(on, |d| d.justify_end())
                    .child(div().size(self.s(18.0)).rounded_full().bg(gpui_kit::white()).shadow_sm())
                    .into_any_element()
            }
            NodeType::Checkbox => {
                let on = n.checked.unwrap_or(false);
                let primary = self.doc.color("$primary").map(rgba_u32).unwrap_or(hex(0x4F46E5));
                let mut row = div().flex().items_center().gap(self.s(8.0)).child(
                    div()
                        .size(self.s(18.0))
                        .rounded(self.s(5.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(on, |d| d.bg(primary))
                        .when(!on, |d| d.border_1().border_color(gpui_kit::black().opacity(0.3)))
                        .when(on, |d| d.child(icon_dyn("check").size(self.s(13.0)).text_color(gpui_kit::white()))),
                );
                if let Some(t) = text(n) {
                    row = self.text_style(row, st).child(t);
                }
                row.into_any_element()
            }
            NodeType::Progress => {
                let v = n.value.unwrap_or(0.5).clamp(0.0, 1.0);
                let primary = self.color(&st.fg).or(self.doc.color("$primary").map(rgba_u32)).unwrap_or(hex(0x4F46E5));
                let track = self.color(&st.bg).unwrap_or(gpui_kit::black().opacity(0.1));
                let mut d = div().h(self.s(8.0)).rounded_full().bg(track).overflow_hidden();
                d = match st.width {
                    Dim::Px(w) => d.w(self.s(w)),
                    _ => d.w_full(),
                };
                d.child(div().h_full().rounded_full().bg(primary).w(relative(v))).into_any_element()
            }
            NodeType::Ref => {
                let name = n.reference.as_deref().unwrap_or("");
                let Some(comp) = self.doc.component(name) else {
                    return div()
                        .p(self.s(8.0))
                        .border_1()
                        .border_color(hex(0xEF4444))
                        .text_size(self.s(12.0))
                        .text_color(hex(0xEF4444))
                        .child(format!("Missing component “{name}”"))
                        .into_any_element();
                };
                let variants = comp.variants();
                let v = n
                    .variant
                    .as_deref()
                    .and_then(|vn| variants.iter().find(|v| v.name.eq_ignore_ascii_case(vn)))
                    .or(variants.first());
                let Some(v) = v else { return div().into_any_element() };
                let mut ov = n.overrides.clone();
                if let Some(t) = &n.text {
                    ov.entry("text".into()).or_insert(t.clone());
                }
                let saved = self.text_used.replace(false);
                let el = self.node_inner(&v.node, Some(&ov), depth + 1);
                self.text_used.set(saved);
                // Instance-level sizing wraps the component.
                if !matches!(st.width, Dim::Auto) || st.margin.is_some() {
                    self.style(div(), st, true).child(el).into_any_element()
                } else {
                    el
                }
            }
        }
    }
}
