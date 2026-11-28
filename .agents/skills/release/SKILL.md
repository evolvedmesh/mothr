---
name: release
description: How Mothr releases are cut (git flow: release and hotfix pull requests opened by the Prepare release workflow, stable releases on main, nightly dev builds of dev) with Linux x64 tarball, Windows x64 installer and zip, macOS app tarballs and disk images, and Android APK, and how to fix a failed or missing release. Use when preparing a release or hotfix, choosing a commit type, asking why no release happened, or debugging the release workflows.
---

# Releases

Git flow: `dev` holds unstable, untested work; `main` is stable and only changes through a
release or hotfix pull request. No version is ever committed to `dev` or `main` directly.

```
feature/* ──PR──▶ dev ──(nightly.yml)──▶ v0.3.0-dev.N   prerelease with every platform's files
                   │
        Prepare release (prepare-release.yml, by hand)
                   ▼
            release/0.3.0  ← one bot commit: scripts/set-version.sh 0.3.0
                   │ PR "chore: release 0.3.0" (description = release notes, edit them)
                   ▼
                 main ──(release.yml)──▶ tag v0.3.0 → draft → build → publish → merge main into dev
hotfix/0.2.2 (cut from main by Prepare release, kind hotfix) ──PR──▶ main  (same release)
```

## Workflows

- **prepare-release.yml** (Actions → Prepare release → Run workflow, or
  `gh workflow run prepare-release.yml [-f kind=hotfix] [-f version=x.y.z]`): as the release bot
  App (`RELEASE_APP_ID`, `RELEASE_APP_PRIVATE_KEY`, so the pull request starts the checks).
  - `release`: cuts `release/<x.y.z>` from `dev`. `scripts/changes.sh version` works out the
    version from the pull requests merged since the last release (see Versions) and
    fails if there's nothing to release.
  - `hotfix`: cuts `hotfix/<x.y.z>` from `main` with the next patch version. Open the fix pull
    requests against that branch.
  - Commits `chore: set version to <x.y.z>` (`scripts/set-version.sh`: `[workspace.package]
    version` in `Cargo.toml`, the `mothr*` entries in `Cargo.lock`, and through `Cargo.toml` the
    phone's `versionName`), pushes the branch and opens `chore: release <x.y.z>` into `main`.
    The description's "Release notes" section (between `<!-- notes:start -->` and
    `<!-- notes:end -->`) becomes the GitHub release's notes; edit it before merging. Left empty
    (hotfixes start that way), the notes are written from the commits on merge.
  - CI builds every platform on that pull request (`ci.yml` → `build`, the same `build.yml` the
    release uses), and `pr-checklist` lets the bot's release pull requests through without the
    local-build boxes.
- **release.yml** (every push to `main`, or by hand):
  1. **release**: reads the version from `Cargo.toml`. If `v<version>` is already published
     there's nothing to release (only sync-dev runs). Otherwise it takes the notes from the merged
     pull request (`commits/<sha>/pulls`), falls back to `scripts/changes.sh notes` over the
     pull request's branch, pushes an annotated tag (`kind: release|hotfix`, and for a release
     `cut: <sha>`, the `dev` commit it was cut from) and creates a **draft** release. Drafts because the repo has immutable
     releases: once published, a release takes no new files. It picks up where an earlier run
     stopped: a pushed tag or an unpublished draft is reused, so running Release again (or
     `gh run rerun <id> --failed`) finishes a half-done release.
  2. **build** (`build.yml`, below) from the tag.
  3. **publish** attaches the files and `SHA256SUMS`, then publishes the draft as latest. If a
     build failed, the draft stays unpublished (users see nothing): re-run the failed jobs and
     **publish** finishes it.
  4. **sync-dev** merges `main` into `dev` (a normal merge commit, `chore: merge main into dev`)
     as the release bot. `dev` gets the version change and the tag, so the next release branch
     doesn't conflict with `main` on the version lines and `dev` contains the new tag. If
     it doesn't merge cleanly it pushes `sync/main-into-dev-<sha>` (a copy of `main`) and opens a
     pull request from it into `dev`: conflicts are resolved on a pull request's head branch, and
     `main` and `dev` are both protected. Nothing runs on pushes to `dev`, so the merge starts no
     workflow. Then it merges `main` into every open `release/*` and `hotfix/*` branch whose
     version is above `main`'s (a hotfix shipped while a release was being prepared), keeping
     that branch's version, as one `chore: merge main into <branch>` commit. A real conflict
     leaves the branch alone with a warning (merge `main` into it by hand); a branch at or below
     the new release is flagged to be prepared again.
