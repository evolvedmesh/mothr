//! Exporting a design from the viewer. The formats themselves live in `mothr_design::export`.

use super::viewer::DesignViewer;
use gpui_kit::component::{Sizable as _, WindowExt as _, button::Button, notification::Notification};
use gpui_kit::*;
use mothr_design::export::{ExportAssets, FontFile, build_storybook, export_html, export_storybook, find_js_runtime};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ExportKind {
    Html,
    Storybook,
}

impl DesignViewer {
    /// Name used for export titles and file names.
    fn export_name(&self, cx: &App) -> String {
        self.name(cx)
    }

    /// Save the whole design (screens, tokens, components) as one self-contained HTML file.
    pub(super) fn export_html(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.doc.is_empty() {
            window.push_notification(Notification::warning("Nothing to export yet. Ask the agent to create a design system or a screen first."), cx);
            return;
        }
        let name = self.export_name(cx);
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
                this.exporting = Some(ExportKind::Html);
                cx.notify();
            });
            let target = path.clone();
            let res = cx
                .background_spawn(async move { std::fs::write(&target, export_html(&doc, &name, &export_assets())) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.exporting = None;
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
    pub(super) fn export_storybook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.doc.is_empty() {
            window.push_notification(Notification::warning("Nothing to export yet. Ask the agent to create a design system or a screen first."), cx);
            return;
        }
        let name = self.export_name(cx);
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
                this.exporting = Some(ExportKind::Storybook);
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
                        this.exporting = None;
                        cx.notify();
                        window.push_notification(Notification::error(format!("Couldn't write the Storybook project: {e}")), cx);
                    });
                    return;
                }
            };
            let Some(rt) = runtime else {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.exporting = None;
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
                this.exporting = None;
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
}

/// Fonts and icons that exports embed: the app's bundled Geist families and the Lucide catalog.
fn export_assets() -> ExportAssets {
    ExportAssets {
        fonts: crate::ui::theme::FONTS
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
