//! The desktop installs that update themselves, and putting a new version in place.
//!
//! **Linux** ([`Layout::Versions`], made by `install.sh`):
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
//!
//! **Windows** ([`Layout::InPlace`], made by the setup, `apps/desktop/windows/mothr.iss`):
//!
//! ```text
//! %LOCALAPPDATA%\Programs\Mothr\   the install root (the setup's {app})
//!   install.json                   marker, and the version installed: {"installer":"setup","version":"0.2.0"}
//!   mothr.exe                      what the Start menu entry starts
//!   LICENSE.txt  licenses\  unins000.exe  unins000.dat
//! ```
//!
//! Windows has no symlink to swap, but it lets a running exe be renamed. An update unpacks the
//! release zip next to it, renames the running `mothr.exe` to `mothr.exe.<n>.old`, moves the new
//! one in and records its version (in `install.json` and the Apps & features entry). The next
//! start runs it and deletes the `.old` files ([`Install::clean_up`]).
//!
//! **macOS** ([`Layout::Bundle`], made by `install.sh` or by dragging `Mothr.app` from the disk
//! image to Applications):
//!
//! ```text
//! ~/Applications/Mothr.app/        the install root (install.sh's --dir; any folder the user can write)
//!   Contents/Info.plist            CFBundleShortVersionString: the version installed
//!   Contents/MacOS/mothr           what Launchpad, the Dock and `~/.local/bin/mothr` start
//!   Contents/Resources/            icon, licenses
//! ~/Applications/.mothr-update/    while updating: the download, the new bundle, the one it replaced
//! ```
//!
//! Mac apps are installed by copying the bundle, so every `Mothr.app` in a folder the user can
//! write updates itself; one that can't be replaced (in a read-only or translocated folder, or on
//! the disk image) doesn't. An update unpacks the release tarball into `.mothr-update/` next to
//! the bundle, moves the running bundle aside into it and the new one into its place (macOS lets a
//! running app's bundle move). The next start runs the new one and deletes `.mothr-update/`.
//! A restart runs the bundle's binary directly, not through `open`, so it keeps Mothr's
//! environment (`MOTHR_HOME` and the like) as on Linux.

use crate::{Client, Release, linux_asset, macos_arch, macos_asset, windows_asset};
use anyhow::{Context as _, bail};
use std::path::{Path, PathBuf};

pub const MARKER: &str = "install.json";
pub const BINARY: &str = "mothr";
/// The binary in a Windows install.
pub const WINDOWS_BINARY: &str = "mothr.exe";
/// The binary inside a macOS app bundle.
pub const MACOS_BINARY: &str = "Contents/MacOS/mothr";
/// Next to a macOS bundle while it updates (see the module docs).
pub const MACOS_WORK_DIR: &str = ".mothr-update";
/// The setup's Apps & features entry (`HKCU`). Keep the GUID in step with `AppId` in
/// `apps/desktop/windows/mothr.iss`.
pub const WINDOWS_UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{0E591738-8A65-45C8-8C9B-61E1FE0ECC9E}_is1";

/// How an install keeps its versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Linux: `versions/<version>/` and a `current` symlink.
    Versions,
    /// Windows: one folder whose `mothr.exe` an update replaces.
    InPlace,
    /// macOS: `root` is the `.app` bundle, which an update replaces as a whole.
    Bundle,
}

/// An installer-made copy of Mothr.
#[derive(Debug, Clone, PartialEq)]
pub struct Install {
    pub root: PathBuf,
    /// The version directory the running binary lives in, when it is one of ours (Linux).
    pub running: Option<String>,
    pub layout: Layout,
}

