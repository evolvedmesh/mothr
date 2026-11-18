//! Export a design to shareable formats.
//!
//! - [`export_html`]: one self-contained HTML file with every screen, the design system page and
//!   navigation between them. Fonts and the icons in use are embedded, so it works offline.
//! - [`export_storybook`]: a Storybook project with a story per component variant and per screen,
//!   plus a tokens page. [`build_storybook`] turns it into a static site.
//!
//! Both use the same HTML renderer, a port of the desktop canvas renderer
//! (`apps/desktop/src/ui/design/canvas.rs`): the same flex semantics (a `fill` size grows only along the
//! parent's main axis, column children don't shrink), token resolution, defaults and component
//! overrides. Keep the two in step.

use crate::{DesignDoc, Device, Dim, Length, Node, NodeType, Screen, Style, TokenScale};
use base64::Engine as _;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// A font file to embed (`@font-face`).
#[derive(Clone)]
pub struct FontFile {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    /// File name used when the font is written next to the export (Storybook).
    pub file_name: String,
    pub data: Vec<u8>,
}

/// Resources the export embeds. The design crate doesn't know where icons and fonts live, so the
/// caller (the desktop app) provides them.
#[derive(Default)]
pub struct ExportAssets {
    pub fonts: Vec<FontFile>,
    /// Returns the SVG source for a Lucide icon name (`"search"`, `"chevron-right"`).
    #[allow(clippy::type_complexity)]
    pub icons: Option<Box<dyn Fn(&str) -> Option<String> + Send + Sync>>,
}

impl ExportAssets {
    fn icon_svg(&self, name: &str) -> Option<String> {
        self.icons.as_ref().and_then(|f| f(name))
    }
}

/// The title shown for a design: its system name, else the fallback (usually the project name).
pub fn design_title(doc: &DesignDoc, fallback: &str) -> String {
    doc.system
        .as_ref()
        .map(|s| s.name.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| if fallback.trim().is_empty() { "Untitled design".into() } else { fallback.trim().into() })
}

// ---------------------------------------------------------------------------------------------
// Tiny HTML tree
// ---------------------------------------------------------------------------------------------

