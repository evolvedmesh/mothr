//! Skills: folders holding a `SKILL.md` (YAML frontmatter with a `name` and a `description`,
//! then instructions) plus any scripts or references it uses. Mothr keeps one library of them
//! and offers it to every agent: typed as `/name`, a skill is expanded into the prompt, and the
//! enabled ones are listed in each session's instructions so the agent can pick one itself.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// The file that makes a folder a skill.
pub const SKILL_FILE: &str = "SKILL.md";

/// A skill in Mothr's library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    /// What you type after `/`; also the library folder's name.
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Argument hint from the frontmatter (`argument-hint`), e.g. "<file>".
    #[serde(default)]
    pub hint: Option<String>,
    /// The skill's folder (holds `SKILL.md`).
    pub dir: PathBuf,
    /// Where it was installed from (None: put in the library folder by hand).
    #[serde(default)]
    pub source: Option<SkillSource>,
    /// Disabled skills stay installed but aren't offered to agents.
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub installed_at: Option<DateTime<Utc>>,
}

fn yes() -> bool {
    true
}

impl Skill {
    pub fn file(&self) -> PathBuf {
        self.dir.join(SKILL_FILE)
    }
}

/// Where skills come from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillSource {
    /// A folder on this computer: one skill, or a folder of skills.
    Folder { path: PathBuf },
    /// A GitHub repo. `path` narrows it to a folder in the repo; `skill` picks one skill by name.
    #[serde(rename = "github")]
    GitHub {
        repo: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        skill: Option<String>,
        /// Branch, tag or commit; None = the default branch.
        #[serde(default)]
        git_ref: Option<String>,
    },
    /// Any other git repository (cloned).
    Git {
        url: String,
        #[serde(default)]
        path: Option<String>,
    },
    /// A single `SKILL.md` on the web.
    Url { url: String },
}

impl SkillSource {
    pub fn github(repo: impl Into<String>) -> Self {
        SkillSource::GitHub { repo: repo.into(), path: None, skill: None, git_ref: None }
    }

    /// Understand what someone pasted: `owner/repo`, `owner/repo@skill`, `owner/repo/path`,
    /// GitHub or skills.sh links, a git URL, a link to a `SKILL.md`, or a folder path.
    pub fn parse(input: &str) -> Result<Self, String> {
        let s = input.trim().trim_end_matches('/');
        if s.is_empty() {
            return Err("Paste a GitHub repo, a git or SKILL.md link, or a folder".into());
        }
        if s.starts_with('~') || s.starts_with('/') || s.starts_with("./") || s.starts_with("..") || looks_like_windows_path(s) {
            return Ok(SkillSource::Folder { path: crate::config::expand_tilde(s) });
        }
        if let Some(rest) = s.strip_prefix("github:") {
            return parse_github_short(rest).ok_or_else(|| format!("“{input}” isn't a GitHub repo"));
        }
        let lower = s.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            let after = s.split_once("://").map(|(_, r)| r).unwrap_or(s);
            let (host, path) = after.split_once('/').unwrap_or((after, ""));
            let host = host.to_ascii_lowercase();
            let host = host.strip_prefix("www.").unwrap_or(&host);
            if host == "github.com" {
                return parse_github_url(path).ok_or_else(|| format!("“{input}” isn't a GitHub repo link"));
            }
            if host == "skills.sh" {
                // skills.sh/<owner>/<repo>/<skill>
                let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
                if parts.len() >= 2 {
                    return Ok(SkillSource::GitHub {
                        repo: format!("{}/{}", parts[0], parts[1]),
                        path: None,
                        skill: parts.get(2).map(|s| s.to_string()),
                        git_ref: None,
                    });
                }
                return Err(format!("“{input}” isn't a skill page"));
            }
            if lower.ends_with(".md") {
                return Ok(SkillSource::Url { url: s.to_string() });
            }
            return Ok(SkillSource::Git { url: s.to_string(), path: None });
        }
        if lower.starts_with("git@") || lower.starts_with("ssh://") || lower.ends_with(".git") {
            return Ok(SkillSource::Git { url: s.to_string(), path: None });
        }
        parse_github_short(s).ok_or_else(|| format!("Couldn't tell what “{input}” is. Try owner/repo, a link, or a folder path."))
    }

    /// A stable key, to tell installs apart while they run.
    pub fn key(&self) -> String {
        match self {
            SkillSource::Folder { path } => format!("folder:{}", path.display()),
            SkillSource::GitHub { repo, path, skill, git_ref } => format!(
                "github:{repo}{}{}{}",
                path.as_deref().map(|p| format!("/{p}")).unwrap_or_default(),
                git_ref.as_deref().map(|r| format!("#{r}")).unwrap_or_default(),
                skill.as_deref().map(|s| format!("@{s}")).unwrap_or_default()
            ),
            SkillSource::Git { url, path } => format!("git:{url}{}", path.as_deref().map(|p| format!("//{p}")).unwrap_or_default()),
            SkillSource::Url { url } => format!("url:{url}"),
        }
    }

    /// Where it's from, for display: "anthropics/skills", "~/skills/pdf", a link.
    pub fn label(&self) -> String {
        match self {
            SkillSource::Folder { path } => crate::config::tildify(path),
            SkillSource::GitHub { repo, .. } => repo.clone(),
            SkillSource::Git { url, .. } => url.trim_end_matches(".git").to_string(),
            SkillSource::Url { url } => url.split_once("://").map(|(_, r)| r).unwrap_or(url).to_string(),
        }
    }

    /// A web page for it, if there is one.
    pub fn web_url(&self) -> Option<String> {
        match self {
            SkillSource::GitHub { repo, path, git_ref, .. } => Some(match path {
                Some(p) if !p.is_empty() => format!("https://github.com/{repo}/tree/{}/{p}", git_ref.as_deref().unwrap_or("HEAD")),
                _ => format!("https://github.com/{repo}"),
            }),
            SkillSource::Git { url, .. } if url.starts_with("http") => Some(url.trim_end_matches(".git").to_string()),
            SkillSource::Url { url } => Some(url.clone()),
            _ => None,
        }
    }

    /// Fetching it needs the network.
    pub fn is_remote(&self) -> bool {
        !matches!(self, SkillSource::Folder { .. })
    }
}