- **nightly.yml** (03:00 UTC daily, or by hand): if `dev` moved since the last dev build, tags
  `dev` as `v<next>-dev.<n>` (`<next>`: the version the next release would get, or the next patch
  if there's nothing releasable; `<n>` counts up), builds it with that version stamped in (not
  committed) and publishes a **prerelease** (not latest) with every file and `SHA256SUMS`. Notes
  list the changes since the previous dev build. Only the newest 7 dev builds keep their release
  (`KEEP`); tags stay. The stable channel never sees them: `/releases/latest` skips prereleases.
- **sync-guard.yml** (when a `sync/*` pull request into `dev` is merged): if it was squashed or
  rebased, records `main` as merged into `dev` (see Merging).
- **build.yml** (called by the three above): checks out `ref`, runs `scripts/set-version.sh
  <version>` (a no-op on a release tag), builds and uploads each platform's files as workflow
  artifacts for the caller's publish job:
   - `mothr_<v>_linux_x64.tar.gz`: `cargo build --release --locked -p mothr-app` on
     ubuntu-24.04 (glibc 2.39+), with the `.desktop` entry and license files.
   - `mothr_<v>_windows_x64.zip`: the same build on `windows-2025` (MSVC), one folder
     `mothr_<v>_windows_x64/` with `mothr.exe`, `LICENSE.txt` and `licenses/`. It must build on
     Windows: GPUI's build script compiles its HLSL shaders with the SDK's `fxc.exe` only on a
     Windows host, so a cross-compiled release has no shaders. `.cargo/config.toml` links the
     CRT statically (no VC++ redistributable needed); check with
     `llvm-objdump -p mothr.exe | grep "DLL Name"` that no `VCRUNTIME140`/`api-ms-win-crt-*`
     appears. Installed copies update from this zip, so keep its name and shape.
   - `mothr_<v>_windows_x64_setup.exe`: the installer, built from the same folder by
     `apps/desktop/windows/build-installer.ps1` (Inno Setup, pinned by version and SHA-256 in
     that script) from `apps/desktop/windows/mothr.iss`. Never change its `AppId`: Windows and
     the updater (`WINDOWS_UNINSTALL_KEY`) find the install by it.
   - `mothr_<v>_macos_<arch>.tar.gz` and `.dmg` (`arch`: `arm64` for Apple silicon, `x64` for
     Intel): both built on `macos-15` (Apple silicon; x64 cross-compiled with
     `--target x86_64-apple-darwin`) with `MACOSX_DEPLOYMENT_TARGET=11.0`. GPUI's build compiles
     its Metal shaders with Xcode's `metal`. `apps/desktop/macos/bundle.sh` wraps the binary in
     `Mothr.app` (Info.plist from `apps/desktop/macos/Info.plist`, icon `Mothr.icns` made from
     `icon.svg` by `make-icns.py`), signs it ad hoc (Apple silicon only runs signed code) and
     packs one folder `mothr_<v>_macos_<arch>/Mothr.app` into the tarball, and `Mothr.app` plus
     an Applications link into the disk image. It isn't Developer ID signed or notarized, so a
     browser-downloaded disk image needs **Open Anyway** once; `install.sh` (curl) doesn't.
     Installed copies update from the tarball, so keep its name and shape, and keep
     `CFBundleShortVersionString` in an XML Info.plist (the updater reads it).
   - `mothr_<v>_android_arm64-v8a.apk`: `assembleRelease -Pmothr.abis=arm64-v8a
     -Pmothr.versionCode=<build time in minutes>`, R8-minified, signed with the
     `ANDROID_KEYSTORE_*` secrets (the job fails without them). The versionCode is the build
     time, not a run number, so it keeps growing across stable and nightly builds: phones only
     update to a higher one.