impl Install {
    /// The install the running binary belongs to, or `None` when it wasn't installed by the
    /// installer (`cargo run`, a distro package, a copied binary, the portable Windows zip, a
    /// Mac app the user can't replace): those don't update themselves.
    pub fn detect() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        if cfg!(windows) {
            // Not canonicalized: that gives a `\\?\` path, which child processes can't start in.
            Self::in_place_for_binary(&exe)
        } else if cfg!(target_os = "macos") {
            // Resolved, so `~/.local/bin/mothr` (a symlink into the bundle) finds the bundle.
            Self::bundle_for_binary(&exe.canonicalize().ok()?)
        } else {
            Self::for_binary(&exe.canonicalize().ok()?)
        }
    }

    /// The Linux install `exe` (a resolved path) belongs to.
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
            layout: Layout::Versions,
        })
    }

    /// The Windows install `exe` belongs to: its folder holds the marker.
    pub fn in_place_for_binary(exe: &Path) -> Option<Self> {
        let root = exe.parent()?;
        (exe.file_name()?.eq_ignore_ascii_case(WINDOWS_BINARY) && root.join(MARKER).is_file())
            .then(|| Self { root: root.to_path_buf(), running: None, layout: Layout::InPlace })
    }

    /// The macOS app `exe` (a resolved path) is the binary of: `<name>.app/Contents/MacOS/mothr`,
    /// when the bundle can be replaced (it and its folder are writable).
    pub fn bundle_for_binary(exe: &Path) -> Option<Self> {
        let macos = exe.parent()?;
        let contents = macos.parent()?;
        let bundle = contents.parent()?;
        let shaped = macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && bundle.extension()? == "app";
        (shaped && writable(bundle.parent()?) && writable(bundle))
            .then(|| Self { root: bundle.to_path_buf(), running: None, layout: Layout::Bundle })
    }

    /// The version the next start runs.
    pub fn current_version(&self) -> Option<String> {
        match self.layout {
            Layout::Versions => {
                let target = std::fs::read_link(self.root.join("current")).ok()?;
                target.file_name().map(|n| n.to_string_lossy().into_owned())
            }
            Layout::InPlace => {
                let marker: serde_json::Value = serde_json::from_slice(&std::fs::read(self.root.join(MARKER)).ok()?).ok()?;
                marker.get("version")?.as_str().map(str::to_string)
            }
            Layout::Bundle => plist_string(&self.root.join("Contents/Info.plist"), "CFBundleShortVersionString"),
        }
    }

    /// The binary the next start runs.
    pub fn current_binary(&self) -> PathBuf {
        match self.layout {
            Layout::Versions => self.root.join("current").join(BINARY),
            Layout::InPlace => self.root.join(WINDOWS_BINARY),
            Layout::Bundle => self.root.join(MACOS_BINARY),
        }
    }

    /// Where a macOS update works: `.mothr-update/` next to the bundle, on the same volume so
    /// the bundles move by renaming.
    fn work_dir(&self) -> PathBuf {
        self.root.parent().unwrap_or(&self.root).join(MACOS_WORK_DIR)
    }

    /// Remove what earlier updates left behind: the binaries they replaced (in use until that
    /// Mothr exited) and interrupted downloads. Windows and macOS; Linux prunes while staging.
    pub fn clean_up(&self) {
        match self.layout {
            Layout::Versions => return,
            Layout::Bundle => {
                let _ = std::fs::remove_dir_all(self.work_dir());
                return;
            }
            Layout::InPlace => {}
        }
        let _ = std::fs::remove_dir_all(self.root.join("downloads"));
        let Ok(entries) = std::fs::read_dir(&self.root) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".old") {
                let _ = std::fs::remove_file(e.path());
            } else if name.starts_with(".staging-") {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }

    /// Download `release`, check it and put it in place. Returns once the next start will run
    /// it.
    pub async fn stage(&self, client: &Client, release: &Release) -> anyhow::Result<()> {
        match self.layout {
            Layout::InPlace => return self.stage_in_place(client, release).await,
            Layout::Bundle => return self.stage_bundle(client, release).await,
            Layout::Versions => {}
        }
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
        {
            std::os::unix::fs::symlink(&target, &tmp)?;
            std::fs::rename(&tmp, self.root.join("current"))?;
            Ok(())
        }
        #[cfg(not(unix))]
        bail!("updating in place is only supported on Linux")
    }

    async fn stage_in_place(&self, client: &Client, release: &Release) -> anyhow::Result<()> {
        let version = release.version.to_string();
        let name = windows_asset(&release.version);
        let archive = self.root.join("downloads").join(&name);
        client.download_verified(release, &name, &archive).await?;
        let this = self.clone();
        let zip = archive.clone();
        let result = tokio::task::spawn_blocking(move || this.swap_in(&zip, &version)).await;
        let _ = tokio::fs::remove_file(&archive).await;
        result??;
        Ok(())
    }

    /// Unpack a release zip and replace this install's binary with its one (Windows). The
    /// running binary is renamed out of the way, which Windows allows while it runs.
    pub fn swap_in(&self, archive: &Path, version: &str) -> anyhow::Result<()> {
        let staging = self.root.join(format!(".staging-{version}"));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;
        let result = self.swap_in_from(archive, &staging, version);
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    fn swap_in_from(&self, archive: &Path, staging: &Path, version: &str) -> anyhow::Result<()> {
        let file = std::fs::File::open(archive)?;
        // `extract` keeps every entry inside `staging` (no `..` or absolute paths).
        zip::ZipArchive::new(file)
            .and_then(|mut z| z.extract(staging))
            .context("couldn't unpack the update")?;
        // The zip holds one folder, `mothr_<version>_windows_x64/`.
        let unpacked = single_dir(staging).unwrap_or_else(|| staging.to_path_buf());
        let new_binary = unpacked.join(WINDOWS_BINARY);
        if !new_binary.is_file() {
            bail!("the update has no {WINDOWS_BINARY}");
        }
        let binary = self.root.join(WINDOWS_BINARY);
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
        let aside = self.root.join(format!("{WINDOWS_BINARY}.{nonce}.old"));
        if binary.exists() {
            std::fs::rename(&binary, &aside).context("couldn't move the running Mothr aside")?;
        }
        if let Err(e) = std::fs::rename(&new_binary, &binary) {
            let _ = std::fs::rename(&aside, &binary);
            return Err(anyhow::Error::from(e).context("couldn't put the new Mothr in place"));
        }
        // Licenses and the like; the binary already moved.
        copy_over(&unpacked, &self.root)?;
        self.record_version(version)
    }

    async fn stage_bundle(&self, client: &Client, release: &Release) -> anyhow::Result<()> {
        let version = release.version.to_string();
        let name = macos_asset(&release.version, macos_arch());
        let archive = self.work_dir().join("downloads").join(&name);
        client.download_verified(release, &name, &archive).await?;
        let this = self.clone();
        let tarball = archive.clone();
        let result = tokio::task::spawn_blocking(move || this.swap_bundle(&tarball, &version)).await;
        let _ = tokio::fs::remove_file(&archive).await;
        result??;
        Ok(())
    }

    /// Unpack a release tarball and replace this install's app bundle with its one (macOS). The
    /// running bundle moves aside into the work folder; the next start deletes it.
    pub fn swap_bundle(&self, archive: &Path, version: &str) -> anyhow::Result<()> {
        let staging = self.work_dir().join(format!("staging-{version}"));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;
        let result = self.swap_bundle_from(archive, &staging, version);
        let _ = std::fs::remove_dir_all(&staging);
        result
    }

    fn swap_bundle_from(&self, archive: &Path, staging: &Path, version: &str) -> anyhow::Result<()> {
        let out = std::process::Command::new("tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(staging)
            .output()
            .context("couldn't run tar")?;
        if !out.status.success() {
            bail!("couldn't unpack the update: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        // The tarball holds one folder, `mothr_<version>_macos_<arch>/`, with `Mothr.app` in it.
        let unpacked = single_dir(staging).unwrap_or_else(|| staging.to_path_buf());
        let new_bundle = find_app(&unpacked).context("the update has no Mothr.app")?;
        let binary = new_bundle.join(MACOS_BINARY);
        if !binary.is_file() {
            bail!("the update has no {MACOS_BINARY}");
        }
        set_executable(&binary)?;
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
        let aside = self.work_dir().join(format!("replaced-{nonce}"));
        std::fs::rename(&self.root, &aside).context("couldn't move the running Mothr aside")?;
        if let Err(e) = std::fs::rename(&new_bundle, &self.root) {
            let _ = std::fs::rename(&aside, &self.root);
            return Err(anyhow::Error::from(e).context("couldn't put the new Mothr in place"));
        }
        if self.current_version().as_deref() != Some(version) {
            tracing::warn!("the update's Info.plist doesn't say version {version}");
        }
        Ok(())
    }

    /// Note the installed version: in the marker (what the next start runs) and in Apps &
    /// features.
    fn record_version(&self, version: &str) -> anyhow::Result<()> {
        let path = self.root.join(MARKER);
        let mut marker: serde_json::Value =
            std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| serde_json::json!({}));
        if !marker.is_object() {
            marker = serde_json::json!({});
        }
        marker["version"] = version.into();
        let tmp = self.root.join(".install.json.new");
        std::fs::write(&tmp, format!("{marker}\n"))?;
        std::fs::rename(&tmp, &path)?;
        #[cfg(windows)]
        {
            // Only an entry the setup made; a copy without one doesn't get a new one.
            if let Ok(key) = windows_registry::CURRENT_USER.options().read().write().open(WINDOWS_UNINSTALL_KEY)
                && let Err(e) = key.set_string("DisplayVersion", version)
            {
                tracing::warn!("couldn't update the Apps & features entry: {e}");
            }
        }
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

/// Copy everything in `from` into `to`, replacing files that exist.
fn copy_over(from: &Path, to: &Path) -> anyhow::Result<()> {
    for e in std::fs::read_dir(from)?.flatten() {
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            std::fs::create_dir_all(&dest)?;
            copy_over(&e.path(), &dest)?;
        } else {
            std::fs::copy(e.path(), &dest)?;
        }
    }
    Ok(())
}

/// Wait up to `timeout` for process `pid` to end. A restart into a new version starts it right
/// away; it waits here so the old Mothr can finish saving chats first.
#[cfg(windows)]
pub fn wait_for_exit(pid: u32, timeout: std::time::Duration) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    // SAFETY: plain Win32 calls on a handle this function owns and closes.
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return; // gone already
        }
        WaitForSingleObject(handle, u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX));
        CloseHandle(handle);
    }
}

