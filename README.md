# Mothr

A native, GPU-rendered desktop app for coding-agent chats. Mothr runs **Claude Code, GitHub
Copilot, Codex, OpenCode and Hermes** behind one interface of its own. It talks to each agent
over its machine protocol and never shows the agent's terminal UI. A companion Android app and a
web client let you follow and answer your chats from your phone.

Built in Rust with [GPUI](https://gpui.rs) via [`gpui-kit`](https://gpui-kit.com). Mothr is
released for Linux (x86_64, X11 and Wayland) and Android. GPUI also runs on macOS and Windows,
but Mothr isn't built or tested there yet. Dark and light themes follow your system live, and
the interface size can be set from 100% to 145% for high-resolution screens. The visual
language is documented in [DESIGN.md](DESIGN.md); type is set in Geist, which is bundled with
the app.

## Install

| Platform | How | Updates |
| --- | --- | --- |
| **Linux** (x86_64) | the install script below | automatic, in the app |
| **Android** phone app | APK from the [latest release](https://github.com/evolvedmesh/mothr/releases/latest) | in the app |
| macOS, Windows, iOS | not released yet; see [Building from source](#building-from-source) | — |

Mothr runs the agents you already use. Install and sign in to at least one of
[Claude Code](https://docs.anthropic.com/en/docs/claude-code), GitHub Copilot CLI, Codex
(`codex-acp`), OpenCode or Hermes; Mothr finds them on its own.

### Linux

```sh
curl -fsSL https://raw.githubusercontent.com/evolvedmesh/mothr/main/install.sh | bash
```

That installs the newest release for your user, without `sudo`:

- the app in `~/.local/lib/mothr`, one folder per version
- the `mothr` command in `~/.local/bin`
- a Mothr entry in your application menu

The download is checked against the release's `SHA256SUMS` before anything is installed. To
read the script first:

```sh
curl -fsSLO https://raw.githubusercontent.com/evolvedmesh/mothr/main/install.sh
less install.sh
bash install.sh
```

**Requirements.** A glibc 2.39+ distribution (Ubuntu 24.04+, Debian 13+, Fedora 40+, Arch and
other rolling releases), a Wayland or X11 desktop, and a working Vulkan driver (Mesa or
NVIDIA). The installer tells you which packages to add if a library is missing.

**Options.** Pass them after `bash -s --`, e.g.
`curl -fsSL …/install.sh | bash -s -- --version 0.2.0`:

| Option | Does |
| --- | --- |
| `--version <x.y.z>` | install that release instead of the newest |
| `--dir <path>` | put the app somewhere else (default `~/.local/lib/mothr`, or `MOTHR_INSTALL_DIR`) |
| `--bin-dir <path>` | put the `mothr` command somewhere else (default `~/.local/bin`, or `MOTHR_BIN_DIR`) |
| `--no-menu-entry` | don't add Mothr to the application menu |
| `--uninstall` | remove Mothr; your chats and settings stay |

**Updates.** Mothr checks for a new release shortly after it starts and every 6 hours. When
there is one, **Update to Mothr x.y.z** appears at the bottom of the sidebar. Click it and
Mothr downloads the new version, checks it against the release's `SHA256SUMS`, installs it
next to the running one and restarts into it. If chats are still working, it waits and shows
**Restart to update** instead, so nothing is cut off. Settings → Updates has the same controls,
plus **Install updates automatically** (off by default): with it on, new versions download and
install in the background and the next start uses them. Running the install script again also
updates (or repairs) an install. Update checks only contact GitHub, and only send Mothr's
version (as the user agent).

**Uninstall.**

```sh
curl -fsSL https://raw.githubusercontent.com/evolvedmesh/mothr/main/install.sh | bash -s -- --uninstall
```

Chats and settings stay in `~/.config/mothr` and `~/.local/share/mothr`; delete those folders
to remove them too.

**Manual install.** Download `mothr_<version>_linux_x64.tar.gz` and `SHA256SUMS` from the
[latest release](https://github.com/evolvedmesh/mothr/releases/latest), check it with
`sha256sum -c --ignore-missing SHA256SUMS`, unpack it anywhere and run `./mothr`. A copy
installed this way doesn't update itself; Settings → Updates tells you when there's a new
version.

### Android

1. On the phone, open the [latest release](https://github.com/evolvedmesh/mothr/releases/latest)
   and download `mothr_<version>_android_arm64-v8a.apk`.
2. Open it and allow your browser to install apps when Android asks.
3. On the desktop, turn on Settings → Remote, then scan its QR code from the app. See
   [apps/mobile/README.md](apps/mobile/README.md#pairing) for Tailscale and other options.

**Updates.** The app checks for a new release when it starts and twice a day. When there is
one, **Update** appears above your chats: tap it and the app downloads the new version, checks
it against the release's `SHA256SUMS` and hands it to Android's installer. The first update
asks you to allow installs from Mothr and to confirm; on Android 12 and later, updates after
that usually install without asking again. The chat list's settings menu has **Check for
updates**, and **Download updates** (off by default) to download new versions in the
background so they're ready to install.

## Features

- **Agent detection.** Finds installed agents on `PATH` and in the usual install locations, and
  shows their versions. Copilot runs through its native binary directly, so a broken Node install
  doesn't break it.
- **One chat UI for every agent**:
  - streaming Markdown, collapsible thinking
  - tool cards with diffs and output
  - plan checklists, turn timing and cost
  - file and image attachments (picker or drag and drop)
  - model, reasoning-effort and context-window (200k, 1M) pickers, remembered for new chats
  - generated chat titles (a small Claude model names each chat when Claude Code is installed)
  - a Settings switch to hide the agent's reasoning text
- **Interactive agents.** Permission requests appear as inline Allow / Always allow / Deny cards.
  Agent questions (`AskUserQuestion`, ACP elicitation) appear as option chips with a free-text
  "Other" field. Plan approval (`ExitPlanMode`) can switch the chat to accept-edits.
- **Unified permission modes**: Ask, Accept edits, Auto, Plan and Allow all. Each is mapped onto
  the agent's own modes and also enforced on Mothr's side for ACP agents. Allow all is shown
  in red.
- **Orchestrator chats and subagents.** When an agent spawns subagents (Claude Code's
  Agent/Task tool), the chat is marked as an orchestrator:
  - the sidebar shows a running/total pill, with each subagent nested beneath it
  - the chat header shows a live subagent strip
  - every subagent opens as a normal chat with its own transcript; there you can approve its
    tool requests, answer its questions, or stop it
- **Working with messages**:
  - **Copy:** any message, from the hover actions.
  - **Edit & resend** (Claude Code): edit a message you sent. Mothr reverts the agent's file
    changes back to that point (Claude's file checkpoints), rewinds the conversation with a
    truncating resume, and sends the new text.
  - **Queue:** messages sent while the agent is busy wait in a visible queue and send
    themselves when the turn ends. Each queued item has "Send now" and remove.
  - **Keys:** Enter steers a running turn (Claude Code; other agents queue), Alt+Enter stops the
    turn and sends now, Ctrl+Shift+Enter queues, Shift+Enter inserts a newline.
  - **Drafts:** text typed in a new-chat box shows up under *Drafts* in the sidebar, so you can
    start another chat without losing it.
  - **Steer** (Claude Code): inject a message into the running turn; the agent reads it at its
    next step.
  - **Send now:** stops the current turn and sends immediately.
- **Slash commands.** Type `/` to list the agent's own commands, with descriptions and argument
  hints: Claude Code's commands and skills, and ACP agents' `available_commands`, plus every
  skill in Mothr's library (tagged *skill*).
- **Skills for every agent.** Mothr keeps one skill library (folders with a `SKILL.md`, in
  `<data>/skills`) and gives it to every agent, including ones without skill support of their
  own:
  - `/name` in any chat runs a skill: Mothr sends its instructions with what you typed after
    the name (`$ARGUMENTS` is filled in). An agent's own command of the same name wins.
  - With *Let agents pick a skill on their own* on (the default), new chats list the enabled
    skills in their instructions, so the agent reads one when a request fits it.
  - The **Skills** page lists what's installed (turn skills off, update them from their source,
    remove them) and adds more from a GitHub repo (`owner/repo`, `owner/repo@skill`, or a
    link to a folder), any git URL, a link to a `SKILL.md`, or a folder.
  - **Discover** searches the open skills index ([skills.sh](https://skills.sh)), browses
    popular collections (Anthropic, Vercel, Superpowers, OpenAI, Hugging Face, Microsoft,
    Awesome Copilot, Awesome Claude Skills), and lists skills your other agents keep on this
    computer (`~/.claude/skills`, `~/.agents/skills`, `~/.codex/skills`, `~/.copilot/skills`,
    OpenCode's) so you can share them with the rest. GitHub's API is used anonymously unless
    `GITHUB_TOKEN`/`GH_TOKEN` is set or the GitHub CLI is signed in.
- **Chats across several repos.** A chat runs in one primary repo and can work in more:
  add them under the new-chat box (any of them can be made primary there) or later from the
  `+N repos` button in the chat's header. Claude Code gets them as `--add-dir` folders (a
  running chat restarts on the same conversation to pick up a change); other agents are told
  about them with the next message.
- **Resume any agent's chats.** "Resume a chat" under the new-chat box (`Ctrl+Shift+R`)
  lists the conversations every installed agent keeps on its own, newest first, with search
  and an agent filter. They include ones started in the agent's CLI, another app, or a Mothr
  chat since deleted. Pick one and it opens as a Mothr chat with its history, and the agent
  picks the conversation up:
  - Claude Code's are read from `~/.claude/projects`, history included.
  - ACP agents (Copilot, Hermes, OpenCode, codex-acp) are asked with `session/list` and replay
    their history on `session/load`.
  - Codex rollouts in `~/.codex/sessions` are read directly when the adapter can't list them.

  A conversation that's already a Mothr chat just opens that chat.
- **A terminal beside every chat.** `` Ctrl+` `` (or `Ctrl+~`), or the terminal button in a chat's
  header or under the new-chat box, opens your own shell in the chat's primary repo. It starts
  your account's login shell, so your profile, rc files and prompt load. It works for a chat
  you haven't started yet too, and carries over to the chat once you send:
  - dock it top, right, bottom or left (the menu in its header, or Settings), and drag the
    divider to size it; a side dock moves below the chat while the window is too narrow
  - keys go to the shell (`Ctrl+P`, `Ctrl+R` and friends included); copy and paste with
    `Ctrl+Shift+C` / `Ctrl+Shift+V`, select with the mouse, scroll back with the wheel or
    `Shift+PageUp`
  - hiding it keeps the shell running; the hub owns it, so the web client and the phone show
    and type into the same shell
- **General chats.** These aren't tied to any repo. They're for questions and quick tasks
  anywhere on the system, such as editing a config file. They run from your home directory.
  Start one with the globe button, the "General" chip, the first row of the repo finder, or
  `Ctrl+Shift+G`.
- **Make it yours.** Pick an accent (eight presets or any hex color) and a surface tone
  (Graphite, Slate, Warm, or pure Black), and put the sidebar on either side. The phone app has
  its own theme, accent and tone.
- **No title bar.** The sidebar is the window's frame: its header finds repos and goes
  back/forward, a live row counts chats working or waiting on you, a new Mothr version shows up
  above the footer, and the footer holds your limits, Mothr Remote, light/dark and Settings. Collapsed (Ctrl+B) it's a rail of the same
  controls.
- **Saved preferences**:
  - one app-wide default permission mode for every agent (an agent that lacks that mode falls
    back to its closest equivalent)
  - optionally, remember the last agent and model you used and start new chats with them
    (Settings → Defaults)
- **Workspaces.** Groups of chats you switch between, like a browser's spaces. Each keeps its
  own chat list and remembers the chat it had open. Up to ten, shown as dots at the foot of the
  sidebar (the open one is a pill, one waiting on you turns amber):
  - switch with `Ctrl+Alt+←/→` (`⌘⌥←/→` on macOS), jump with `Ctrl+1` … `Ctrl+0`, or click a dot
  - the **Workspaces** page (`Ctrl+Shift+W`) shows every workspace and its chats; make, rename,
    give an icon, reorder and delete them there (a deleted workspace's chats move to the first)
  - right-click a chat to move it to another workspace; new chats start in the open one
- **Smart grouping.** Chats are grouped as Pinned → Needs you → Working → your own groups → by
  recency. You can also group by repo, agent or date. Unread dots, and toasts when a
  background chat needs you.
- **Fast fuzzy repo finder** (`Ctrl+P`). Uses `nucleo` over a parallel scan of your repo
  roots plus Claude/Codex history, with highlighted matches, git branch and stack tags. Repos
  can be pinned and recents are tracked.
- **Limits and usage**:
  - Claude 5-hour, weekly and per-model weekly windows (OAuth usage API, plus live
    `rate_limit_event`s)
  - Codex 5h and weekly (from rollout logs)
  - Copilot AI credits (AIC), also per chat
  - Hermes, via `hermes usage --json`
  - context-window usage and cost per chat
  - warnings at a threshold you configure
  - all of it lives behind one button at the foot of the sidebar: a progress ring per agent
    account (with the agent's logo, turning amber/red near a limit). Hover it for a card with
    every account's windows and reset times; click to pin the card open.
- **Claude Remote Control.** A per-chat toggle that hands the session to claude.ai / the Claude
  app.
- **Mothr Remote.** Controls the whole app from a phone or another computer. It's a
  token-protected web client (installable as a PWA) with live streaming, approvals, answers,
  new chats and each chat's terminal (your shell on the computer, in the chat's repo). It can
  be reached on the LAN, directly over your **Tailscale** tailnet (no extra setup beyond being
  signed in), or through **Dev Tunnels**. The app shows a QR code for pairing.
- **Mothr mobile** (`apps/mobile`). A native remote built on the same Rust session model,
  released for Android (the iOS app builds from source on a Mac). It stays connected in the background and sends a notification when an agent needs a
  permission or an answer. Allow, Deny or an answer can be tapped right on the notification.
- **Review agent changes.** Every file edit shows as a real line diff with syntax highlighting.
  The **Changes** panel (the `+N −M` button in a chat's header) lists all of a chat's edits,
  including its subagents', in unified or split view.
  - When the permission mode asks before editing (**Ask**), the edit appears as a review card:
    nothing is written until you **Accept** it, and **Reject** tells the agent no.
    **Accept all edits** switches Claude Code to Accept edits for the rest of the chat.
  - In modes that write without asking (**Accept edits**, **Auto**, **Allow all**), changes are
    shown as already applied.
- **Design.** A page of its own in the sidebar, holding every design system your agents have
  made. Agents write a design system and mockup screens as JSON in `mothr-design/`, and Mothr
  renders them natively and live. Open one to browse it like a storybook:
  - an overview, then color, type, spacing, radius and shadow tokens
  - each component with its variants, preview backgrounds and JSON source
  - device screens that fit the window, with zoom
  - the agent's chat alongside, with hot reload on every edit
  - **Export** (top right):
    - **HTML:** one self-contained file with every screen, the design system page and
      navigation between them. Fonts and icons are embedded, so it opens offline and can be sent
      as is.
    - **Storybook:** a Storybook 10 project with a story per component variant, per screen and
      for the tokens. If Bun or Node is installed, Mothr also builds it to `storybook-static/`, a
      static site you can host anywhere.

    Both use the same renderer, ported from the canvas, so exports match what you see in the app.
- **Repos.** A page in the sidebar (Ctrl+Shift+P) listing every indexed repo, pinned and
  recent first, with fuzzy search over names and paths. Open one to read it without leaving
  Mothr:
  - folders with their README rendered underneath; `.gitignore`d files and `.git` stay hidden
  - Markdown rendered for reading, with Preview / Source, links that open other files of the
    repo, and relative images
  - code with highlighting and line numbers, images shown as pictures
  - **Full screen** (F11): the app sidebar steps aside for a tree of the whole repo, a fuzzy
    **Go to file** (Ctrl+P), and an "On this page" outline for Markdown
  - pin the repo or start a chat in it from its header

  The phone app has the same page: the folder button on the chat list. Only folders Mothr knows
  as repos can be browsed remotely, and paths can't leave the repo.

## Shortcuts

| Shortcut (Cmd on macOS) | Action                  |
| ----------------------- | ----------------------- |
| `Ctrl+N`                | New chat                |
| `Ctrl+Shift+N`          | New design              |
| `Ctrl+Shift+G`          | New general chat        |
| `Ctrl+Shift+R`          | Resume an agent's chat  |
| `` Ctrl+` `` / `Ctrl+~` | Show/hide the terminal  |
| `Ctrl+P` / `Ctrl+K`     | Find repo               |
| `Ctrl+B`                | Toggle sidebar          |
| `Ctrl+L`                | Focus composer          |
| `Ctrl+,`                | Settings                |
| `Enter` / `Shift+Enter` | Send / newline          |

## Repository layout

A monorepo managed with [Turborepo](https://turborepo.com) (bun workspaces) on top of a Cargo
workspace:

```
apps/
  desktop          mothr-app: the GPUI desktop app (binary: `mothr`)
  mobile           Kotlin Multiplatform + Compose app (Android, iOS): a native remote for Mothr
crates/
  mothr-core       domain model: agents, sessions, transcript entries, events + reducer,
                   usage model, settings, the HubApi trait
  mothr-agents     detection + drivers: Claude stream-json control protocol, ACP JSON-RPC
  mothr-hub        orchestrator: sessions, persistence, usage polling, discovery; implements HubApi
  mothr-projects   project discovery, fuzzy search, pins/recents
  mothr-usage      account limits for Claude / Codex / Copilot / Hermes
  mothr-design     Design-mode document model, loader, file watcher, exporters, agent instructions
  mothr-skills     skill library: installs from GitHub, git, links and folders; skills.sh search
  mothr-terminal   terminals beside chats: the user's shell on a pty (alacritty_terminal), shared
                   by the desktop view and remote clients
  mothr-remote     axum server + embedded web client, tunnels, QR
  mothr-client     remote-API client shared with the mobile app (UniFFI)
  mothr-update     updates: newest GitHub release, checksum-verified downloads, Linux install layout
mothr-design/      Mothr's own design system, written in its Design format
install.sh         the Linux installer (see Install)
scripts/           release helpers (set-version.sh)
```

- **One state model everywhere.** Every view of a session (hub, desktop, phone through
  `mothr-client`, web client) applies the same `SessionEvent`s through the same reducer
  (`SessionState::apply`). The web client runs a hand-written JS port of it, kept in step by
  hand.
- **Transports**:
  - Claude Code uses `claude -p --input-format stream-json --permission-prompt-tool stdio`. That
    gives permission prompts, rate-limit events, permission-mode and model switching, and Remote
    Control.
  - Copilot (`copilot --acp`), OpenCode (`opencode acp`), Hermes (`hermes acp`) and Codex (the
    `codex-acp` adapter) use the Agent Client Protocol.
- **Data locations.** Transcripts go to `~/.local/share/mothr/sessions/*.json` (or the
  platform equivalent) and settings to `~/.config/mothr/settings.toml`. Set `MOTHR_HOME`
  to relocate both. Mothr was called Shepherd before; on first launch it moves the old
  `shepherd` config and data folders over, and Design mode still reads a repo's old
  `shepherd-design/` folder.

## Building from source

```sh
bun install                    # once: installs Turborepo
bun run dev                    # launches the desktop app (cargo run -p mothr-app)
bun run build                  # builds every app (turbo run build)
bun run test                   # all tests: Rust workspace + mobile
bun run lint                   # clippy -D warnings + mobile lint
```

Plain Cargo works too: `cargo run --release` builds and launches `mothr`, and
`cargo test --workspace` runs the Rust tests. The mobile app's own build steps are in
[apps/mobile/README.md](apps/mobile/README.md).

Linux needs Rust (stable) and GPUI's system libraries. On Ubuntu/Debian:

```sh
sudo apt install clang pkg-config libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev libvulkan-dev
```

macOS and Windows builds aren't set up or tested yet.

### Releases

Every push to `main` or `dev` runs `.github/workflows/release.yml`. semantic-release reads the
[Conventional Commits](https://www.conventionalcommits.org/) since the last release, sets the
version with `scripts/set-version.sh` (the Cargo workspace, and through it the Android app),
then tags and publishes a GitHub release. Versions start at `0.1.0`; `dev` publishes
prereleases (`0.1.0-dev.1`, …). Stable releases on `main` also attach the Linux x64 desktop
build (`mothr_<version>_linux_x64.tar.gz`) and the R8-optimized Android APK
(`mothr_<version>_android_arm64-v8a.apk`) plus `SHA256SUMS`, which `install.sh` and both in-app
updaters (`crates/mothr-update`) check every download against, and `main` is merged back into
`dev` afterwards. Pull requests into `dev` and `main` must pass a lint-only CI (clippy, PR title,
and the template's ticked "both apps compile" checklist); the apps themselves are built only
for releases.
The release workflow's header lists the secrets it needs.

### Developer hooks

- `MOTHR_HOME=<dir>`: isolated config and data.
- `MOTHR_OPEN=settings|remote|picker|resume|terminal|usage|latest|changes|designs|design|subagent|projects|workspaces|skills`:
  open a view at startup (`MOTHR_OPEN_DELAY_MS` delays it). `resume:<agent id>:<session id>`
  resumes that past chat. `changes` opens the newest chat's Changes
  panel; `design` opens the newest design. `project:<path>[:<file>[:full]]` opens a repo, a
  file in it, and full screen. `image:<path>` opens the image viewer. `skills:discover[:<owner/repo>]`
  opens Discover on the Skills page, browsing a popular repo; `skills:search:<query>` searches.
- `MOTHR_DESIGN_PAGE=overview|color|type|space|elevation|component:<n>|screen:<n>`: the page a
  design opens on. `MOTHR_DESIGN_EXPORT=menu|html|storybook` opens the export menu or exports.
- `MOTHR_DEV_REVIEW=accept|reject`: answer the first waiting edits from the Changes panel.
  `MOTHR_DIFF_MODE=split` starts the panel in split view.
- `MOTHR_TRACE=<file>`: append every agent's raw protocol lines (Claude Code and ACP) to a file.
- `MOTHR_PREFILL=<text>`: type text into the open chat's composer at startup.
- `MOTHR_DEV_PROMPT="<agent_id>|<mode>|<model>|<chat|design>|<prompt>"`: start a real chat in
  the current directory.
- `MOTHR_EXPORT_DIR=<dir>`: Design exports skip the save dialogs and write into this folder.
- `cargo run -p mothr-app --example export_design -- <project> <out.html> [storybook-dir] [--build]`:
  export a repo's design without opening the app.
- `cargo run -p mothr-agents --example drive -- <agent_id> <cwd> "<prompt>" [model]`: headless
  driver smoke test. `DRIVE_RESUME=<agent session id>` resumes that conversation, showing its
  history.
- `cargo run -p mothr-agents --example past -- <agent_id> [session id]`: list an agent's past
  chats as the resume picker sees them (with a Claude Code session id, print its history).
- `cargo run -p mothr-remote --example demo`: the remote web client against a fake hub.
- `cargo run -p mothr-skills --example skills -- search|browse|add|local [arg]`: the skill library
  from the shell (use a throwaway `MOTHR_HOME`).

## Known gaps

- Verification so far:
  - **Claude Code and Copilot** were verified end to end: streaming, tools, permissions,
    questions, usage.
  - **Hermes** connects, creates sessions, and lists its models and modes. A full turn wasn't
    verified because its configured model endpoint wasn't responding on the development machine.
  - **Codex and OpenCode** weren't installed there, so they're untested against real binaries.
- Edit & resend and Steer need Claude Code. Other agents can queue messages or interrupt and
  send, but they can't rewind or take input mid-turn.
- Subagent tracking needs Claude Code. The ACP agents don't expose subagents in a form Mothr
  reads yet.
- The ACP driver doesn't offer client-side terminals (`terminal: false`). Agents run commands
  themselves.
- Design mode renders a JSON DSL rather than HTML/React. Exports are HTML and Storybook (static
  mocks); export to framework code (React, SwiftUI…) is not implemented yet.

## License

Mothr is released under the [MIT License](LICENSE). Bundled third-party assets keep their own
licenses: the Geist fonts (SIL OFL), Lucide icons (ISC) and the agent logos from Lobe Icons
(MIT; the marks are their owners' trademarks); see the notices next to each.
