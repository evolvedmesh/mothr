//! Mothr's updates, shared by the desktop app and the phone app (through `mothr-client`).
//!
//! Releases are GitHub releases of [`REPO`] (see the `release` skill). Every stable release
//! carries `SHA256SUMS` next to its assets; a download is only kept when it matches.
//!
//! - [`Client::latest`] finds the newest stable release (prereleases are never offered).
//! - [`Client::download_verified`] downloads one asset and checks it against `SHA256SUMS`.
//! - [`install`] is the desktop install layouts (Linux, Windows, macOS) and how a new version
//!   is put in place next to the running one.
//!
//! `MOTHR_UPDATE_API` points everything at another releases API (tests, a mirror).

pub mod install;

use anyhow::{Context as _, bail};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// Where Mothr is released.
pub const REPO: &str = "evolvedmesh/mothr";
/// The checksum file every stable release carries.
pub const SUMS: &str = "SHA256SUMS";

/// One published release.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: semver::Version,
    pub tag: String,
    /// The release page (notes, assets).
    pub url: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }

    /// Whether this release is newer than `current` (a version string like `0.1.0`). An
    /// unparseable current version is treated as older, so a broken build can still update.
    pub fn is_newer_than(&self, current: &str) -> bool {
        match semver::Version::parse(current.trim_start_matches('v')) {
            Ok(cur) => self.version > cur,
            Err(_) => true,
        }
    }
}

/// The Linux desktop tarball for a version (what `install.sh` and the updater install).
pub fn linux_asset(version: &semver::Version) -> String {
    format!("mothr_{version}_linux_x64.tar.gz")
}

/// The Windows desktop zip for a version: a portable folder with `mothr.exe`. Updates of an
/// installed copy unpack it.
pub fn windows_asset(version: &semver::Version) -> String {
    format!("mothr_{version}_windows_x64.zip")
}

/// The Windows installer for a version (`apps/desktop/windows/mothr.iss`).
pub fn windows_setup_asset(version: &semver::Version) -> String {
    format!("mothr_{version}_windows_x64_setup.exe")
}

/// The macOS desktop tarball for a version and an architecture ([`macos_arch`]): one folder
/// holding `Mothr.app`. `install.sh` and the updater install it.
pub fn macos_asset(version: &semver::Version, arch: &str) -> String {
    format!("mothr_{version}_macos_{arch}.tar.gz")
}

/// The macOS disk image for a version and an architecture: `Mothr.app` to drag to Applications.
pub fn macos_dmg_asset(version: &semver::Version, arch: &str) -> String {
    format!("mothr_{version}_macos_{arch}.dmg")
}

/// This build's macOS architecture as release assets name it: `arm64` (Apple silicon) or `x64`
/// (Intel).
pub fn macos_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" }
}

/// The phone APK for a version and an Android ABI (`arm64-v8a`, `x86_64`, …).
pub fn android_asset(version: &semver::Version, abi: &str) -> String {
    format!("mothr_{version}_android_{abi}.apk")
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

fn parse_release(json: &[u8]) -> anyhow::Result<Release> {
    let r: GhRelease = serde_json::from_slice(json).context("unexpected reply from GitHub")?;
    if r.draft || r.prerelease {
        bail!("{} isn't a stable release", r.tag_name);
    }
    let version = semver::Version::parse(r.tag_name.trim_start_matches('v'))
        .with_context(|| format!("{} isn't a version", r.tag_name))?;
    Ok(Release {
        version,
        tag: r.tag_name,
        url: r.html_url,
        assets: r.assets.into_iter().map(|a| Asset { name: a.name, url: a.browser_download_url, size: a.size }).collect(),
    })
}

/// `SHA256SUMS` as written by `sha256sum`: `<hex>  <name>` (or `<hex> *<name>`) per line.
pub fn parse_sums(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (hash, name) = line.trim().split_once(char::is_whitespace)?;
            let name = name.trim_start().trim_start_matches('*');
            (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && !name.is_empty())
                .then(|| (name.to_string(), hash.to_ascii_lowercase()))
        })
        .collect()
}

/// Talks to the releases API.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    api: String,
}