/// Inline style with "last write wins" semantics, like chaining GPUI style calls.
#[derive(Default, Clone)]
struct Css(Vec<(&'static str, String)>);

impl Css {
    fn set(&mut self, prop: &'static str, val: impl Into<String>) {
        let val = val.into();
        if let Some(e) = self.0.iter_mut().find(|(p, _)| *p == prop) {
            e.1 = val;
        } else {
            self.0.push((prop, val));
        }
    }
    fn render(&self) -> String {
        let mut s = String::new();
        for (p, v) in &self.0 {
            let _ = write!(s, "{p}:{v};");
        }
        s
    }
}

enum Html {
    El(El),
    Text(String),
    Raw(String),
}

struct El {
    tag: &'static str,
    class: Option<&'static str>,
    css: Css,
    attrs: Vec<(&'static str, String)>,
    children: Vec<Html>,
}

impl El {
    fn div() -> Self {
        El { tag: "div", class: None, css: Css::default(), attrs: Vec::new(), children: Vec::new() }
    }
    fn set(mut self, prop: &'static str, val: impl Into<String>) -> Self {
        self.css.set(prop, val);
        self
    }
    fn child(mut self, c: Html) -> Self {
        self.children.push(c);
        self
    }
    fn text(self, t: impl Into<String>) -> Self {
        self.child(Html::Text(t.into()))
    }
    fn render(&self, out: &mut String) {
        let _ = write!(out, "<{}", self.tag);
        if let Some(c) = self.class {
            let _ = write!(out, " class=\"{c}\"");
        }
        for (k, v) in &self.attrs {
            let _ = write!(out, " {k}=\"{}\"", esc(v));
        }
        if !self.css.0.is_empty() {
            let _ = write!(out, " style=\"{}\"", esc(&self.css.render()));
        }
        out.push('>');
        for c in &self.children {
            c.render(out);
        }
        let _ = write!(out, "</{}>", self.tag);
    }
}

impl Html {
    fn render(&self, out: &mut String) {
        match self {
            Html::El(e) => e.render(out),
            Html::Text(t) => out.push_str(&esc(t)),
            Html::Raw(r) => out.push_str(r),
        }
    }
    fn html(&self) -> String {
        let mut s = String::new();
        self.render(&mut s);
        s
    }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

fn px(v: f32) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 { format!("{}px", r as i64) } else { format!("{r}px") }
}

/// 0xRRGGBBAA → CSS.
fn css_color(c: u32) -> String {
    let (r, g, b, a) = (c >> 24, (c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
    if a == 0xFF {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("rgba({r},{g},{b},{:.3})", a as f32 / 255.0)
    }
}

fn black(alpha: f32) -> String {
    format!("rgba(0,0,0,{alpha})")
}

const WHITE: &str = "#ffffff";
const DEFAULT_PRIMARY: &str = "#4f46e5";
const SHADOW_MD: &str = "0 4px 6px -1px rgba(0,0,0,0.1),0 2px 4px -2px rgba(0,0,0,0.1)";
const SHADOW_SM: &str = "0 1px 3px 0 rgba(0,0,0,0.1),0 1px 2px -1px rgba(0,0,0,0.1)";

/// Relative luminance of 0xRRGGBBAA (ignores alpha).
fn luminance(c: u32) -> f32 {
    let ch = |v: u32| {
        let v = v as f32 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * ch(c >> 24) + 0.7152 * ch((c >> 16) & 0xFF) + 0.0722 * ch((c >> 8) & 0xFF)
}

// ---------------------------------------------------------------------------------------------
// Node renderer (port of the desktop canvas `Renderer`)
// ---------------------------------------------------------------------------------------------

/// Converts design nodes to HTML. Collects the icons it uses so they can be embedded once.
pub struct HtmlRenderer<'a> {
    doc: &'a DesignDoc,
    text_used: Cell<bool>,
    parent_row: Cell<bool>,
    icons: RefCell<BTreeSet<String>>,
}

type Overrides = BTreeMap<String, String>;

impl<'a> HtmlRenderer<'a> {
    pub fn new(doc: &'a DesignDoc) -> Self {
        Self { doc, text_used: Cell::new(false), parent_row: Cell::new(false), icons: RefCell::default() }
    }

    /// Icon names referenced so far.
    pub fn used_icons(&self) -> Vec<String> {
        self.icons.borrow().iter().cloned().collect()
    }

    /// Render one node (a component variant or a screen root) as an HTML fragment.
    pub fn node_html(&self, n: &Node) -> String {
        self.node(n, None, 0).html()
    }

    /// Render a screen as a sized artboard, mirroring the canvas frame.
    pub fn screen_html(&self, s: &Screen) -> String {
        let (w, h) = s.size();
        let bg = s.background.as_deref().and_then(|b| self.doc.color(b)).unwrap_or(0xFFFFFFFF);
        let fg = if luminance(bg) < 0.35 { "#f4f4f5" } else { "#111114" };
        let radius = if matches!(s.device, Device::Mobile) { 36.0 } else { 10.0 };
        self.parent_row.set(false);
        let root = El::div()
            .set("width", "100%")
            .set("height", "100%")
            .set("display", "flex")
            .set("flex-direction", "column")
            .child(self.node(&s.root, None, 0));
        let mut frame = El::div()
            .set("width", px(w))
            .set("height", px(h))
            .set("border-radius", px(radius))
            .set("overflow", "hidden")
            .set("background", css_color(bg))
            .set("color", fg)
            .set("box-shadow", "0 20px 25px -5px rgba(0,0,0,0.1),0 8px 10px -6px rgba(0,0,0,0.1)")
            .set("flex", "none")
            .child(Html::El(root));
        frame.class = Some("mk artboard");
        Html::El(frame).html()
    }

    fn color(&self, v: &Option<String>) -> Option<String> {
        v.as_deref().and_then(|c| self.doc.color(c)).map(css_color)
    }

    fn primary(&self) -> String {
        self.doc.color("$primary").map(css_color).unwrap_or_else(|| DEFAULT_PRIMARY.into())
    }

    fn len(&self, l: &Length, scale: TokenScale) -> f32 {
        self.doc.length(l, scale)
    }

    fn shadow(&self, name: &str) -> String {
        let Some(sh) = self.doc.shadow(name) else { return SHADOW_MD.into() };
        let color = self.color(&sh.color).unwrap_or_else(|| black(0.25));
        format!(
            "{} {} {} {} {color}",
            px(sh.x.unwrap_or(0.0)),
            px(sh.y.unwrap_or(4.0)),
            px(sh.blur.unwrap_or(12.0)),
            px(sh.spread.unwrap_or(0.0))
        )
    }

    fn style(&self, mut d: El, st: &Style, default_dir_row: bool) -> El {
        let row = st.direction.as_deref().map(|x| x.starts_with("row") || x == "horizontal").unwrap_or(default_dir_row);
        d.css.set("display", "flex");
        let in_row = self.parent_row.get();
        if !in_row && !matches!(st.height, Dim::Fill) && st.grow.is_none() {
            d.css.set("flex-shrink", "0");
        }
        d.css.set("flex-direction", if row { "row" } else { "column" });
        match st.width {
            Dim::Px(v) => {
                d.css.set("width", px(v));
                flex_none(&mut d.css);
            }
            Dim::Fill if in_row => {
                d.css.set("width", "100%");
                d.css.set("flex-grow", "1");
                d.css.set("flex-basis", "0px");
            }
            Dim::Fill => d.css.set("width", "100%"),
            Dim::Percent(p) => d.css.set("width", format!("{p}%")),
            Dim::Auto => {}
        }
        match st.height {
            Dim::Px(v) => d.css.set("height", px(v)),
            Dim::Fill if !in_row => {
                d.css.set("height", "100%");
                d.css.set("flex-grow", "1");
                d.css.set("flex-basis", "0px");
            }
            Dim::Fill => d.css.set("height", "100%"),
            Dim::Percent(p) => d.css.set("height", format!("{p}%")),
            Dim::Auto => {}
        }
        if let Some(v) = st.min_width {
            d.css.set("min-width", px(v));
        }
        if let Some(v) = st.max_width {
            d.css.set("max-width", px(v));
        }
        if let Some(v) = st.min_height {
            d.css.set("min-height", px(v));
        }
        if let Some(g) = st.grow {
            if g == 0.0 {
                flex_none(&mut d.css);
            } else {
                d.css.set("flex-grow", format!("{g}"));
            }
        }
        if let Some(p) = &st.padding {
            d.css.set(
                "padding",
                format!(
                    "{} {} {} {}",
                    px(self.len(&p.top, TokenScale::Spacing)),
                    px(self.len(&p.right, TokenScale::Spacing)),
                    px(self.len(&p.bottom, TokenScale::Spacing)),
                    px(self.len(&p.left, TokenScale::Spacing))
                ),
            );
        }
        if let Some(m) = &st.margin {
            d.css.set(
                "margin",
                format!(
                    "{} {} {} {}",
                    px(self.len(&m.top, TokenScale::Spacing)),
                    px(self.len(&m.right, TokenScale::Spacing)),
                    px(self.len(&m.bottom, TokenScale::Spacing)),
                    px(self.len(&m.left, TokenScale::Spacing))
                ),
            );
        }
        if let Some(g) = &st.gap {
            d.css.set("gap", px(self.len(g, TokenScale::Spacing)));
        }
        match st.align.as_deref() {
            Some("center") => d.css.set("align-items", "center"),
            Some("end" | "flex-end" | "bottom" | "right") => d.css.set("align-items", "flex-end"),
            Some("start" | "flex-start" | "top" | "left") => d.css.set("align-items", "flex-start"),
            Some("baseline") => d.css.set("align-items", "baseline"),
            _ => {}
        }
        match st.justify.as_deref() {
            Some("center") => d.css.set("justify-content", "center"),
            Some("end" | "flex-end") => d.css.set("justify-content", "flex-end"),
            Some("between" | "space-between") => d.css.set("justify-content", "space-between"),
            Some("around" | "space-around" | "evenly" | "space-evenly") => d.css.set("justify-content", "space-around"),
            _ => {}
        }
        if st.wrap.unwrap_or(false) {
            d.css.set("flex-wrap", "wrap");
        }
        if let Some(grad) = st.gradient.as_ref().filter(|g| g.len() >= 2) {
            if let (Some(a), Some(b)) = (self.doc.color(&grad[0]), self.doc.color(&grad[1])) {
                d.css.set("background", format!("linear-gradient(180deg,{},{})", css_color(a), css_color(b)));
            }
        } else if let Some(bg) = self.color(&st.bg) {
            d.css.set("background", bg);
        }
        if let Some(r) = &st.radius {
            d.css.set("border-radius", px(self.len(r, TokenScale::Radius)));
        }
        if let Some(b) = &st.border {
            let w = b.width.unwrap_or(1.0);
            d.css.set("border", format!("{} solid {}", px(w), self.color(&b.color).unwrap_or_else(|| black(0.15))));
        }
        if let Some(sh) = &st.shadow {
            d.css.set("box-shadow", self.shadow(sh));
        }
        if let Some(o) = st.opacity {
            d.css.set("opacity", format!("{}", o.clamp(0.0, 1.0)));
        }
        let clipped = st.overflow.as_deref() == Some("hidden");
        if clipped {
            d.css.set("overflow", "hidden");
        }
        // GPUI measures text unwrapped for min/max-content, so inside a row an item never shrinks
        // below its one-line width (taffy's automatic minimum size). CSS would use the longest
        // word instead and wrap; match the canvas.
        if in_row && st.min_width.is_none() && !clipped && !matches!(st.width, Dim::Px(_)) {
            d.css.set("min-width", "max-content");
        }
        self.text_style(d, st)
    }

    fn text_style(&self, mut d: El, st: &Style) -> El {
        let ts = st.font.as_deref().and_then(|f| self.doc.type_style(f)).cloned().unwrap_or_default();
        if let Some(fg) = self.color(&st.fg) {
            d.css.set("color", fg);
        }
        d.css.set("font-size", px(st.size.or(ts.size).unwrap_or(15.0)));
        if let Some(w) = st.weight.or(ts.weight) {
            d.css.set("font-weight", format!("{}", w.round() as i32));
        }
        if let Some(lh) = ts.line_height {
            d.css.set("line-height", if lh <= 4.0 { format!("{lh}") } else { px(lh) });
        }
        if let Some(ls) = ts.letter_spacing {
            d.css.set("letter-spacing", px(ls));
        }
        if let Some(f) = ts.family.clone() {
            d.css.set("font-family", format!("{},Geist,system-ui,sans-serif", css_family(&f)));
        }
        if st.italic.or(ts.italic).unwrap_or(false) {
            d.css.set("font-style", "italic");
        }
        match st.text_align.as_deref() {
            Some("center") => d.css.set("text-align", "center"),
            Some("right" | "end") => d.css.set("text-align", "right"),
            _ => {}
        }
        d
    }

    fn icon(&self, name: &str, size: f32, color: &str) -> Html {
        let name = icon_name(name);
        self.icons.borrow_mut().insert(name.clone());
        Html::Raw(format!(
            "<svg class=\"i\" width=\"{s}\" height=\"{s}\" style=\"color:{c};width:{s};height:{s};flex-shrink:0\"><use href=\"#i-{n}\"/></svg>",
            s = px(size),
            c = esc(color),
            n = esc(&name)
        ))
    }

    fn node(&self, n: &Node, ov: Option<&Overrides>, depth: usize) -> Html {
        if depth > 48 {
            return Html::El(El::div());
        }
        let text = |n: &Node| -> Option<String> {
            if let Some(ov) = ov {
                if let Some(name) = &n.name
                    && let Some(t) = ov.get(name)
                {
                    return Some(t.clone());
                }
                if let Some(t) = ov.get("text").or(ov.get("label"))
                    && n.text.is_some()
                    && !self.text_used.replace(true)
                {
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
        let children = |mut d: El| -> El {
            for c in &n.children {
                let saved = self.parent_row.replace(is_row);
                let el = self.node(c, ov, depth + 1);
                self.parent_row.set(saved);
                if n.kind == NodeType::Stack {
                    d.children.push(Html::El(
                        El::div()
                            .set("position", "absolute")
                            .set("left", px(c.style.x.unwrap_or(0.0)))
                            .set("top", px(c.style.y.unwrap_or(0.0)))
                            .child(el),
                    ));
                } else {
                    d.children.push(el);
                }
            }
            d
        };
        let el = match n.kind {
            NodeType::Box | NodeType::Column | NodeType::Unknown => children(self.style(El::div(), st, false)),
            NodeType::Row => children(self.style(El::div(), st, true)),
            NodeType::Stack => children(self.style(El::div().set("position", "relative"), st, false)),
            NodeType::Text => self.style(El::div(), st, true).text(text(n).unwrap_or_default()),
            NodeType::Icon => {
                let size = st.size.unwrap_or(20.0);
                let color = self.color(&st.fg).unwrap_or_else(|| black(0.7));
                return self.icon(n.icon.as_deref().unwrap_or("circle"), size, &color);
            }
            NodeType::Image => {
                let mut d = self
                    .style(El::div(), st, false)
                    .set("align-items", "center")
                    .set("justify-content", "center")
                    .set("overflow", "hidden");
                if st.bg.is_none() && st.gradient.is_none() {
                    d.css.set("background", black(0.08));
                }
                if matches!(st.height, Dim::Auto) {
                    d.css.set("height", px(160.0));
                }
                if matches!(st.width, Dim::Auto) {
                    d.css.set("width", "100%");
                }
                if let Some(alt) = &n.placeholder {
                    d.attrs.push(("title", alt.clone()));
                }
                d.child(self.icon("image", 28.0, &black(0.3)))
            }
            NodeType::Spacer => El::div().set("flex", "1 1 0%"),
            NodeType::Divider => {
                let c = self.color(&st.bg).or(self.color(&st.fg)).unwrap_or_else(|| black(0.1));
                El::div().set("width", "100%").set("height", "1px").set("flex-shrink", "0").set("background", c)
            }
            NodeType::Input => {
                let mut d = self.style(El::div(), st, true).set("align-items", "center");
                if st.padding.is_none() {
                    d.css.set("padding-left", px(14.0));
                    d.css.set("padding-right", px(14.0));
                }
                if matches!(st.height, Dim::Auto) {
                    d.css.set("height", px(44.0));
                }
                if st.radius.is_none() {
                    d.css.set("border-radius", px(10.0));
                }
                if st.border.is_none() {
                    d.css.set("border", format!("1px solid {}", black(0.15)));
                }
                d.css.set("gap", px(8.0));
                if let Some(ic) = &n.icon {
                    d = d.child(self.icon(ic, 16.0, &black(0.45)));
                }
                let value = text(n);
                let shown = value.clone().or(n.placeholder.clone()).unwrap_or_default();
                let mut span = El::div().text(shown);
                if value.is_none() {
                    span.css.set("opacity", "0.5");
                }
                d.child(Html::El(span))
            }
            NodeType::Button => {
                let mut d = self
                    .style(El::div(), st, true)
                    .set("flex-shrink", "0")
                    .set("align-items", "center")
                    .set("justify-content", "center")
                    .set("gap", px(8.0));
                if st.padding.is_none() {
                    d.css.set("padding", format!("{} {}", px(10.0), px(18.0)));
                }
                if st.radius.is_none() {
                    d.css.set("border-radius", px(10.0));
                }
                if st.bg.is_none() && st.gradient.is_none() && st.border.is_none() {
                    d.css.set("background", self.primary());
                    d.css.set("color", WHITE);
                }
                if st.weight.is_none() {
                    d.css.set("font-weight", "600");
                }
                if let Some(ic) = &n.icon {
                    let c = self.color(&st.fg).unwrap_or_else(|| WHITE.into());
                    d = d.child(self.icon(ic, 16.0, &c));
                }
                match text(n) {
                    Some(t) => d.text(t),
                    None => d,
                }
            }
            NodeType::Switch => {
                let on = n.checked.unwrap_or(false);
                let primary = self.color(&st.bg).unwrap_or_else(|| self.primary());
                let mut d = El::div()
                    .set("display", "flex")
                    .set("flex", "none")
                    .set("width", px(40.0))
                    .set("height", px(24.0))
                    .set("border-radius", "9999px")
                    .set("padding", px(3.0))
                    .set("background", if on { primary } else { black(0.18) });
                if on {
                    d.css.set("justify-content", "flex-end");
                }
                d.child(Html::El(
                    El::div()
                        .set("width", px(18.0))
                        .set("height", px(18.0))
                        .set("border-radius", "9999px")
                        .set("background", WHITE)
                        .set("box-shadow", SHADOW_SM),
                ))
            }
            NodeType::Checkbox => {
                let on = n.checked.unwrap_or(false);
                let primary = self.primary();
                let mut bx = El::div()
                    .set("width", px(18.0))
                    .set("height", px(18.0))
                    .set("border-radius", px(5.0))
                    .set("display", "flex")
                    .set("align-items", "center")
                    .set("justify-content", "center")
                    .set("flex-shrink", "0");
                if on {
                    bx.css.set("background", primary);
                    bx = bx.child(self.icon("check", 13.0, WHITE));
                } else {
                    bx.css.set("border", format!("1px solid {}", black(0.3)));
                }
                let mut row = El::div().set("display", "flex").set("align-items", "center").set("gap", px(8.0)).child(Html::El(bx));
                if let Some(t) = text(n) {
                    row = self.text_style(row, st).text(t);
                }
                row
            }
            NodeType::Progress => {
                let v = n.value.unwrap_or(0.5).clamp(0.0, 1.0);
                let primary = self.color(&st.fg).unwrap_or_else(|| self.primary());
                let track = self.color(&st.bg).unwrap_or_else(|| black(0.1));
                let w = match st.width {
                    Dim::Px(w) => px(w),
                    _ => "100%".into(),
                };
                El::div()
                    .set("height", px(8.0))
                    .set("border-radius", "9999px")
                    .set("background", track)
                    .set("overflow", "hidden")
                    .set("width", w)
                    .set("flex-shrink", "0")
                    .child(Html::El(
                        El::div()
                            .set("height", "100%")
                            .set("border-radius", "9999px")
                            .set("background", primary)
                            .set("width", format!("{}%", (v * 1000.0).round() / 10.0)),
                    ))
            }
            NodeType::Ref => {
                let name = n.reference.as_deref().unwrap_or("");
                let Some(comp) = self.doc.component(name) else {
                    return Html::El(
                        El::div()
                            .set("padding", px(8.0))
                            .set("border", "1px solid #ef4444")
                            .set("font-size", px(12.0))
                            .set("color", "#ef4444")
                            .text(format!("Missing component “{name}”")),
                    );
                };
                let variants = comp.variants();
                let v = n
                    .variant
                    .as_deref()
                    .and_then(|vn| variants.iter().find(|v| v.name.eq_ignore_ascii_case(vn)))
                    .or(variants.first());
                let Some(v) = v else { return Html::El(El::div()) };
                let mut inst = n.overrides.clone();
                if let Some(t) = &n.text {
                    inst.entry("text".into()).or_insert(t.clone());
                }
                let saved = self.text_used.replace(false);
                let el = self.node(&v.node, Some(&inst), depth + 1);
                self.text_used.set(saved);
                if !matches!(st.width, Dim::Auto) || st.margin.is_some() {
                    return Html::El(self.style(El::div(), st, true).child(el));
                }
                return el;
            }
        };
        Html::El(el)
    }

    /// `<svg>` sprite with a `<symbol>` for every icon used so far.
    pub fn sprite(&self, assets: &ExportAssets) -> String {
        let mut out = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" style=\"display:none\" aria-hidden=\"true\">");
        for name in self.icons.borrow().iter() {
            out.push_str(&icon_symbol(name, assets.icon_svg(name).as_deref()));
        }
        out.push_str("</svg>");
        out
    }
}

fn flex_none(css: &mut Css) {
    css.set("flex-grow", "0");
    css.set("flex-shrink", "0");
    css.set("flex-basis", "auto");
}

fn css_family(f: &str) -> String {
    let f = f.trim().trim_matches(['"', '\'']);
    format!("\"{}\"", f.replace('"', ""))
}

/// The canvas looks icons up as `icons/<name>.svg` after normalising the name.
fn icon_name(name: &str) -> String {
    name.trim().to_ascii_lowercase().replace(' ', "-")
}

/// Turn an icon's SVG source into a `<symbol>`. Missing icons render as nothing, like the canvas.
fn icon_symbol(name: &str, svg: Option<&str>) -> String {
    let id = format!("i-{}", esc(name));
    let Some(svg) = svg else { return format!("<symbol id=\"{id}\" viewBox=\"0 0 24 24\"></symbol>") };
    let Some(start) = svg.find("<svg") else { return format!("<symbol id=\"{id}\" viewBox=\"0 0 24 24\"></symbol>") };
    let Some(open_end) = svg[start..].find('>').map(|i| start + i) else { return String::new() };
    let close = svg.rfind("</svg>").unwrap_or(svg.len());
    let attrs_src = &svg[start + 4..open_end];
    let inner = svg.get(open_end + 1..close).unwrap_or("").trim();
    // Keep presentation attributes (fill, stroke, viewBox…); drop size and namespace.
    let mut attrs = String::new();
    let mut has_viewbox = false;
    for (k, v) in parse_attrs(attrs_src) {
        match k.as_str() {
            "width" | "height" | "xmlns" | "class" | "id" | "style" => {}
            _ => {
                if k == "viewBox" {
                    has_viewbox = true;
                }
                let _ = write!(attrs, " {k}=\"{}\"", esc(&v));
            }
        }
    }
    if !has_viewbox {
        attrs.push_str(" viewBox=\"0 0 24 24\"");
    }
    let inner = inner.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("<symbol id=\"{id}\"{attrs}>{inner}</symbol>")
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
        let Some(end) = after[1..].find(q) else { break };
        out.push((key, after[1..1 + end].to_string()));
        rest = after[end + 2..].trim_start();
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Design-system page (shared by the HTML export and the Storybook tokens story)
// ---------------------------------------------------------------------------------------------

fn canvas_bg(doc: &DesignDoc) -> String {
    doc.color("$bg")
        .or_else(|| doc.color("$background"))
        .or_else(|| doc.color("$surface"))
        .map(css_color)
        .unwrap_or_else(|| "var(--muted)".into())
}

/// Tokens section: color, typography, spacing, radii, elevation.
fn tokens_html(doc: &DesignDoc, r: &HtmlRenderer) -> String {
    let Some(sys) = &doc.system else { return String::new() };
    let t = &sys.tokens;
    let mut o = String::new();
    if !t.colors.is_empty() {
        o.push_str("<section class=\"ds-sec\"><h2>Color</h2><div class=\"swatches\">");
        for (name, val) in t.colors.iter() {
            let c = doc.color(val).map(css_color).unwrap_or_else(|| "transparent".into());
            let _ = write!(
                o,
                "<div class=\"swatch\"><div class=\"chip\" style=\"background:{}\"></div><div class=\"meta\"><b>{}</b><span>{}</span></div></div>",
                esc(&c),
                esc(name),
                esc(val)
            );
        }
        o.push_str("</div></section>");
    }
    if !t.typography.is_empty() {
        o.push_str("<section class=\"ds-sec\"><h2>Typography</h2><div class=\"type-list\">");
        for (name, ts) in t.typography.iter() {
            let size = ts.size.unwrap_or(16.0);
            let weight = ts.weight.unwrap_or(400.0) as u32;
            let mut style = format!("font-size:{};font-weight:{weight};", px(size));
            if let Some(f) = &ts.family {
                let _ = write!(style, "font-family:{},Geist,system-ui,sans-serif;", css_family(f));
            }
            if ts.italic.unwrap_or(false) {
                style.push_str("font-style:italic;");
            }
            if let Some(lh) = ts.line_height {
                let _ = write!(style, "line-height:{};", if lh <= 4.0 { format!("{lh}") } else { px(lh) });
            }
            if let Some(ls) = ts.letter_spacing {
                let _ = write!(style, "letter-spacing:{};", px(ls));
            }
            let lh = ts.line_height.map(|l| format!(" · {l}")).unwrap_or_default();
            let _ = write!(
                o,
                "<div class=\"type-row\"><div class=\"meta\"><b>{}</b><span>{size}px · {weight}{lh}</span></div><div class=\"sample\" style=\"{}\">The quick brown fox jumps over the lazy dog</div></div>",
                esc(name),
                esc(&style)
            );
        }
        o.push_str("</div></section>");
    }
    if !t.spacing.is_empty() || !t.radii.is_empty() {
        o.push_str("<section class=\"ds-sec ds-scales\">");
        if !t.spacing.is_empty() {
            o.push_str("<div><h3>Spacing</h3><div class=\"spacing\">");
            for (n, v) in t.spacing.iter() {
                let _ = write!(
                    o,
                    "<div class=\"sp-row\"><span class=\"k\">{}</span><span class=\"bar\" style=\"width:{}\"></span><span class=\"v\">{}</span></div>",
                    esc(n),
                    px(v.0.max(1.0)),
                    v.0
                );
            }
            o.push_str("</div></div>");
        }
        if !t.radii.is_empty() {
            o.push_str("<div><h3>Radii</h3><div class=\"radii\">");
            for (n, v) in t.radii.iter() {
                let _ = write!(
                    o,
                    "<div class=\"radius\"><div class=\"box\" style=\"border-radius:{}\"></div><span>{} · {}</span></div>",
                    px(v.0.min(28.0)),
                    esc(n),
                    v.0
                );
            }
            o.push_str("</div></div>");
        }
        o.push_str("</section>");
    }
    if !t.shadows.is_empty() {
        o.push_str("<section class=\"ds-sec\"><h2>Elevation</h2><div class=\"shadows\">");
        for (n, _) in t.shadows.iter() {
            let _ = write!(
                o,
                "<div class=\"shadow\"><div class=\"box\" style=\"box-shadow:{}\"></div><span>{}</span></div>",
                esc(&r.shadow(&format!("${n}"))),
                esc(n)
            );
        }
        o.push_str("</div></section>");
    }
    o
}

/// Full design-system page: description, tokens and every component variant.
fn system_html(doc: &DesignDoc, r: &HtmlRenderer) -> String {
    let Some(sys) = &doc.system else {
        return "<p class=\"empty\">This design has no system.json yet.</p>".into();
    };
    let mut o = String::new();
    if let Some(d) = &sys.description {
        let _ = write!(o, "<p class=\"lede\">{}</p>", esc(d));
    }
    o.push_str(&tokens_html(doc, r));
    if !sys.components.is_empty() {
        o.push_str("<section class=\"ds-sec\"><h2>Components</h2><div class=\"components\">");
        let bg = canvas_bg(doc);
        for c in &sys.components {
            let _ = write!(o, "<div class=\"component\" id=\"c-{}\"><div class=\"c-head\"><b>{}</b>", slug(&c.name), esc(&c.name));
            if let Some(d) = &c.description {
                let _ = write!(o, "<span>{}</span>", esc(d));
            }
            let _ = write!(o, "</div><div class=\"c-stage\" style=\"background:{}\">", esc(&bg));
            for v in c.variants() {
                let _ = write!(o, "<div class=\"variant\"><div class=\"mk\">{}</div><span>{}</span></div>", r.node_html(&v.node), esc(&v.name));
            }
            o.push_str("</div></div>");
        }
        o.push_str("</div></section>");
    }
    o
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "item".into() } else { out }
}

fn font_faces(assets: &ExportAssets, url: impl Fn(&FontFile) -> String) -> String {
    let mut o = String::new();
    for f in &assets.fonts {
        let _ = writeln!(
            o,
            "@font-face{{font-family:{};src:url({}) format(\"truetype\");font-weight:{};font-style:{};font-display:swap}}",
            css_family(&f.family),
            url(f),
            f.weight,
            if f.italic { "italic" } else { "normal" }
        );
    }
    o
}

/// Base CSS for rendered mock content.
const MOCK_CSS: &str = r#"
.mk,.mk *{box-sizing:border-box}
.mk{font-family:Geist,system-ui,-apple-system,"Segoe UI",sans-serif;font-size:15px;line-height:1.618;-webkit-font-smoothing:antialiased;text-rendering:optimizeLegibility}
.mk svg.i{display:block}
"#;

/// CSS for the design-system page (also used by the Storybook tokens story).
const SYSTEM_CSS: &str = r#"
.ds{display:flex;flex-direction:column;gap:40px;padding:32px;font-family:Geist,system-ui,sans-serif;color:var(--text)}
.ds h2{font-size:18px;font-weight:600;margin:0 0 14px}
.ds h3{font-size:11.5px;font-weight:600;letter-spacing:.06em;text-transform:uppercase;color:var(--faint);margin:0 0 10px}
.ds .lede{color:var(--muted-text);max-width:640px;margin:0;line-height:1.6}
.ds .swatches{display:flex;flex-wrap:wrap;gap:12px}
.ds .swatch{width:128px;border:1px solid var(--border);border-radius:12px;overflow:hidden;background:var(--surface)}
.ds .swatch .chip{height:72px}
.ds .meta{display:flex;flex-direction:column;padding:8px;font-size:12px;gap:2px}
.ds .meta span{color:var(--muted-text);font-family:"Geist Mono",ui-monospace,monospace;font-size:11px;word-break:break-all}
.ds .type-list{display:flex;flex-direction:column;gap:16px}
.ds .type-row{display:flex;gap:24px;align-items:center}
.ds .type-row .meta{width:140px;flex:none;padding:0}
.ds .type-row .sample{flex:1;min-width:0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.ds .ds-scales{display:flex;gap:40px;align-items:flex-start;flex-wrap:wrap}
.ds .spacing{display:flex;flex-direction:column;gap:8px}
.ds .sp-row{display:flex;gap:12px;align-items:center;font-size:12px;color:var(--muted-text)}
.ds .sp-row .k{width:48px}
.ds .sp-row .bar{height:10px;border-radius:2px;background:var(--accent);opacity:.7}
.ds .radii,.ds .shadows{display:flex;gap:16px;flex-wrap:wrap}
.ds .radius,.ds .shadow{display:flex;flex-direction:column;align-items:center;gap:6px;font-size:12px;color:var(--muted-text)}
.ds .radius .box{width:56px;height:56px;border:2px solid var(--accent);background:color-mix(in srgb,var(--accent) 8%,transparent)}
.ds .shadows{padding:16px;gap:24px}
.ds .shadow .box{width:88px;height:88px;border-radius:12px;background:var(--surface)}
.ds .components{display:flex;flex-direction:column;gap:24px}
.ds .component{display:flex;flex-direction:column;gap:12px;padding:20px;border:1px solid var(--border);border-radius:14px}
.ds .c-head{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap}
.ds .c-head span{font-size:13px;color:var(--muted-text)}
.ds .c-stage{display:flex;flex-wrap:wrap;gap:24px;align-items:flex-end;padding:24px;border-radius:10px}
.ds .variant{display:flex;flex-direction:column;gap:8px}
.ds .variant>span{font-size:12px;color:var(--muted-text)}
"#;

/// Viewer chrome (quiet, follows the OS light/dark preference).
const VIEWER_CSS: &str = r#"
:root{--bg:#0b0c0f;--chrome:#0f1014;--surface:#14151a;--muted:#1a1b21;--border:#1f2027;--text:#ececf1;--muted-text:#9b9daa;--faint:#60626e;--accent:#8b7cf6;--canvas:#0b0c0f;color-scheme:dark}
@media (prefers-color-scheme:light){:root{--bg:#fcfcfd;--chrome:#f5f5f8;--surface:#fff;--muted:#f1f1f5;--border:#e7e7ec;--text:#15161a;--muted-text:#5f6170;--faint:#9698a4;--accent:#6d5bef;--canvas:#f1f1f4;color-scheme:light}}
*{box-sizing:border-box}
html,body{margin:0;height:100%}
body{display:flex;background:var(--bg);color:var(--text);font-family:Geist,system-ui,-apple-system,"Segoe UI",sans-serif;font-size:14px;-webkit-font-smoothing:antialiased}
nav{width:248px;flex:none;background:var(--chrome);border-right:1px solid var(--border);padding:18px 12px;display:flex;flex-direction:column;gap:2px;overflow-y:auto}
nav .title{font-weight:600;font-size:15px;padding:0 10px 4px}
nav .sub{font-size:12px;color:var(--faint);padding:0 10px 14px}
nav .group{font-size:11.5px;font-weight:500;color:var(--faint);padding:14px 10px 6px}
nav a{display:flex;justify-content:space-between;gap:8px;padding:7px 10px;border-radius:8px;color:var(--muted-text);text-decoration:none}
nav a:hover{background:var(--muted);color:var(--text)}
nav a.on{background:var(--muted);color:var(--text);font-weight:500;box-shadow:inset 2.5px 0 0 var(--accent)}
nav a small{color:var(--faint);font-size:11.5px}
nav .foot{margin-top:auto;padding:16px 10px 0;font-size:11.5px;color:var(--faint)}
main{flex:1;min-width:0;overflow:auto;background:var(--canvas)}
.page{display:none}
.page.on{display:block}
.bar{position:sticky;top:0;z-index:2;display:flex;align-items:center;gap:12px;padding:12px 24px;background:var(--bg);border-bottom:1px solid var(--border)}
.bar h1{font-size:15px;font-weight:600;margin:0}
.bar .dim{color:var(--faint);font-size:12.5px}
.bar .grow{flex:1}
.zoom{display:flex;gap:2px;padding:3px;border-radius:9px;background:var(--muted)}
.zoom button{border:0;background:none;color:var(--muted-text);font:inherit;font-size:12.5px;padding:4px 10px;border-radius:6px;cursor:pointer}
.zoom button.on{background:var(--bg);color:var(--text);box-shadow:0 1px 2px rgba(0,0,0,.2)}
.stage{padding:40px;display:flex;flex-direction:column;align-items:flex-start;gap:12px}
.stage .desc{color:var(--muted-text);font-size:13px;max-width:720px}
.overview{display:flex;flex-wrap:wrap;gap:40px;padding:40px;align-items:flex-start}
.overview a{display:flex;flex-direction:column;gap:8px;text-decoration:none;color:var(--text)}
.overview a span{font-size:13px;font-weight:500}
.overview a small{color:var(--faint);font-weight:400;margin-left:6px}
.empty{color:var(--muted-text);padding:32px}
"#;

const VIEWER_JS: &str = r#"
(function(){
  var pages=[].slice.call(document.querySelectorAll('.page'));
  var links=[].slice.call(document.querySelectorAll('nav a[data-page]'));
  function fit(page){
    var wrap=page.querySelector('[data-zoom]'); if(!wrap) return;
    var mode=wrap.getAttribute('data-mode')||'fit';
    var z=1;
    if(mode==='fit'){
      var main=document.querySelector('main');
      var w=+wrap.getAttribute('data-w'), h=+wrap.getAttribute('data-h');
      z=Math.min(1,(main.clientWidth-80)/w,(main.clientHeight-140)/h);
      z=Math.max(z,0.2);
    } else { z=+mode; }
    wrap.style.zoom=z;
    [].forEach.call(page.querySelectorAll('.zoom button'),function(b){b.classList.toggle('on',b.getAttribute('data-z')===mode)});
  }
  function show(){
    var id=(location.hash||'').slice(1)||pages[0].id;
    var page=document.getElementById(id)||pages[0];
    pages.forEach(function(p){p.classList.toggle('on',p===page)});
    links.forEach(function(a){a.classList.toggle('on',a.getAttribute('data-page')===page.id)});
    document.querySelector('main').scrollTop=0;
    fit(page);
  }
  document.addEventListener('click',function(e){
    var b=e.target.closest('.zoom button'); if(!b) return;
    var page=b.closest('.page'); page.querySelector('[data-zoom]').setAttribute('data-mode',b.getAttribute('data-z')); fit(page);
  });
  window.addEventListener('hashchange',show);
  window.addEventListener('resize',function(){var p=document.querySelector('.page.on'); if(p) fit(p)});
  show();
})();
"#;

/// Render the whole design as one self-contained HTML file.
pub fn export_html(doc: &DesignDoc, title: &str, assets: &ExportAssets) -> String {
    let title = design_title(doc, title);
    let r = HtmlRenderer::new(doc);
    let system = system_html(doc, &r);
    let screens: Vec<(String, String)> = doc
        .screens
        .iter()
        .enumerate()
        .map(|(i, s)| (format!("screen-{}-{}", i + 1, slug(&s.name)), r.screen_html(s)))
        .collect();

    let mut nav = String::new();
    let _ = write!(nav, "<div class=\"title\">{}</div><div class=\"sub\">Design export</div>", esc(&title));
    if screens.len() > 1 {
        let _ = write!(nav, "<a href=\"#overview\" data-page=\"overview\">All screens<small>{}</small></a>", screens.len());
    }
    nav.push_str("<a href=\"#system\" data-page=\"system\">Design system</a>");
    if !screens.is_empty() {
        nav.push_str("<div class=\"group\">Screens</div>");
        for ((id, _), s) in screens.iter().zip(&doc.screens) {
            let (w, h) = s.size();
            let _ = write!(nav, "<a href=\"#{id}\" data-page=\"{id}\">{}<small>{w:.0}×{h:.0}</small></a>", esc(&s.name));
        }
    }
    nav.push_str("<div class=\"foot\">Made with Mothr</div>");

    let mut pages = String::new();
    if screens.len() > 1 {
        pages.push_str("<section class=\"page\" id=\"overview\"><div class=\"bar\"><h1>All screens</h1></div><div class=\"overview\">");
        for ((id, html), s) in screens.iter().zip(&doc.screens) {
            let (w, h) = s.size();
            let _ = write!(
                pages,
                "<a href=\"#{id}\"><span>{}<small>{w:.0}×{h:.0}</small></span><div style=\"zoom:0.4\">{html}</div></a>",
                esc(&s.name)
            );
        }
        pages.push_str("</div></section>");
    }
    for ((id, html), s) in screens.iter().zip(&doc.screens) {
        let (w, h) = s.size();
        let _ = write!(
            pages,
            "<section class=\"page\" id=\"{id}\"><div class=\"bar\"><h1>{}</h1><span class=\"dim\">{w:.0} × {h:.0}{}</span><span class=\"grow\"></span><div class=\"zoom\"><button data-z=\"fit\">Fit</button><button data-z=\"0.5\">50%</button><button data-z=\"0.75\">75%</button><button data-z=\"1\">100%</button></div></div><div class=\"stage\"><div data-zoom data-mode=\"fit\" data-w=\"{w}\" data-h=\"{h}\">{html}</div>",
            esc(&s.name),
            s.theme.as_deref().map(|t| format!(" · {}", esc(t))).unwrap_or_default()
        );
        if let Some(d) = &s.description {
            let _ = write!(pages, "<div class=\"desc\">{}</div>", esc(d));
        }
        pages.push_str("</div></section>");
    }
    let _ = write!(
        pages,
        "<section class=\"page\" id=\"system\"><div class=\"bar\"><h1>Design system</h1></div><div class=\"ds\">{system}</div></section>"
    );

    let fonts = font_faces(assets, |f| format!("data:font/ttf;base64,{}", base64::engine::general_purpose::STANDARD.encode(&f.data)));
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<meta name=\"generator\" content=\"Mothr\">\n<title>{}</title>\n<style>\n{fonts}{VIEWER_CSS}{SYSTEM_CSS}{MOCK_CSS}</style>\n</head>\n<body>\n{}\n<nav>{nav}</nav>\n<main>{pages}</main>\n<script>{VIEWER_JS}</script>\n</body>\n</html>\n",
        esc(&title),
        r.sprite(assets)
    )
}

// ---------------------------------------------------------------------------------------------
// Storybook
// ---------------------------------------------------------------------------------------------

/// Storybook version the generated project pins.
pub const STORYBOOK_VERSION: &str = "^10.0.0";

fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Story title segments: "Button / Primary" → "Button/Primary".
fn story_title(name: &str) -> String {
    let parts: Vec<String> = name.split('/').map(|p| p.trim().replace(['"', '\\'], "")).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() { "Untitled".into() } else { parts.join("/") }
}

fn ident(name: &str, used: &mut BTreeSet<String>) -> String {
    let mut out = String::new();
    let mut upper = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(if upper { c.to_ascii_uppercase() } else { c });
            upper = false;
        } else {
            upper = true;
        }
    }
    if out.is_empty() || out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, 'S');
    }
    // Reserved by CSF.
    if out == "Default" {
        out = "DefaultStory".into();
    }
    let mut candidate = out.clone();
    let mut i = 2;
    while !used.insert(candidate.clone()) {
        candidate = format!("{out}{i}");
        i += 1;
    }
    candidate
}

/// Write a Storybook project for the design into `out_dir` (created if needed). Returns the
/// files written, relative to `out_dir`.
pub fn export_storybook(doc: &DesignDoc, title: &str, assets: &ExportAssets, out_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let title = design_title(doc, title);
    let mut written = Vec::new();
    let mut write = |rel: &str, data: &[u8]| -> anyhow::Result<()> {
        let path = out_dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, data)?;
        written.push(PathBuf::from(rel));
        Ok(())
    };

    let pkg = serde_json::json!({
        "name": slug(&title),
        "private": true,
        "version": "0.0.0",
        "type": "module",
        "description": format!("{title}: design system and screens exported from Mothr"),
        "scripts": {
            "storybook": "storybook dev -p 6006",
            "build-storybook": "storybook build -o storybook-static"
        },
        "devDependencies": {
            "storybook": STORYBOOK_VERSION,
            "@storybook/html-vite": STORYBOOK_VERSION,
            "vite": "^7.0.0"
        }
    });
    write("package.json", serde_json::to_string_pretty(&pkg)?.as_bytes())?;
    write(".gitignore", b"node_modules/\nstorybook-static/\n")?;
    write(
        ".storybook/main.js",
        b"/** Generated by Mothr. */\nexport default {\n  framework: '@storybook/html-vite',\n  stories: ['../stories/**/*.stories.js'],\n};\n",
    )?;
    write(
        ".storybook/manager.js",
        b"/** Generated by Mothr. The stories are static mocks, so the controls panel starts hidden. */\nimport { addons } from 'storybook/manager-api';\n\naddons.setConfig({ showPanel: false });\n",
    )?;
    write(
        ".storybook/preview.js",
        "import './mothr.css';\n\n/** Generated by Mothr. */\nexport default {\n  parameters: {\n    layout: 'centered',\n    options: { storySort: { order: ['Design system', 'Components', 'Screens'] } },\n  },\n};\n".as_bytes(),
    )?;
    let fonts = font_faces(assets, |f| format!("\"./fonts/{}\"", f.file_name));
    write(
        ".storybook/mothr.css",
        format!(
            "{fonts}:root{{--bg:#fcfcfd;--surface:#fff;--muted:#f1f1f5;--border:#e7e7ec;--text:#15161a;--muted-text:#5f6170;--faint:#9698a4;--accent:#6d5bef}}\nbody{{font-family:Geist,system-ui,sans-serif}}\n{SYSTEM_CSS}{MOCK_CSS}"
        )
        .as_bytes(),
    )?;
    for f in &assets.fonts {
        write(&format!(".storybook/fonts/{}", f.file_name), &f.data)?;
    }

    // Tokens story.
    {
        let r = HtmlRenderer::new(doc);
        let mut body = String::new();
        if let Some(d) = doc.system.as_ref().and_then(|s| s.description.as_ref()) {
            let _ = write!(body, "<p class=\"lede\">{}</p>", esc(d));
        }
        body.push_str(&tokens_html(doc, &r));
        let html = format!("{}<div class=\"ds\">{body}</div>", r.sprite(assets));
        let src = format!(
            "/** Generated by Mothr. */\nexport default {{ title: 'Design system', parameters: {{ layout: 'fullscreen' }} }};\n\nexport const Tokens = {{\n  render: () => {},\n}};\n",
            js(&html)
        );
        write("stories/DesignSystem.stories.js", src.as_bytes())?;
    }

    // One file per component, a story per variant.
    if let Some(sys) = &doc.system {
        let bg = canvas_bg(doc);
        let mut files = BTreeSet::new();
        for c in &sys.components {
            let r = HtmlRenderer::new(doc);
            let mut used = BTreeSet::new();
            let mut stories = String::new();
            for v in c.variants() {
                let html = r.node_html(&v.node);
                let id = ident(&v.name, &mut used);
                let _ = write!(
                    stories,
                    "\nexport const {id} = {{\n  name: {},\n  render: () => wrap({}),\n}};\n",
                    js(&v.name),
                    js(&html)
                );
            }
            let mut src = format!(
                "/** Generated by Mothr. */\nconst sprite = {};\nconst wrap = (html) => `${{sprite}}<div class=\"mk\" style=\"background:{};padding:24px;border-radius:10px\">${{html}}</div>`;\n\nexport default {{\n  title: {},\n",
                js(&r.sprite(assets)),
                bg.replace('`', ""),
                js(&format!("Components/{}", story_title(&c.name)))
            );
            if let Some(d) = &c.description {
                let _ = writeln!(src, "  parameters: {{ docs: {{ description: {{ component: {} }} }} }},", js(d));
            }
            src.push_str("};\n");
            src.push_str(&stories);
            let mut file = slug(&c.name);
            let mut i = 2;
            while !files.insert(file.clone()) {
                file = format!("{}-{i}", slug(&c.name));
                i += 1;
            }
            write(&format!("stories/components/{file}.stories.js"), src.as_bytes())?;
        }
    }

    // Screens.
    if !doc.screens.is_empty() {
        let r = HtmlRenderer::new(doc);
        let mut used = BTreeSet::new();
        let mut stories = String::new();
        for s in &doc.screens {
            let html = r.screen_html(s);
            let id = ident(&s.name, &mut used);
            let _ = write!(
                stories,
                "\nexport const {id} = {{\n  name: {},\n  render: () => stage({}),\n}};\n",
                js(&s.name),
                js(&html)
            );
        }
        let src = format!(
            "/** Generated by Mothr. */\nconst sprite = {};\nconst stage = (html) => `${{sprite}}<div style=\"padding:32px;display:inline-block\">${{html}}</div>`;\n\nexport default {{\n  title: 'Screens',\n  parameters: {{ layout: 'fullscreen' }},\n}};\n{stories}",
            js(&r.sprite(assets))
        );
        write("stories/Screens.stories.js", src.as_bytes())?;
    }

    let readme = format!(
        "# {title}\n\nA Storybook of the **{title}** design: its tokens, every component variant and every screen.\nExported from [Mothr](https://github.com/evolvedmesh/mothr) Design mode; the stories are\npre-rendered HTML, so they match the in-app canvas.\n\n## View it\n\n- **Static build:** if `storybook-static/` exists, host that folder anywhere (GitHub Pages,\n  Netlify, an S3 bucket) or serve it locally, e.g. `npx serve storybook-static`.\n- **Build it:** `npm install && npm run build-storybook` (or `bun install && bun run build-storybook`).\n- **Develop:** `npm run storybook`, then open http://localhost:6006.\n\n## Layout\n\n- `stories/DesignSystem.stories.js` — color, typography, spacing, radii and elevation tokens\n- `stories/components/*.stories.js` — one file per component, one story per variant\n- `stories/Screens.stories.js` — one story per screen\n- `.storybook/fonts/` — fonts the design uses (Geist is SIL OFL licensed)\n\nRe-export from Mothr after changing the design; this folder is generated.\n"
    );
    write("README.md", readme.as_bytes())?;
    Ok(written)
}

/// How a Storybook project can be built on this machine.
#[derive(Debug, Clone)]
pub struct JsRuntime {
    pub program: PathBuf,
    pub kind: JsRuntimeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsRuntimeKind {
    Bun,
    Npm,
}

/// Find a working JS package runner: bun first, then npm (checked by running `--version`, since
/// a broken Node install can leave a dead `npm` on `PATH`).
pub fn find_js_runtime() -> Option<JsRuntime> {
    let candidates = [("bun", JsRuntimeKind::Bun), ("npm", JsRuntimeKind::Npm)];
    let path = std::env::var_os("PATH")?;
    let home_bun = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".bun/bin/bun"));
    for (name, kind) in candidates {
        let mut found: Option<PathBuf> = std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file());
        if found.is_none() && kind == JsRuntimeKind::Bun {
            found = home_bun.clone().filter(|p| p.is_file());
        }
        if let Some(program) = found {
            let ok = std::process::Command::new(&program)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                return Some(JsRuntime { program, kind });
            }
        }
    }
    None
}

/// Install dependencies and build the Storybook into `<dir>/storybook-static`. Blocking; can take
/// a few minutes and needs network access for the install.
pub fn build_storybook(dir: &Path, rt: &JsRuntime) -> anyhow::Result<PathBuf> {
    let run = |args: &[&str]| -> anyhow::Result<()> {
        let out = std::process::Command::new(&rt.program)
            .args(args)
            .current_dir(dir)
            .env("STORYBOOK_DISABLE_TELEMETRY", "1")
            .env("CI", "1")
            .output()?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let tail: Vec<&str> = err.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();
            anyhow::bail!("`{} {}` failed:\n{}", rt.program.display(), args.join(" "), tail.join("\n"));
        }
        Ok(())
    };
    run(&["install"])?;
    match rt.kind {
        JsRuntimeKind::Bun => run(&["run", "--bun", "build-storybook"])?,
        JsRuntimeKind::Npm => run(&["run", "build-storybook"])?,
    }
    let out = dir.join("storybook-static");
    anyhow::ensure!(out.join("index.html").is_file(), "the build finished but storybook-static/index.html is missing");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DesignSystem, Screen};

    fn doc() -> DesignDoc {
        let sys: DesignSystem = serde_json::from_str(
            r##"{"name":"Acme","description":"Calm <fintech>",
                "tokens":{"colors":{"bg":"#0B0B0F","primary":"#6366F1","text":"$fg","fg":"#F4F4F5"},
                          "typography":{"body":{"size":15,"weight":500,"line_height":1.5}},
                          "spacing":{"md":16},"radii":{"md":10},"shadows":{"md":{"y":8,"blur":24,"color":"#00000055"}}},
                "components":[{"name":"Button / Primary","variants":[{"name":"Default","node":
                  {"type":"button","icon":"plus","text":"Continue","style":{"bg":"$primary","fg":"#fff","radius":"$md","padding":[12,20]}}}]},
                  {"name":"Card","node":{"type":"column","children":[
                    {"type":"text","name":"title","text":"Title"},{"type":"text","text":"Body"}]}}]}"##,
        )
        .unwrap();
        let screen: Screen = serde_json::from_str(
            r##"{"name":"Home","device":"mobile","background":"$bg","root":{"type":"row","style":{"width":"fill","height":"fill","gap":"$md"},
                "children":[{"type":"column","style":{"width":"fill"}},{"type":"ref","ref":"card","overrides":{"title":"Hello","text":"First"}},
                {"type":"icon","icon":"Chevron Right"},{"type":"ref","ref":"Nope"}]}}"##,
        )
        .unwrap();
        DesignDoc { system: Some(sys), screens: vec![screen], errors: vec![] }
    }

    fn assets() -> ExportAssets {
        ExportAssets {
            fonts: vec![FontFile { family: "Geist".into(), weight: 400, italic: false, file_name: "Geist-400.ttf".into(), data: vec![1, 2, 3] }],
            icons: Some(Box::new(|name| {
                (name == "plus").then(|| "<svg\n  xmlns=\"http://www.w3.org/2000/svg\"\n  width=\"24\"\n  viewBox=\"0 0 24 24\"\n  stroke=\"currentColor\"\n>\n  <path d=\"M5 12h14\" />\n</svg>".to_string())
            })),
        }
    }

    #[test]
    fn layout_semantics_match_canvas() {
        let d = doc();
        let r = HtmlRenderer::new(&d);
        let html = r.screen_html(&d.screens[0]);
        // Fill inside a row grows along the row; the root fills the column artboard.
        assert!(html.contains("width:100%;flex-grow:1;flex-basis:0px"), "{html}");
        assert!(html.contains("height:100%;flex-grow:1;flex-basis:0px"));
        // Mobile artboard size and corner radius.
        assert!(html.contains("width:390px;height:844px;border-radius:36px"));
        // Dark background → light default text.
        assert!(html.contains("color:#f4f4f5"));
        // Named and text overrides land on the right nodes, each once.
        assert!(html.contains(">Hello<") && html.contains(">First<") && !html.contains(">Title<") && !html.contains(">Body<"));
        // Icon names are normalised the way the canvas does it.
        assert_eq!(r.used_icons(), vec!["chevron-right".to_string()]);
        assert!(html.contains("Missing component “Nope”"));
    }

    #[test]
    fn tokens_resolve() {
        let d = doc();
        let r = HtmlRenderer::new(&d);
        let html = r.node_html(&d.system.as_ref().unwrap().components[0].variants[0].node);
        assert!(html.contains("background:#6366f1"));
        assert!(html.contains("border-radius:10px"));
        assert!(html.contains("padding:12px 20px 12px 20px"));
        assert!(html.contains("<use href=\"#i-plus\"/>"));
        assert_eq!(css_color(0x00000055), "rgba(0,0,0,0.333)");
        assert_eq!(r.shadow("$md"), "0px 8px 24px 0px rgba(0,0,0,0.333)");
    }

    #[test]
    fn html_is_self_contained() {
        let d = doc();
        let html = export_html(&d, "fallback", &assets());
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<title>Acme</title>"));
        assert!(html.contains("data:font/ttf;base64,AQID"));
        assert!(html.contains("<symbol id=\"i-plus\" viewBox=\"0 0 24 24\" stroke=\"currentColor\"><path d=\"M5 12h14\" /></symbol>"));
        assert!(html.contains("Calm &lt;fintech&gt;"));
        assert!(html.contains("id=\"screen-1-home\""));
        // No network references.
        assert!(!html.replace("http://www.w3.org/2000/svg", "").contains("http"));
    }

    #[test]
    fn storybook_project() {
        let d = doc();
        let dir = std::env::temp_dir().join(format!("mothr-sb-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = export_storybook(&d, "x", &assets(), &dir).unwrap();
        let has = |p: &str| files.iter().any(|f| f == Path::new(p));
        assert!(has("package.json") && has(".storybook/main.js") && has(".storybook/fonts/Geist-400.ttf"));
        assert!(has("stories/components/button-primary.stories.js") && has("stories/components/card.stories.js"));
        let comp = std::fs::read_to_string(dir.join("stories/components/button-primary.stories.js")).unwrap();
        assert!(comp.contains("title: \"Components/Button/Primary\""));
        assert!(comp.contains("export const DefaultStory"));
        let screens = std::fs::read_to_string(dir.join("stories/Screens.stories.js")).unwrap();
        assert!(screens.contains("export const Home"));
        let pkg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap()).unwrap();
        assert_eq!(pkg["name"], "acme");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn identifiers_are_unique_and_valid() {
        let mut used = BTreeSet::new();
        assert_eq!(ident("Default", &mut used), "DefaultStory");
        assert_eq!(ident("hover state", &mut used), "HoverState");
        assert_eq!(ident("Hover-state", &mut used), "HoverState2");
        assert_eq!(ident("2 col", &mut used), "S2Col");
        assert_eq!(story_title("Button / Primary"), "Button/Primary");
    }
}
