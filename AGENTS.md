# AGENTS.md

Guidance for coding agents (Claude Code, Codex, Copilot, OpenCode, Hermes, …) working in this
repository. Product overview and features are in [README.md](README.md); the visual language
is in [DESIGN.md](DESIGN.md); the phone app is in [apps/mobile/README.md](apps/mobile/README.md).

## What this is

Mothr is a Rust desktop app (GPUI via `gpui-kit`) that runs coding agents behind its own chat
UI. It talks to Claude Code over its native stream-json control protocol and to the other agents
(Codex via `codex-acp`, Copilot, OpenCode, Hermes) over the Agent Client Protocol (ACP). It
never embeds an agent's terminal UI. A remote-control server lets a browser (embedded web
client) and the Kotlin Multiplatform phone app drive the same chats.

Mothr is itself one of the agents' hosts: you may be running inside it. Never test against the
user's real profile; see "Running it" below.

## Finding your way around: use CodeGraph first

This repo is indexed by CodeGraph, a local code knowledge graph. The index lives in
`.codegraph/` and is **not committed** (only its `.gitignore` is). After cloning, build it once:

```sh
codegraph init -y      # build the index
codegraph sync         # refresh after pulling or large edits (the MCP server also syncs live)
```

Reach for CodeGraph **before** grep/find or opening files when you need to understand or locate
code:

- **MCP tool** (when your harness has it): `codegraph_explore` with `projectPath` set to this
  repo. One call returns the verbatim, line-numbered source of the relevant symbols plus the
  call paths between them, including trait/dynamic-dispatch hops grep can't follow.
- **Shell** (always works): `codegraph explore "<symbols or question>"`, e.g.
  `codegraph explore "Hub edit_message spawn_driver_at"` or
  `codegraph explore "how does a queued prompt get sent"`.
- `codegraph node <symbol>` for one symbol with its callers/callees;
  `codegraph status` to check the index is fresh.

Treat source shown by CodeGraph as already read. Fall back to grep/Read for non-code files
(TOML, JSON, Markdown, Kotlin resources, and above all `crates/mothr-remote/assets/index.html`,
the web client, which is one big HTML/JS file) or when the index is stale.

## Layout

Turborepo monorepo (bun workspaces: `apps/*`, `crates`) over a Cargo workspace
(`crates/*`, `apps/desktop`). `bun run build|test|lint|dev` fans out through turbo; each
workspace's `package.json` just calls cargo or gradle.

```
apps/desktop        mothr-app: GPUI desktop app, binary `mothr`
  src/main.rs         startup, dev hooks (MOTHR_OPEN, MOTHR_DEV_PROMPT, …)
  src/store.rs        GPUI-side mirror of the hub: applies HubEvents, dispatches AppCommands
  src/update.rs       in-app updates (Settings → Updates)
  src/ui/             views: workspace (shell), chat, sidebar, home, settings, changes, diff,
                      terminal, skills, projects/, design/, tokens.rs + theme.rs (design system)
apps/mobile         Kotlin Multiplatform + Compose phone app over mothr-client (see its README)
crates/
  mothr-core        domain model: agents (AgentKind, capabilities), sessions (Entry,
                    SessionEvent, SessionCommand, SessionState::apply), AppCommand/HubEvent/
                    HubApi (hub.rs), settings (config.rs), skills, workspaces, terminals
  mothr-agents      detection (detect.rs), drivers (claude.rs: stream-json; acp.rs: ACP
                    JSON-RPC), past chats (history.rs), ask_mcp.rs, policy.rs, edits.rs
  mothr-hub         orchestrator implementing HubApi: sessions, persistence, queue, edit &
                    resend, subagents, usage polling, skills
  mothr-projects    project discovery, fuzzy search (nucleo), pins/recents, repo browsing
  mothr-usage       account limits: Claude OAuth usage, Codex logs, Copilot quotas, Hermes
  mothr-design      Design-mode JSON DSL, loader, watcher, exporters, agent instructions
  mothr-skills      skill library: install from GitHub/git/links/folders, discovery (skills.sh)
  mothr-terminal    terminals beside chats: user's login shell on a pty, screens for remote
  mothr-remote      axum server (server.rs) + embedded web client (assets/index.html),
                    auth, tunnels (Tailscale, Dev Tunnels), QR
  mothr-client      remote-API client exported to Kotlin via UniFFI: model.rs (state +
                    entry_view), views.rs (flat uniffi records), pairing.rs, update.rs
  mothr-update      updates: newest GitHub release, SHA256SUMS-checked downloads, the Linux,
                    Windows and macOS install layouts (install.rs); used by the desktop app
                    and mothr-client
mothr-design/       Mothr's own design system written in its Design DSL (system.json)
install.sh          the public Linux and macOS installer (raw.githubusercontent.com/…/main/install.sh);
                    keep it to bash 3.2 and BSD tools (macOS's)
scripts/            set-version.sh (used by semantic-release)
.agents/skills/     task skills for agents working on this repo (.claude/skills links here)
```

## Invariants

