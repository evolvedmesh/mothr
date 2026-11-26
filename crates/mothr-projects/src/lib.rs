//! Fast fuzzy finding of development projects on this machine, plus pinned and recent projects.
//!
//! [`ProjectIndex`] discovers projects by walking configured roots in parallel (stopping at git
//! repository roots and skipping heavy directories such as `node_modules`/`target`) and by
//! seeding from coding-agent history (`~/.claude/projects`, `~/.codex/sessions`). The index,
//! pins and recents persist to `data_dir()/projects.json` so a fresh start can search
//! immediately, before the first rescan finishes.

pub mod browse;
mod detect;
mod discover;

use chrono::{DateTime, Utc};
use discover::Found;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use mothr_core::{ProjectHit, ProjectRef, data_dir, home_dir, tildify, write_atomic};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Maximum number of remembered recent projects.
pub const MAX_RECENT: usize = 50;

/// Multiplier applied to a fuzzy match against the project name (vs. its full path).
const NAME_WEIGHT: u32 = 2;
/// Bonus when the query equals the project name (case-insensitive).
const EXACT_NAME_BOOST: u32 = 120;
/// Bonus when the project name starts with the query (case-insensitive).
const PREFIX_NAME_BOOST: u32 = 40;
const PINNED_BOOST: u32 = 40;
/// Bonus for the most recent project; decays linearly over the recents list.
const RECENT_BOOST: u32 = 30;

/// Construction options for [`ProjectIndex`].
#[derive(Debug, Clone)]
pub struct IndexOptions {
    /// Where the index, pins and recents persist. `None` keeps everything in memory.
    pub store_path: Option<PathBuf>,
    /// Whether [`ProjectIndex::scan`] also seeds projects from Claude Code / Codex history.
    pub seed_from_history: bool,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            store_path: Some(default_store_path()),
            seed_from_history: true,
        }
    }
}

/// `data_dir()/projects.json`.
pub fn default_store_path() -> PathBuf {
    data_dir().join("projects.json")
}

/// A recently used project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Recent {
    path: PathBuf,
    at: DateTime<Utc>,
}

/// One project as persisted in `projects.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredProject {
    #[serde(flatten)]
    project: ProjectRef,
    /// Last activity, unix seconds.
    #[serde(default)]
    mtime: i64,
    /// Added by the user via [`ProjectIndex::add_manual`]; survives rescans.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    manual: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    projects: Vec<StoredProject>,
    #[serde(default)]
    pinned: Vec<PathBuf>,
    #[serde(default)]
    recent: Vec<Recent>,
}

/// An indexed project with precomputed matcher haystacks.
struct Entry {
    project: ProjectRef,
    mtime: i64,
    manual: bool,
    name_lower: String,
    name32: Utf32String,
    path32: Utf32String,
}

impl Entry {
    fn new(project: ProjectRef, mtime: i64, manual: bool) -> Self {
        let tilde = tildify(&project.path);
        Self {
            name_lower: project.name.to_lowercase(),
            name32: utf32(&project.name),
            path32: utf32(&tilde),
            project,
            mtime,
            manual,
        }
    }

    fn from_found(f: Found, manual: bool) -> Self {
        Self::new(f.project, f.mtime, manual)
    }
}

/// Owned matcher haystack whose indices are plain `char` indices (nucleo's `From<&str>`
/// segments by grapheme, which would misalign highlights for combining characters).
fn utf32(s: &str) -> Utf32String {
    if s.is_ascii() {
        Utf32String::Ascii(s.into())
    } else {
        Utf32String::Unicode(s.chars().collect())
    }
}

#[derive(Default)]
struct State {
    entries: Vec<Entry>,
    by_path: HashMap<PathBuf, usize>,
    /// In the order the user pinned them.
    pinned: Vec<PathBuf>,
    /// Newest first, at most [`MAX_RECENT`].
    recent: Vec<Recent>,
}

