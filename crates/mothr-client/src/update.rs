//! Updates for the phone app, which isn't in a store: the newest release's APK for this
//! device, found and downloaded (checksum-verified) by the same code the desktop uses
//! (`mothr-update`). The platform hands the file to its installer.
//!
//! Both calls block; call them off the main thread.

use crate::ClientError;
use mothr_update::{Client, android_asset};

/// A newer release with a build for this device.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AppUpdate {
    pub version: String,
    /// The release page (notes).
    pub url: String,
    /// Download size in bytes.
    pub size: u64,
}

fn net(e: impl std::fmt::Display) -> ClientError {
    ClientError::Network { reason: e.to_string() }
}

fn run<T>(api: Option<String>, f: impl AsyncFnOnce(Client) -> anyhow::Result<T>) -> Result<T, ClientError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(net)?;
    let agent = format!("mothr-android/{}", env!("CARGO_PKG_VERSION"));
    let client = match api.filter(|a| !a.trim().is_empty()) {
        Some(api) => Client::with_api(&agent, api),
        None => Client::new(&agent),
    }
    .map_err(net)?;
    rt.block_on(f(client)).map_err(|e| net(format!("{e:#}")))
}

/// The newest stable release if it's newer than `current_version` and has an APK for `abi`
/// (`arm64-v8a`, `x86_64`, …). `api` points at another releases API (debug builds only).
#[uniffi::export]
pub fn check_app_update(current_version: String, abi: String, api: Option<String>) -> Result<Option<AppUpdate>, ClientError> {
    run(api, async |client| {
        let release = client.latest().await?;
        if !release.is_newer_than(&current_version) {
            return Ok(None);
        }
        Ok(release.asset(&android_asset(&release.version, &abi)).map(|a| AppUpdate {
            version: release.version.to_string(),
            url: release.url.clone(),
            size: a.size,
        }))
    })
}

/// Download the APK of `version` for `abi` to `dest`, keeping it only if it matches the
/// release's checksums.
#[uniffi::export]
pub fn download_app_update(version: String, abi: String, dest: String, api: Option<String>) -> Result<(), ClientError> {
    run(api, async |client| {
        let release = client.latest().await?;
        anyhow::ensure!(release.version.to_string() == version, "Mothr {version} is no longer the newest release");
        client.download_verified(&release, &android_asset(&release.version, &abi), std::path::Path::new(&dest)).await
    })
}