fn looks_like_windows_path(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

fn valid_gh_segment(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && s != "." && s != ".."
}

/// `owner/repo`, `owner/repo@skill`, `owner/repo/some/path`.
fn parse_github_short(s: &str) -> Option<SkillSource> {
    let (main, skill) = match s.split_once('@') {
        Some((m, k)) if !k.is_empty() => (m, Some(k.to_string())),
        _ => (s, None),
    };
    let parts: Vec<&str> = main.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 || !valid_gh_segment(parts[0]) || !valid_gh_segment(parts[1]) {
        return None;
    }
    let repo = format!("{}/{}", parts[0], parts[1].trim_end_matches(".git"));
    let path = (parts.len() > 2).then(|| parts[2..].join("/"));
    Some(SkillSource::GitHub { repo, path, skill, git_ref: None })
}

/// The part of a github.com link after the host.
fn parse_github_url(path: &str) -> Option<SkillSource> {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 || !valid_gh_segment(parts[0]) {
        return None;
    }
    let repo = format!("{}/{}", parts[0], parts[1].trim_end_matches(".git"));
    // …/tree/<ref>/<path> or …/blob/<ref>/<path>/SKILL.md
    let (git_ref, sub) = match parts.get(2) {
        Some(&"tree") | Some(&"blob") if parts.len() >= 4 => {
            let mut rest: Vec<&str> = parts[4..].to_vec();
            if rest.last().is_some_and(|l| l.eq_ignore_ascii_case(SKILL_FILE)) {
                rest.pop();
            }
            (Some(parts[3].to_string()), (!rest.is_empty()).then(|| rest.join("/")))
        }
        _ => (None, None),
    };
    Some(SkillSource::GitHub { repo, path: sub, skill: None, git_ref })
}

/// A skill found while browsing or searching, not (necessarily) installed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillListing {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// How to install exactly this skill.
    pub source: SkillSource,
    /// Where it comes from, for display ("anthropics/skills", "~/.claude/skills").
    pub origin: String,
    /// Install count, when the index knows it.
    #[serde(default)]
    pub installs: Option<u64>,
}

/// The frontmatter fields of a `SKILL.md` and the instructions after it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkillDoc {
    pub fields: BTreeMap<String, String>,
    pub body: String,
}

impl SkillDoc {
    pub fn name(&self) -> Option<&str> {
        self.fields.get("name").map(String::as_str).filter(|s| !s.is_empty())
    }
    pub fn description(&self) -> Option<&str> {
        self.fields.get("description").map(String::as_str).filter(|s| !s.is_empty())
    }
    pub fn hint(&self) -> Option<&str> {
        self.fields.get("argument-hint").map(String::as_str).filter(|s| !s.is_empty())
    }
}

