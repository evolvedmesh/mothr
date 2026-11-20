//! Reading a project's files for the Projects page: directory listings, file contents and a
//! fuzzy file finder.
//!
//! Every path comes in relative to a project root and is resolved by [`resolve`], which refuses
//! anything that would leave the project (`..`, absolute paths, symlinks pointing outside).
//! Listings honour `.gitignore` and skip `.git`, so build output and dependencies stay out of
//! the way.

use anyhow::{Context as _, bail};
use ignore::WalkBuilder;
use mothr_core::{
    DirListing, FileContent, FileEntry, FileHit, MAX_DIR_ENTRIES, MAX_FILE_BYTES, file_name, is_readme,
    language_for,
};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

/// The finder looks at no more than this many files.
const MAX_FINDER_FILES: usize = 60_000;
/// Bytes inspected to tell text from binary.
const SNIFF_BYTES: usize = 8 * 1024;

/// Resolve `rel` (relative, `/`-separated) inside `root`, refusing paths that leave it.
pub fn resolve(root: &Path, rel: &str) -> anyhow::Result<PathBuf> {
    let root = root
        .canonicalize()
        .with_context(|| format!("{} isn't readable", root.display()))?;
    let mut path = root.clone();
    for part in rel.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        let mut comps = Path::new(part).components();
        match (comps.next(), comps.next()) {
            (Some(Component::Normal(c)), None) => path.push(c),
            _ => bail!("“{rel}” isn't a path inside the project"),
        }
    }
    let real = path.canonicalize().with_context(|| format!("“{rel}” doesn't exist"))?;
    if !real.starts_with(&root) {
        bail!("“{rel}” points outside the project");
    }
    Ok(real)
}

/// `path` relative to `root`, `/`-separated.
fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn walker(dir: &Path) -> WalkBuilder {
    let mut b = WalkBuilder::new(dir);
    b.hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .parents(true)
        .ignore(true)
        .follow_links(false)
        .filter_entry(|e| e.file_name() != ".git");
    b
}

/// List one directory of a project (`dir` = `""` for the root).
pub fn list_dir(root: &Path, dir: &str) -> anyhow::Result<DirListing> {
    let root_real = root.canonicalize().with_context(|| format!("{} isn't readable", root.display()))?;
    let path = resolve(&root_real, dir)?;
    if !path.is_dir() {
        bail!("“{dir}” isn't a folder");
    }
    let mut entries = Vec::new();
    let mut truncated = false;
    for item in walker(&path).max_depth(Some(1)).build() {
        let Ok(item) = item else { continue };
        if item.depth() == 0 {
            continue;
        }
        if entries.len() >= MAX_DIR_ENTRIES {
            truncated = true;
            break;
        }
        // Follow symlinks for the kind, so a linked folder opens like a folder.
        let meta = std::fs::metadata(item.path()).ok();
        let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
        entries.push(FileEntry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: relative(&root_real, item.path()),
            dir: is_dir,
            size: if is_dir { 0 } else { meta.map_or(0, |m| m.len()) },
        });
    }
    sort_entries(&mut entries);
    let readme = pick_readme(&entries);
    Ok(DirListing { path: relative(&root_real, &path), entries, truncated, readme })
}

/// Folders first, then files; each case-insensitively by name.
fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// The README to show for a listing: Markdown first, then any other README file.
fn pick_readme(entries: &[FileEntry]) -> Option<String> {
    let readmes = || entries.iter().filter(|e| !e.dir && is_readme(&e.name));
    readmes()
        .find(|e| language_for(&e.name) == "markdown")
        .or_else(|| readmes().next())
        .map(|e| e.path.clone())
}

/// Read one file of a project, at most [`MAX_FILE_BYTES`] of it.
pub fn read_file(root: &Path, file: &str) -> anyhow::Result<FileContent> {
    let root_real = root.canonicalize().with_context(|| format!("{} isn't readable", root.display()))?;
    let path = resolve(&root_real, file)?;
    if path.is_dir() {
        bail!("“{file}” is a folder");
    }
    let mut f = std::fs::File::open(&path).with_context(|| format!("can't open “{file}”"))?;
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut bytes = Vec::with_capacity(size.min(MAX_FILE_BYTES) as usize);
    f.by_ref().take(MAX_FILE_BYTES).read_to_end(&mut bytes).with_context(|| format!("can't read “{file}”"))?;
    let truncated = size > MAX_FILE_BYTES;
    let rel = relative(&root_real, &path);
    let language = language_for(file_name(&rel)).to_string();
    Ok(FileContent { text: decode_text(bytes, truncated), path: rel, size, language, truncated })
}

