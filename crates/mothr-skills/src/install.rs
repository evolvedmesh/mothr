//! Fetching skills into the library.

use super::Library;
use super::github::Blob;
use mothr_core::{SKILL_FILE, SkillSource, parse_skill_md, skill_slug};
use std::path::{Component, Path, PathBuf};

/// A skill folder may hold at most this many files, and this many bytes.
const MAX_FILES: usize = 400;
const MAX_BYTES: u64 = 50 * 1024 * 1024;
/// Installing a whole source stops at this many skills (pick one instead).
const MAX_SKILLS: usize = 60;
/// How deep to look for skills inside a folder or cloned repo.
const SCAN_DEPTH: usize = 5;

/// What an install added.
#[derive(Debug, Clone, Default)]
pub struct Installed {
    /// Every skill added, by name.
    pub names: Vec<String>,
    /// Those that replaced a skill of the same name.
    pub replaced: Vec<String>,
}

impl Installed {
    /// "Added pdf", "Added 3 skills from anthropics/skills", "Updated pdf".
    pub fn summary(&self, source: &SkillSource) -> String {
        match self.names.as_slice() {
            [one] if self.replaced.len() == 1 => format!("Updated /{one}"),
            [one] => format!("Added /{one}"),
            many => format!("Added {} skills from {}", many.len(), source.label()),
        }
    }
}

/// A skill ready to be copied in: its folder and the source that fetches exactly it.
struct Found {
    dir: PathBuf,
    source: SkillSource,
}

pub async fn install(lib: &Library, source: &SkillSource) -> anyhow::Result<Installed> {
    std::fs::create_dir_all(&lib.root)?;
    let stage = lib.root.join(format!(".staging-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&stage)?;
    let result = async {
        let found = fetch(lib, source, &stage).await?;
        anyhow::ensure!(!found.is_empty(), "No skills there: a skill is a folder with a {SKILL_FILE}");
        anyhow::ensure!(
            found.len() <= MAX_SKILLS,
            "{} holds {} skills. Pick the ones you want on the Discover page, or add owner/repo@skill",
            source.label(),
            found.len()
        );
        let mut out = Installed::default();
        for f in found {
            let name = skill_name(&f.dir)?;
            if out.names.contains(&name) {
                continue;
            }
            let target = lib.root.join(&name);
            let existed = target.exists();
            let tmp = lib.root.join(format!(".incoming-{name}-{}", uuid::Uuid::new_v4().simple()));
            copy_dir(&f.dir, &tmp)?;
            if existed {
                std::fs::remove_dir_all(&target)?;
                out.replaced.push(name.clone());
            }
            std::fs::rename(&tmp, &target)?;
            lib.record(&name, f.source)?;
            out.names.push(name);
        }
        out.names.sort();
        Ok(out)
    }
    .await;
    let _ = std::fs::remove_dir_all(&stage);
    result
}

/// The skill's name: its frontmatter `name`, else its folder's.
fn skill_name(dir: &Path) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(dir.join(SKILL_FILE))?;
    let doc = parse_skill_md(&text);
    let raw = doc.name().map(str::to_string).or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default();
    let name = skill_slug(&raw);
    anyhow::ensure!(!name.is_empty(), "A skill in {} has no usable name", dir.display());
    Ok(name)
}

async fn fetch(lib: &Library, source: &SkillSource, stage: &Path) -> anyhow::Result<Vec<Found>> {
    match source {
        SkillSource::Folder { path } => {
            anyhow::ensure!(path.is_dir(), "{} isn't a folder", path.display());
            Ok(scan(path).into_iter().map(|dir| Found { source: SkillSource::Folder { path: dir.clone() }, dir }).collect())
        }
        SkillSource::Url { url } => {
            let bytes = lib.client.get_bytes(url).await?;
            let text = String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{url} isn't a text file"))?;
            let doc = parse_skill_md(&text);
            let name = doc
                .name()
                .map(skill_slug)
                .or_else(|| url.trim_end_matches('/').rsplit('/').nth(1).map(skill_slug))
                .filter(|n| !n.is_empty())
                .ok_or_else(|| anyhow::anyhow!("{url} doesn't look like a SKILL.md (no name in its frontmatter)"))?;
            anyhow::ensure!(doc.description().is_some(), "{url} doesn't look like a SKILL.md (no description in its frontmatter)");
            let dir = stage.join(&name);
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join(SKILL_FILE), text)?;
            Ok(vec![Found { dir, source: source.clone() }])
        }
        SkillSource::Git { url, path } => {
            let checkout = stage.join("repo");
            let mut git = tokio::process::Command::new("git");
            mothr_core::process::no_window(git.as_std_mut());
            let out = git
                .args(["clone", "--depth", "1", "--quiet", "--", url])
                .arg(&checkout)
                .env("GIT_TERMINAL_PROMPT", "0")
                .stdin(std::process::Stdio::null())
                .output()
                .await
                .map_err(|e| anyhow::anyhow!("Couldn't run git ({e}); is it installed?"))?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                anyhow::bail!("Couldn't clone {url}: {}", if err.is_empty() { "git failed".into() } else { err });
            }
            let base = match path {
                Some(p) => checkout.join(safe_rel(p).ok_or_else(|| anyhow::anyhow!("Bad path {p}"))?),
                None => checkout.clone(),
            };
            Ok(scan(&base)
                .into_iter()
                .map(|dir| {
                    let rel = dir.strip_prefix(&checkout).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                    Found { source: SkillSource::Git { url: url.clone(), path: (!rel.is_empty()).then_some(rel) }, dir }
                })
                .collect())
        }
        SkillSource::GitHub { repo, path, skill, git_ref } => {
            let tree = lib.client.tree(repo, git_ref.as_deref()).await?;
            let prefix = path.as_deref().map(|p| p.trim_matches('/')).filter(|p| !p.is_empty());
            let mut dirs = skill_dirs(&tree, prefix);
            if let Some(want) = skill {
                dirs = vec![pick_skill(lib, repo, git_ref.as_deref(), &dirs, want).await?];
            }
            if dirs.len() > MAX_SKILLS {
                // Don't download hundreds of skills only to refuse them.
                return Ok(dirs.into_iter().map(|d| Found { dir: PathBuf::from(d), source: source.clone() }).collect());
            }
            let mut found = vec![];
            for (i, d) in dirs.iter().enumerate() {
                let local = stage.join(format!("{i}")).join(d.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or("skill"));
                download_dir(lib, repo, git_ref.as_deref(), &tree, d, &local).await?;
                found.push(Found {
                    dir: local,
                    source: SkillSource::GitHub { repo: repo.clone(), path: (!d.is_empty()).then(|| d.clone()), skill: None, git_ref: git_ref.clone() },
                });
            }
            Ok(found)
        }
    }
}