/// A string value from an XML property list (the `Info.plist` the release workflow writes).
fn plist_string(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let (_, after) = text.split_once(&format!("<key>{key}</key>"))?;
    let value = after.trim_start().strip_prefix("<string>")?;
    Some(value.split_once("</string>")?.0.trim().to_string())
}

/// `dir` itself when it's an `.app` bundle, else the first one in it.
fn find_app(dir: &Path) -> Option<PathBuf> {
    let is_app = |p: &Path| p.is_dir() && p.extension().is_some_and(|e| e == "app");
    if is_app(dir) {
        return Some(dir.to_path_buf());
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| is_app(p))
}

/// Whether this process can create files in `dir` (permissions alone don't tell: a read-only
/// volume, App Translocation).
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".mothr-write-test-{}", std::process::id()));
    let ok = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn single_dir(dir: &Path) -> Option<PathBuf> {
    let mut entries = std::fs::read_dir(dir).ok()?.flatten();
    let first = entries.next()?;
    (entries.next().is_none() && first.path().is_dir()).then(|| first.path())
}

#[cfg_attr(not(unix), allow(unused_variables))]
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
        let install = Install { root: root.clone(), running: Some("0.1.0".into()), layout: Layout::Versions };
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

/// The Windows layout's file handling is plain renames and copies, so it's tested everywhere.
#[cfg(test)]
mod in_place_tests {
    use super::*;
    use std::io::Write as _;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mothr-update-win-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A zip shaped like the release workflow's: one folder with the binary and licenses.
    fn release_zip(dir: &Path, version: &str) -> PathBuf {
        let path = dir.join(format!("mothr_{version}_windows_x64.zip"));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        let top = format!("mothr_{version}_windows_x64");
        zip.start_file(format!("{top}/{WINDOWS_BINARY}"), opts).unwrap();
        zip.write_all(format!("mothr {version}").as_bytes()).unwrap();
        zip.start_file(format!("{top}/licenses/NOTICE.md"), opts).unwrap();
        zip.write_all(version.as_bytes()).unwrap();
        zip.finish().unwrap();
        path
    }