/// Split a `SKILL.md` into its frontmatter (the simple YAML subset skills use: `key: value`,
/// quoted values, and `>` / `|` blocks) and its body.
pub fn parse_skill_md(text: &str) -> SkillDoc {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    let first = lines.clone().next().map(str::trim_end);
    if first != Some("---") {
        return SkillDoc { fields: BTreeMap::new(), body: text.to_string() };
    }
    lines.next();
    let mut front: Vec<&str> = vec![];
    let mut closed = false;
    for l in lines.by_ref() {
        if l.trim_end() == "---" || l.trim_end() == "..." {
            closed = true;
            break;
        }
        front.push(l);
    }
    if !closed {
        return SkillDoc { fields: BTreeMap::new(), body: text.to_string() };
    }
    let body = lines.collect::<Vec<_>>().join("\n").trim_start_matches('\n').to_string();
    let mut fields = BTreeMap::new();
    let mut i = 0;
    while i < front.len() {
        let line = front[i];
        i += 1;
        if line.starts_with([' ', '\t']) || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let key = key.trim().to_string();
        let value = value.trim();
        // Indented lines that follow belong to this key.
        let mut block: Vec<&str> = vec![];
        while i < front.len() && (front[i].starts_with([' ', '\t']) || front[i].trim().is_empty()) {
            block.push(front[i].trim());
            i += 1;
        }
        while block.last().is_some_and(|l| l.is_empty()) {
            block.pop();
        }
        let v = if value.starts_with('|') {
            block.join("\n")
        } else if value.starts_with('>') || value.is_empty() {
            // A nested map (`metadata:`) isn't a value we use; don't glue its lines together.
            if value.is_empty() && block.iter().any(|l| l.contains(": ")) {
                String::new()
            } else {
                block.iter().filter(|l| !l.is_empty()).copied().collect::<Vec<_>>().join(" ")
            }
        } else {
            let mut v = unquote(value);
            // A plain scalar may wrap onto indented lines.
            if !block.is_empty() && !value.starts_with(['"', '\'']) {
                v = std::iter::once(v.as_str()).chain(block.iter().copied()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
            }
            v
        };
        fields.insert(key, v);
    }
    SkillDoc { fields, body }
}

fn unquote(v: &str) -> String {
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        return v[1..v.len() - 1].replace("\\\"", "\"").replace("\\n", "\n");
    }
    if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        return v[1..v.len() - 1].replace("''", "'");
    }
    v.to_string()
}

/// A name that works as a folder and after `/`: lowercase letters, digits, `-`, `_` and `.`.
pub fn skill_slug(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.') {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches(['-', '.']).to_string();
    out.chars().take(64).collect()
}

/// Read a skill folder into a [`Skill`] (source and state are filled in by the library).
pub fn read_skill_dir(dir: &Path) -> Option<Skill> {
    let text = std::fs::read_to_string(dir.join(SKILL_FILE)).ok()?;
    let doc = parse_skill_md(&text);
    let folder = dir.file_name()?.to_string_lossy().to_string();
    let name = skill_slug(&folder);
    if name.is_empty() {
        return None;
    }
    Some(Skill {
        name,
        description: doc.description().map(str::to_string).unwrap_or_else(|| first_line(&doc.body)),
        hint: doc.hint().map(str::to_string),
        dir: dir.to_path_buf(),
        source: None,
        enabled: true,
        installed_at: None,
    })
}

fn first_line(body: &str) -> String {
    body.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty()).unwrap_or_default().chars().take(200).collect()
}

/// What to send instead of `text` when it runs a library skill (`/name args`): the skill's
/// instructions with the arguments. None when `text` isn't a skill command, or the agent
/// has a command of that name itself (`native`), which then wins.
pub fn expand_prompt(text: &str, skills: &[Skill], native: &HashSet<String>) -> Option<String> {
    let rest = text.trim_start().strip_prefix('/')?;
    let (name, args) = match rest.find(char::is_whitespace) {
        Some(i) => (&rest[..i], rest[i..].trim()),
        None => (rest, ""),
    };
    if name.is_empty() || native.contains(name) {
        return None;
    }
    let skill = skills.iter().find(|s| s.enabled && s.name == name)?;
    let raw = std::fs::read_to_string(skill.file()).ok()?;
    let doc = parse_skill_md(&raw);
    let mut body = doc.body.trim().to_string();
    let substituted = body.contains("$ARGUMENTS");
    if substituted {
        body = body.replace("$ARGUMENTS", args);
    }
    let dir = skill.dir.display();
    let mut out = format!(
        "[Mothr] I'm running the skill /{name}. Follow its instructions below for this request. Its folder is {dir}; paths it mentions are relative to that folder.\n\n<skill name=\"{name}\">\n{body}\n</skill>"
    );
    if !args.is_empty() && !substituted {
        out.push_str("\n\n");
        out.push_str(args);
    }
    Some(out)
}

