---
name: release
description: How Mothr releases are cut (semantic-release on main and dev, Linux x64 tarball and Android APK) and how to fix a failed or missing release. Use when preparing a merge to dev/main, choosing a commit type, asking why no release happened, or debugging .github/workflows/release.yml.
---

# Releases

`.github/workflows/release.yml` runs on every push to `main` and `dev`:

1. **release** job: semantic-release (`.releaserc.json`) reads the Conventional Commits since
   the last `v*` tag, runs `scripts/set-version.sh <version>` (rewrites `[workspace.package]
   version` in `Cargo.toml` and the `mothr*` entries in `Cargo.lock`), commits
   `ci(release): <version> [skip ci]`, tags `v<version>` and publishes a GitHub release with
   notes. It runs as the release bot GitHub App (`RELEASE_APP_ID`, `RELEASE_APP_PRIVATE_KEY`)
   because only that App may push to the protected branches.
2. **linux** and **android** jobs (only on `main`, only when a release was published) build
   from the tag:
   - `mothr_<v>_linux_x64.tar.gz`: `cargo build --release --locked -p mothr-app` on
     ubuntu-24.04 (glibc 2.39+), with the `.desktop` entry and license files.
   - `mothr_<v>_android_arm64-v8a.apk`: `assembleRelease -Pmothr.abis=arm64-v8a
     -Pmothr.versionCode=<run number>`, R8-minified, signed with `ANDROID_KEYSTORE_*` secrets
     (debug key with a warning if they're missing).
3. **publish** attaches both to the release.
4. **sync-dev** (every push to `main`, after the release job) merges `main` into `dev`.

`dev` publishes prereleases (`0.2.0-dev.1`, …) with notes but no binaries, to save storage.

## Versions

- Mothr started at `0.1.0`. On the first run the workflow tags the root commit `v0.0.0` as a
  base, because semantic-release would otherwise start at 1.0.0.
- `feat` → minor, `fix` / `perf` → patch, `feat!` or a `BREAKING CHANGE:` footer → major (that
  takes Mothr to 1.0.0). `docs`, `chore`, `ci`, `refactor`, `test`, `style`, `build` don't
  release.
- Never edit `version` by hand; the next release overwrites it. The Android `versionName`
  comes from `Cargo.toml` (`composeApp/build.gradle.kts`); the crates report it through
  `CARGO_PKG_VERSION` (e.g. ACP `clientInfo`).

## Merging

- Merge PRs with a merge commit so semantic-release sees every commit. With squash merges, the
  PR title alone decides the release; make it a Conventional Commit.
- Changes reach `dev` and `main` only through pull requests: admins' ruleset bypass is
  "pull requests only", and the required checks (`rust`, `pr-title`, `pr-checklist`) must pass or
  be explicitly bypassed in the merge dialog. Only the release bot pushes directly.
- Flow: feature → PR into `dev` → PR `dev` into `main`. After every push to `main`, the
  `sync-dev` job merges `main` back into `dev` (a normal merge commit,
  `chore: merge main into dev [skip ci]`), so `dev` sees the new release tag. Without it, `dev`
  would compute a prerelease of a version that's already out.
- If `sync-dev` fails with "doesn't merge cleanly", open a PR from `main` into `dev` and resolve
  the conflict there.
- `v*` tags are protected org-wide ("release tags" ruleset): only the release bot and org
  admins can create, move or delete them.

## Updates

Users get releases three ways, all from the same GitHub release and all checked against its
`SHA256SUMS` (written by the publish job):

- `install.sh` (repo root, served from `main` via raw.githubusercontent.com): installs to
  `~/.local/lib/mothr/versions/<v>` with a `current` symlink and an `install.json` marker.
  It resolves "latest" from the `/releases/latest` redirect, so asset names must stay
  `mothr_<v>_linux_x64.tar.gz`.
- The desktop updater (`apps/desktop/src/update.rs` on `crates/mothr-update`): checks 20 s
  after start and every 6 h, and offers a newer release in the sidebar (`ui/workspace/frame.rs`)
  and Settings → Updates. Clicking stages the new version next to the running one, swaps
  `current` and restarts (or waits while chats work). "Install updates automatically" (off by
  default) stages it unprompted. Only copies with the marker update themselves.
- The phone (`AndroidUpdates.kt` via `mothr-client`'s `check_app_update` /
  `download_app_update`): downloads `mothr_<v>_android_<abi>.apk` and installs it with
  `PackageInstaller`. Phones only accept it when it's signed with the release key, so the
  android job fails without `ANDROID_KEYSTORE_*`. The key and its password live outside the
  repo with the maintainer (backed up); never regenerate it, or every installed phone app
  loses updates.

Renaming an asset, the tag format or the install layout breaks updates for everyone already
installed. Change them only together with a fallback in the updaters.

### Test updates locally

`scripts/mock-releases.py` stands in for GitHub:

```sh
R=$(mktemp -d); mkdir -p $R/0.1.1
# put mothr_0.1.1_linux_x64.tar.gz (one folder mothr_0.1.1_linux_x64/ inside) and/or
# mothr_0.1.1_android_<abi>.apk in $R/0.1.1, then:
(cd $R/0.1.1 && sha256sum -- * > SHA256SUMS); echo 0.1.1 > $R/latest
python3 -I .agents/skills/release/scripts/mock-releases.py $R 8799 &
```

- Installer, into a throwaway home:
  `HOME=$(mktemp -d) MOTHR_RELEASES_URL=http://127.0.0.1:8799/releases bash install.sh`
- Desktop updater: install an older version that way, then run it (e.g. with the `desktop-ui`
  skill's `ui-shot.sh`, `UI_SHOT_BIN=<home>/.local/lib/mothr/current/mothr`) with
  `MOTHR_UPDATE_API=http://127.0.0.1:8799/api`. The first check runs 20 s after start,
  then the sidebar offers the update. Seed `auto_update = true` (`UI_SHOT_SETTINGS`) to install
  without clicking; `readlink <root>/current` shows the switch.
- Phone (with "Download updates" off, the banner's **Update** downloads, then installs): run a second mock with `PUBLIC_BASE=http://10.0.2.2:<port>`, install an older debug
  APK, and point the debug build at it:
  `adb shell run-as dev.mothr.mobile sh -c 'mkdir -p files && echo http://10.0.2.2:<port>/api > files/update-api'`.
  The newer APK must be signed with the same key (debug builds share the debug key) and have a
  higher versionCode (`-Pmothr.versionCode=`). Build it with `scripts/set-version.sh <newer>`,
  and set the version back afterwards.

## When a run fails

```sh
gh run list --workflow release.yml --limit 5
gh run view <id> --log-failed
```

| Symptom | Cause |
| --- | --- |
| "There are no relevant changes, so no new version is released" | only non-releasing commit types since the last tag; expected |
| push of the release commit or tag rejected | the App isn't a bypass actor on the branch/tag ruleset, or isn't installed on the repo |
| `cargoBuildAndroidArm64Release` fails reading `target/release/build/<crate>/<hash>/output` | Gobley 0.3.7 vs Cargo 1.100+'s new build-dir layout; the job pins `RUSTUP_TOOLCHAIN` 1.93.0. Keep it until Gobley supports the new layout |
| `--locked` fails in the linux job | `Cargo.lock` out of date on the branch; run `cargo update -w` and commit |
| APK can't be installed over the previous one | signed with a different key (missing or rotated `ANDROID_KEYSTORE_*`) |
| release exists but has no assets | a build job failed after the release job; fix and re-run the failed jobs (`gh run rerun <id> --failed`), `publish` uses `--clobber` |

A failed release job after the tag was pushed leaves a tag without a GitHub release; delete the
tag (`git push origin :refs/tags/v<x>`) only if no release was published from it, then re-run.
Ask the user before deleting tags or releases.

## Before changing the workflow

- Runners are pinned (`ubuntu-24.04`); bump them on purpose.
- `NDK_VERSION` must match `ndkVersion` in `composeApp/build.gradle.kts`.
- The apt list in the linux job is the reference for GPUI's system libraries.
