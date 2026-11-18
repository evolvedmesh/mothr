//! Design mode canvas: renders `mothr-design/` JSON natively and hot-reloads it.

use super::kit::*;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    v_flex,
};
use gpui_kit::*;
use super::tokens::sz;
use gpui_kit::prelude::FluentBuilder as _;
use mothr_design::{Device, Dim, DesignDoc, Length as DLength, Node, NodeType, Style as DStyle, TokenScale, design_root, load, watch};
use mothr_design::export::{ExportAssets, FontFile, build_storybook, export_html, export_storybook, find_js_runtime};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    System,
    Screens,
}

pub struct DesignView {
    project: PathBuf,
    doc: DesignDoc,
    tab: Tab,
    zoom: f32,
    focus_screen: Option<usize>,
    /// An export is being written or built (shows a spinner on the Export button).
    exporting: bool,
    _watcher: Option<notify::RecommendedWatcher>,
    _task: Task<()>,
}

impl DesignView {
    pub fn new(project: PathBuf, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let doc = load(&project);
        let (watcher, task) = match watch(&project) {
            Ok((w, rx)) => {
                let task = cx.spawn(async move |this, cx| {
                    while rx.recv().await.is_ok() {
                        if this
                            .update(cx, |this, cx| {
                                let had_screens = this.doc.screens.len();
                                this.doc = load(&this.project);
                                // Jump to screens when the first one appears.
                                if had_screens == 0 && !this.doc.screens.is_empty() {
                                    this.tab = Tab::Screens;
                                }
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                (Some(w), task)
            }
            Err(e) => {
                tracing::warn!("design watch failed: {e}");
                (None, Task::ready(()))
            }
        };
        let tab = if doc.screens.is_empty() || std::env::var("MOTHR_DESIGN_TAB").as_deref() == Ok("system") { Tab::System } else { Tab::Screens };
        Self { project, doc, tab, zoom: 0.6, focus_screen: None, exporting: false, _watcher: watcher, _task: task }
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let tab = |t: Tab, label: String, ic: &'static str, cx: &mut Context<Self>| {
            let active = self.tab == t;
            h_flex()
                .id(SharedString::from(label.clone()))
                .gap_1()
                .px_3()
                .py(sz(5.))
                .rounded(sz(7.))
                .text_sm()
                .items_center()
                .flex_none()
                .cursor_pointer()
                .when(active, |d| d.bg(theme.background).shadow_sm().font_weight(FontWeight::MEDIUM))
                .when(!active, |d| d.text_color(theme.muted_foreground))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = t;
                    cx.notify();
                }))
                .child(icon(ic).size(sz(14.)))
                .child(label)
        };
        let name = self.doc.system.as_ref().map(|s| s.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Untitled design".into());
        // The canvas pane can be narrow (the chat takes the rest), so the title gives way first
        // and the bar clips rather than spilling past the pane.
        h_flex()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .overflow_hidden()
            .border_b_1()
            .border_color(theme.border)
            .child(icon("palette").flex_none().size(sz(16.)).text_color(theme.primary))
            .child(div().flex_1().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(name))
            .child(
                h_flex()
                    .flex_none()
                    .p(sz(3.))
                    .gap(sz(2.))
                    .rounded(sz(9.))
                    .bg(theme.muted)
                    .child(tab(Tab::System, "System".into(), "swatch-book", cx))
                    .child(tab(Tab::Screens, format!("Screens · {}", self.doc.screens.len()), "layout-template", cx)),
            )
            .child(div().flex_1().min_w(sz(4.)))
            .child(
                Button::new("zoom-out")
                    .ghost()
                    .small()
                    .icon(icon("zoom-out"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.zoom = (this.zoom - 0.1).max(0.2);
                        cx.notify();
                    })),
            )
            .child(div().w(sz(42.)).text_center().text_xs().text_color(theme.muted_foreground).child(format!("{:.0}%", self.zoom * 100.0)))
            .child(
                Button::new("zoom-in")
                    .ghost()
                    .small()
                    .icon(icon("zoom-in"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.zoom = (this.zoom + 0.1).min(2.0);
                        cx.notify();
                    })),
            )
            .child({
                let weak = cx.entity().downgrade();
                let project = self.project.clone();
                Button::new("design-export")
                    .ghost()
                    .small()
                    .icon(icon("share"))
                    .label("Export")
                    .loading(self.exporting)
                    .disabled(self.exporting)
                    .dropdown_menu(move |menu, _, _| {
                        let (w1, w2) = (weak.clone(), weak.clone());
                        let folder = design_root(&project);
                        menu.item(PopupMenuItem::new("Export as HTML…").icon(icon("file-code")).on_click(move |_, window, cx| {
                            if let Some(v) = w1.upgrade() {
                                v.update(cx, |v, cx| v.export_html(window, cx));
                            }
                        }))
                        .item(PopupMenuItem::new("Export Storybook…").icon(icon("book-open")).on_click(move |_, window, cx| {
                            if let Some(v) = w2.upgrade() {
                                v.update(cx, |v, cx| v.export_storybook(window, cx));
                            }
                        }))
                        .separator()
                        .item(PopupMenuItem::new("Show design files").icon(icon("folder-open")).on_click(move |_, _, cx| {
                            cx.open_url(&format!("file://{}", folder.display()));
                        }))
                    })
            })
    }