/// Folders (repo-relative, "" for the root) holding a `SKILL.md`, under `prefix`.
pub(crate) fn skill_dirs(tree: &[Blob], prefix: Option<&str>) -> Vec<String> {
    let mut dirs: Vec<String> = tree
        .iter()
        .filter_map(|b| {
            let p = b.path.as_str();
            if p == SKILL_FILE {
                Some(String::new())
            } else {
                p.strip_suffix(&format!("/{SKILL_FILE}")).map(str::to_string)
            }
        })
        .filter(|d| match prefix {
            Some(pre) => d == pre || d.starts_with(&format!("{pre}/")),
            None => true,
        })
        // A template isn't a skill to install.
        .filter(|d| !d.rsplit('/').next().is_some_and(|last| last == "template" || last == "skill-template"))
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

/// The folder of the skill called `want`: by folder name first, else by the name in its
/// frontmatter (the index may know a skill by a name its folder doesn't have).
async fn pick_skill(lib: &Library, repo: &str, git_ref: Option<&str>, dirs: &[String], want: &str) -> anyhow::Result<String> {
    let want_slug = skill_slug(want);
    let base = |d: &str| d.rsplit('/').next().unwrap_or(d).to_string();
    if let Some(d) = dirs.iter().find(|d| base(d) == want || skill_slug(&base(d)) == want_slug) {
        return Ok(d.clone());
    }
    // Likely candidates first: folders whose name is part of the wanted one.
    let mut order: Vec<&String> = dirs.iter().collect();
    order.sort_by_key(|d| !want_slug.contains(&skill_slug(&base(d))));
    for d in order.into_iter().take(80) {
        let file = if d.is_empty() { SKILL_FILE.to_string() } else { format!("{d}/{SKILL_FILE}") };
        if let Ok(bytes) = lib.client.raw(repo, git_ref, &file).await {
            let doc = parse_skill_md(&String::from_utf8_lossy(&bytes));
            if doc.name().is_some_and(|n| n == want || skill_slug(n) == want_slug) {
                return Ok(d.clone());
            }
        }
    }
    anyhow::bail!("{repo} has no skill called {want}")
}

/// Download every file under `dir` (repo-relative) into `into`.
async fn download_dir(lib: &Library, repo: &str, git_ref: Option<&str>, tree: &[Blob], dir: &str, into: &Path) -> anyhow::Result<()> {
    let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
    let files: Vec<&Blob> = tree.iter().filter(|b| b.path.starts_with(&prefix)).collect();
    // A skill at a repo's root shouldn't drag in the whole repo: only its own top-level files
    // and the folders skills conventionally use.
    let files: Vec<&Blob> = if dir.is_empty() {
        files
            .into_iter()
            .filter(|b| !b.path.contains('/') || ["scripts/", "references/", "assets/", "templates/", "examples/"].iter().any(|p| b.path.starts_with(p)))
            .collect()
    } else {
        files
    };
    anyhow::ensure!(files.len() <= MAX_FILES, "{dir} has too many files ({}) to be a skill", files.len());
    let total: u64 = files.iter().map(|b| b.size).sum();
    anyhow::ensure!(total <= MAX_BYTES, "{dir} is too big ({} MB) to be a skill", total / (1024 * 1024));
    use futures::StreamExt as _;
    let cut = prefix.len();
    let jobs: Vec<_> = files
        .into_iter()
        .map(|b| {
            let (client, repo, git_ref, path) = (lib.client.clone(), repo.to_string(), git_ref.map(str::to_string), b.path.clone());
            async move {
                let rel = safe_rel(&path[cut..]).ok_or_else(|| anyhow::anyhow!("Bad path {path}"))?;
                let bytes = client.raw(&repo, git_ref.as_deref(), &path).await?;
                anyhow::Ok((rel, bytes))
            }
        })
        .collect();
    let mut stream = futures::stream::iter(jobs).buffer_unordered(8);
    std::fs::create_dir_all(into)?;
    while let Some(r) = stream.next().await {
        let (rel, bytes) = r?;
        let target = into.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, bytes)?;
    }
    anyhow::ensure!(into.join(SKILL_FILE).is_file(), "{dir} has no {SKILL_FILE}");
    Ok(())
}

