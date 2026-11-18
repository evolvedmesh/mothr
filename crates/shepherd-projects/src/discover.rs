//! Finding project directories: a parallel filesystem walk plus agent-history seeding.

use crate::detect::{self, activity_time, git_branch, git_dir, skip_dir_name};
use ignore::{WalkBuilder, WalkState};
use serde::Deserialize;
use shepherd_core::ProjectRef;
use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// A discovered project plus its activity time (unix seconds).
#[derive(Debug, Clone)]
pub(crate) struct Found {
    pub project: ProjectRef,
    pub mtime: i64,
}

/// Describe `dir` as a project. Returns `None` when it has no project markers and
/// `require_marker` is set.
pub(crate) fn describe(dir: &Path, require_marker: bool) -> Option<Found> {
    let d = detect::detect(dir);
    if require_marker && !d.is_project {
        return None;
    }
    Some(build(dir.to_path_buf(), d))
}

fn build(path: PathBuf, d: detect::Detection) -> Found {
    let gd = if d.has_git { git_dir(&path) } else { None };
    let branch = gd.as_deref().and_then(git_branch);
    let mtime = activity_time(&path, gd.as_deref());
    let mut project = ProjectRef::from_path(path);
    project.tags = d.tags;
    project.git_branch = branch;
    Found { project, mtime }
}

/// Walk `roots` in parallel (not following symlinks, at most `max_depth` levels below each
/// root), returning every project directory. Descent stops at git repository roots and at
/// heavy/irrelevant directories (see [`detect::SKIP_DIRS`]).
pub(crate) fn walk(roots: &[PathBuf], max_depth: usize) -> Vec<Found> {
    let roots: Vec<&PathBuf> = roots.iter().filter(|r| r.is_dir()).collect();
    let Some((first, rest)) = roots.split_first() else {
        return Vec::new();
    };
    let mut builder = WalkBuilder::new(first);
    for r in rest {
        builder.add(r);
    }
    builder
        .standard_filters(false)
        .follow_links(false)
        .same_file_system(false)
        .max_depth(Some(max_depth))
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            e.file_type().is_some_and(|t| t.is_dir())
                && e.file_name().to_str().is_some_and(|n| !skip_dir_name(n))
        });

    let (tx, rx) = mpsc::channel::<Found>();
    builder.build_parallel().run(|| {
        let tx = tx.clone();
        Box::new(move |res| {
            let Ok(entry) = res else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|t| t.is_dir()) {
                return WalkState::Continue;
            }
            let d = detect::detect(entry.path());
            if !d.is_project {
                return WalkState::Continue;
            }
            let has_git = d.has_git;
            let _ = tx.send(build(entry.into_path(), d));
            if has_git {
                WalkState::Skip
            } else {
                WalkState::Continue
            }
        })
    });
    drop(tx);

    // Overlapping roots can yield the same directory twice.
    let mut seen = HashSet::new();
    rx.into_iter()
        .filter(|f| seen.insert(f.project.path.clone()))
        .collect()
}

/// Maximum bytes read from the head of a history file.
const HEAD_BYTES: u64 = 512 * 1024;

/// Read up to `max_lines` lines (bounded by [`HEAD_BYTES`]) from the start of a file.
fn head_lines(path: &Path, max_lines: usize) -> Vec<String> {
    let Ok(f) = fs::File::open(path) else {
        return Vec::new();
    };
    BufReader::new(f.take(HEAD_BYTES))
        .lines()
        .map_while(Result::ok)
        .take(max_lines)
        .collect()
}

#[derive(Deserialize)]
struct CwdLine {
    cwd: Option<String>,
}

#[derive(Deserialize)]
struct CodexLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    payload: Option<CwdLine>,
}

