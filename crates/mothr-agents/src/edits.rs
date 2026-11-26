//! Turns Claude Code's file-editing tool calls into [`FileDiff`]s.
//!
//! A tool call only carries the replaced snippet. Reading the file adds what a reviewer needs:
//! the real line number and a few lines of context. Once the tool has run, the CLI reports the
//! exact patch, which replaces this preview.

use mothr_core::FileDiff;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Files larger than this aren't read for previews.
const MAX_PREVIEW_BYTES: u64 = 1024 * 1024;
const CONTEXT_LINES: usize = 3;
/// `replace_all` edits show at most this many places.
const MAX_OCCURRENCES: usize = 50;

fn read_text(path: &Path) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > MAX_PREVIEW_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// The diff for replacing `content[at..at + len]`, widened to whole lines plus context.
/// `old`/`new` are the two versions of that span (one of them is what `content` holds).
fn with_context(path: &Path, content: &str, at: usize, len: usize, old: &str, new: &str) -> FileDiff {
    let end = at + len;
    let line_start = content[..at].rfind('\n').map_or(0, |i| i + 1);
    let next_line = |from: usize| content[from..].find('\n').map_or(content.len(), |i| from + i + 1);
    let line_end = if end > at && content[..end].ends_with('\n') { end } else { next_line(end) };
    let mut start = line_start;
    for _ in 0..CONTEXT_LINES {
        if start == 0 {
            break;
        }
        start = content[..start - 1].rfind('\n').map_or(0, |i| i + 1);
    }
    let mut stop = line_end;
    for _ in 0..CONTEXT_LINES {
        if stop >= content.len() {
            break;
        }
        stop = next_line(stop);
    }
    let (before, after) = (&content[start..at], &content[end..stop]);
    FileDiff {
        path: path.to_path_buf(),
        old_text: Some(format!("{before}{old}{after}")),
        new_text: format!("{before}{new}{after}"),
        start_line: Some(content[..start].matches('\n').count() as u32 + 1),
    }
}

/// One `old` → `new` replacement. Works before the edit runs and, because the tool may already
/// have run by the time we look, after it too.
fn replacement(path: &Path, content: Option<&str>, old: &str, new: &str, all: bool) -> Vec<FileDiff> {
    let snippet = || vec![FileDiff { path: path.to_path_buf(), old_text: Some(old.to_string()), new_text: new.to_string(), start_line: None }];
    let Some(content) = content else { return snippet() };
    if old.is_empty() {
        return snippet();
    }
    let limit = if all { MAX_OCCURRENCES } else { 1 };
    let found: Vec<FileDiff> = content.match_indices(old).take(limit).map(|(at, _)| with_context(path, content, at, old.len(), old, new)).collect();
    if !found.is_empty() {
        return found;
    }
    if new.is_empty() {
        return snippet();
    }
    let applied: Vec<FileDiff> = content.match_indices(new).take(limit).map(|(at, _)| with_context(path, content, at, new.len(), old, new)).collect();
    if applied.is_empty() { snippet() } else { applied }
}

/// The diffs a file-editing tool call will make; empty for every other tool.
pub fn edit_diffs(input: &Value) -> Vec<FileDiff> {
    let Some(path) = input["file_path"].as_str().map(PathBuf::from) else { return vec![] };
    if let Some(new) = input["new_string"].as_str() {
        let old = input["old_string"].as_str().unwrap_or("");
        let content = read_text(&path);
        if old.is_empty() && content.as_deref().is_none_or(str::is_empty) {
            return vec![FileDiff { path, old_text: None, new_text: new.to_string(), start_line: None }];
        }
        return replacement(&path, content.as_deref(), old, new, input["replace_all"].as_bool().unwrap_or(false));
    }
    if let Some(new) = input["content"].as_str() {
        // Identical text means the write already happened, so the original is gone.
        let old_text = read_text(&path).filter(|old| old != new);
        return vec![FileDiff { path, old_text, new_text: new.to_string(), start_line: None }];
    }
    let Some(edits) = input["edits"].as_array() else { return vec![] };
    let mut content = read_text(&path);
    let mut diffs = vec![];
    for e in edits {
        let old = e["old_string"].as_str().unwrap_or("");
        let new = e["new_string"].as_str().unwrap_or("");
        diffs.extend(replacement(&path, content.as_deref(), old, new, e["replace_all"].as_bool().unwrap_or(false)));
        // Later edits apply on top of earlier ones.
        if let Some(c) = &mut content
            && !old.is_empty()
            && c.contains(old)
        {
            *c = c.replacen(old, new, 1);
        }
    }
    diffs
}