    /// Name used for export titles and file names.
    fn export_name(&self) -> String {
        self.project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Save the whole design (screens, tokens, components) as one self-contained HTML file.
    fn export_html(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.doc.is_empty() {
            window.push_notification(Notification::warning("Nothing to export yet. Ask the agent to create a design system or a screen first."), cx);
            return;
        }
        let name = self.export_name();
        let title = mothr_design::export::design_title(&self.doc, &name);
        let file = format!("{}-design.html", file_stem(&title));
        let dev = dev_export_dir().map(|d| d.join(&file));
        let rx = dev.is_none().then(|| cx.prompt_for_new_path(&self.project, Some(&file)));
        let doc = self.doc.clone();
        cx.spawn_in(window, async move |this, cx| {
            let path = match (dev, rx) {
                (Some(p), _) => p,
                (None, Some(rx)) => match rx.await {
                    Ok(Ok(Some(p))) => p,
                    _ => return,
                },
                (None, None) => return,
            };
            let _ = this.update(cx, |this, cx| {
                this.exporting = true;
                cx.notify();
            });
            let target = path.clone();
            let res = cx
                .background_spawn(async move { std::fs::write(&target, export_html(&doc, &name, &export_assets())) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.exporting = false;
                cx.notify();
                match res {
                    Ok(()) => window.push_notification(
                        done_notification("Design exported", format!("Saved {}", display_name(&path)), "Open", path, false),
                        cx,
                    ),
                    Err(e) => window.push_notification(Notification::error(format!("Couldn't save the HTML export: {e}")), cx),
                }
            });
        })
        .detach();
    }

    /// Write a Storybook project for the design, then build it to a static site when a JS runtime
    /// is available.
    fn export_storybook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.doc.is_empty() {
            window.push_notification(Notification::warning("Nothing to export yet. Ask the agent to create a design system or a screen first."), cx);
            return;
        }
        let name = self.export_name();
        let title = mothr_design::export::design_title(&self.doc, &name);
        let dev = dev_export_dir();
        let rx = dev.is_none().then(|| {
            cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Export Storybook here".into()),
            })
        });
        let doc = self.doc.clone();
        cx.spawn_in(window, async move |this, cx| {
            let parent = match (dev, rx) {
                (Some(d), _) => d,
                (None, Some(rx)) => match rx.await {
                    Ok(Ok(Some(mut picked))) => match picked.pop() {
                        Some(p) => p,
                        None => return,
                    },
                    _ => return,
                },
                (None, None) => return,
            };
            // An empty folder is used as is; otherwise the project goes into a subfolder.
            let empty = std::fs::read_dir(&parent).map(|mut d| d.next().is_none()).unwrap_or(false);
            let dir = if empty { parent } else { parent.join(format!("{}-storybook", file_stem(&title))) };
            let _ = this.update(cx, |this, cx| {
                this.exporting = true;
                cx.notify();
            });
            let target = dir.clone();
            let written = cx
                .background_spawn(async move { export_storybook(&doc, &name, &export_assets(), &target).map(|_| find_js_runtime()) })
                .await;
            let runtime = match written {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.exporting = false;
                        cx.notify();
                        window.push_notification(Notification::error(format!("Couldn't write the Storybook project: {e}")), cx);
                    });
                    return;
                }
            };
            let Some(rt) = runtime else {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.exporting = false;
                    cx.notify();
                    window.push_notification(
                        done_notification(
                            "Storybook project exported",
                            format!("Saved to {}. Install Node or Bun, then run `npm install && npm run build-storybook` there to build the shareable site.", tildify(&dir)),
                            "Show folder",
                            dir,
                            true,
                        ),
                        cx,
                    );
                });
                return;
            };
            let _ = this.update_in(cx, |_, window, cx| {
                window.push_notification(
                    Notification::info(format!("Building the static Storybook in {}…", display_name(&dir))).autohide(true),
                    cx,
                );
            });
            let target = dir.clone();
            let built = cx.background_spawn(async move { build_storybook(&target, &rt) }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.exporting = false;
                cx.notify();
                match built {
                    Ok(site) => window.push_notification(
                        done_notification(
                            "Storybook exported",
                            format!("Static site in {}. Host that folder anywhere to share it.", display_name(&site)),
                            "Show folder",
                            site,
                            true,
                        ),
                        cx,
                    ),
                    Err(e) => window.push_notification(
                        Notification::warning(format!(
                            "The Storybook project is in {}, but building it failed: {e}. Run `npm run build-storybook` there to retry.",
                            tildify(&dir)
                        ))
                        .autohide(false),
                        cx,
                    ),
                }
            });
        })
        .detach();
    }

    fn render_system(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(sys) = &self.doc.system else {
            return self.empty(cx);
        };
        let doc = &self.doc;
        let r = Renderer { doc, scale: 1.0, text_used: Default::default(), parent_row: Default::default() };
        let mut col = v_flex().gap_8().p_8();
        if let Some(d) = &sys.description {
            col = col.child(div().text_color(theme.muted_foreground).max_w(sz(640.)).child(d.clone()));
        }
        let t = &sys.tokens;
        if !t.colors.is_empty() {
            let mut grid = h_flex().flex_wrap().gap_3();
            for (name, val) in t.colors.iter() {
                let c = doc.color(val).map(rgba_u32).unwrap_or(theme.muted);
                grid = grid.child(
                    v_flex()
                        .w(sz(128.))
                        .rounded(sz(12.))
                        .border_1()
                        .border_color(theme.border)
                        .overflow_hidden()
                        .child(div().h(sz(72.)).bg(c))
                        .child(
                            v_flex()
                                .p_2()
                                .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                                .child(div().text_xs().text_color(theme.muted_foreground).child(val.clone())),
                        ),
                );
            }
            col = col.child(section("Color", grid, cx));
        }
        if !t.typography.is_empty() {
            let mut list = v_flex().gap_4();
            for (name, ts) in t.typography.iter() {
                let size = ts.size.unwrap_or(16.0);
                list = list.child(
                    h_flex()
                        .gap_6()
                        .items_center()
                        .child(
                            v_flex()
                                .w(sz(140.))
                                .flex_none()
                                .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                                .child(div().text_xs().text_color(theme.muted_foreground).child(format!(
                                    "{size:.0}px · {}{}",
                                    ts.weight.unwrap_or(400.0) as u32,
                                    ts.line_height.map(|l| format!(" · {l}")).unwrap_or_default()
                                ))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(sz(size))
                                .font_weight(FontWeight(ts.weight.unwrap_or(400.0)))
                                .when_some(ts.family.clone(), |d, f| d.font_family(f))
                                .when(ts.italic.unwrap_or(false), |d| d.italic())
                                .child("The quick brown fox jumps over the lazy dog"),
                        ),
                );
            }
            col = col.child(section("Typography", list, cx));
        }
        if !t.spacing.is_empty() || !t.radii.is_empty() {
            let mut row = h_flex().gap_10().items_start();
            if !t.spacing.is_empty() {
                let mut sp = v_flex().gap_2();
                for (n, v) in t.spacing.iter() {
                    sp = sp.child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(div().w(sz(48.)).text_xs().text_color(theme.muted_foreground).child(n.clone()))
                            .child(div().h(sz(10.)).w(sz(v.0.max(1.0))).rounded(sz(2.)).bg(theme.primary.opacity(0.7)))
                            .child(div().text_xs().text_color(theme.muted_foreground).child(format!("{}", v.0))),
                    );
                }
                row = row.child(v_flex().gap_2().child(section_label("SPACING", cx)).child(sp));
            }
            if !t.radii.is_empty() {
                let mut rr = h_flex().gap_3();
                for (n, v) in t.radii.iter() {
                    rr = rr.child(
                        v_flex()
                            .items_center()
                            .gap_1()
                            .child(div().size(sz(56.)).rounded(sz(v.0.min(28.0))).border_2().border_color(theme.primary.opacity(0.7)).bg(theme.primary.opacity(0.08)))
                            .child(div().text_xs().text_color(theme.muted_foreground).child(format!("{n} · {}", v.0))),
                    );
                }
                row = row.child(v_flex().gap_2().child(section_label("RADII", cx)).child(rr));
            }
            col = col.child(row);
        }
        if !t.shadows.is_empty() {
            let mut row = h_flex().gap_6().p_4();
            for (n, _) in t.shadows.iter() {
                row = row.child(
                    v_flex()
                        .items_center()
                        .gap_2()
                        .child(r.apply_shadow(div().size(sz(88.)).rounded(sz(12.)).bg(theme.background), &format!("${n}")))
                        .child(div().text_xs().text_color(theme.muted_foreground).child(n.clone())),
                );
            }
            col = col.child(section("Elevation", row, cx));
        }
        if !sys.components.is_empty() {
            let mut comps = v_flex().gap_6();
            for c in &sys.components {
                let mut variants = h_flex().gap_6().flex_wrap().items_end();
                for v in c.variants() {
                    variants = variants.child(
                        v_flex()
                            .gap_2()
                            .child(r.node(&v.node, None))
                            .child(div().text_xs().text_color(theme.muted_foreground).child(v.name.clone())),
                    );
                }
                comps = comps.child(
                    v_flex()
                        .gap_3()
                        .p_5()
                        .rounded(sz(14.))
                        .border_1()
                        .border_color(theme.border)
                        .child(
                            h_flex()
                                .gap_2()
                                .items_baseline()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(c.name.clone()))
                                .when_some(c.description.clone(), |d, desc| d.child(div().text_sm().text_color(theme.muted_foreground).child(desc))),
                        )
                        .child(div().p_6().rounded(sz(10.)).bg(canvas_bg(doc, &theme)).child(variants)),
                );
            }
            col = col.child(section("Components", comps, cx));
        }
        div().id("system-scroll").size_full().overflow_y_scroll().child(col).into_any_element()
    }

    fn render_screens(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        if self.doc.screens.is_empty() {
            return self.empty(cx);
        }
        let r = Renderer { doc: &self.doc, scale: self.zoom, text_used: Default::default(), parent_row: Default::default() };
        let mut row = h_flex().gap_10().p_10().items_start();
        for (i, s) in self.doc.screens.iter().enumerate() {
            if self.focus_screen.is_some_and(|f| f != i) {
                continue;
            }
            let (w, h) = s.size();
            let bg = s.background.as_deref().and_then(|b| self.doc.color(b)).map(rgba_u32).unwrap_or(gpui_kit::white());
            let focused = self.focus_screen == Some(i);
            row = row.child(
                v_flex()
                    .gap_2()
                    .flex_none()
                    .child(
                        h_flex()
                            .id(ElementId::Name(format!("screen-title-{i}").into()))
                            .gap_2()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.focus_screen = if focused { None } else { Some(i) };
                                cx.notify();
                            }))
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(s.name.clone()))
                            .child(div().text_xs().text_color(theme.muted_foreground).child(format!("{w:.0}×{h:.0}")))
                            .child(icon(if focused { "minimize" } else { "maximize-2" }).size(sz(12.)).text_color(theme.muted_foreground)),
                    )
                    .child(
                        div()
                            .w(sz(w * self.zoom))
                            .h(sz(h * self.zoom))
                            .rounded(sz(if matches!(s.device, Device::Mobile) { 36.0 * self.zoom } else { 10.0 }))
                            .overflow_hidden()
                            .bg(bg)
                            .shadow_xl()
                            .border_1()
                            .border_color(theme.border)
                            .child(div().size_full().flex().flex_col().child(r.node(&s.root, None))),
                    )
                    .when_some(s.description.clone(), |d, desc| d.child(div().max_w(sz(w * self.zoom)).text_xs().text_color(theme.muted_foreground).child(desc))),
            );
        }
        div().id("screens-scroll").size_full().overflow_scroll().child(row).into_any_element()
    }

    fn empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_8()
            .child(empty_state(
                "pen-tool",
                "Your canvas is empty",
                "Describe the product, audience and mood in the chat. The agent will create design tokens, components and screens here, live.",
                cx,
            ))
            .child(
                v_flex().gap_1().text_xs().text_color(theme.muted_foreground).children(
                    [
                        "“Create a calm, premium design system for a personal finance app”",
                        "“Design onboarding, home and settings screens for mobile”",
                        "“Make the primary color warmer and increase contrast”",
                    ]
                    .map(|s| div().child(s)),
                ),
            )
            .into_any_element()
    }
}

