//! Line diffs of agent file edits, and the reviewable list of changes in a session.
//!
//! Views never interpret tool calls themselves: [`SessionState::file_changes`] folds the
//! transcript into [`FileChange`]s, and [`unified_patch`] turns their diffs into a standard
//! unified patch for display.

use crate::session::{Entry, FileDiff, PermissionOption, PermissionOptionKind, PermissionRequest, SessionState, ToolStatus};
use similar::{ChangeTag, TextDiff};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

/// Unchanged lines kept around each change in a patch.
const CONTEXT_LINES: usize = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffStats {
    pub added: usize,
    pub removed: usize,
}

impl DiffStats {
    pub fn add(&mut self, other: DiffStats) {
        self.added += other.added;
        self.removed += other.removed;
    }
}

/// Lines compare equal whether or not the snippet ended in a newline.
fn terminated(s: &str) -> Cow<'_, str> {
    if s.is_empty() || s.ends_with('\n') { Cow::Borrowed(s) } else { Cow::Owned(format!("{s}\n")) }
}

impl FileDiff {
    pub fn stats(&self) -> DiffStats {
        let old = terminated(self.old_text.as_deref().unwrap_or(""));
        let new = terminated(&self.new_text);
        let diff = TextDiff::from_lines(old.as_ref(), new.as_ref());
        let mut stats = DiffStats::default();
        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Insert => stats.added += 1,
                ChangeTag::Delete => stats.removed += 1,
                ChangeTag::Equal => {}
            }
        }
        stats
    }
}

/// A unified patch for edits to one file, labelled `label`.
///
/// Hunks are written in file order and never overlap, even when the edits don't say where they
/// landed, so the result is always a valid patch.
pub fn unified_patch(label: &str, diffs: &[&FileDiff]) -> String {
    let new_file = diffs.iter().all(|d| d.old_text.is_none());
    let mut out = String::new();
    let _ = writeln!(out, "--- {}", if new_file { "/dev/null".to_string() } else { format!("a/{label}") });
    let _ = writeln!(out, "+++ b/{label}");
    let mut ordered: Vec<&FileDiff> = diffs.to_vec();
    ordered.sort_by_key(|d| d.start_line.unwrap_or(0));
    // First old-side line the next hunk may use, and how far the new side has drifted from it.
    let mut next_old = 1usize;
    let mut drift = 0isize;
    for d in ordered {
        let old = terminated(d.old_text.as_deref().unwrap_or(""));
        let new = terminated(&d.new_text);
        let diff = TextDiff::from_lines(old.as_ref(), new.as_ref());
        let mut base = d.start_line.map_or(next_old, |l| l.max(1) as usize);
        for group in diff.grouped_ops(CONTEXT_LINES) {
            let (Some(first), Some(last)) = (group.first(), group.last()) else { continue };
            let old_range = first.old_range().start..last.old_range().end;
            let new_range = first.new_range().start..last.new_range().end;
            let mut old_first = base + old_range.start;
            if old_first < next_old {
                base += next_old - old_first;
                old_first = next_old;
            }
            let new_first = old_first.saturating_add_signed(drift).max(1);
            let (old_len, new_len) = (old_range.len(), new_range.len());
            // An empty side names the line before it, per the unified format.
            let _ = writeln!(
                out,
                "@@ -{},{old_len} +{},{new_len} @@",
                if old_len == 0 { old_first - 1 } else { old_first },
                if new_len == 0 { new_first - 1 } else { new_first },
            );
            for op in &group {
                for change in diff.iter_changes(op) {
                    let sign = match change.tag() {
                        ChangeTag::Equal => ' ',
                        ChangeTag::Delete => '-',
                        ChangeTag::Insert => '+',
                    };
                    let _ = writeln!(out, "{sign}{}", change.value().trim_end_matches(['\n', '\r']));
                }
            }
            next_old = old_first + old_len;
            drift += new_len as isize - old_len as isize;
        }
    }
    out
}

/// Where a file change stands.
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeState {
    /// Waiting for the user to accept or reject it; nothing has been written.
    Pending { permission_id: String, options: Vec<PermissionOption> },
    /// The agent is applying it.
    Running,
    Applied,
    /// The user (or the permission mode) turned it down.
    Rejected,
    /// The agent tried and the edit failed.
    Failed,
}

/// One tool call's worth of file edits.
#[derive(Debug, Clone, PartialEq)]
pub struct FileChange {
    /// The tool call id, or the permission id when no tool call is known.
    pub id: String,
    pub diffs: Vec<FileDiff>,
    pub state: ChangeState,
}