/// The exact change the CLI reports in a tool result's `tool_use_result`.
pub fn result_diffs(result: &Value) -> Option<Vec<FileDiff>> {
    let path = PathBuf::from(result["filePath"].as_str()?);
    let hunks = result["structuredPatch"].as_array().filter(|h| !h.is_empty());
    if let Some(hunks) = hunks {
        let diffs = hunks
            .iter()
            .map(|h| {
                let (mut old, mut new) = (String::new(), String::new());
                for line in h["lines"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    let (sign, text) = line.split_at(line.len().min(1));
                    let targets: &mut [&mut String] = match sign {
                        " " => &mut [&mut old, &mut new],
                        "-" => &mut [&mut old],
                        "+" => &mut [&mut new],
                        // "\ No newline at end of file"
                        _ => continue,
                    };
                    for t in targets {
                        t.push_str(text);
                        t.push('\n');
                    }
                }
                // A hunk with no old lines names the line before the insertion.
                let start = h["oldStart"].as_u64().unwrap_or(1) as u32 + u32::from(h["oldLines"].as_u64() == Some(0));
                FileDiff { path: path.clone(), old_text: Some(old), new_text: new, start_line: Some(start) }
            })
            .collect();
        return Some(diffs);
    }
    if result["type"] == "create" {
        let new_text = result["content"].as_str()?.to_string();
        return Some(vec![FileDiff { path, old_text: None, new_text, start_line: None }]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str, text: Option<&str>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mothr-edits-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("file.txt");
        let _ = std::fs::remove_file(&path);
        if let Some(t) = text {
            std::fs::write(&path, t).unwrap();
        }
        path
    }

    const TEXT: &str = "1\n2\n3\n4 target here\n5\n6\n7\n8\n";

    #[test]
    fn edit_gets_line_number_and_context() {
        let p = temp("ctx", Some(TEXT));
        let d = edit_diffs(&json!({"file_path": p, "old_string": "target", "new_string": "goal"}));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].start_line, Some(1));
        assert_eq!(d[0].old_text.as_deref(), Some("1\n2\n3\n4 target here\n5\n6\n7\n"));
        assert_eq!(d[0].new_text, "1\n2\n3\n4 goal here\n5\n6\n7\n");
    }

    #[test]
    fn edit_already_applied_still_shows_the_change() {
        let p = temp("applied", Some(TEXT));
        let d = edit_diffs(&json!({"file_path": p, "old_string": "before", "new_string": "target"}));
        assert_eq!(d[0].old_text.as_deref(), Some("1\n2\n3\n4 before here\n5\n6\n7\n"));
        assert_eq!(d[0].new_text, "1\n2\n3\n4 target here\n5\n6\n7\n");
    }

    #[test]
    fn unknown_text_falls_back_to_the_snippet() {
        let p = temp("miss", Some(TEXT));
        let d = edit_diffs(&json!({"file_path": p, "old_string": "nope", "new_string": "nada"}));
        assert_eq!((d[0].old_text.as_deref(), d[0].new_text.as_str(), d[0].start_line), (Some("nope"), "nada", None));
    }

    #[test]
    fn write_compares_with_the_file_on_disk() {
        let p = temp("write", Some("old\n"));
        let d = edit_diffs(&json!({"file_path": p, "content": "new\n"}));
        assert_eq!(d[0].old_text.as_deref(), Some("old\n"));
        let fresh = temp("create", None);
        assert_eq!(edit_diffs(&json!({"file_path": fresh, "content": "new\n"}))[0].old_text, None);
    }

    #[test]
    fn multi_edit_applies_in_sequence() {
        let p = temp("multi", Some("a\nb\n"));
        let d = edit_diffs(&json!({"file_path": p, "edits": [
            {"old_string": "a", "new_string": "x"},
            {"old_string": "x\nb", "new_string": "y"},
        ]}));
        assert_eq!(d.len(), 2);
        assert_eq!(d[1].old_text.as_deref(), Some("x\nb\n"));
    }

    #[test]
    fn replace_all_shows_every_place() {
        let p = temp("all", Some("x\ny\nx\n"));
        let d = edit_diffs(&json!({"file_path": p, "old_string": "x", "new_string": "z", "replace_all": true}));
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn other_tools_have_no_diffs() {
        assert!(edit_diffs(&json!({"command": "ls"})).is_empty());
    }

    #[test]
    fn result_patch_becomes_exact_hunks() {
        // Captured from Claude Code 2.1.
        let r = json!({"filePath": "/p/a.txt", "structuredPatch": [
            {"oldStart": 7, "oldLines": 3, "newStart": 7, "newLines": 3, "lines": [" alpha", "-hello world", "+hello there", " omega"]}
        ]});
        let d = result_diffs(&r).unwrap();
        assert_eq!(d[0].start_line, Some(7));
        assert_eq!(d[0].old_text.as_deref(), Some("alpha\nhello world\nomega\n"));
        assert_eq!(d[0].new_text, "alpha\nhello there\nomega\n");
    }

    #[test]
    fn created_files_have_no_original() {
        let r = json!({"type": "create", "filePath": "/p/n.txt", "content": "hi\n", "structuredPatch": [], "originalFile": null});
        assert_eq!(result_diffs(&r).unwrap()[0].old_text, None);
        assert!(result_diffs(&json!({"stdout": "x"})).is_none());
    }
}
