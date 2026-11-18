//! Cheap, single-directory project detection: markers, stack tags, git branch and activity time.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What a single `read_dir` of a directory tells us about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Detection {
    /// The directory contains at least one project marker.
    pub is_project: bool,
    /// The directory contains a `.git` dir or file (repo root or worktree).
    pub has_git: bool,
    /// Sorted, deduplicated stack tags.
    pub tags: Vec<String>,
}

/// Directory names never descended into while scanning.
pub(crate) const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "bower_components",
    "jspm_packages",
    "target",
    "vendor",
    "dist",
    "build",
    "out-tsc",
    "venv",
    "env",
    "site-packages",
    "__pycache__",
    "Library",
    "AppData",
    "Applications",
    "Pods",
    "DerivedData",
    "Carthage",
    "snap",
    "flatpak",
    "Trash",
    "$RECYCLE.BIN",
    "System Volume Information",
    "Music",
    "Pictures",
    "Videos",
    "Movies",
    "Downloads",
    "Steam",
    "steamapps",
    "go-build",
];

/// Should the walker skip a directory with this name? Hidden directories (`.git`, `.cache`,
/// `.venv`, `.cargo`, `.rustup`, `.npm`, `.gradle`, …) are always skipped.
pub(crate) fn skip_dir_name(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

/// Inspect the entries of `dir` for project markers. Unreadable directories yield the default
/// (not a project).
pub(crate) fn detect(dir: &Path) -> Detection {
    let Ok(rd) = fs::read_dir(dir) else {
        return Detection::default();
    };
    let mut d = Detection::default();
    let mut tags: Vec<&'static str> = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let tag: Option<&'static str> = match name {
            ".git" => {
                d.has_git = true;
                Some("git")
            }
            "Cargo.toml" => Some("rust"),
            "package.json" => Some("node"),
            "tsconfig.json" | "deno.json" | "deno.jsonc" => Some("ts"),
            "go.mod" => Some("go"),
            "pyproject.toml" | "setup.py" | "requirements.txt" | "Pipfile" | "setup.cfg" => {
                Some("python")
            }
            "Gemfile" => Some("ruby"),
            "composer.json" => Some("php"),
            "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "settings.gradle"
            | "settings.gradle.kts" => Some("java"),
            "Package.swift" => Some("swift"),
            "pubspec.yaml" => Some("dart"),
            "mix.exs" => Some("elixir"),
            "CMakeLists.txt" | "meson.build" => Some("cpp"),
            "flake.nix" | "default.nix" | "shell.nix" => Some("nix"),
            "project.godot" => Some("godot"),
            _ => {
                let ext = Path::new(name).extension().and_then(|e| e.to_str());
                match ext {
                    Some("sln" | "csproj" | "fsproj" | "vbproj") => Some("dotnet"),
                    Some("xcodeproj" | "xcworkspace") => Some("swift"),
                    _ => None,
                }
            }
        };
        if let Some(tag) = tag {
            tags.push(tag);
        }
    }
    // A bare Makefile is deliberately not a marker: it only counts together with `.git`,
    // which already marks the directory on its own.
    d.is_project = !tags.is_empty();
    tags.sort_unstable();
    tags.dedup();
    d.tags = tags.into_iter().map(str::to_owned).collect();
    d
}

/// Resolve the git directory for a repo root: `.git` itself, or the `gitdir:` target of a
/// `.git` file (worktrees, submodules).
pub(crate) fn git_dir(dir: &Path) -> Option<PathBuf> {
    let dot = dir.join(".git");
    let meta = fs::metadata(&dot).ok()?;
    if meta.is_dir() {
        return Some(dot);
    }
    let text = fs::read_to_string(&dot).ok()?;
    let target = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    if target.is_empty() {
        return None;
    }
    let p = Path::new(target);
    Some(if p.is_absolute() {
        p.to_path_buf()
    } else {
        dir.join(p)
    })
}

/// The current branch (or short detached commit) read straight from `HEAD`.
pub(crate) fn git_branch(git_dir: &Path) -> Option<String> {
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(r) = head.strip_prefix("ref:") {
        let r = r.trim();
        let branch = r.strip_prefix("refs/heads/").unwrap_or(r);
        return (!branch.is_empty()).then(|| branch.to_owned());
    }
    // Detached HEAD: a hex commit id.
    (head.len() >= 7 && head.bytes().all(|b| b.is_ascii_hexdigit())).then(|| head[..7].to_owned())
}

fn mtime_secs(path: &Path) -> Option<i64> {
    let t = fs::metadata(path).ok()?.modified().ok()?;
    let secs = match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    };
    Some(secs)
}

/// Best-effort "last activity" time in unix seconds: the newest of the directory itself and,
/// for git repos, `HEAD`, `index` and `logs/HEAD`.
pub(crate) fn activity_time(dir: &Path, git_dir: Option<&Path>) -> i64 {
    let mut t = mtime_secs(dir).unwrap_or(0);
    if let Some(g) = git_dir {
        for f in ["HEAD", "index", "logs/HEAD"] {
            if let Some(m) = mtime_secs(&g.join(f)) {
                t = t.max(m);
            }
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_parsing() {
        let tmp = tempfile::tempdir().unwrap();
        let g = tmp.path().join(".git");
        fs::create_dir(&g).unwrap();
        fs::write(g.join("HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(git_dir(tmp.path()).as_deref(), Some(g.as_path()));
        assert_eq!(git_branch(&g).as_deref(), Some("feature/x"));
        fs::write(g.join("HEAD"), "0123456789abcdef0123456789abcdef01234567\n").unwrap();
        assert_eq!(git_branch(&g).as_deref(), Some("0123456"));
    }

    #[test]
    fn worktree_gitdir_file() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("main/.git/worktrees/wt");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("HEAD"), "ref: refs/heads/wt-branch\n").unwrap();
        let wt = tmp.path().join("wt");
        fs::create_dir(&wt).unwrap();
        fs::write(wt.join(".git"), "gitdir: ../main/.git/worktrees/wt\n").unwrap();
        let d = detect(&wt);
        assert!(d.is_project && d.has_git);
        let g = git_dir(&wt).unwrap();
        assert_eq!(git_branch(&g).as_deref(), Some("wt-branch"));
    }

    #[test]
    fn makefile_alone_is_not_a_project() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("Makefile"), "").unwrap();
        assert!(!detect(tmp.path()).is_project);
        fs::write(tmp.path().join("App.csproj"), "").unwrap();
        let d = detect(tmp.path());
        assert!(d.is_project);
        assert_eq!(d.tags, vec!["dotnet"]);
    }
}