- **One reducer.** Every view (hub, desktop, web client, phone via mothr-client) applies the
  same `SessionEvent`s through `SessionState::apply` (`crates/mothr-core/src/session.rs`).
  The web client has a hand-written JS port, `applyEvent` in `crates/mothr-remote/assets/index.html`.
  Change both together. No test compares them, so check the web client by hand.
- **Drivers only translate.** Agent-protocol specifics stay in `mothr-agents`; the hub and UI
  speak only `SessionCommand` / `SessionEvent` / `AppCommand` / `HubEvent`.
- **The hub is reached only through `HubApi`.** The desktop `Store` and the remote server both
  go through it; the server turns `POST /api/command` bodies into `AppCommand`s verbatim, so a
  new `AppCommand` is remote-callable as soon as the hub handles it.
- **Capabilities are explicit.** Per-agent features are gated by `AgentKind::supports_*`
  (`supports_edit`, `supports_steering`, `supports_remote_control`) and `supported_modes`.
  Don't branch on the agent kind in the UI for something a capability should say.
- **Windows is a release target.** Start child processes through
  `mothr_core::process::no_window` (tokio: `no_window(cmd.as_std_mut())`), or every agent,
  `git` or probe flashes a console window from the GUI app. Get the home directory from
  `mothr_core::config::home_dir()`, never `$HOME` (Windows doesn't set it), and gate
  Unix-only code with `cfg(unix)`.
- **macOS is a release target too.** The app ships as `Mothr.app` (`apps/desktop/macos/`), so
  it starts from Launchpad with launchd's minimal `PATH`; agent detection adds the usual bin
  folders (`detect.rs`). Keyboard shortcuts use `cmd` there (`k!` in `main.rs`). From Linux,
  macOS-only code can't be compiled; CI's `macos` job lints it (see the `release` skill).
- **Persisted data stays readable.** Transcripts (`<data>/sessions/<id>.json`) and
  `settings.toml` are serde of the core types. New fields get `#[serde(default)]` (or
  `skip_serializing_if`) so files from older versions still load; see
  `crates/mothr-hub/tests/workspaces.rs`, which loads a chat saved before workspaces existed.

## Where a change has to land

Most features cross layers. Before you call something done, walk its row:

| Change | Touch |
| --- | --- |
| New/changed `SessionEvent` | `session.rs` enum + `SessionState::apply` (+ unit test there); JS `applyEvent` in `index.html`; `mothr-client/src/model.rs` `entry_view` / `views.rs` if the phone shows it; desktop rendering in `ui/chat.rs`; driver(s) that emit it |
| New/changed `SessionCommand` | `session.rs`; both drivers (`claude.rs`, `acp.rs`), or a capability gate; desktop UI; web client sender; `mothr-client/src/lib.rs` method + Kotlin caller |
| New `AppCommand` | `hub.rs` enum; `Hub::dispatch` in `mothr-hub/src/lib.rs`; desktop caller via `Store::dispatch`; web client / `mothr-client` if remote users need it |
| New `HubEvent` | `hub.rs`; hub broadcast; `apps/desktop/src/store.rs::on_event`; web client handler; `mothr-client/src/model.rs` |
| New setting | `config.rs` `Settings` with a serde default; `ui/settings.rs`; a `config.rs` test if it has logic |
| New agent or agent capability | `agent.rs` (`AgentKind`, `ALL`, names, `supported_modes`, `supports_*`); `detect.rs` (binary candidates, `Transport`); logo in `apps/desktop/assets/logos` + `assets.rs` + `NOTICE.md`; the web client's `AGENTS` table in `index.html` (names, color, modes, mirrors `supported_modes`); `crates/mothr-remote/examples/demo.rs` |
| Design token / color | `ui/tokens.rs` (+ `theme.rs::tune` for component colors); `DESIGN.md`; `mothr-design/system.json`. The web client (`:root` CSS vars) and the phone (`Theme.kt`) have their own palettes; update them when the change should show there too |

## Building and checking

```sh
bun install                                   # once: Turborepo
bun run dev                                   # run the desktop app
bun run test                                  # every workspace's tests must pass
bun run lint                                  # clippy -D warnings (+ mobile lint)
cargo test --workspace                        # Rust only, without turbo
cargo test -p mothr-core                      # one crate (fast loop for reducer changes)
cargo clippy --workspace --all-targets        # keep it warning-free
```

- CI only lints. Required checks on `dev` and `main`: `rust` (clippy `-D warnings`,
  `.github/workflows/ci.yml`), `pr-title` (a lowercase Conventional Commit title) and
  `pr-checklist` (`pr-title.yml`). The apps are built only by the release on `main`, so before
  opening a PR build both locally and tick them in the template, or `pr-checklist` fails:
  `cargo build -p mothr-app`, `RUSTUP_TOOLCHAIN=1.93.0 ./gradlew :composeApp:assembleDebug`
  (in `apps/mobile`), `cargo test --workspace`, and clippy.
- Debug builds use `opt-level = 1` (dependencies at 3); release uses thin LTO and one codegen
  unit, so `cargo build --release` takes several minutes. Don't use it as an edit loop.
- Windows code paths compile only for a Windows target. From Linux, check them with
  `XWIN_ACCEPT_LICENSE=1 cargo xwin clippy --target x86_64-pc-windows-msvc --workspace --all-targets`
  (CI's `windows` job runs the same, but only on pull requests into `main`); the `release` skill
  has the Wine recipe.
- Linux needs the GPUI system libraries: Vulkan, xkbcommon (+x11), wayland, x11-xcb,
  fontconfig, freetype (the release workflow's apt list is the reference).
- **Mobile** needs JDK 17, the Android SDK and NDK r28. Gotchas:
  - Gobley 0.3.7 can't read Cargo 1.100+'s build-dir layout; build with
    `RUSTUP_TOOLCHAIN=1.93.0 ./gradlew …` (CI pins the same).
  - If both `ANDROID_HOME` and `ANDROID_SDK_ROOT` are set to different SDKs, AGP refuses to
    configure; unset one (`env -u ANDROID_SDK_ROOT ./gradlew …`).
  - Release builds are R8-minified. JNA and the UniFFI bindings (`dev.mothr.client`) are kept
    by `composeApp/proguard-rules.pro`; keep it in step if bindings move packages.

## Tests

- Unit tests sit next to the code (`#[cfg(test)]`); the reducer (`session.rs`), diffing
  (`diff.rs`), edit tracking (`agents/edits.rs`), skills and config have the most.
- Integration tests: `crates/mothr-hub/tests` (a real `Hub` on a throwaway `MOTHR_HOME`, one
  test per file because it sets the env var), `crates/mothr-remote/tests/server.rs` (HTTP/WS
  API), `crates/mothr-client/tests/client.rs` (client against a server).
- No test runs real agents. For a driver change, run the headless `drive` example against the
  real binary (see the `agent-drivers` skill) and say in your report which agents you tried.

## Running it

Always on an isolated profile: `MOTHR_HOME=$(mktemp -d) cargo run`. The agents keep using
their own logins (`~/.claude`, `~/.codex`, …). Developer hooks (`MOTHR_OPEN`,
`MOTHR_DEV_PROMPT`, `MOTHR_PREFILL`, `MOTHR_TRACE`, …) and examples (`drive`, `past`, `demo`,
`skills`, `export_design`) are listed in the README. The `run-mothr` skill has the full recipes
for the desktop app, the web client and the phone app on an emulator. For UI work, verify with
screenshots: `.agents/skills/desktop-ui/scripts/ui-shot.sh shot out.png MOTHR_OPEN=settings`
runs Mothr in a headless gamescope (nothing opens on screen) and writes a PNG to look at.

## UI rules

- Every size goes through `tokens::sz()` (UI scale), never raw `px()` for UI dimensions.
- Colors come from the palette (`tokens::p(cx)`); never hard-code hex in views. New component
  colors are mapped in `ui/theme.rs::tune`.
- Follow DESIGN.md: quiet chrome, one accent (violet) for live state and selection, monochrome
  primary controls, no gradients/glows/sparkles in the chrome, calm 120–320 ms motion.
- Agents keep their identity through their brand tint and logo, never through chrome.
- Keep copy short and plain ("Needs you", "Remote on"); name the real agent, command or path;
  never overstate state (DESIGN.md → Voice).

## Git workflow

- Branches: `main` (releases) ← `dev` (integration) ← feature branches. The initial MVP lives on
  `mvp`, branched from `dev`. Feature branches and PRs target `dev`. `dev` and `main` change only
  through pull requests (only the release bot pushes directly); after each merge into `main`,
  the release workflow merges `main` back into `dev`.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/):
  `feat(hub): …`, `fix(desktop): …`, `docs: …`, `refactor(agents): …`, `test(core): …`,
  `ci: …`, `chore: …`. The scope is the crate without the `mothr-` prefix (`desktop` and
  `mobile` for the apps); list several, comma-separated, when a change spans them.
- Releases are cut by semantic-release from those commit types (`.github/workflows/release.yml`,
  `release.config.cjs`): `feat` → minor, `fix`/`perf` → patch, `!`/`BREAKING CHANGE` → major.
  Don't bump versions by hand; it owns `version` in Cargo.toml (the Android app reads it from
  there). See the `release` skill.
- Don't commit `target/`, `.codegraph/` contents, `apps/mobile/**/build`, or anything from a
  real `MOTHR_HOME` (transcripts, settings, tokens). Pairing links carry a secret token.

## Skills for this repo

Task recipes live in `.agents/skills/` (Claude Code sees them through `.claude/skills`):

- `run-mothr`: launch the desktop app, web client or phone app on an isolated profile and check
  a change works.
- `protocol-change`: add or change a `SessionEvent`, `SessionCommand`, `AppCommand` or
  `HubEvent` across every layer.
- `agent-drivers`: debug or extend the Claude Code and ACP drivers, or add an agent.
- `desktop-ui`: build or restyle GPUI views with the design tokens, and check them through a
  headless gamescope screenshot loop (`scripts/ui-shot.sh`): look, fix, look again.
- `release`: how releases are cut, how users get updates (installer, desktop, phone), how to
  test updates locally, and how to fix a failed release run.