/// Skill descriptions longer than this are cut in the catalog.
const CATALOG_DESC: usize = 280;

/// Instructions listing the enabled skills, so an agent can use one when a request matches.
pub fn catalog_note(skills: &[Skill]) -> Option<String> {
    let enabled: Vec<&Skill> = skills.iter().filter(|s| s.enabled).collect();
    if enabled.is_empty() {
        return None;
    }
    let mut out = String::from(
        "[Mothr] Skills: the user installed these skills for you. Each is a folder with a SKILL.md holding instructions (and sometimes scripts or reference files). When a request matches a skill's description, read its SKILL.md before you start and follow it; paths in it are relative to its folder. The user may also run one by typing /<name>.\n",
    );
    for s in enabled {
        let mut desc: String = s.description.split_whitespace().collect::<Vec<_>>().join(" ");
        if desc.chars().count() > CATALOG_DESC {
            desc = desc.chars().take(CATALOG_DESC).collect::<String>() + "…";
        }
        out.push_str(&format!("- {}: {} ({})\n", s.name, desc, s.file().display()));
    }
    Some(out.trim_end().to_string())
}

/// Library skills as slash commands, for those `native` doesn't already offer.
pub fn skill_commands(skills: &[Skill], native: &HashSet<String>) -> Vec<crate::CommandInfo> {
    skills
        .iter()
        .filter(|s| s.enabled && !native.contains(&s.name))
        .map(|s| crate::CommandInfo {
            name: s.name.clone(),
            description: (!s.description.is_empty()).then(|| s.description.clone()),
            hint: s.hint.clone(),
            skill: true,
        })
        .collect()
}

