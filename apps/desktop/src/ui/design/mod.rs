//! The Design page: a gallery of design systems and a storybook-style viewer for each one.

mod canvas;
mod composer;
mod export;
mod gallery;
mod pages;
mod viewer;

pub use gallery::{DesignsEvent, DesignsPage};
pub use viewer::Page as DesignPage;

use crate::store::Store;
use gpui_kit::*;
use mothr_core::*;
use mothr_design::DesignDoc;
use std::path::Path;

/// The newest top-level design chat in `project`.
fn latest_session<'a>(store: &'a Store, project: &Path) -> Option<&'a SessionState> {
    store
        .sessions
        .iter()
        .filter(|s| s.meta.kind == SessionKind::Design && s.meta.parent.is_none() && s.meta.project == project)
        .max_by_key(|s| s.meta.updated_at)
}

/// What to call a design. A new system is named after its folder until the agent names it, so
/// the chat's title reads better for a while.
fn design_name(doc: &DesignDoc, project: &Path, store: &Store) -> String {
    let folder = project.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let named = doc.system.as_ref().map(|s| s.name.trim()).filter(|n| !n.is_empty() && *n != folder);
    if let Some(name) = named {
        return name.to_string();
    }
    latest_session(store, project)
        .map(|s| s.meta.title.clone())
        .filter(|t| !t.starts_with("New "))
        .unwrap_or(if folder.is_empty() { "Untitled design".into() } else { folder })
}

fn channel(v: u32) -> f32 {
    let c = (v & 0xFF) as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// WCAG relative luminance of `0xRRGGBBAA`.
fn luminance(c: u32) -> f32 {
    0.2126 * channel(c >> 24) + 0.7152 * channel(c >> 16) + 0.0722 * channel(c >> 8)
}

/// WCAG contrast ratio, 1 to 21.
fn contrast(a: u32, b: u32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

const WHITE: u32 = 0xFFFFFFFF;
const BLACK: u32 = 0x000000FF;

/// Black or white, whichever reads better on `bg`.
fn readable_on(bg: u32) -> u32 {
    if contrast(bg, WHITE) >= contrast(bg, BLACK) { WHITE } else { BLACK }
}

/// `#RRGGBB`, with alpha only when it isn't opaque.
fn hex_label(c: u32) -> String {
    if c & 0xFF == 0xFF { format!("#{:06X}", c >> 8) } else { format!("#{c:08X}") }
}

/// The surface color of a design as `0xRRGGBBAA`.
fn surface_of(doc: &DesignDoc) -> Option<u32> {
    ["$bg", "$background", "$surface"].iter().find_map(|t| doc.color(t))
}

/// The design's own text color on its surface, or a readable fallback.
fn ink_of(doc: &DesignDoc, surface: u32) -> u32 {
    ["$text", "$foreground", "$fg", "$on-surface"]
        .iter()
        .find_map(|t| doc.color(t))
        .filter(|c| contrast(*c, surface) >= 3.0)
        .unwrap_or_else(|| readable_on(surface))
}

fn hsla(c: u32) -> Hsla {
    rgba(c).into()
}

/// `color` as `0xRRGGBBAA`.
fn rgba_of(color: Hsla) -> u32 {
    let c = Rgba::from(color);
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    byte(c.r) << 24 | byte(c.g) << 16 | byte(c.b) << 8 | byte(c.a)
}

#[cfg(test)]
mod tests {
    use super::{BLACK, WHITE, contrast, hex_label, readable_on};

    #[test]
    fn readable_text_follows_the_background() {
        assert_eq!(readable_on(0x0B0C0FFF), WHITE);
        assert_eq!(readable_on(0xFAFAF7FF), BLACK);
        assert!((contrast(WHITE, BLACK) - 21.0).abs() < 0.01);
    }

    #[test]
    fn hex_labels_hide_full_opacity() {
        assert_eq!(hex_label(0x6366F1FF), "#6366F1");
        assert_eq!(hex_label(0x00000055), "#00000055");
    }
}