/// The bytes as text, or `None` when they look binary. A cut at the size limit may split a
/// character; that tail is dropped.
fn decode_text(bytes: Vec<u8>, truncated: bool) -> Option<String> {
    if bytes[..bytes.len().min(SNIFF_BYTES)].contains(&0) {
        return None;
    }
    match String::from_utf8(bytes) {
        Ok(s) => Some(s),
        Err(e) => {
            let err = e.utf8_error();
            let mut bytes = e.into_bytes();
            if truncated && err.error_len().is_none() {
                bytes.truncate(err.valid_up_to());
                String::from_utf8(bytes).ok()
            } else if err.valid_up_to() >= bytes.len().min(SNIFF_BYTES) / 2 {
                // Mostly text with a few stray bytes (Latin-1 comments and the like).
                Some(String::from_utf8_lossy(&bytes).into_owned())
            } else {
                None
            }
        }
    }
}

/// Fuzzy-find files in a project by path. Returns the best `limit` matches, with the matched
/// char indices into each path for highlighting. An empty query returns nothing.
pub fn find_files(root: &Path, query: &str, limit: usize) -> anyhow::Result<Vec<FileHit>> {
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    if pattern.atoms.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let root_real = root.canonicalize().with_context(|| format!("{} isn't readable", root.display()))?;
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let name_lower = query.trim().to_lowercase();
    let mut scored: Vec<(u32, String)> = Vec::new();
    for (seen, item) in walker(&root_real).build().enumerate() {
        if seen >= MAX_FINDER_FILES {
            break;
        }
        let Ok(item) = item else { continue };
        if !item.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let rel = relative(&root_real, item.path());
        let hay = Utf32String::from(rel.as_str());
        let Some(mut score) = pattern.score(hay.slice(..), &mut matcher) else { continue };
        // Prefer hits in the file name over hits spread across folders.
        let name = file_name(&rel).to_lowercase();
        if name == name_lower {
            score += 200;
        } else if name.starts_with(&name_lower) {
            score += 80;
        } else if name.contains(&name_lower) {
            score += 40;
        }
        scored.push((score, rel));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())).then(a.1.cmp(&b.1)));
    scored.truncate(limit);
    let mut buf = Vec::new();
    Ok(scored
        .into_iter()
        .map(|(_, path)| {
            let hay = Utf32String::from(path.as_str());
            buf.clear();
            for atom in &pattern.atoms {
                let len = buf.len();
                if atom.indices(hay.slice(..), &mut matcher, &mut buf).is_none() {
                    buf.truncate(len);
                }
            }
            buf.sort_unstable();
            buf.dedup();
            FileHit { indices: buf.clone(), path }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/ui")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(root.join("README.md"), "# Hello\n").unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("src/ui/sidebar.rs"), "// sidebar\n").unwrap();
        std::fs::write(root.join("target/debug/out"), "x").unwrap();
        std::fs::write(root.join("logo.png"), [0x89, b'P', b'N', b'G', 0, 0, 1]).unwrap();
        dir
    }

    #[test]
    fn lists_folders_first_and_hides_ignored() {
        let p = project();
        let l = list_dir(p.path(), "").unwrap();
        let names: Vec<&str> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src", ".gitignore", "Cargo.toml", "logo.png", "README.md"]);
        assert_eq!(l.readme.as_deref(), Some("README.md"));
        let sub = list_dir(p.path(), "src").unwrap();
        assert_eq!(sub.path, "src");
        assert_eq!(sub.entries[0].path, "src/ui");
        assert!(sub.entries[0].dir);
    }

    #[test]
    fn reads_text_and_flags_binary() {
        let p = project();
        let f = read_file(p.path(), "src/main.rs").unwrap();
        assert_eq!(f.text.as_deref(), Some("fn main() {}\n"));
        assert_eq!(f.language, "rust");
        assert_eq!(f.name(), "main.rs");
        assert!(read_file(p.path(), "logo.png").unwrap().text.is_none());
    }

    #[test]
    fn refuses_paths_outside_the_project() {
        let p = project();
        assert!(read_file(p.path(), "../etc/passwd").is_err());
        assert!(read_file(p.path(), "/etc/passwd").is_err());
        assert!(list_dir(p.path(), "src/../..").is_err());
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("secret"), "s").unwrap();
            std::os::unix::fs::symlink(outside.path().join("secret"), p.path().join("link")).unwrap();
            assert!(read_file(p.path(), "link").is_err());
        }
    }

    #[test]
    fn finds_files_by_fuzzy_path() {
        let p = project();
        let hits = find_files(p.path(), "sidebar", 10).unwrap();
        assert_eq!(hits[0].path, "src/ui/sidebar.rs");
        assert!(!hits[0].indices.is_empty());
        assert!(find_files(p.path(), "out", 10).unwrap().iter().all(|h| !h.path.starts_with("target")));
        assert!(find_files(p.path(), "", 10).unwrap().is_empty());
    }

    #[test]
    fn truncated_text_drops_a_split_character() {
        let mut bytes = "héllo".as_bytes().to_vec();
        bytes.truncate(2); // "h" + half of "é"
        assert_eq!(decode_text(bytes, true).as_deref(), Some("h"));
    }
}
