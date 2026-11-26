//! Mothr's skill library.
//!
//! Skills live in [`mothr_core::skills_dir`], one folder each (named after the skill, holding its
//! `SKILL.md`). `skills.json` next to it remembers where each came from and whether it's
//! enabled. Skills are installed from GitHub (tree API + raw files, no git needed), any git
//! repository (cloned), a link to a `SKILL.md`, or a folder; [`discover`] finds new ones.

pub mod discover;
mod github;
mod install;

pub use discover::{POPULAR, SkillCatalog};
pub use install::Installed;

use mothr_core::{Skill, SkillSource, data_dir, read_skill_dir, skills_dir, write_atomic};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What `skills.json` remembers about a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    #[serde(default)]
    source: Option<SkillSource>,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    installed_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    #[serde(default)]
    skills: BTreeMap<String, Entry>,
}

fn manifest_path(root: &Path) -> PathBuf {
    // `<data>/skills.json` for the real library; beside the folder for any other root (tests).
    if root == skills_dir() { data_dir().join("skills.json") } else { root.with_extension("json") }
}

impl Manifest {
    fn load(root: &Path) -> Self {
        std::fs::read(manifest_path(root)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self, root: &Path) -> anyhow::Result<()> {
        write_atomic(&manifest_path(root), &serde_json::to_vec_pretty(self)?)
    }
}

/// The library at `root` (normally [`skills_dir`]).
#[derive(Clone)]
pub struct Library {
    root: PathBuf,
    client: Arc<github::Client>,
}

impl Default for Library {
    fn default() -> Self {
        Self::new(skills_dir())
    }
}

impl Library {
    pub fn new(root: PathBuf) -> Self {
        Self { root, client: Arc::new(github::Client::new()) }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every skill in the library, by name.
    pub fn load(&self) -> Vec<Skill> {
        let manifest = Manifest::load(&self.root);
        let Ok(rd) = std::fs::read_dir(&self.root) else { return vec![] };
        let mut out: Vec<Skill> = rd
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter(|e| e.path().is_dir())
            .filter_map(|e| read_skill_dir(&e.path()))
            .map(|mut s| {
                if let Some(m) = manifest.skills.get(&s.name) {
                    s.source = m.source.clone();
                    s.enabled = m.enabled;
                    s.installed_at = m.installed_at;
                }
                s
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn set_enabled(&self, name: &str, enabled: bool) -> anyhow::Result<()> {
        anyhow::ensure!(self.root.join(name).join(mothr_core::SKILL_FILE).is_file(), "No skill named {name}");
        let mut m = Manifest::load(&self.root);
        m.skills.entry(name.to_string()).or_insert(Entry { source: None, enabled: true, installed_at: None }).enabled = enabled;
        m.save(&self.root)
    }

    pub fn remove(&self, name: &str) -> anyhow::Result<()> {
        anyhow::ensure!(!name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != "..", "No skill named {name}");
        let dir = self.root.join(name);
        anyhow::ensure!(dir.is_dir(), "No skill named {name}");
        std::fs::remove_dir_all(&dir)?;
        let mut m = Manifest::load(&self.root);
        if m.skills.remove(name).is_some() {
            m.save(&self.root)?;
        }
        Ok(())
    }

    /// Fetch `source` and add every skill it holds (or the one it names), replacing skills of
    /// the same name. Returns the names added.
    pub async fn install(&self, source: &SkillSource) -> anyhow::Result<Installed> {
        install::install(self, source).await
    }

    /// Record a freshly installed skill (keeping whether it was enabled).
    fn record(&self, name: &str, source: SkillSource) -> anyhow::Result<()> {
        let mut m = Manifest::load(&self.root);
        let enabled = m.skills.get(name).map(|e| e.enabled).unwrap_or(true);
        m.skills.insert(name.to_string(), Entry { source: Some(source), enabled, installed_at: Some(chrono::Utc::now()) });
        m.save(&self.root)
    }

    /// Search the open skills index (skills.sh).
    pub async fn search(&self, query: &str) -> anyhow::Result<Vec<mothr_core::SkillListing>> {
        discover::search(&self.client, query).await
    }

    /// Every skill in a GitHub repo.
    pub async fn browse(&self, repo: &str) -> anyhow::Result<Vec<mothr_core::SkillListing>> {
        discover::browse(&self.client, repo).await
    }
}

/// Skills other agents keep on this computer (`~/.claude/skills`, `~/.codex/skills` …).
pub fn local_agent_skills() -> Vec<mothr_core::SkillListing> {
    discover::local_agent_skills()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(dir: &Path, name: &str, desc: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), format!("---\nname: {name}\ndescription: {desc}\n---\nDo it.")).unwrap();
    }

    #[tokio::test]
    async fn installs_from_folders_and_manages_them() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::new(tmp.path().join("lib"));
        let src = tmp.path().join("src");
        write_skill(&src.join("one"), "one", "First");
        write_skill(&src.join("nested/two"), "Two Words", "Second");
        std::fs::write(src.join("one/helper.py"), "print(1)").unwrap();

        let got = lib.install(&SkillSource::Folder { path: src.clone() }).await.unwrap();
        assert_eq!(got.names, vec!["one".to_string(), "two-words".to_string()]);
        let skills = lib.load();
        assert_eq!(skills.len(), 2);
        assert!(skills[0].dir.join("helper.py").is_file());
        assert!(matches!(&skills[0].source, Some(SkillSource::Folder { path }) if path.ends_with("one")));
        assert!(skills.iter().all(|s| s.enabled));

        lib.set_enabled("one", false).unwrap();
        assert!(!lib.load()[0].enabled);
        // Reinstalling keeps it disabled.
        lib.install(&SkillSource::Folder { path: src.join("one") }).await.unwrap();
        assert!(!lib.load()[0].enabled);

        lib.remove("one").unwrap();
        assert_eq!(lib.load().len(), 1);
        assert!(lib.remove("../x").is_err());
    }

    #[tokio::test]
    async fn empty_folder_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::new(tmp.path().join("lib"));
        std::fs::create_dir_all(tmp.path().join("empty")).unwrap();
        assert!(lib.install(&SkillSource::Folder { path: tmp.path().join("empty") }).await.is_err());
    }
}
