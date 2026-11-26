//! Full-window image viewer. Images in chats and composers open here instead of in another app;
//! "Open in default app" is still one click away.

use super::kit::*;
use super::tokens::{self, sz};
use gpui_kit::component::{h_flex, tooltip::Tooltip, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::PathBuf;
use std::time::Duration;

/// Show `path` in the image viewer (handled by the workspace).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = mothr, no_json)]
pub struct ViewImage {
    pub path: PathBuf,
}

pub enum LightboxEvent {
    Close,
}

impl EventEmitter<LightboxEvent> for Lightbox {}

pub struct Lightbox {
    /// The images you can step through (the chat's, in order), and the one on screen.
    images: Vec<PathBuf>,
    index: usize,
    /// Actual size (scrolls) instead of fitting the window.
    actual: bool,
    focus: FocusHandle,
}

impl Lightbox {
    pub fn new(images: Vec<PathBuf>, index: usize, cx: &mut Context<Self>) -> Self {
        let index = index.min(images.len().saturating_sub(1));
        Self { images, index, actual: false, focus: cx.focus_handle() }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let n = self.images.len() as isize;
        if n > 1 {
            self.index = (self.index as isize + by).rem_euclid(n) as usize;
            self.actual = false;
            cx.notify();
        }
    }

    fn show(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.images.len() && index != self.index {
            self.index = index;
            self.actual = false;
            cx.notify();
        }
    }

    fn toggle_size(&mut self, cx: &mut Context<Self>) {
        self.actual = !self.actual;
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.modifiers.modified() {
            return;
        }
        match ev.keystroke.key.as_str() {
            "escape" => cx.emit(LightboxEvent::Close),
            "left" | "up" => self.step(-1, cx),
            "right" | "down" => self.step(1, cx),
            "space" | "0" | "f" => self.toggle_size(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

/// A round icon button drawn on the dark backdrop (light ink in either theme).
fn chrome_button(id: &'static str, name: &'static str, tip: &'static str) -> Stateful<Div> {
    let ink = gpui_kit::white();
    div()
        .id(id)
        .flex_none()
        .size(sz(34.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .text_color(ink.opacity(0.78))
        .hover(move |d| d.bg(ink.opacity(0.12)).text_color(ink))
        .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
        .child(icon(name).size(sz(17.)))
}

impl Render for Lightbox {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ink = gpui_kit::white();
        let Some(path) = self.images.get(self.index).cloned() else {
            return div().into_any_element();
        };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let many = self.images.len() > 1;
        let actual = self.actual;

        let fallback = {
            let name = name.clone();
            move || div().text_size(sz(tokens::text::SM)).text_color(gpui_kit::white().opacity(0.7)).child(format!("Can't show {name}")).into_any_element()
        };
        let picture = img(path.clone()).with_fallback(fallback);
        // Clicking the picture switches between fitting the window and actual size; clicking
        // around it closes the viewer.
        let picture = div()
            .id("lb-picture")
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_size(cx);
            }))
            .map(|d| {
                if actual {
                    d.flex_none().child(picture.flex_none())
                } else {
                    d.max_w_full().max_h_full().flex().child(picture.max_w_full().max_h_full().object_fit(ObjectFit::ScaleDown))
                }
            });

        let stage = div()
            .id("lb-stage")
            .flex_1()
            .min_h_0()
            .w_full()
            .relative()
            .on_click(cx.listener(|_, _, _, cx| cx.emit(LightboxEvent::Close)))
            .child(
                div()
                    .id("lb-scroll")
                    .size_full()
                    .when(actual, |d| d.overflow_scroll())
                    .child(
                        div()
                            .min_w_full()
                            .min_h_full()
                            .when(!actual, |d| d.size_full())
                            .px(sz(if many { 72. } else { 32. }))
                            .py(sz(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(picture),
                    ),
            )
            .when(many, |d| {
                d.child(
                    div().absolute().left(sz(16.)).top_0().bottom_0().flex().items_center().child(
                        chrome_button("lb-prev", "chevron-left", "Previous  (←)").bg(ink.opacity(0.08)).on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.step(-1, cx);
                        })),
                    ),
                )
                .child(
                    div().absolute().right(sz(16.)).top_0().bottom_0().flex().items_center().child(
                        chrome_button("lb-next", "chevron-right", "Next  (→)").bg(ink.opacity(0.08)).on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.step(1, cx);
                        })),
                    ),
                )
            });

        let open = path.clone();
        let top = h_flex()
            .id("lb-top")
            .flex_none()
            .h(sz(52.))
            .px(sz(16.))
            .gap(sz(12.))
            .items_center()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(icon("image").size(sz(16.)).text_color(ink.opacity(0.6)))
            .child(div().min_w_0().truncate().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::MEDIUM).text_color(ink.opacity(0.92)).child(name))
            .when(many, |d| d.child(div().flex_none().text_size(sz(tokens::text::SM)).text_color(ink.opacity(0.55)).child(format!("{} / {}", self.index + 1, self.images.len()))))
            .child(div().flex_1())
            .child(
                chrome_button("lb-size", if actual { "zoom-out" } else { "zoom-in" }, if actual { "Fit to window  (Space)" } else { "Actual size  (Space)" })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_size(cx))),
            )
            .child(chrome_button("lb-external", "external-link", "Open in default app").on_click(move |_, _, cx| cx.open_with_system(&open)))
            .child(chrome_button("lb-close", "x", "Close  (Esc)").on_click(cx.listener(|_, _, _, cx| cx.emit(LightboxEvent::Close))));

        let strip = many.then(|| {
            h_flex()
                .id("lb-strip")
                .flex_none()
                .max_w_full()
                .overflow_x_scroll()
                .justify_center()
                .gap(sz(8.))
                .px(sz(16.))
                .pb(sz(16.))
                .pt(sz(4.))
                .on_click(|_, _, cx| cx.stop_propagation())
                .children(self.images.iter().enumerate().map(|(i, p)| {
                    let active = i == self.index;
                    div()
                        .id(ElementId::Name(format!("lb-thumb-{i}").into()))
                        .flex_none()
                        .size(sz(52.))
                        .rounded(sz(tokens::radius::MD))
                        .overflow_hidden()
                        .border_2()
                        .border_color(if active { ink.opacity(0.9) } else { ink.opacity(0.0) })
                        .opacity(if active { 1.0 } else { 0.55 })
                        .hover(|d| d.opacity(1.0))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.show(i, cx)))
                        .child(img(p.clone()).size_full().object_fit(ObjectFit::Cover))
                }))
        });

        v_flex()
            .id("lightbox")
            .key_context("Lightbox")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .occlude()
            .bg(gpui_kit::black().opacity(0.95))
            .child(top)
            .child(stage)
            .children(strip)
            .with_animation("lb-fade", Animation::new(Duration::from_millis(160)).with_easing(ease_out_quint()), |d, t| d.opacity(t))
            .into_any_element()
    }
}