impl State {
    fn from_stored(stored: Stored) -> Self {
        let mut st = State {
            pinned: stored.pinned,
            recent: stored.recent,
            ..Default::default()
        };
        st.recent.sort_by_key(|r| std::cmp::Reverse(r.at));
        st.recent.truncate(MAX_RECENT);
        st.set_entries(
            stored
                .projects
                .into_iter()
                .map(|p| Entry::new(p.project, p.mtime, p.manual))
                .collect(),
        );
        st
    }

    fn to_stored(&self) -> Stored {
        Stored {
            projects: self
                .entries
                .iter()
                .map(|e| StoredProject {
                    project: e.project.clone(),
                    mtime: e.mtime,
                    manual: e.manual,
                })
                .collect(),
            pinned: self.pinned.clone(),
            recent: self.recent.clone(),
        }
    }

    /// Replace all entries (deduplicating by path) and resync derived data.
    fn set_entries(&mut self, entries: Vec<Entry>) {
        self.entries.clear();
        self.by_path.clear();
        for e in entries {
            if !self.by_path.contains_key(&e.project.path) {
                self.by_path
                    .insert(e.project.path.clone(), self.entries.len());
                self.entries.push(e);
            }
        }
        let pinned: HashSet<&PathBuf> = self.pinned.iter().collect();
        for e in &mut self.entries {
            e.project.pinned = pinned.contains(&e.project.path);
        }
    }

    /// Insert or replace an entry, keeping its pinned flag in sync.
    fn upsert(&mut self, mut entry: Entry) -> usize {
        entry.project.pinned = self.pinned.contains(&entry.project.path);
        match self.by_path.get(&entry.project.path) {
            Some(&i) => {
                entry.manual |= self.entries[i].manual;
                self.entries[i] = entry;
                i
            }
            None => {
                let i = self.entries.len();
                self.by_path.insert(entry.project.path.clone(), i);
                self.entries.push(entry);
                i
            }
        }
    }

    fn entry(&self, path: &Path) -> Option<&Entry> {
        self.by_path.get(path).map(|&i| &self.entries[i])
    }

    /// The indexed project at `path`, or a bare placeholder for unindexed (e.g. deleted
    /// but still pinned) paths.
    fn project_or_placeholder(&self, path: &Path) -> ProjectRef {
        match self.entry(path) {
            Some(e) => e.project.clone(),
            None => {
                let mut p = ProjectRef::from_path(path.to_path_buf());
                p.pinned = self.pinned.iter().any(|x| x == path);
                p
            }
        }
    }
}

/// Thread-safe index of development projects. Cheap to share via `Arc`; all methods take
/// `&self`. Searches never block on a running [`scan`](Self::scan) except for the brief swap
/// at its end.
pub struct ProjectIndex {
    state: RwLock<State>,
    matcher: Mutex<Matcher>,
    options: IndexOptions,
    /// Serializes writes of the store file.
    save_lock: Mutex<()>,
}

impl Default for ProjectIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectIndex {
    /// Load the persisted index, pins and recents from `data_dir()/projects.json` (empty if
    /// missing or unreadable). Does not scan; call [`scan`](Self::scan) for fresh results.
    pub fn new() -> Self {
        Self::with_options(IndexOptions::default())
    }

    /// Like [`new`](Self::new) but with explicit storage / seeding options.
    pub fn with_options(options: IndexOptions) -> Self {
        let stored = options
            .store_path
            .as_deref()
            .map(load_store)
            .unwrap_or_default();
        Self {
            state: RwLock::new(State::from_stored(stored)),
            matcher: Mutex::new(Matcher::new(Config::DEFAULT.match_paths())),
            options,
            save_lock: Mutex::new(()),
        }
    }

