# AGENTS.md

Guidance for coding agents (Claude Code, Codex, Copilot, OpenCode, Hermes, …) working in this
repository. Product overview and features are in [README.md](README.md); the visual language
is in [DESIGN.md](DESIGN.md).

## What this is

Mothr is a Rust desktop app (GPUI via `gpui-kit`) that runs coding agents behind its own chat
UI. It talks to Claude Code over its native stream-json control protocol and to the other agents
over the Agent Client Protocol (ACP). It never embeds an agent's terminal UI.

## Finding your way around: use CodeGraph first

This repo is indexed by CodeGraph, a local code knowledge graph. The index lives
in `.codegraph/` and is **not committed** (only its `.gitignore` is). After cloning, build it
once:

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
(TOML, JSON, Markdown, HTML assets) or when the index is stale.

## Layout

Turborepo monorepo (bun workspaces: `apps/*`, `crates`) over a Cargo workspace
(`crates/*`, `apps/desktop`). `bun run build|test|lint|dev` fans out through turbo; each
workspace's `package.json` just calls cargo or gradle.

```
apps/desktop        mothr-app: GPUI desktop app, binary `mothr`; UI in src/ui/
apps/mobile         Kotlin Multiplatform + Compose remote app (see its README)
crates/
  mothr-core        domain model: agents, sessions, entries, SessionEvent + reducer
                    (SessionState::apply), settings, HubApi trait
  mothr-agents      agent detection + drivers (claude.rs: stream-json; acp.rs: ACP JSON-RPC)
  mothr-hub         orchestrator: sessions, persistence, queue, edit & resend, subagents, usage
  mothr-projects    project discovery, fuzzy search (nucleo), pins/recents
  mothr-usage       account limits: Claude OAuth usage, Codex logs, Copilot quotas, Hermes
  mothr-design      Design-mode JSON DSL, loader, watcher, exporters, agent instructions
  mothr-skills      skill library: install from GitHub/git/links/folders, discovery (skills.sh)
  mothr-terminal    terminals beside chats: user's login shell on a pty, screens for remote
  mothr-remote      axum server + embedded web client (assets/index.html), tunnels, QR
  mothr-client      remote-API client exported to Kotlin via UniFFI (mobile app)
mothr-design/       Mothr's own design system written in its Design DSL
```

Key invariants:

- **One reducer.** Every view (hub, desktop, web client, mobile via mothr-client) applies the same `SessionEvent`s through
  `SessionState::apply`. The web client has a JS port of it; change both together.
- **Drivers only translate.** Agent-protocol specifics stay in `mothr-agents`; the hub and UI
  speak only `SessionCommand` / `SessionEvent`.
- **Capabilities are explicit.** Per-agent features are gated by `AgentKind::supports_*`
  (`supports_edit`, `supports_steering`, `supports_remote_control`) and `supported_modes`.

## Building and checking

```sh
bun install                                   # once: Turborepo
bun run dev                                   # run the desktop app
bun run test                                  # every workspace's tests must pass
bun run lint                                  # clippy -D warnings (+ mobile lint)
cargo test --workspace                        # Rust only, without turbo
cargo clippy --workspace --all-targets        # keep it warning-free
```

The mobile app needs JDK 17 and the Android SDK/NDK; see apps/mobile/README.md.

Linux needs the usual GPUI system libraries (Vulkan, xkbcommon, wayland/x11 dev packages).

Run against an isolated profile so you don't touch real chats or settings:
`MOTHR_HOME=$(mktemp -d) cargo run`. Other developer hooks (`MOTHR_OPEN`,
`MOTHR_DEV_PROMPT`, `MOTHR_TRACE`, …) are listed in the README.

## UI rules

- Every size goes through `tokens::sz()` (UI scale), never raw `px()` for UI dimensions.
- Colors come from the palette (`tokens::p(cx)`); never hard-code hex in views. New component
  colors are mapped in `ui/theme.rs::tune`.
- Follow DESIGN.md: quiet chrome, one accent (violet) for live state and selection, monochrome
  primary controls, no gradients/glows/sparkles in the chrome, calm 120–320 ms motion.
- Keep copy short and plain; name the real agent, command or path.

## Git workflow

- Branches: `main` (releases) ← `dev` (integration) ← feature branches. The initial MVP lives on
  `mvp`, branched from `dev`.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/):
  `feat(hub): …`, `fix(app): …`, `docs: …`, `refactor(agents): …`, `test(core): …`,
  `chore: …`. Use the crate name without the `mothr-` prefix as the scope (`desktop` and
  `mobile` for the apps).
- Don't commit `target/`, `.codegraph/` contents, or anything from a real `MOTHR_HOME`
  (transcripts, settings, tokens).