/// An agent's own commands plus the library's skills (replacing skills merged in earlier).
pub fn merge_skill_commands(commands: &[crate::CommandInfo], skills: &[Skill]) -> Vec<crate::CommandInfo> {
    let mut out: Vec<crate::CommandInfo> = commands.iter().filter(|c| !c.skill).cloned().collect();
    let native: HashSet<String> = out.iter().map(|c| c.name.clone()).collect();
    out.extend(skill_commands(skills, &native));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_simple_and_quoted() {
        let d = parse_skill_md("---\nname: pdf\ndescription: \"Work with PDFs: read, merge\"\nlicense: MIT\n---\n# PDF\n\nDo things.");
        assert_eq!(d.name(), Some("pdf"));
        assert_eq!(d.description(), Some("Work with PDFs: read, merge"));
        assert_eq!(d.body, "# PDF\n\nDo things.");
    }

    #[test]
    fn frontmatter_blocks_and_nested_maps() {
        let d = parse_skill_md("---\nname: x\ndescription: >\n  Folded line one\n  and two.\nmetadata:\n  short-description: Nope\nargument-hint: <file>\n---\nBody");
        assert_eq!(d.description(), Some("Folded line one and two."));
        assert_eq!(d.fields.get("metadata").map(String::as_str), Some(""));
        assert_eq!(d.hint(), Some("<file>"));
        let d = parse_skill_md("---\nname: y\ndescription: |\n  a\n  b\n---\n");
        assert_eq!(d.description(), Some("a\nb"));
        let d = parse_skill_md("---\nname: z\ndescription: wraps over\n  two lines\n---\n");
        assert_eq!(d.description(), Some("wraps over two lines"));
    }

    #[test]
    fn no_frontmatter_is_all_body() {
        let d = parse_skill_md("# Just text");
        assert!(d.fields.is_empty());
        assert_eq!(d.body, "# Just text");
    }

    #[test]
    fn slugs() {
        assert_eq!(skill_slug("My Skill!"), "my-skill");
        assert_eq!(skill_slug("agent_builder.v2"), "agent_builder.v2");
        assert_eq!(skill_slug("--x--"), "x");
    }

    #[test]
    fn parses_sources() {
        assert_eq!(SkillSource::parse("anthropics/skills").unwrap(), SkillSource::github("anthropics/skills"));
        assert_eq!(
            SkillSource::parse("vercel-labs/agent-skills@react-best-practices").unwrap(),
            SkillSource::GitHub { repo: "vercel-labs/agent-skills".into(), path: None, skill: Some("react-best-practices".into()), git_ref: None }
        );
        assert_eq!(
            SkillSource::parse("https://github.com/anthropics/skills/tree/main/skills/pdf").unwrap(),
            SkillSource::GitHub { repo: "anthropics/skills".into(), path: Some("skills/pdf".into()), skill: None, git_ref: Some("main".into()) }
        );
        assert_eq!(
            SkillSource::parse("https://github.com/anthropics/skills/blob/main/skills/pdf/SKILL.md").unwrap(),
            SkillSource::GitHub { repo: "anthropics/skills".into(), path: Some("skills/pdf".into()), skill: None, git_ref: Some("main".into()) }
        );
        assert_eq!(SkillSource::parse("https://github.com/obra/superpowers.git").unwrap(), SkillSource::github("obra/superpowers"));
        assert_eq!(
            SkillSource::parse("https://skills.sh/anthropics/skills/pdf").unwrap(),
            SkillSource::GitHub { repo: "anthropics/skills".into(), path: None, skill: Some("pdf".into()), git_ref: None }
        );
        assert!(matches!(SkillSource::parse("https://example.com/x/SKILL.md").unwrap(), SkillSource::Url { .. }));
        assert!(matches!(SkillSource::parse("https://gitlab.com/a/b.git").unwrap(), SkillSource::Git { .. }));
        assert!(matches!(SkillSource::parse("git@gitlab.com:a/b.git").unwrap(), SkillSource::Git { .. }));
        assert!(matches!(SkillSource::parse("~/my-skills").unwrap(), SkillSource::Folder { .. }));
        assert!(SkillSource::parse("hello").is_err());
        assert!(SkillSource::parse("  ").is_err());
    }

    fn temp_skill(name: &str, md: &str) -> (PathBuf, Skill) {
        let root = std::env::temp_dir().join(format!("mothr-skill-test-{}-{name}", std::process::id()));
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SKILL_FILE), md).unwrap();
        let skill = read_skill_dir(&dir).unwrap();
        (root, skill)
    }

    #[test]
    fn expands_skill_commands() {
        let (root, skill) = temp_skill("review", "---\nname: review\ndescription: Review code\n---\nReview carefully.");
        let skills = vec![skill];
        let none = HashSet::new();
        let out = expand_prompt("/review src/lib.rs", &skills, &none).unwrap();
        assert!(out.contains("<skill name=\"review\">\nReview carefully.\n</skill>"));
        assert!(out.ends_with("src/lib.rs"));
        assert!(expand_prompt("/other", &skills, &none).is_none());
        assert!(expand_prompt("review", &skills, &none).is_none());
        let native: HashSet<String> = ["review".to_string()].into();
        assert!(expand_prompt("/review", &skills, &native).is_none());
        let mut off = skills.clone();
        off[0].enabled = false;
        assert!(expand_prompt("/review", &off, &none).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn substitutes_arguments() {
        let (root, skill) = temp_skill("fix", "---\nname: fix\ndescription: Fix\n---\nFix issue $ARGUMENTS now.");
        let out = expand_prompt("/fix #42", &[skill], &HashSet::new()).unwrap();
        assert!(out.contains("Fix issue #42 now."));
        assert!(!out.ends_with("#42"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn catalog_and_merge() {
        let s = |name: &str, enabled| Skill {
            name: name.into(),
            description: "Does a thing".into(),
            hint: None,
            dir: PathBuf::from(format!("/lib/{name}")),
            source: None,
            enabled,
            installed_at: None,
        };
        let skills = vec![s("a", true), s("b", false), s("compact", true)];
        let note = catalog_note(&skills).unwrap();
        assert!(note.contains("- a: Does a thing"));
        assert!(!note.contains("- b:"));
        assert!(catalog_note(&[s("b", false)]).is_none());

        let native = vec![crate::CommandInfo { name: "compact".into(), ..Default::default() }];
        let merged = merge_skill_commands(&native, &skills);
        let names: Vec<_> = merged.iter().map(|c| (c.name.as_str(), c.skill)).collect();
        assert_eq!(names, vec![("compact", false), ("a", true)]);
        // Merging again replaces the earlier skill entries instead of doubling them.
        assert_eq!(merge_skill_commands(&merged, &skills).len(), 2);
    }
}