    fn installed(name: &str) -> Install {
        let root = scratch(name);
        std::fs::write(root.join(WINDOWS_BINARY), "mothr 0.1.0").unwrap();
        std::fs::write(root.join(MARKER), r#"{"installer":"setup","version":"0.1.0"}"#).unwrap();
        Install::in_place_for_binary(&root.join(WINDOWS_BINARY)).expect("the setup's folder is an install")
    }

    #[test]
    fn only_the_setups_folder_is_detected() {
        let root = scratch("detect");
        let exe = root.join(WINDOWS_BINARY);
        std::fs::write(&exe, "").unwrap();
        assert_eq!(Install::in_place_for_binary(&exe), None, "no marker: the portable zip");
        std::fs::write(root.join(MARKER), "{}").unwrap();
        let found = Install::in_place_for_binary(&exe).unwrap();
        assert_eq!((found.root.as_path(), found.layout), (root.as_path(), Layout::InPlace));
        assert_eq!(Install::in_place_for_binary(&root.join("other.exe")), None);
    }

    #[test]
    fn swaps_the_binary_and_records_the_version() {
        let install = installed("swap");
        let root = install.root.clone();
        assert_eq!(install.current_version().as_deref(), Some("0.1.0"));

        install.swap_in(&release_zip(&root, "0.2.0"), "0.2.0").unwrap();
        assert_eq!(std::fs::read_to_string(install.current_binary()).unwrap(), "mothr 0.2.0");
        assert_eq!(std::fs::read_to_string(root.join("licenses/NOTICE.md")).unwrap(), "0.2.0");
        assert_eq!(install.current_version().as_deref(), Some("0.2.0"));
        let marker: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(MARKER)).unwrap()).unwrap();
        assert_eq!(marker["installer"], "setup", "the rest of the marker stays");