    /// Rescan for projects. Blocking: walks `roots` in parallel up to `max_depth` levels deep
    /// (and seeds from agent history if enabled), replaces the index, keeps manual, pinned
    /// and recent projects, and persists the result. Returns the number of indexed projects.
    pub fn scan(&self, roots: &[PathBuf], max_depth: usize) -> usize {
        let started = std::time::Instant::now();
        let seed = self.options.seed_from_history;
        let (walked, history) = std::thread::scope(|s| {
            let history = s.spawn(move || {
                if seed {
                    discover::from_history(&home_dir())
                } else {
                    Vec::new()
                }
            });
            let walked = discover::walk(roots, max_depth);
            (walked, history.join().unwrap_or_default())
        });

        // Projects that must survive a rescan even if the walk did not find them.
        let (manual, keep): (Vec<PathBuf>, Vec<PathBuf>) = {
            let st = self.state.read();
            let manual = st
                .entries
                .iter()
                .filter(|e| e.manual)
                .map(|e| e.project.path.clone())
                .collect();
            let keep = st
                .pinned
                .iter()
                .chain(st.recent.iter().map(|r| &r.path))
                .cloned()
                .collect();
            (manual, keep)
        };

        let mut entries: Vec<Entry> =
            Vec::with_capacity(walked.len() + history.len() + manual.len());
        let mut manual_set: HashSet<PathBuf> = HashSet::new();
        for path in manual {
            if let Some(f) = discover::describe(&path, false) {
                manual_set.insert(path);
                entries.push(Entry::from_found(f, true));
            }
        }
        entries.extend(
            walked
                .into_iter()
                .chain(history)
                .map(|f| Entry::from_found(f, false)),
        );
        let mut have: HashSet<PathBuf> = entries.iter().map(|e| e.project.path.clone()).collect();
        for path in keep {
            if !have.contains(&path)
                && path.is_dir()
                && let Some(f) = discover::describe(&path, false)
            {
                have.insert(path);
                entries.push(Entry::from_found(f, false));
            }
        }

        let count = {
            let mut st = self.state.write();
            st.set_entries(entries);
            st.entries.len()
        };
        tracing::debug!(count, elapsed = ?started.elapsed(), "project scan finished");
        self.save();
        count
    }

    /// Fuzzy-search projects. Matches the query against both the project name (weighted
    /// higher) and its `~`-relative path; pinned and recent projects get a boost.
    ///
    /// An empty (or whitespace-only) query lists pinned projects first, then recent ones,
    /// then the rest by most recent activity.
    ///
    /// `name_indices` are char indices into `project.name`; `path_indices` are char indices
    /// into [`tildify`]`(&project.path)`.
    pub fn search(&self, query: &str, limit: usize) -> Vec<ProjectHit> {
        if limit == 0 {
            return Vec::new();
        }
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let st = self.state.read();
        if pattern.atoms.is_empty() {
            return default_order(&st, limit);
        }

        let query_lower = query.trim().to_lowercase();
        let recent_rank: HashMap<&Path, usize> = st
            .recent
            .iter()
            .enumerate()
            .map(|(i, r)| (r.path.as_path(), i))
            .collect();
        let now = Utc::now().timestamp();

        let mut matcher = self.matcher.lock();
        let mut scored: Vec<(u32, usize)> = Vec::new();
        for (i, e) in st.entries.iter().enumerate() {
            let name = pattern
                .score(e.name32.slice(..), &mut matcher)
                .map(|s| s * NAME_WEIGHT);
            let path = pattern.score(e.path32.slice(..), &mut matcher);
            let Some(mut score) = name.max(path) else {
                continue;
            };
            if e.name_lower == query_lower {
                score += EXACT_NAME_BOOST;
            } else if e.name_lower.starts_with(&query_lower) {
                score += PREFIX_NAME_BOOST;
            }
            if e.project.pinned {
                score += PINNED_BOOST;
            }
            if let Some(&rank) = recent_rank.get(e.project.path.as_path()) {
                score +=
                    RECENT_BOOST.saturating_sub((rank * RECENT_BOOST as usize / MAX_RECENT) as u32);
            }
            score += freshness_boost(now - e.mtime);
            scored.push((score, i));
        }

        let cmp = |a: &(u32, usize), b: &(u32, usize)| {
            let (ea, eb) = (&st.entries[a.1], &st.entries[b.1]);
            b.0.cmp(&a.0)
                .then(eb.mtime.cmp(&ea.mtime))
                .then(ea.project.name.len().cmp(&eb.project.name.len()))
                .then(ea.project.path.cmp(&eb.project.path))
        };
        if scored.len() > limit {
            scored.select_nth_unstable_by(limit - 1, cmp);
            scored.truncate(limit);
        }
        scored.sort_unstable_by(cmp);

        let mut buf = Vec::new();
        scored
            .into_iter()
            .map(|(score, i)| {
                let e = &st.entries[i];
                let name_indices = match_indices(&pattern, &e.name32, &mut matcher, &mut buf);
                let path_indices = match_indices(&pattern, &e.path32, &mut matcher, &mut buf);
                ProjectHit {
                    project: e.project.clone(),
                    score,
                    name_indices,
                    path_indices,
                    display_path: mothr_core::tildify(&e.project.path),
                }
            })
            .collect()
    }

