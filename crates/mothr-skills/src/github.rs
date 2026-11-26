//! Just enough of GitHub to list a repo's files and download some of them.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// How long a repo listing is reused before asking GitHub again.
const TREE_TTL: Duration = Duration::from_secs(15 * 60);

/// A file in a repo.
#[derive(Debug, Clone)]
pub struct Blob {
    pub path: String,
    pub size: u64,
}

/// A repo listing and when it was fetched.
type CachedTree = (Instant, Arc<Vec<Blob>>);

pub struct Client {
    pub http: reqwest::Client,
    trees: Mutex<HashMap<String, CachedTree>>,
    token: OnceLock<Option<String>>,
}

impl Client {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent("mothr")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self { http, trees: Mutex::new(HashMap::new()), token: OnceLock::new() }
    }

    /// A token for the GitHub API (60 requests an hour without one): `GITHUB_TOKEN`, `GH_TOKEN`,
    /// or the GitHub CLI's login.
    async fn token(&self) -> Option<String> {
        if let Some(t) = self.token.get() {
            return t.clone();
        }
        let t = lookup_token().await;
        let _ = self.token.set(t.clone());
        t
    }

    /// Every file of `repo` ("owner/name") at `git_ref` (None: the default branch).
    pub async fn tree(&self, repo: &str, git_ref: Option<&str>) -> anyhow::Result<Arc<Vec<Blob>>> {
        let r = git_ref.unwrap_or("HEAD");
        let key = format!("{repo}#{r}");
        if let Some((at, t)) = self.trees.lock().get(&key)
            && at.elapsed() < TREE_TTL
        {
            return Ok(t.clone());
        }
        let url = format!("https://api.github.com/repos/{repo}/git/trees/{r}?recursive=1");
        let token = self.token().await;
        let mut req = self.http.get(&url).header("Accept", "application/vnd.github+json");
        if let Some(t) = &token {
            req = req.bearer_auth(t);
        }
        let res = req.send().await.map_err(|e| anyhow::anyhow!("Couldn't reach GitHub: {e}"))?;
        let status = res.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            anyhow::bail!("GitHub has no repo {repo}{}", git_ref.map(|r| format!(" at {r}")).unwrap_or_default());
        }
        if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            anyhow::bail!(if token.is_some() {
                "GitHub's rate limit is used up; try again in a while".to_string()
            } else {
                "GitHub's rate limit for anonymous use is used up. Set GITHUB_TOKEN or sign in with `gh auth login`, or try again in an hour".to_string()
            });
        }
        let body: serde_json::Value = res.error_for_status().map_err(|e| anyhow::anyhow!("GitHub: {e}"))?.json().await?;
        let blobs: Vec<Blob> = body["tree"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|e| e["type"] == "blob")
                    .filter_map(|e| Some(Blob { path: e["path"].as_str()?.to_string(), size: e["size"].as_u64().unwrap_or(0) }))
                    .collect()
            })
            .unwrap_or_default();
        let blobs = Arc::new(blobs);
        self.trees.lock().insert(key, (Instant::now(), blobs.clone()));
        Ok(blobs)
    }

    /// One file's bytes.
    pub async fn raw(&self, repo: &str, git_ref: Option<&str>, path: &str) -> anyhow::Result<Vec<u8>> {
        let url = format!("https://raw.githubusercontent.com/{repo}/{}/{}", git_ref.unwrap_or("HEAD"), encode_path(path));
        self.get_bytes(&url).await
    }

    pub async fn get_bytes(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let res = self.http.get(url).send().await.map_err(|e| anyhow::anyhow!("Couldn't download {url}: {e}"))?;
        let res = res.error_for_status().map_err(|e| anyhow::anyhow!("Couldn't download {url}: {}", e.status().map(|s| s.to_string()).unwrap_or_else(|| e.to_string())))?;
        Ok(res.bytes().await?.to_vec())
    }
}

async fn lookup_token() -> Option<String> {
    for var in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(t) = std::env::var(var)
            && !t.trim().is_empty()
        {
            return Some(t.trim().to_string());
        }
    }
    let out = tokio::process::Command::new("gh").args(["auth", "token"]).stdin(std::process::Stdio::null()).output().await.ok()?;
    let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !t.is_empty()).then_some(t)
}

/// Percent-encode what a path segment can't hold as is.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn encodes_paths() {
        assert_eq!(super::encode_path("skills/a b/SKILL.md"), "skills/a%20b/SKILL.md");
    }
}