impl FileChange {
    pub fn is_pending(&self) -> bool {
        matches!(self.state, ChangeState::Pending { .. })
    }

    fn option(&self, kinds: &[PermissionOptionKind]) -> Option<(&str, &PermissionOption)> {
        let ChangeState::Pending { permission_id, options } = &self.state else { return None };
        kinds.iter().find_map(|k| options.iter().find(|o| o.kind == *k)).map(|o| (permission_id.as_str(), o))
    }

    /// `(permission id, option)` that accepts this one change.
    pub fn accept_option(&self) -> Option<(&str, &PermissionOption)> {
        self.option(&[PermissionOptionKind::AllowOnce, PermissionOptionKind::AllowAlways])
    }

    /// `(permission id, option)` that rejects this one change.
    pub fn reject_option(&self) -> Option<(&str, &PermissionOption)> {
        self.option(&[PermissionOptionKind::RejectOnce, PermissionOptionKind::RejectAlways])
    }

    /// The edits grouped by file, in the order the files were first touched.
    pub fn files(&self) -> Vec<(&Path, Vec<&FileDiff>)> {
        let mut out: Vec<(&Path, Vec<&FileDiff>)> = vec![];
        for d in &self.diffs {
            match out.iter_mut().find(|(p, _)| *p == d.path.as_path()) {
                Some((_, v)) => v.push(d),
                None => out.push((d.path.as_path(), vec![d])),
            }
        }
        out
    }

    pub fn stats(&self) -> DiffStats {
        let mut total = DiffStats::default();
        for d in &self.diffs {
            total.add(d.stats());
        }
        total
    }
}

/// Totals for a session's changes. Rejected and failed edits never reached the disk, so they
/// don't count towards the line totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChangeSummary {
    /// Every change in the session, whatever became of it.
    pub total: usize,
    pub files: usize,
    pub stats: DiffStats,
    pub pending: usize,
}

impl ChangeSummary {
    pub fn of<'a>(changes: impl IntoIterator<Item = &'a FileChange>) -> Self {
        let mut files: HashSet<&PathBuf> = HashSet::new();
        let mut summary = ChangeSummary::default();
        for c in changes {
            summary.total += 1;
            if matches!(c.state, ChangeState::Rejected | ChangeState::Failed) {
                continue;
            }
            summary.pending += usize::from(c.is_pending());
            files.extend(c.diffs.iter().map(|d| &d.path));
            summary.stats.add(c.stats());
        }
        summary.files = files.len();
        summary
    }
}

fn was_rejected(p: &PermissionRequest) -> bool {
    let Some(choice) = &p.resolved else { return false };
    // "cancelled", "expired" and "dismissed" aren't options: the edit didn't go ahead either.
    !p.options
        .iter()
        .any(|o| &o.id == choice && matches!(o.kind, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways))
}

fn pending_state(p: &PermissionRequest) -> ChangeState {
    ChangeState::Pending { permission_id: p.id.clone(), options: p.options.clone() }
}

impl SessionState {
    /// Every file edit in the transcript, oldest first.
    pub fn file_changes(&self) -> Vec<FileChange> {
        let permissions: HashMap<&str, &PermissionRequest> = self
            .entries
            .iter()
            .filter_map(|e| match e {
                Entry::Permission(p) => Some((p.tool_call_id.as_deref()?, p)),
                _ => None,
            })
            .collect();
        let mut seen: HashSet<&str> = HashSet::new();
        let mut out = vec![];
        for e in &self.entries {
            match e {
                Entry::Tool(t) => {
                    let permission = permissions.get(t.id.as_str()).copied();
                    let reviewed = permission.filter(|p| !p.diffs.is_empty());
                    let completed = t.status == ToolStatus::Completed;
                    // The permission's diff has context from disk; a finished tool has the real patch.
                    let diffs = match reviewed {
                        Some(p) if !completed || t.diffs.is_empty() => &p.diffs,
                        _ => &t.diffs,
                    };
                    if diffs.is_empty() {
                        continue;
                    }
                    seen.insert(t.id.as_str());
                    let state = match (permission, t.status) {
                        (Some(p), _) if p.resolved.is_none() => pending_state(p),
                        (Some(p), _) if was_rejected(p) => ChangeState::Rejected,
                        (_, ToolStatus::Completed) => ChangeState::Applied,
                        (_, ToolStatus::Failed) => ChangeState::Failed,
                        _ => ChangeState::Running,
                    };
                    out.push(FileChange { id: t.id.clone(), diffs: diffs.clone(), state });
                }
                // Agents may ask before announcing the tool call, or never announce it.
                Entry::Permission(p) if !p.diffs.is_empty() => {
                    if p.tool_call_id.as_deref().is_some_and(|id| seen.contains(id)) {
                        continue;
                    }
                    let tool_follows = p.tool_call_id.as_deref().is_some_and(|id| {
                        self.entries.iter().any(|e| matches!(e, Entry::Tool(t) if t.id == id && !t.diffs.is_empty()))
                    });
                    if tool_follows {
                        continue;
                    }
                    let state = match &p.resolved {
                        None => pending_state(p),
                        Some(_) if was_rejected(p) => ChangeState::Rejected,
                        Some(_) => ChangeState::Running,
                    };
                    out.push(FileChange { id: p.tool_call_id.clone().unwrap_or_else(|| p.id.clone()), diffs: p.diffs.clone(), state });
                }
                _ => {}
            }
        }
        out
    }