impl Client {
    /// `user_agent` names the app asking (GitHub requires one), e.g. `mothr/0.1.0`.
    pub fn new(user_agent: &str) -> anyhow::Result<Self> {
        let api = std::env::var("MOTHR_UPDATE_API")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| format!("https://api.github.com/repos/{REPO}"));
        Self::with_api(user_agent, api)
    }

    /// Use another releases API (it must answer `GET {api}/releases/latest` like GitHub).
    pub fn with_api(user_agent: &str, api: impl Into<String>) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(600))
            .build()?;
        Ok(Self { http, api: api.into().trim_end_matches('/').to_string() })
    }

    /// The newest stable release.
    pub async fn latest(&self) -> anyhow::Result<Release> {
        let res = self
            .http
            .get(format!("{}/releases/latest", self.api))
            .header("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .context("couldn't reach GitHub")?;
        match res.status().as_u16() {
            404 => bail!("no release has been published yet"),
            403 | 429 => bail!("GitHub is rate limiting update checks; try again later"),
            _ => {}
        }
        let bytes = res.error_for_status().context("GitHub refused the update check")?.bytes().await?;
        parse_release(&bytes)
    }

    async fn get(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        let res = self.http.get(url).send().await.with_context(|| format!("couldn't download {url}"))?;
        Ok(res.error_for_status()?.bytes().await?.to_vec())
    }

    /// Download `name` from `release` to `dest`, keeping it only if it matches the release's
    /// `SHA256SUMS`. The file appears at `dest` complete or not at all.
    pub async fn download_verified(&self, release: &Release, name: &str, dest: &Path) -> anyhow::Result<()> {
        let asset = release.asset(name).with_context(|| format!("{} has no {name}", release.tag))?;
        let sums = release.asset(SUMS).with_context(|| format!("{} has no {SUMS}", release.tag))?;
        let sums = parse_sums(&String::from_utf8_lossy(&self.get(&sums.url).await?));
        let want = sums.get(name).with_context(|| format!("{SUMS} doesn't list {name}"))?;
        let bytes = self.get(&asset.url).await?;
        let got = hex::encode(Sha256::digest(&bytes));
        if &got != want {
            bail!("{name} is damaged (checksum mismatch); not installing it");
        }
        if let Some(dir) = dest.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let tmp = dest.with_extension("part");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, dest).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = r#"{
        "tag_name": "v0.2.0", "html_url": "https://github.com/evolvedmesh/mothr/releases/tag/v0.2.0",
        "draft": false, "prerelease": false,
        "assets": [
            {"name": "mothr_0.2.0_linux_x64.tar.gz", "browser_download_url": "https://x/a.tar.gz", "size": 10},
            {"name": "SHA256SUMS", "browser_download_url": "https://x/SHA256SUMS", "size": 1}
        ]
    }"#;

    #[test]
    fn reads_a_release() {
        let r = parse_release(RELEASE.as_bytes()).unwrap();
        assert_eq!(r.version, semver::Version::new(0, 2, 0));
        assert_eq!(r.asset(&linux_asset(&r.version)).unwrap().url, "https://x/a.tar.gz");
        assert!(r.is_newer_than("0.1.0"));
        assert!(r.is_newer_than("0.2.0-dev.4"), "a stable release beats its own prereleases");
        assert!(!r.is_newer_than("0.2.0"));
        assert!(!r.is_newer_than("v0.3.0"));
        assert!(r.is_newer_than("not a version"));
    }

    #[test]
    fn refuses_prereleases() {
        let pre = RELEASE.replace(r#""prerelease": false"#, r#""prerelease": true"#);
        assert!(parse_release(pre.as_bytes()).is_err());
    }

    #[test]
    fn reads_sha256sums() {
        let a = "a".repeat(64);
        let b = "B".repeat(64);
        let sums = parse_sums(&format!("{a}  mothr.tar.gz\n{b} *mothr.apk\nnot a line\n"));
        assert_eq!(sums.get("mothr.tar.gz"), Some(&a));
        assert_eq!(sums.get("mothr.apk"), Some(&"b".repeat(64)));
        assert_eq!(sums.len(), 2);
    }

    #[test]
    fn asset_names_match_the_release_workflow() {
        let v = semver::Version::new(1, 2, 3);
        assert_eq!(linux_asset(&v), "mothr_1.2.3_linux_x64.tar.gz");
        assert_eq!(windows_asset(&v), "mothr_1.2.3_windows_x64.zip");
        assert_eq!(windows_setup_asset(&v), "mothr_1.2.3_windows_x64_setup.exe");
        assert_eq!(macos_asset(&v, "arm64"), "mothr_1.2.3_macos_arm64.tar.gz");
        assert_eq!(macos_dmg_asset(&v, "x64"), "mothr_1.2.3_macos_x64.dmg");
        assert_eq!(android_asset(&v, "arm64-v8a"), "mothr_1.2.3_android_arm64-v8a.apk");
    }
}