impl Render for DesignView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let body = match self.tab {
            Tab::System => self.render_system(cx),
            Tab::Screens => self.render_screens(cx),
        };
        v_flex()
            .size_full()
            .bg(if theme.mode.is_dark() { hex(0x0B0C0F) } else { hex(0xF1F1F4) })
            .child(self.render_toolbar(cx))
            .when(!self.doc.errors.is_empty(), |d| {
                d.child(
                    v_flex()
                        .px_4()
                        .py_2()
                        .gap_1()
                        .bg(theme.danger.opacity(0.08))
                        .text_xs()
                        .text_color(theme.danger)
                        .children(self.doc.errors.iter().map(|e| div().child(e.clone()))),
                )
            })
            .child(div().flex_1().min_h_0().child(body))
    }
}

fn section(title: &'static str, body: impl IntoElement, cx: &App) -> impl IntoElement {
    v_flex().gap_3().child(div().text_lg().font_weight(FontWeight::SEMIBOLD).text_color(cx.theme().foreground).child(title)).child(body)
}

fn canvas_bg(doc: &DesignDoc, theme: &gpui_kit::component::Theme) -> Hsla {
    doc.color("$bg")
        .or_else(|| doc.color("$background"))
        .or_else(|| doc.color("$surface"))
        .map(rgba_u32)
        .unwrap_or(theme.muted)
}

