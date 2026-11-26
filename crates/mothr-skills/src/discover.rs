//! Finding skills: the open skills index (skills.sh), well-known skill repos, and skills other
//! agents keep on this computer.

use super::github::Client;
use std::sync::Arc;
use super::install::{scan, skill_dirs};
use mothr_core::{SKILL_FILE, SkillListing, SkillSource, data_dir, home_dir, parse_skill_md, read_skill_dir, tildify, write_atomic};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// A well-known collection of skills.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkillCatalog {
    /// "owner/name" on GitHub.
    pub repo: &'static str,
    pub title: &'static str,
    pub blurb: &'static str,
}

/// Popular skill repos, offered on the Discover page.
pub const POPULAR: &[SkillCatalog] = &[
    SkillCatalog { repo: "anthropics/skills", title: "Anthropic", blurb: "Official skills: PDFs, Word, Excel, slides, design, MCP servers" },
    SkillCatalog { repo: "vercel-labs/agent-skills", title: "Vercel", blurb: "React, Next.js and web design best practices" },
    SkillCatalog { repo: "obra/superpowers", title: "Superpowers", blurb: "Planning, TDD, debugging and review workflows" },
    SkillCatalog { repo: "openai/skills", title: "OpenAI", blurb: "Curated skills from the Codex team" },
    SkillCatalog { repo: "huggingface/skills", title: "Hugging Face", blurb: "Models, datasets, training and the Hub" },
    SkillCatalog { repo: "microsoft/skills", title: "Microsoft", blurb: "Azure, AKS and Microsoft developer tools" },
    SkillCatalog { repo: "github/awesome-copilot", title: "Awesome Copilot", blurb: "GitHub's community collection" },
    SkillCatalog { repo: "ComposioHQ/awesome-claude-skills", title: "Awesome Claude Skills", blurb: "A large community list, including app automations" },
];

/// The open skills index's search.
const SEARCH_URL: &str = "https://skills.sh/api/search";
/// Search results whose description is looked up.
const DESCRIBE_TOP: usize = 12;
/// Skills of a repo whose description is fetched when browsing it (the rest show by name).
const DESCRIBE_BROWSE: usize = 60;
/// A browsed repo is fetched again after this long.
const BROWSE_TTL: chrono::TimeDelta = chrono::TimeDelta::hours(12);

#[derive(Deserialize)]
struct SearchReply {
    #[serde(default)]
    skills: Vec<SearchHit>,
}

#[derive(Deserialize)]
struct SearchHit {
    #[serde(default)]
    source: String,
    #[serde(default, rename = "skillId")]
    skill_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    installs: Option<u64>,
    #[serde(default, rename = "isDuplicate")]
    is_duplicate: bool,
}

fn is_github_repo(s: &str) -> bool {
    let valid = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    match s.split_once('/') {
        // Owners can't have dots; index entries hosted on a website ("docs.example.com") do.
        Some((owner, repo)) => valid(owner) && !owner.contains('.') && valid(repo) && !repo.contains('/'),
        None => false,
    }
}

/// Search skills.sh. Results that only live outside GitHub are left out (they can't be
/// installed from here).
pub async fn search(client: &Arc<Client>, query: &str) -> anyhow::Result<Vec<SkillListing>> {
    let q = query.trim();
    if q.chars().count() < 2 {
        return Ok(vec![]);
    }
    let res = client
        .http
        .get(SEARCH_URL)
        .query(&[("q", q), ("limit", "40")])
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Couldn't reach skills.sh: {e}"))?;
    let reply: SearchReply = res.error_for_status().map_err(|e| anyhow::anyhow!("skills.sh: {e}"))?.json().await?;
    let mut out: Vec<SkillListing> = reply
        .skills
        .into_iter()
        .filter(|h| !h.is_duplicate && is_github_repo(&h.source) && !h.skill_id.is_empty())
        .map(|h| SkillListing {
            name: if h.name.is_empty() { h.skill_id.clone() } else { h.name },
            description: None,
            source: SkillSource::GitHub { repo: h.source.clone(), path: None, skill: Some(h.skill_id), git_ref: None },
            origin: h.source,
            installs: h.installs,
        })
        .collect();
    describe(client, &mut out).await;
    Ok(out)
}

/// Fill in descriptions for the first results: from repos browsed before, else by trying the
/// usual places a skill sits in its repo (no API calls, so no rate limit).
async fn describe(client: &Arc<Client>, list: &mut [SkillListing]) {
    use futures::StreamExt as _;
    for l in list.iter_mut() {
        if let SkillSource::GitHub { repo, skill: Some(skill), .. } = &l.source
            && let Some(cached) = read_cache(repo)
            && let Some(hit) = cached.skills.iter().find(|c| &c.name == skill || matches!(&c.source, SkillSource::GitHub { path: Some(p), .. } if p.rsplit('/').next() == Some(skill.as_str())))
        {
            l.description = hit.description.clone();
        }
    }
    let jobs = list.iter().enumerate().take(DESCRIBE_TOP).filter(|(_, l)| l.description.is_none()).filter_map(|(i, l)| match &l.source {
        SkillSource::GitHub { repo, skill: Some(skill), .. } => Some((i, repo.clone(), skill.clone())),
        _ => None,
    });
    let jobs: Vec<_> = jobs
        .map(|(i, repo, skill)| {
            let client = client.clone();
            async move {
                for path in [format!("skills/{skill}/{SKILL_FILE}"), format!("{skill}/{SKILL_FILE}")] {
                    if let Ok(bytes) = client.raw(&repo, None, &path).await {
                        let doc = parse_skill_md(&String::from_utf8_lossy(&bytes));
                        if let Some(d) = doc.description() {
                            return Some((i, d.to_string()));
                        }
                    }
                }
                None
            }
        })
        .collect();
    let found: Vec<(usize, String)> = futures::stream::iter(jobs)
    .buffer_unordered(6)
    .filter_map(|x| async move { x })
    .collect()
    .await;
    for (i, d) in found {
        list[i].description = Some(d);
    }
}