    /// Changes whenever [`Self::file_changes`] would, without comparing file contents.
    pub fn changes_fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        let hash_diffs = |diffs: &[FileDiff], h: &mut DefaultHasher| {
            for d in diffs {
                d.path.hash(h);
                d.start_line.hash(h);
                d.old_text.as_ref().map(String::len).hash(h);
                d.new_text.len().hash(h);
            }
        };
        for e in &self.entries {
            match e {
                Entry::Tool(t) if !t.diffs.is_empty() => {
                    t.id.hash(&mut h);
                    (t.status as u8).hash(&mut h);
                    hash_diffs(&t.diffs, &mut h);
                }
                Entry::Permission(p) if !p.diffs.is_empty() => {
                    p.id.hash(&mut h);
                    p.resolved.hash(&mut h);
                    hash_diffs(&p.diffs, &mut h);
                }
                _ => {}
            }
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentKind, PermissionMode};
    use crate::session::{SessionEvent, SessionKind, SessionMeta, ToolCall, ToolKind};

    fn diff(old: Option<&str>, new: &str, line: Option<u32>) -> FileDiff {
        FileDiff { path: "src/a.rs".into(), old_text: old.map(str::to_string), new_text: new.into(), start_line: line }
    }

    fn options() -> Vec<PermissionOption> {
        vec![
            PermissionOption { id: "allow".into(), label: "Allow".into(), kind: PermissionOptionKind::AllowOnce },
            PermissionOption { id: "deny".into(), label: "Deny".into(), kind: PermissionOptionKind::RejectOnce },
        ]
    }

    fn tool(id: &str, status: ToolStatus, diffs: Vec<FileDiff>) -> Entry {
        Entry::Tool(ToolCall {
            id: id.into(),
            name: "Edit".into(),
            title: "Edit a.rs".into(),
            kind: ToolKind::Edit,
            status,
            input: None,
            output: None,
            diffs,
            locations: vec![],
        })
    }

    fn permission(id: &str, tool_id: &str, diffs: Vec<FileDiff>) -> Entry {
        Entry::Permission(PermissionRequest {
            id: id.into(),
            tool_call_id: Some(tool_id.into()),
            title: "Edit a.rs".into(),
            detail: None,
            kind: ToolKind::Edit,
            options: options(),
            resolved: None,
            diffs,
        })
    }

    fn session(entries: Vec<Entry>) -> SessionState {
        let mut s = SessionState::new(SessionMeta::new(AgentKind::ClaudeCode, SessionKind::Chat, "/tmp".into(), PermissionMode::Ask));
        s.entries = entries;
        s
    }

    #[test]
    fn stats_count_only_changed_lines() {
        let d = diff(Some("a\nb\nc"), "a\nB\nc\nd\n", None);
        assert_eq!(d.stats(), DiffStats { added: 2, removed: 1 });
    }

    #[test]
    fn patch_uses_real_line_numbers() {
        let d = diff(Some("one\ntwo\nthree\n"), "one\n2\nthree\n", Some(40));
        let patch = unified_patch("a.rs", &[&d]);
        assert_eq!(patch, "--- a/a.rs\n+++ b/a.rs\n@@ -40,3 +40,3 @@\n one\n-two\n+2\n three\n");
    }

    #[test]
    fn new_files_patch_from_dev_null() {
        let d = diff(None, "hello\nworld", None);
        assert_eq!(unified_patch("a.rs", &[&d]), "--- /dev/null\n+++ b/a.rs\n@@ -0,0 +1,2 @@\n+hello\n+world\n");
    }

    #[test]
    fn hunks_stay_ordered_without_positions() {
        // Two edits that don't say where they landed must not produce overlapping hunks.
        let (a, b) = (diff(Some("a\n"), "A\n", None), diff(Some("b\n"), "B\nC\n", None));
        let patch = unified_patch("a.rs", &[&a, &b]);
        assert!(patch.contains("@@ -1,1 +1,1 @@"), "{patch}");
        assert!(patch.contains("@@ -2,1 +2,2 @@"), "{patch}");
    }