/// Converts design nodes into GPUI elements at a given zoom scale.
struct Renderer<'a> {
    doc: &'a DesignDoc,
    scale: f32,
    /// Whether the current instance's `text` override was already consumed.
    text_used: std::cell::Cell<bool>,
    /// Direction of the container currently laying out children.
    parent_row: std::cell::Cell<bool>,
}

impl Renderer<'_> {
    fn s(&self, v: f32) -> Pixels {
        sz(v * self.scale)
    }

    fn color(&self, v: &Option<String>) -> Option<Hsla> {
        v.as_deref().and_then(|c| self.doc.color(c)).map(rgba_u32)
    }

    fn len(&self, l: &DLength, scale: TokenScale) -> f32 {
        self.doc.length(l, scale)
    }

    fn apply_shadow(&self, d: Div, name: &str) -> Div {
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

    fn node(&self, n: &Node, overrides: Option<&std::collections::BTreeMap<String, String>>) -> AnyElement {
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

/// Fonts and icons that exports embed: the app's bundled Geist families and the Lucide catalog.
fn export_assets() -> ExportAssets {
    ExportAssets {
        fonts: super::theme::FONTS
            .iter()
            .map(|f| FontFile { family: f.family.into(), weight: f.weight, italic: false, file_name: f.file_name.into(), data: f.data.to_vec() })
            .collect(),
        icons: Some(Box::new(|name| {
            let bytes = gpui_kit::assets::AllAssets.load(&format!("icons/{name}.svg")).ok().flatten()?;
            Some(String::from_utf8_lossy(&bytes).into_owned())
        })),
    }
}

/// Developer hook: `MOTHR_EXPORT_DIR=<dir>` skips the save prompts and exports into that folder.
fn dev_export_dir() -> Option<PathBuf> {
    std::env::var_os("MOTHR_EXPORT_DIR").filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// A file-name-safe version of a design title.
fn file_stem(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "design".into() } else { s }
}

/// Last two path components ("app-out/mothr-design.html"), for toasts.
fn display_name(p: &Path) -> String {
    let file = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    match p.parent().and_then(|d| d.file_name()) {
        Some(dir) => format!("{}/{file}", dir.to_string_lossy()),
        None => file,
    }
}

fn tildify(p: &Path) -> String {
    mothr_core::tildify(p)
}

/// Success toast with one action that opens (or reveals) the exported file or folder.
fn done_notification(title: &'static str, message: String, action: &'static str, path: PathBuf, reveal: bool) -> Notification {
    Notification::success(message).title(title).action(move |_, _, _| {
        let path = path.clone();
        Button::new("export-open").small().label(action).on_click(move |_, _, cx| {
            if reveal { cx.reveal_path(&path) } else { cx.open_with_system(&path) }
        })
    })
}