/// Working directories of Claude Code sessions: `~/.claude/projects/<encoded>/*.jsonl`. The
/// directory names are lossy, so the `cwd` field of the newest session file is used instead.
fn claude_cwds(home: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(home.join(".claude/projects")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dir in rd.flatten() {
        let Ok(files) = fs::read_dir(dir.path()) else {
            continue;
        };
        let newest = files
            .flatten()
            .filter(|f| f.path().extension().is_some_and(|e| e == "jsonl"))
            .filter_map(|f| Some((f.metadata().ok()?.modified().ok()?, f.path())))
            .max_by_key(|(t, _)| *t);
        let Some((_, file)) = newest else { continue };
        let cwd = head_lines(&file, 20)
            .iter()
            .filter(|l| l.contains("\"cwd\""))
            .find_map(|l| serde_json::from_str::<CwdLine>(l).ok()?.cwd);
        if let Some(cwd) = cwd {
            out.push(PathBuf::from(cwd));
        }
    }
    out
}

/// Working directories of Codex sessions: `session_meta` lines of the newest 200
/// `~/.codex/sessions/**/rollout-*.jsonl` files.
fn codex_cwds(home: &Path) -> Vec<PathBuf> {
    let root = home.join(".codex/sessions");
    if !root.is_dir() {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = WalkBuilder::new(&root)
        .standard_filters(false)
        .follow_links(false)
        .max_depth(Some(5))
        .build()
        .flatten()
        .filter(|e| {
            e.file_type().is_some_and(|t| t.is_file())
                && e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
        })
        .map(|e| e.into_path())
        .collect();
    // Rollout names embed a sortable timestamp (rollout-YYYY-MM-DDTHH-MM-SS-…), as do the
    // YYYY/MM/DD parent directories, so sorting by file name is newest-first without stat calls.
    files.sort_unstable_by(|a, b| b.file_name().cmp(&a.file_name()));
    files.truncate(200);
    files
        .iter()
        .filter_map(|f| {
            head_lines(f, 5)
                .iter()
                .filter(|l| l.contains("session_meta"))
                .find_map(|l| {
                    let line: CodexLine = serde_json::from_str(l).ok()?;
                    (line.kind.as_deref() == Some("session_meta")).then_some(())?;
                    line.payload?.cwd
                })
                .map(PathBuf::from)
        })
        .collect()
}

/// Map a session's working directory to the project it belongs to: the nearest enclosing git
/// repository root (stopping at `home`), or the directory itself. Returns `None` for paths
/// that are not useful projects (missing, `/`, home itself, temp dirs).
fn project_for_cwd(cwd: &Path, home: &Path) -> Option<PathBuf> {
    if !cwd.is_absolute() || !cwd.is_dir() || cwd == home || cwd.parent().is_none() {
        return None;
    }
    let tmp = std::env::temp_dir();
    if cwd.starts_with(&tmp) || cwd.starts_with("/tmp") || cwd.starts_with("/var/folders") {
        return None;
    }
    Some(enclosing_repo(cwd, home).unwrap_or_else(|| cwd.to_path_buf()))
}

/// The nearest ancestor-or-self of `dir` (below `home` / `/`) that has a `.git` entry.
fn enclosing_repo(dir: &Path, home: &Path) -> Option<PathBuf> {
    let mut cur = Some(dir);
    while let Some(dir) = cur {
        if dir == home || dir.parent().is_none() {
            break;
        }
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

/// Projects that coding agents (Claude Code, Codex) have been used in, whether or not they
/// contain markers. Missing history directories are fine.
pub(crate) fn from_history(home: &Path) -> Vec<Found> {
    let mut cwds = claude_cwds(home);
    cwds.extend(codex_cwds(home));
    let mut seen = HashSet::new();
    cwds.iter()
        .filter_map(|c| project_for_cwd(c, home))
        .filter(|p| seen.insert(p.clone()))
        .filter_map(|p| describe(&p, false))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_seeding() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let proj = home.join("work/alpha");
        fs::create_dir_all(proj.join(".git")).unwrap();
        fs::create_dir_all(proj.join("src/deep")).unwrap();

        let cdir = home.join(".claude/projects/-work-alpha");
        fs::create_dir_all(&cdir).unwrap();
        let line = serde_json::json!({"type": "user", "cwd": proj.join("src/deep")});
        fs::write(
            cdir.join("a.jsonl"),
            format!("{{\"type\":\"mode\"}}\n{line}\n"),
        )
        .unwrap();
        assert_eq!(claude_cwds(&home), vec![proj.join("src/deep")]);

        let xdir = home.join(".codex/sessions/2026/01/02");
        fs::create_dir_all(&xdir).unwrap();
        let meta =
            serde_json::json!({"type": "session_meta", "payload": {"cwd": "/nonexistent/x"}});
        fs::write(
            xdir.join("rollout-2026-01-02T00-00-00-abc.jsonl"),
            format!("{meta}\n"),
        )
        .unwrap();
        assert_eq!(codex_cwds(&home), vec![PathBuf::from("/nonexistent/x")]);
    }

    #[test]
    fn cwd_maps_to_git_root() {
        let tmp = tempfile::tempdir().unwrap();
        let home = PathBuf::from("/definitely/not/home");
        let proj = tmp.path().join("alpha");
        fs::create_dir_all(proj.join(".git")).unwrap();
        fs::create_dir_all(proj.join("src")).unwrap();
        assert_eq!(enclosing_repo(&proj.join("src"), &home), Some(proj.clone()));
        assert_eq!(enclosing_repo(&proj, &home), Some(proj.clone()));
        assert_eq!(enclosing_repo(tmp.path(), &home), None);
        // Temp dirs are filtered out of history seeding.
        assert_eq!(project_for_cwd(&proj.join("src"), &home), None);
        assert_eq!(project_for_cwd(Path::new("/nonexistent/dir"), &home), None);
    }
}