    #[test]
    fn later_hunks_account_for_earlier_growth() {
        let (a, b) = (diff(Some("a\n"), "a\nnew\n", Some(1)), diff(Some("z\n"), "Z\n", Some(10)));
        // Passed out of order on purpose.
        let patch = unified_patch("a.rs", &[&b, &a]);
        assert!(patch.contains("@@ -1,1 +1,2 @@"), "{patch}");
        assert!(patch.contains("@@ -10,1 +11,1 @@"), "{patch}");
    }

    #[test]
    fn pending_edit_is_reviewable_then_applied() {
        let snippet = diff(Some("a"), "b", None);
        let with_context = diff(Some("x\na\n"), "x\nb\n", Some(4));
        let mut s = session(vec![tool("t", ToolStatus::Running, vec![snippet]), permission("p", "t", vec![with_context.clone()])]);
        let c = s.file_changes().remove(0);
        assert!(c.is_pending());
        assert_eq!(c.diffs, vec![with_context]);
        assert_eq!(c.accept_option().map(|(p, o)| (p, o.id.as_str())), Some(("p", "allow")));
        assert_eq!(c.reject_option().map(|(_, o)| o.id.as_str()), Some("deny"));
        assert_eq!(ChangeSummary::of(&[c]).pending, 1);

        s.apply(&SessionEvent::Resolved { entry_id: "p".into(), choice: Some("allow".into()), answers: None });
        let patch = diff(Some("a\n"), "b\n", Some(5));
        s.apply(&SessionEvent::Upsert { entry: tool("t", ToolStatus::Completed, vec![patch.clone()]) });
        let c = s.file_changes().remove(0);
        assert_eq!(c.state, ChangeState::Applied);
        assert_eq!(c.diffs, vec![patch]);
    }

    #[test]
    fn rejected_edits_keep_their_diff_but_leave_the_totals() {
        let d = diff(Some("a"), "b", None);
        let mut s = session(vec![tool("t", ToolStatus::Running, vec![d.clone()]), permission("p", "t", vec![d])]);
        s.apply(&SessionEvent::Resolved { entry_id: "p".into(), choice: Some("deny".into()), answers: None });
        s.apply(&SessionEvent::Upsert { entry: tool("t", ToolStatus::Failed, vec![diff(Some("a"), "b", None)]) });
        let changes = s.file_changes();
        assert_eq!(changes[0].state, ChangeState::Rejected);
        assert_eq!(ChangeSummary::of(&changes), ChangeSummary { total: 1, ..Default::default() });
    }

    #[test]
    fn cancelled_requests_count_as_rejected() {
        let d = diff(Some("a"), "b", None);
        let mut s = session(vec![tool("t", ToolStatus::Running, vec![d.clone()]), permission("p", "t", vec![d])]);
        s.apply(&SessionEvent::Resolved { entry_id: "p".into(), choice: Some("cancelled".into()), answers: None });
        assert_eq!(s.file_changes()[0].state, ChangeState::Rejected);
    }

    #[test]
    fn permission_without_a_tool_call_is_still_listed_once() {
        let d = diff(None, "new", None);
        let s = session(vec![permission("p", "unknown", vec![d.clone()])]);
        assert_eq!(s.file_changes().len(), 1);
        // Announced after the request: one change, not two.
        let s = session(vec![permission("p", "t", vec![d.clone()]), tool("t", ToolStatus::Running, vec![d])]);
        assert_eq!(s.file_changes().len(), 1);
    }

    #[test]
    fn summary_counts_each_file_once() {
        let a = FileChange { id: "1".into(), diffs: vec![diff(Some("a"), "b", None)], state: ChangeState::Applied };
        let b = FileChange { id: "2".into(), diffs: vec![diff(Some("c"), "d", None)], state: ChangeState::Applied };
        let s = ChangeSummary::of([&a, &b]);
        assert_eq!((s.files, s.stats), (1, DiffStats { added: 2, removed: 2 }));
    }

    #[test]
    fn fingerprint_follows_review_state() {
        let d = diff(Some("a"), "b", None);
        let mut s = session(vec![tool("t", ToolStatus::Running, vec![d.clone()]), permission("p", "t", vec![d])]);
        let before = s.changes_fingerprint();
        s.apply(&SessionEvent::Resolved { entry_id: "p".into(), choice: Some("allow".into()), answers: None });
        assert_ne!(before, s.changes_fingerprint());
    }
}