/// A relative path that stays inside its folder.
fn safe_rel(p: &str) -> Option<PathBuf> {
    let path = Path::new(p);
    let ok = !p.is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)));
    ok.then(|| path.to_path_buf())
}

/// Skill folders in `root`: itself if it is one, else those found below it.
pub(crate) fn scan(root: &Path) -> Vec<PathBuf> {
    if root.join(SKILL_FILE).is_file() {
        return vec![root.to_path_buf()];
    }
    let mut out = vec![];
    walk(root, 0, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth >= SCAN_DEPTH {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" || name == "node_modules" || name == "target" || name.starts_with(".staging") {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let p = e.path();
        if p.join(SKILL_FILE).is_file() {
            out.push(p);
        } else {
            walk(&p, depth + 1, out);
        }
    }
}

/// Copy a skill folder (without `.git`), within the size limits.
fn copy_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    let mut files = 0usize;
    let mut bytes = 0u64;
    copy_rec(from, to, &mut files, &mut bytes).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(to);
    })
}

fn copy_rec(from: &Path, to: &Path, files: &mut usize, bytes: &mut u64) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let name = e.file_name();
        if name == ".git" || name == "node_modules" {
            continue;
        }
        let ft = e.file_type()?;
        let src = e.path();
        let dst = to.join(&name);
        if ft.is_dir() {
            copy_rec(&src, &dst, files, bytes)?;
        } else if ft.is_file() || (ft.is_symlink() && src.is_file()) {
            *files += 1;
            *bytes += std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
            anyhow::ensure!(*files <= MAX_FILES && *bytes <= MAX_BYTES, "{} is too big to be a skill", from.display());
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blobs(paths: &[&str]) -> Vec<Blob> {
        paths.iter().map(|p| Blob { path: p.to_string(), size: 1 }).collect()
    }

    #[test]
    fn finds_skill_dirs_in_a_tree() {
        let t = blobs(&["README.md", "skills/pdf/SKILL.md", "skills/pdf/x.py", "skills/docx/SKILL.md", "template/SKILL.md", "other/SKILL.md"]);
        assert_eq!(skill_dirs(&t, None), vec!["other", "skills/docx", "skills/pdf"]);
        assert_eq!(skill_dirs(&t, Some("skills")), vec!["skills/docx", "skills/pdf"]);
        assert_eq!(skill_dirs(&t, Some("skills/pdf")), vec!["skills/pdf"]);
        assert_eq!(skill_dirs(&blobs(&["SKILL.md"]), None), vec![""]);
    }

    #[test]
    fn rejects_escaping_paths() {
        assert!(safe_rel("a/b.md").is_some());
        assert!(safe_rel("../x").is_none());
        assert!(safe_rel("/etc/passwd").is_none());
        assert!(safe_rel("").is_none());
    }
}