    /// Pinned projects in pin order. Pins whose directory no longer exists are still returned
    /// (as placeholders if they are no longer indexed).
    pub fn pinned(&self) -> Vec<ProjectRef> {
        let st = self.state.read();
        st.pinned
            .iter()
            .map(|p| st.project_or_placeholder(p))
            .collect()
    }

    /// Up to `limit` recently used projects, newest first.
    pub fn recent(&self, limit: usize) -> Vec<ProjectRef> {
        let st = self.state.read();
        st.recent
            .iter()
            .take(limit)
            .map(|r| st.project_or_placeholder(&r.path))
            .collect()
    }

    /// Pin or unpin a project. Pinning an unindexed directory adds it to the index.
    pub fn set_pinned(&self, path: &Path, pinned: bool) {
        let path = normalize(path);
        let found = if pinned && path.is_dir() && self.state.read().entry(&path).is_none() {
            discover::describe(&path, false)
        } else {
            None
        };
        {
            let mut st = self.state.write();
            let was = st.pinned.contains(&path);
            if pinned && !was {
                st.pinned.push(path.clone());
            } else if !pinned && was {
                st.pinned.retain(|p| *p != path);
            } else if found.is_none() {
                return;
            }
            if let Some(f) = found {
                st.upsert(Entry::from_found(f, false));
            }
            if let Some(&i) = st.by_path.get(&path) {
                st.entries[i].project.pinned = pinned;
            }
        }
        self.save();
    }

    /// Record that a chat was started in `path`, moving it to the front of the recents list
    /// (capped at [`MAX_RECENT`]) and adding it to the index if needed.
    pub fn touch_recent(&self, path: &Path) {
        let path = normalize(path);
        let found = if path.is_dir() && self.state.read().entry(&path).is_none() {
            discover::describe(&path, false)
        } else {
            None
        };
        {
            let now = Utc::now();
            let mut st = self.state.write();
            st.recent.retain(|r| r.path != path);
            st.recent.insert(
                0,
                Recent {
                    path: path.clone(),
                    at: now,
                },
            );
            st.recent.truncate(MAX_RECENT);
            if let Some(f) = found {
                st.upsert(Entry::from_found(f, false));
            }
            if let Some(&i) = st.by_path.get(&path) {
                let e = &mut st.entries[i];
                e.mtime = e.mtime.max(now.timestamp());
            }
        }
        self.save();
    }

    /// Add a folder the user picked explicitly. It is indexed even without project markers and
    /// survives rescans. Returns `None` if `path` is not a directory.
    pub fn add_manual(&self, path: PathBuf) -> Option<ProjectRef> {
        let path = normalize(&path);
        let found = discover::describe(&path, false).filter(|_| path.is_dir())?;
        let project = {
            let mut st = self.state.write();
            let i = st.upsert(Entry::from_found(found, true));
            st.entries[i].manual = true;
            st.entries[i].project.clone()
        };
        self.save();
        Some(project)
    }