        // A second update before a restart: the first new binary moves aside too.
        install.swap_in(&release_zip(&root, "0.3.0"), "0.3.0").unwrap();
        assert_eq!(std::fs::read_to_string(install.current_binary()).unwrap(), "mothr 0.3.0");
        let old = |r: &Path| std::fs::read_dir(r).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".old")).count();
        assert_eq!(old(&root), 2, "the replaced binaries wait for the next start");

        install.clean_up();
        assert_eq!(old(&root), 0);
        assert!(install.current_binary().is_file());
    }

    #[test]
    fn a_broken_update_leaves_the_install_alone() {
        let install = installed("broken");
        let root = install.root.clone();
        let bad = root.join("bad.zip");
        std::fs::write(&bad, "not a zip").unwrap();
        assert!(install.swap_in(&bad, "0.2.0").is_err());

        let empty = root.join("empty.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&empty).unwrap());
        zip.start_file("mothr_0.2.0_windows_x64/README.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zip.finish().unwrap();
        assert!(install.swap_in(&empty, "0.2.0").is_err(), "no mothr.exe in it");

        assert_eq!(std::fs::read_to_string(install.current_binary()).unwrap(), "mothr 0.1.0");
        assert_eq!(install.current_version().as_deref(), Some("0.1.0"));
        assert!(!root.join(".staging-0.2.0").exists());
    }
}

/// The macOS layout is plain renames and `tar`, so it's tested on every Unix.
#[cfg(all(test, unix))]
mod bundle_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mothr-update-mac-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A bundle like `apps/desktop/macos/bundle.sh` makes.
    fn bundle(at: &Path, version: &str) {
        std::fs::create_dir_all(at.join("Contents/MacOS")).unwrap();
        std::fs::create_dir_all(at.join("Contents/Resources")).unwrap();
        std::fs::write(at.join(MACOS_BINARY), format!("mothr {version}")).unwrap();
        std::fs::write(
            at.join("Contents/Info.plist"),
            format!("<plist version=\"1.0\">\n<dict>\n\t<key>CFBundleIdentifier</key>\n\t<string>dev.mothr.Mothr</string>\n\t<key>CFBundleShortVersionString</key>\n\t<string>{version}</string>\n</dict>\n</plist>\n"),
        )
        .unwrap();
    }

    /// A tarball shaped like the release workflow's: `mothr_<v>_macos_arm64/Mothr.app`.
    fn tarball(dir: &Path, version: &str) -> PathBuf {
        let top = format!("mothr_{version}_macos_arm64");
        let src = dir.join("src").join(&top);
        bundle(&src.join("Mothr.app"), version);
        let out = dir.join(format!("{top}.tar.gz"));
        let ok = std::process::Command::new("tar").arg("-czf").arg(&out).arg("-C").arg(src.parent().unwrap()).arg(&top).status().unwrap().success();
        assert!(ok);
        out
    }

    fn installed(name: &str) -> (PathBuf, Install) {
        let dir = scratch(name);
        let app = dir.join("Applications/Mothr.app");
        bundle(&app, "0.1.0");
        let install = Install::bundle_for_binary(&app.join(MACOS_BINARY)).expect("a bundle in a writable folder");
        (dir, install)
    }

    #[test]
    fn only_replaceable_bundles_are_detected() {
        let (dir, install) = installed("detect");
        assert_eq!((install.root.as_path(), install.layout), (dir.join("Applications/Mothr.app").as_path(), Layout::Bundle));
        assert_eq!(install.current_version().as_deref(), Some("0.1.0"));
        assert_eq!(Install::bundle_for_binary(&dir.join("target/release/mothr")), None, "not in a bundle");
        let loose = dir.join("Mothr/Contents/MacOS/mothr");
        std::fs::create_dir_all(loose.parent().unwrap()).unwrap();
        std::fs::write(&loose, "").unwrap();
        assert_eq!(Install::bundle_for_binary(&loose), None, "not an .app");

        use std::os::unix::fs::PermissionsExt as _;
        let apps = dir.join("Applications");
        std::fs::set_permissions(&apps, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root writes anywhere; the check only means something for a normal user.
        if !writable(&apps) {
            assert_eq!(Install::bundle_for_binary(&install.current_binary()), None, "a folder the user can't write");
        }
        std::fs::set_permissions(&apps, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn swaps_the_bundle_and_cleans_up_next_start() {
        let (dir, install) = installed("swap");
        install.swap_bundle(&tarball(&dir, "0.2.0"), "0.2.0").unwrap();
        assert_eq!(install.current_version().as_deref(), Some("0.2.0"));
        assert_eq!(std::fs::read_to_string(install.current_binary()).unwrap(), "mothr 0.2.0");
        let work = dir.join("Applications").join(MACOS_WORK_DIR);
        let replaced: Vec<_> = std::fs::read_dir(&work).unwrap().flatten().map(|e| e.path()).collect();
        assert_eq!(replaced.len(), 1, "only the replaced bundle is left: {replaced:?}");
        assert_eq!(std::fs::read_to_string(replaced[0].join(MACOS_BINARY)).unwrap(), "mothr 0.1.0");

        // A second update before a restart.
        install.swap_bundle(&tarball(&dir, "0.3.0"), "0.3.0").unwrap();
        assert_eq!(install.current_version().as_deref(), Some("0.3.0"));

        install.clean_up();
        assert!(!work.exists());
        assert!(install.current_binary().is_file());
    }

    #[test]
    fn a_broken_update_leaves_the_bundle_alone() {
        let (dir, install) = installed("broken");
        let bad = dir.join("bad.tar.gz");
        std::fs::write(&bad, "not a tarball").unwrap();
        assert!(install.swap_bundle(&bad, "0.2.0").is_err());

        let src = dir.join("empty/mothr_0.2.0_macos_arm64");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("README"), "").unwrap();
        let empty = dir.join("empty.tar.gz");
        assert!(std::process::Command::new("tar").arg("-czf").arg(&empty).arg("-C").arg(dir.join("empty")).arg("mothr_0.2.0_macos_arm64").status().unwrap().success());
        assert!(install.swap_bundle(&empty, "0.2.0").is_err(), "no Mothr.app in it");

        assert_eq!(std::fs::read_to_string(install.current_binary()).unwrap(), "mothr 0.1.0");
        assert_eq!(install.current_version().as_deref(), Some("0.1.0"));
        assert!(!dir.join("Applications").join(MACOS_WORK_DIR).join("staging-0.2.0").exists());
    }
}
