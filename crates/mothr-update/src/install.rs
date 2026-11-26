//! The Linux desktop install made by `install.sh`, and putting a new version in place.
//!
//! ```text
//! ~/.local/lib/mothr/            the install root (MOTHR_INSTALL_DIR in install.sh)
//!   install.json                 marker: this copy was installed by the installer and updates itself
//!   current -> versions/0.2.0    what `~/.local/bin/mothr` and the menu entry start
//!   versions/0.1.0/  versions/0.2.0/
//!     mothr  dev.mothr.Mothr.desktop  licenses/
//! ```
//!
//! An update unpacks next to the running version and then swaps `current` in one rename, so
//! the running app is never touched and the next start runs the new one. The version that is
//! running and the new one are kept; older ones are removed.

use crate::{Client, Release, linux_asset};
use anyhow::{Context as _, bail};
use std::path::{Path, PathBuf};

pub const MARKER: &str = "install.json";
pub const BINARY: &str = "mothr";

/// An installer-made copy of Mothr.
#[derive(Debug, Clone, PartialEq)]
pub struct Install {
    pub root: PathBuf,
    /// The version directory the running binary lives in, when it is one of ours.
    pub running: Option<String>,
}

impl Install {
    /// The install the running binary belongs to, or `None` when it wasn't installed by the
    /// installer (`cargo run`, a distro package, a copied binary): those don't update
    /// themselves.
    pub fn detect() -> Option<Self> {
        let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
        Self::for_binary(&exe)
    }

    /// The install `exe` (a resolved path) belongs to.
    pub fn for_binary(exe: &Path) -> Option<Self> {
        let version_dir = exe.parent()?;
        let versions = version_dir.parent()?;
        if versions.file_name()? != "versions" {
            return None;
        }
        let root = versions.parent()?;
        root.join(MARKER).is_file().then(|| Self {
            root: root.to_path_buf(),
            running: version_dir.file_name().map(|n| n.to_string_lossy().into_owned()),
        })
    }

    /// The version `current` points at: what the next start runs.
    pub fn current_version(&self) -> Option<String> {
        let target = std::fs::read_link(self.root.join("current")).ok()?;
        target.file_name().map(|n| n.to_string_lossy().into_owned())
    }

    /// The binary the next start runs.
    pub fn current_binary(&self) -> PathBuf {
        self.root.join("current").join(BINARY)
    }

    /// Download `release`, check it, unpack it next to the running version and make it
    /// `current`. Returns once the next start will run it.
    pub async fn stage(&self, client: &Client, release: &Release) -> anyhow::Result<()> {
        let version = release.version.to_string();
        let name = linux_asset(&release.version);
        let downloads = self.root.join("downloads");
        let archive = downloads.join(&name);
        client.download_verified(release, &name, &archive).await?;
        let result = self.unpack(&archive, &version).await;
        let _ = tokio::fs::remove_file(&archive).await;
        result?;
        self.switch_to(&version)?;
        self.prune(&version);
        Ok(())
    }

    /// Unpack a release tarball into `versions/<version>`.
    pub async fn unpack(&self, archive: &Path, version: &str) -> anyhow::Result<()> {
        let versions = self.root.join("versions");
        let staging = versions.join(format!(".staging-{version}"));
        let _ = tokio::fs::remove_dir_all(&staging).await;
        tokio::fs::create_dir_all(&staging).await?;
        let out = tokio::process::Command::new("tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(&staging)
            .output()
            .await
            .context("couldn't run tar")?;
        if !out.status.success() {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            bail!("couldn't unpack the update: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        // The tarball holds one folder, `mothr_<version>_linux_x64/`.
        let unpacked = match single_dir(&staging) {
            Some(dir) => dir,
            None => staging.clone(),
        };
        let binary = unpacked.join(BINARY);
        if !binary.is_file() {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            bail!("the update has no {BINARY} binary");
        }
        set_executable(&binary)?;
        let dest = versions.join(version);
        let _ = tokio::fs::remove_dir_all(&dest).await;
        tokio::fs::rename(&unpacked, &dest).await?;
        let _ = tokio::fs::remove_dir_all(&staging).await;
        Ok(())
    }

    /// Point `current` at `versions/<version>` atomically.
    pub fn switch_to(&self, version: &str) -> anyhow::Result<()> {
        let target = Path::new("versions").join(version);
        anyhow::ensure!(self.root.join(&target).join(BINARY).is_file(), "version {version} isn't installed");
        let tmp = self.root.join(".current.new");
        let _ = std::fs::remove_file(&tmp);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &tmp)?;
        #[cfg(not(unix))]
        bail!("updating in place is only supported on Linux");
        std::fs::rename(&tmp, self.root.join("current"))?;
        Ok(())
    }

    /// Remove versions other than `keep` and the running one.
    fn prune(&self, keep: &str) {
        let Ok(entries) = std::fs::read_dir(self.root.join("versions")) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == keep || Some(&name) == self.running.as_ref() {
                continue;
            }
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

fn single_dir(dir: &Path) -> Option<PathBuf> {
    let mut entries = std::fs::read_dir(dir).ok()?.flatten();
    let first = entries.next()?;
    (entries.next().is_none() && first.path().is_dir()).then(|| first.path())
}

fn set_executable(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut perm = std::fs::metadata(path)?.permissions();
        perm.set_mode(perm.mode() | 0o755);
        std::fs::set_permissions(path, perm)?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mothr-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A tarball shaped like the release workflow's.
    fn tarball(dir: &Path, version: &str) -> PathBuf {
        let src = dir.join("src").join(format!("mothr_{version}_linux_x64"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join(BINARY), format!("#!/bin/sh\necho {version}\n")).unwrap();
        let out = dir.join(format!("mothr_{version}_linux_x64.tar.gz"));
        let ok = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&out)
            .arg("-C")
            .arg(src.parent().unwrap())
            .arg(src.file_name().unwrap())
            .status()
            .unwrap()
            .success();
        assert!(ok);
        out
    }

    #[test]
    fn only_installer_copies_are_detected() {
        let root = scratch("detect");
        let bin = root.join("versions/0.1.0/mothr");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, "").unwrap();
        assert_eq!(Install::for_binary(&bin), None, "no marker: not ours");
        std::fs::write(root.join(MARKER), "{}").unwrap();
        let found = Install::for_binary(&bin).unwrap();
        assert_eq!(found.root, root);
        assert_eq!(found.running.as_deref(), Some("0.1.0"));
        assert_eq!(Install::for_binary(&root.join("target/debug/mothr")), None);
    }

    #[tokio::test]
    async fn unpacks_switches_and_prunes() {
        let root = scratch("stage");
        std::fs::write(root.join(MARKER), "{}").unwrap();
        let install = Install { root: root.clone(), running: Some("0.1.0".into()) };
        for v in ["0.0.9", "0.1.0", "0.2.0"] {
            let t = tarball(&root, v);
            install.unpack(&t, v).await.unwrap();
        }
        install.switch_to("0.1.0").unwrap();
        assert_eq!(install.current_version().as_deref(), Some("0.1.0"));

        install.switch_to("0.2.0").unwrap();
        install.prune("0.2.0");
        assert_eq!(install.current_version().as_deref(), Some("0.2.0"));
        assert!(install.current_binary().is_file());
        assert!(root.join("versions/0.1.0").is_dir(), "the running version stays");
        assert!(!root.join("versions/0.0.9").exists(), "older ones go");
        assert!(install.switch_to("9.9.9").is_err());
    }
}