## Versions

- `feat` → minor, `fix` / `perf` → patch, `feat!` or a `BREAKING CHANGE:` footer → major (that
  takes Mothr to 1.0.0), even below 1.0.0 (`cliff.toml` → `[bump]`). `docs`, `chore`, `ci`,
  `refactor`, `test`, `style`, `build` don't release and stay out of the notes.
- Stable tags are `v<x.y.z>`; dev builds `v<x.y.z>-dev.<n>`.
- Changes are counted **once per pull request**, whatever the merge method
  (`scripts/changes.sh`): it walks the branch's first-parent line, so a squash commit is one
  change, a "Merge pull request" commit counts as its PR title (`cliff.toml`'s
  `commit_preprocessors`), and the commits inside a merged branch ("wip", …) are never read.
  It counts from the `cut:` recorded in the newest release tag, not from the tag's ancestry,
  so a squash-merged release PR (whose commits aren't ancestors of its tag) isn't counted again.
  Older tags without a `cut:` count from where they meet `dev`. git-cliff only renders the
  notes; the bump rules live in `changes.sh` and match `cliff.toml`'s `[bump]`.
- Never edit `version` by hand: only the release branch's bot commit sets it. The Android
  `versionName` comes from `Cargo.toml` (`composeApp/build.gradle.kts`); the crates report it
  through `CARGO_PKG_VERSION` (e.g. ACP `clientInfo`). Older dev prereleases
  (`0.2.0-dev.1` … `0.2.1-dev.1`) came from semantic-release and have no files.

## Merging

- Feature and fix pull requests target `dev` (or a `release/*` / `hotfix/*` branch). Squash or
  merge commit, either works: the PR title (checked by `pr-title`) is what counts. Release and
  hotfix pull requests into `main` can be squashed or merged too; keep the default message.
- The bot's `sync/main-into-dev-*` pull requests should take a **merge commit**: a squash or
  rebase leaves `main` outside `dev`'s history, so the next release PR or sync would conflict
  again. GitHub can't restrict the merge method by head branch, so **sync-guard.yml** repairs
  it: after such a PR is squashed or rebased, it adds `git merge -s ours <sync head>` to `dev`
  (history only, `dev`'s files unchanged).
- Changes reach `dev` and `main` only through pull requests: admins' ruleset bypass is
  "pull requests only", and the required checks (`rust`, `pr-title`, `pr-checklist`) must pass or
  be explicitly bypassed in the merge dialog. The release bot pushes `release/*`, `hotfix/*` and
  `sync/*` branches, `v*` tags and sync-dev's merge into `dev`; it never pushes to `main`.
- Every commit that lands on `main`, `dev`, `mvp` or `patch/**` must be a Conventional Commit
  (org ruleset "conventional commits", regex on the first line, same as `pr-title`). The repo
  uses the **PR title** as the merge and squash commit title, so GitHub's merges pass; a squash
  merge also checks every commit on the branch, so reword "wip" commits before merging. GitHub's
  **Update branch**, **Resolve conflicts** and **Revert** buttons make "Merge branch …" /
  `Revert "…"` commits that the rule rejects: merge or revert locally with a conventional
  message instead. Only the release bot bypasses it.
- Never write the bracketed skip-ci marker in a commit message or PR title, even quoted: GitHub
  then skips every workflow for that commit, including a pull request's checks.
- `v*` tags are protected org-wide ("release tags" ruleset): only the release bot and org
  admins can create, move or delete them. A tag that belonged to an immutable release can never
  be reused, even after deleting the release.

## Updates

Users get releases these ways, all from the same GitHub release and all checked against its
`SHA256SUMS` (written by the publish job):

- `install.sh` (repo root, served from `main` via raw.githubusercontent.com): checks `uname`
  and installs the Linux or macOS build. On Linux it installs to
  `~/.local/lib/mothr/versions/<v>` with a `current` symlink and an `install.json` marker; on
  macOS it puts `Mothr.app` in `~/Applications` (picking `arm64` or `x64`, also from a Rosetta
  shell) and links `~/.local/bin/mothr` into it. It resolves "latest" from the
  `/releases/latest` redirect, so asset names must stay `mothr_<v>_linux_x64.tar.gz` and
  `mothr_<v>_macos_<arch>.tar.gz`. It runs under macOS's bash 3.2 with BSD tools: no arrays
  under `set -u`, no `mv -T`/`sed -i`/`readlink -f` outside the Linux-only function, and
  `shasum -a 256` where there's no `sha256sum`. Don't run its functions in `$(…)`: bash
  drops `set -e` there. Write `${version}…`, not `$version…`: macOS's bash 3.2 in the C
  locale takes the `…` bytes as part of the name (`version�: unbound variable`); the
  `bash:3.2` Docker image doesn't show this, only a Mac does.
- The desktop updater (`apps/desktop/src/update.rs` on `crates/mothr-update`): checks 20 s
  after start and every 6 h, and offers a newer release in the sidebar (`ui/workspace/frame.rs`)
  and Settings → Updates. Clicking stages the new version next to the running one, swaps
  `current` and restarts (or waits while chats work). "Install updates automatically" (off by
  default) stages it unprompted. Only copies with the marker update themselves.
- Windows (`mothr-update`'s `Layout::InPlace`): the setup installs per user into
  `%LOCALAPPDATA%\Programs\Mothr` with `install.json` (`{"installer":"setup","version":…}`)
  next to `mothr.exe`. An update downloads `mothr_<v>_windows_x64.zip`, renames the running
  `mothr.exe` to `mothr.exe.<n>.old` (Windows allows that), moves the new one in, and writes
  the version to `install.json` and the Apps & features entry. "Restart" starts the new exe
  with `MOTHR_WAIT_PID`, which waits for the old one to exit (`update::wait_for_previous`);
  each start deletes the `.old` files (`Install::clean_up`). The portable zip has no marker
  and only links the installer.
- macOS (`Layout::Bundle`): no marker. Any `<name>.app/Contents/MacOS/mothr` whose bundle and
  folder are writable updates itself (Mac apps are installed by copying), so a copy dragged
  from the disk image updates like one from `install.sh`; one on the disk image, in a
  read-only folder or translocated (opened from Downloads while quarantined) doesn't, and
  links the disk image. An update downloads `mothr_<v>_macos_<arch>.tar.gz`, unpacks it into
  `.mothr-update/` next to the bundle, moves the running bundle aside into it and the new one
  into its place. "Restart" waits for the old process and runs the bundle's binary (not
  `open`, which would drop Mothr's environment, e.g. `MOTHR_HOME`); each start
  deletes `.mothr-update/`. The installed version is the bundle's
  `CFBundleShortVersionString`.
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

- Installer, into a throwaway home. Unset the XDG variables, or the menu entry lands in the
  real `$XDG_DATA_HOME/applications` (and replaces a real install's):
  `env -u XDG_DATA_HOME -u XDG_CONFIG_HOME HOME=$(mktemp -d) MOTHR_RELEASES_URL=http://127.0.0.1:8799/releases bash install.sh`
- macOS installer path, from Linux: put `mothr_<v>_macos_arm64.tar.gz` (made with
  `apps/desktop/macos/bundle.sh <v> arm64 <any executable> <dir>`) in the mock release, and
  shims first on `PATH`: `uname` (`-s` → `Darwin`, `-m` → `arm64`), `sw_vers` (prints
  `15.1`) and `sysctl` (`hw.optional.arm64` → `1` to try a Rosetta shell with `uname -m` =
  `x86_64`). To check bash 3.2 compatibility, run the same in the `bash:3.2` Docker image
  (`--network host`, `apk add curl`). On a Mac, run it as is; `xattr` and `lsregister` only
  run there.
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
gh run list --workflow release.yml --limit 5     # also prepare-release.yml, nightly.yml
gh run view <id> --log-failed
```

| Symptom | Cause |
| --- | --- |
| Prepare release: "dev has no feat, fix or perf commits since v…" | nothing releasable on `dev`; expected. Pass `-f version=x.y.z` to release anyway |
| Prepare release: "release/x.y.z already exists" (or `hotfix/x.y.z`) | a release or hotfix branch with that version is open; merge or close its pull request and delete the branch |
| sync-dev: "main doesn't merge cleanly into release/x.y.z" | a hotfix and the release branch changed the same lines. `git switch release/x.y.z && git merge origin/main -m "chore: merge main into release/x.y.z"`, keep the branch's version (`scripts/set-version.sh x.y.z` if needed), push |
| sync-dev: "release/x.y.z isn't above the release on main" | a hotfix took that version or a newer one shipped. Close the pull request, delete the branch, run Prepare release again |
| the next release repeats already released changes / has a too-high version | the newest release tag lacks `cut:` (made by hand, or a release PR whose branch didn't come from Prepare release). Edit the notes in the release PR and pass `-f version=` to Prepare release |
| Release run: "v… is already released; nothing to release" | the push to `main` didn't change `Cargo.toml`'s version (not a release pull request); expected |
| push of the tag, a `release/*` branch or sync-dev's merge rejected | the App isn't a bypass actor on the tag/`dev` ruleset, or isn't installed on the repo |
| sync-dev: "main doesn't merge cleanly into dev" | it opened a pull request from `sync/main-into-dev-<sha>` into `dev`. Resolve the conflicts on that branch locally (`git switch sync/…`, `git merge origin/dev -m "chore: merge dev into sync/…"`, fix, `git commit --no-edit`, push), then merge it into `dev` with a merge commit before the next release. Not GitHub's **Resolve conflicts** or **Update branch** buttons: their "Merge branch …" commits break the commit message rule. Never resolve them on `main` |
| `cargoBuildAndroidArm64Release` fails reading `target/release/build/<crate>/<hash>/output` | Gobley 0.3.7 vs Cargo 1.100+'s new build-dir layout; the job pins `RUSTUP_TOOLCHAIN` 1.93.0. Keep it until Gobley supports the new layout |
| windows job: `Inno Setup … doesn't match its checksum` | the pinned download changed upstream; check the release on jrsoftware/issrc before updating `$InnoSha256` |
| windows job: `Failed to find fxc.exe` | the runner image lost the Windows SDK; set `GPUI_FXC_PATH` to an `fxc.exe` |
| `--locked` fails in the linux job | `Cargo.lock` out of date on the branch; run `cargo update -w` and commit |
| APK can't be installed over the previous one | signed with a different key (missing or rotated `ANDROID_KEYSTORE_*`) |
| a draft release sits unpublished | a build job (or `publish`) failed; fix and re-run the failed jobs (`gh run rerun <id> --failed`), `publish` uses `--clobber` and then publishes the draft |
| `HTTP 422: Cannot upload assets to an immutable release` | the release was published before `publish` ran (it must be created as a draft). It's locked and its tag can't be reused: fix the workflow and release the next patch version |
| a push to `main` started no Release run | the head commit message has a skip-ci marker; start it by hand: `gh workflow run release.yml --ref main` |
| no dev build last night | `dev` didn't move (the run says so), or the run failed: `gh run list --workflow nightly.yml` |

A failed release job after the tag was pushed leaves a tag without a GitHub release; delete the
tag (`git push origin :refs/tags/v<x>`) only if no release was published from it, then re-run.
Ask the user before deleting tags or releases.

## Before changing the workflow

- Runners are pinned (`ubuntu-24.04`); bump them on purpose.
- `NDK_VERSION` must match `ndkVersion` in `composeApp/build.gradle.kts`.
- The apt list in the linux job is the reference for GPUI's system libraries.
- CI's `windows` job (clippy for the Windows target, and compiling `mothr.iss` with placeholder
  files) and `macos` job (clippy on macOS, and `bundle.sh` with a stand-in binary) run only on
  pull requests into `main` (they take ~5 min), so a Windows- or macOS-only break in a feature
  PR shows up at the release pull request, or as a failed nightly build. Check before: `cargo xwin clippy` for Windows, and for
  either, run CI by hand on the branch (`gh workflow run ci.yml --ref <branch>`). They aren't
  required checks, but when one is red the next release's build for it fails too.

## Trying the Windows build on Linux (Wine)

A debug build cross-compiles with [cargo-xwin](https://github.com/rust-cross/cargo-xwin)
(`cargo install --locked cargo-xwin`, `rustup target add x86_64-pc-windows-msvc`; it downloads
the MSVC CRT and Windows SDK, accepting Microsoft's license with `XWIN_ACCEPT_LICENSE=1`):

```sh
XWIN_ACCEPT_LICENSE=1 cargo xwin build --target x86_64-pc-windows-msvc -p mothr-app
```

Debug builds compile GPUI's shaders at run time (from the crate's source path), so they run
under Wine; `cargo xwin clippy --target x86_64-pc-windows-msvc --workspace --all-targets`
lints the `cfg(windows)` code. Wine (10.x) lacks a few things real Windows has, so to see the
app run there, use a throwaway `WINEPREFIX` and work around them in the test setup only, never
in the shipped code:

| Wine gap | Test-only workaround |
| --- | --- |
| no `icuuc.dll` (GPUI imports `u_strlen`) | a stub DLL exporting `u_strlen` in the prefix's `system32` |
| DirectWrite `IDWriteFontCollection1::GetFontSet` and `IDWriteFontSet::GetMatchingFonts` are stubs: "failed to resolve font '.SystemUIFont'" | a patched copy of `gpui-pre-windows` (`direct_write.rs`: look fonts up with `FindFamilyName`/`GetFirstMatchingFont`), passed with `--config 'patch.crates-io.gpui-pre-windows.path="<copy>"'`; restore `Cargo.lock` afterwards |
| no DirectComposition | `GPUI_DISABLE_DIRECT_COMPOSITION=1` |
| its HLSL compiler rejects `StructuredBuffer` | `winetricks d3dcompiler_47` |
| no Direct Manipulation (touchpad) | make `DirectManipulationHandler::new` failures non-fatal in the same patched copy |

Run it headless with the `desktop-ui` skill's `ui-shot.sh` and `UI_SHOT_BIN` set to a wrapper
that does `MOTHR_HOME="$(winepath -w "$MOTHR_HOME")" exec wine target/x86_64-pc-windows-msvc/debug/mothr.exe`.
**Windows updates under Wine.** No installer needed: lay the install out as the setup does,
in the prefix's `drive_c/users/$USER/AppData/Local/Programs/Mothr/` (`mothr.exe`, `LICENSE.txt`,
`licenses/`, and `install.json` = `{"installer":"setup","version":"<its version>"}`), and add the
Apps & features entry with `wine reg add 'HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{0E591738-8A65-45C8-8C9B-61E1FE0ECC9E}_is1' /v DisplayVersion /d <version> /f`.
Build a second exe with a higher version (`scripts/set-version.sh 9.9.9`, build, copy the exe,
then set the version back) and serve it from `mock-releases.py` as
`9.9.9/mothr_9.9.9_windows_x64.zip` (one `mothr_9.9.9_windows_x64/` folder) with `SHA256SUMS`.
Start the installed exe through a wrapper that runs `wine <exe>` and then `wineserver -w`, so
gamescope outlives the restart, with `MOTHR_UPDATE_API=http://127.0.0.1:8799/api` and
`UI_SHOT_SETTINGS` holding `auto_update = true`. About 20 s later the log says "installed Mothr
9.9.9", `mothr.exe` is the new one, the old sits beside it as `mothr.exe.<n>.old`, and the
sidebar offers **Restart to update** (click it: the mouse works). After the restart, one
`mothr.exe` runs, the `.old` file is gone and Settings → Updates says 9.9.9.

Keyboard input injected into gamescope doesn't reach Wine windows (the mouse does), and opening
a URL from the app hands it to the host's browser. The terminal starts but Wine's PowerShell is
a stub, so it shows no prompt.