    /// The indexed project at `path`, if any.
    pub fn get(&self, path: &Path) -> Option<ProjectRef> {
        let st = self.state.read();
        st.entry(path)
            .or_else(|| st.entry(&normalize(path)))
            .map(|e| e.project.clone())
    }

    /// Number of indexed projects.
    pub fn len(&self) -> usize {
        self.state.read().entries.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Persist the current state (best effort; failures are logged).
    fn save(&self) {
        let Some(path) = self.options.store_path.as_deref() else {
            return;
        };
        let _guard = self.save_lock.lock();
        let bytes = match serde_json::to_vec(&self.state.read().to_stored()) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("failed to serialize project index: {e}");
                return;
            }
        };
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .map_err(anyhow::Error::from)
            .and_then(|()| write_atomic(path, &bytes));
        if let Err(e) = result {
            tracing::warn!("failed to save project index to {}: {e}", path.display());
        }
    }
}

fn load_store(path: &Path) -> Stored {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!("ignoring invalid project index {}: {e}", path.display());
            Stored::default()
        }),
        Err(_) => Stored::default(),
    }
}

/// Absolute path without resolving symlinks; falls back to the input.
fn normalize(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Small bonus for projects touched recently (`age_secs` since last activity).
fn freshness_boost(age_secs: i64) -> u32 {
    const DAY: i64 = 86_400;
    const WEEK: i64 = 7 * DAY;
    const MONTH: i64 = 30 * DAY;
    match age_secs {
        ..DAY => 12,
        DAY..WEEK => 8,
        WEEK..MONTH => 4,
        _ => 0,
    }
}

/// Sorted, unique highlight indices. Each query word is matched on its own so that e.g.
/// `"web app"` still highlights `app` in the name even though `web` only occurs in the path.
fn match_indices(
    pattern: &Pattern,
    hay: &Utf32String,
    matcher: &mut Matcher,
    buf: &mut Vec<u32>,
) -> Vec<u32> {
    buf.clear();
    for atom in &pattern.atoms {
        // A failed atom may leave partial indices behind; roll them back.
        let len = buf.len();
        if atom.indices(hay.slice(..), matcher, buf).is_none() {
            buf.truncate(len);
        }
    }
    buf.sort_unstable();
    buf.dedup();
    buf.clone()
}

/// Empty-query listing: pinned, then recent, then everything else by activity.
fn default_order(st: &State, limit: usize) -> Vec<ProjectHit> {
    let mut seen: HashSet<&Path> = HashSet::new();
    let mut out: Vec<ProjectRef> = Vec::with_capacity(limit);
    let firsts = st.pinned.iter().chain(st.recent.iter().map(|r| &r.path));
    for path in firsts {
        if out.len() >= limit {
            break;
        }
        if seen.insert(path.as_path()) {
            out.push(st.project_or_placeholder(path));
        }
    }
    if out.len() < limit {
        let mut rest: Vec<&Entry> = st
            .entries
            .iter()
            .filter(|e| !seen.contains(e.project.path.as_path()))
            .collect();
        let take = limit - out.len();
        let cmp = |a: &&Entry, b: &&Entry| {
            b.mtime
                .cmp(&a.mtime)
                .then(a.project.name.cmp(&b.project.name))
        };
        if rest.len() > take {
            rest.select_nth_unstable_by(take - 1, cmp);
            rest.truncate(take);
        }
        rest.sort_unstable_by(cmp);
        out.extend(rest.into_iter().map(|e| e.project.clone()));
    }
    let n = out.len() as u32;
    out.into_iter()
        .enumerate()
        .map(|(i, project)| ProjectHit {
            score: n - i as u32,
            name_indices: vec![],
            path_indices: vec![],
            display_path: mothr_core::tildify(&project.path),
            project,
        })
        .collect()
}

#[cfg(test)]
mod tests;
