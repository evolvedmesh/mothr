//! Diff views for agent edits: patch building, compact stats and the inline transcript diff.

use super::tokens::{self, sz};
use gpui_kit::component::{
    diff::{Diff, DiffChangeIndicator, DiffFile, DiffHoverHighlight, DiffHunkSeparator, DiffState},
    h_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mothr_core::{DiffStats, FileDiff, unified_patch};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

/// Height of one diff row at 100%: the kit's mono line height at our 12.5 px mono size. The kit
/// derives it from two scaled values (rem and mono size), so it grows with the scale twice.
const ROW_H: f32 = 20.0;
const FILE_HEADER_H: f32 = 34.0;
const INLINE_MAX_H: f32 = 360.0;

/// A patch-safe, unique label for one file section. The kit keys sections by path, and the same
/// file can appear many times in a chat; keeping the name preserves language detection.
pub fn section_label(index: usize, path: &Path) -> String {
    let name: String = path
        .file_name()
        .map(|n| n.to_string_lossy().chars().map(|c| if c.is_whitespace() { '_' } else { c }).collect())
        .unwrap_or_else(|| "file".into());
    format!("{index}/{name}")
}

pub fn parse_section(label: &str, diffs: &[&FileDiff]) -> Option<DiffFile> {
    match DiffFile::parse(&unified_patch(label, diffs)) {
        Ok(files) => files.into_iter().next(),
        Err(e) => {
            tracing::warn!("couldn't display the diff for {label}: {e}");
            None
        }
    }
}

/// `+12 −3`, colored like the diff itself.
pub fn stat_label(stats: DiffStats, pal: &tokens::Palette) -> Div {
    h_flex()
        .flex_none()
        .gap(sz(5.))
        .font_family(tokens::FONT_MONO)
        .text_size(sz(tokens::text::XS))
        .when(stats.added > 0, |d| d.child(div().text_color(pal.success).child(format!("+{}", stats.added))))
        .when(stats.removed > 0, |d| d.child(div().text_color(pal.danger).child(format!("−{}", stats.removed))))
}

struct Inline {
    fingerprint: u64,
    state: Entity<DiffState>,
    rows: usize,
    files: usize,
}

/// Diff states for the cards in a transcript, kept across renders so scroll position survives
/// streaming updates and syntax highlighting only runs once.
#[derive(Default)]
pub struct InlineDiffs {
    states: HashMap<String, Inline>,
}

impl InlineDiffs {
    pub fn element<T: 'static>(&mut self, key: &str, diffs: &[FileDiff], cx: &mut Context<T>) -> AnyElement {
        let mut h = DefaultHasher::new();
        diffs.hash(&mut h);
        let fingerprint = h.finish();
        let stale = self.states.get(key).is_none_or(|i| i.fingerprint != fingerprint);
        if stale {
            let mut by_file: Vec<(&Path, Vec<&FileDiff>)> = vec![];
            for d in diffs {
                match by_file.iter_mut().find(|(p, _)| *p == d.path.as_path()) {
                    Some((_, v)) => v.push(d),
                    None => by_file.push((d.path.as_path(), vec![d])),
                }
            }
            let mut rows = 0;
            let mut files = vec![];
            for (i, (path, group)) in by_file.iter().enumerate() {
                let label = section_label(i, path);
                rows += unified_patch(&label, group).lines().skip(2).count();
                files.extend(parse_section(&label, group));
            }
            let count = files.len();
            match self.states.get_mut(key) {
                Some(entry) => {
                    entry.state.update(cx, |s, cx| s.set_files(files, cx));
                    (entry.fingerprint, entry.rows, entry.files) = (fingerprint, rows, count);
                }
                None => {
                    let state = cx.new(|cx| DiffState::new(files, cx));
                    self.states.insert(key.to_string(), Inline { fingerprint, state, rows, files: count });
                }
            }
        }
        let inline = &self.states[key];
        let headers = inline.files > 1;
        let rows = inline.rows as f32 * ROW_H * tokens::scale();
        let file_headers = if headers { FILE_HEADER_H * inline.files as f32 } else { 0.0 };
        div()
            .w_full()
            .h(sz((rows + file_headers + 2.0).min(INLINE_MAX_H)))
            .child(styled(Diff::new(&inline.state)).header_visible(headers).border_0())
            .into_any_element()
    }
}

/// The look shared by every diff in the app.
pub fn styled(diff: Diff) -> Diff {
    diff.hunk_separator(DiffHunkSeparator::Simple)
        .change_indicator(DiffChangeIndicator::Bars)
        .hover_highlight(DiffHoverHighlight::Line)
}

#[cfg(test)]
mod tests {
    use super::{DiffFile, FileDiff, Path, section_label, unified_patch};
    use std::path::PathBuf;

    fn diff(old: Option<&str>, new: &str, start_line: Option<u32>) -> FileDiff {
        FileDiff { path: PathBuf::from("/p/my file.rs"), old_text: old.map(str::to_string), new_text: new.into(), start_line }
    }

    #[test]
    fn labels_are_unique_and_keep_the_extension() {
        let path = Path::new("/p/my file.rs");
        assert_eq!(section_label(3, path), "3/my_file.rs");
        assert_ne!(section_label(1, path), section_label(2, path));
    }

    #[test]
    fn the_kit_accepts_every_patch_shape_we_build() {
        let (edit, later) = (diff(Some("a\nb\nc\n"), "a\nB\nc\n", Some(10)), diff(Some("x\n"), "y\nz\n", Some(40)));
        let (loose_a, loose_b) = (diff(Some("a"), "b", None), diff(Some("c"), "d", None));
        let (created, emptied, insert) = (diff(None, "new\n", None), diff(Some("gone\n"), "", Some(3)), diff(Some(""), "added\n", Some(7)));
        let cases: Vec<(&str, Vec<&FileDiff>)> = vec![
            ("edit", vec![&edit]),
            ("two hunks", vec![&edit, &later]),
            ("reversed hunks", vec![&later, &edit]),
            ("no positions", vec![&loose_a, &loose_b]),
            ("new file", vec![&created]),
            ("delete all", vec![&emptied]),
            ("pure insert", vec![&insert]),
        ];
        for (name, diffs) in cases {
            let patch = unified_patch("0/a.rs", &diffs);
            assert!(DiffFile::parse(&patch).is_ok_and(|f| f.len() == 1), "{name}:\n{patch}");
        }
    }
}