#[derive(Serialize, Deserialize)]
struct BrowseCache {
    fetched_at: chrono::DateTime<chrono::Utc>,
    skills: Vec<SkillListing>,
}

fn cache_path(repo: &str) -> PathBuf {
    data_dir().join("skills-cache").join(format!("{}.json", repo.replace('/', "__")))
}

fn read_cache(repo: &str) -> Option<BrowseCache> {
    serde_json::from_slice(&std::fs::read(cache_path(repo)).ok()?).ok()
}

/// Every skill in a GitHub repo, with descriptions for the first [`DESCRIBE_BROWSE`].
pub async fn browse(client: &Arc<Client>, repo: &str) -> anyhow::Result<Vec<SkillListing>> {
    if let Some(c) = read_cache(repo)
        && chrono::Utc::now() - c.fetched_at < BROWSE_TTL
    {
        return Ok(c.skills);
    }
    let tree = client.tree(repo, None).await?;
    let dirs = skill_dirs(&tree, None);
    let old = read_cache(repo);
    use futures::StreamExt as _;
    let jobs: Vec<_> = dirs
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let d = d.clone();
            let known = old.as_ref().and_then(|c| {
                c.skills
                    .iter()
                    .find(|s| matches!(&s.source, SkillSource::GitHub { path, .. } if path.as_deref().unwrap_or("") == d))
                    .map(|s| (s.name.clone(), s.description.clone()))
            });
            let (client, repo) = (client.clone(), repo.to_string());
            async move {
                if let Some((name, desc)) = known.filter(|(_, d)| d.is_some()) {
                    return (i, Some(name), desc);
                }
                if i >= DESCRIBE_BROWSE {
                    return (i, None, None);
                }
                let file = if d.is_empty() { SKILL_FILE.to_string() } else { format!("{d}/{SKILL_FILE}") };
                match client.raw(&repo, None, &file).await {
                    Ok(bytes) => {
                        let doc = parse_skill_md(&String::from_utf8_lossy(&bytes));
                        (i, doc.name().map(str::to_string), doc.description().map(str::to_string))
                    }
                    Err(_) => (i, None, None),
                }
            }
        })
        .collect();
    let described: Vec<(usize, Option<String>, Option<String>)> = futures::stream::iter(jobs)
    .buffer_unordered(8)
    .collect()
    .await;
    let mut skills: Vec<SkillListing> = dirs
        .iter()
        .map(|d| SkillListing {
            name: d.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(repo.rsplit('/').next().unwrap_or(repo)).to_string(),
            description: None,
            source: SkillSource::GitHub { repo: repo.to_string(), path: (!d.is_empty()).then(|| d.clone()), skill: None, git_ref: None },
            origin: repo.to_string(),
            installs: None,
        })
        .collect();
    for (i, name, desc) in described {
        if let Some(n) = name.filter(|n| !n.is_empty()) {
            skills[i].name = n;
        }
        skills[i].description = desc;
    }
    skills.sort_by_key(|s| s.name.to_ascii_lowercase());
    let cache = BrowseCache { fetched_at: chrono::Utc::now(), skills };
    if let Ok(bytes) = serde_json::to_vec(&cache) {
        let p = cache_path(repo);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = write_atomic(&p, &bytes);
    }
    Ok(cache.skills)
}

/// Where agents keep skills of their own, per user.
fn agent_skill_dirs() -> Vec<(PathBuf, &'static str)> {
    let h = home_dir();
    vec![
        (h.join(".claude/skills"), "Claude Code"),
        (h.join(".agents/skills"), "Shared (.agents)"),
        (h.join(".codex/skills"), "Codex"),
        (h.join(".copilot/skills"), "Copilot"),
        (h.join(".config/opencode/skill"), "OpenCode"),
        (h.join(".config/opencode/skills"), "OpenCode"),
        (h.join(".gemini/skills"), "Gemini"),
        (h.join(".cursor/skills"), "Cursor"),
    ]
}

/// Skills other agents keep on this computer.
pub fn local_agent_skills() -> Vec<SkillListing> {
    let mut out: Vec<SkillListing> = vec![];
    let mut seen = std::collections::HashSet::new();
    for (dir, agent) in agent_skill_dirs() {
        if !dir.is_dir() {
            continue;
        }
        for skill_dir in scan(&dir) {
            let Ok(real) = skill_dir.canonicalize() else { continue };
            if !seen.insert(real) {
                continue;
            }
            let Some(s) = read_skill_dir(&skill_dir) else { continue };
            let name = std::fs::read_to_string(skill_dir.join(SKILL_FILE))
                .ok()
                .and_then(|t| parse_skill_md(&t).name().map(str::to_string))
                .unwrap_or(s.name);
            out.push(SkillListing {
                name,
                description: (!s.description.is_empty()).then_some(s.description),
                source: SkillSource::Folder { path: skill_dir.clone() },
                origin: format!("{agent} · {}", tildify(skill_dir.parent().unwrap_or(&skill_dir))),
                installs: None,
            });
        }
    }
    out.sort_by_key(|s| s.name.to_ascii_lowercase());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_sources_only() {
        assert!(is_github_repo("anthropics/skills"));
        assert!(is_github_repo("vercel-labs/agent-skills"));
        assert!(is_github_repo("a/b.js"));
        assert!(!is_github_repo("open.feishu.cn"));
        assert!(!is_github_repo("docs.example.com/skills"));
        assert!(!is_github_repo("a/b/c"));
    }
}
