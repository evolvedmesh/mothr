# Mothr

A native, GPU-rendered desktop manager for coding-agent chats. Mothr runs **Claude Code,
GitHub Copilot, Codex, OpenCode and Hermes** behind one interface of its own. It talks to each
agent over its machine protocol and never shows the agent's terminal UI.

Built in Rust with [GPUI](https://gpui.rs) via [`gpui-kit`](https://gpui-kit.com). It runs on
macOS, Windows and Linux (X11 and Wayland). Dark and light themes follow your system live, and the interface size can be set from 100%
to 145% for high-resolution screens. The
visual language is documented in [DESIGN.md](DESIGN.md); type is set in Geist, which is bundled
with the app.

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
  hints: Claude Code's commands and skills, and ACP agents' `available_commands`.
- **General chats.** These aren't tied to any project. They're for questions and quick tasks
  anywhere on the system, such as editing a config file. They run from your home directory.
  Start one with the globe button, the "General" chip, the first row of the project finder, or
  `Ctrl+Shift+G`.
- **Make it yours.** Pick an accent (eight presets or any hex color) and a surface tone
  (Graphite, Slate, Warm, or pure Black), put the sidebar on either side, move the bar to the top
  or bottom, and choose which bar items show and in what order: in Settings, or by
  right-clicking the bar. The bar shows where you are, a live count of chats working or waiting
  on you, and your limits. The phone app has its own theme, accent and tone.
- **Saved preferences**:
  - one app-wide default permission mode for every agent (an agent that lacks that mode falls
    back to its closest equivalent)
  - optionally, remember the last agent and model you used and start new chats with them
    (Settings → Defaults)
- **Smart grouping.** Chats are grouped as Pinned → Needs you → Working → your own groups → by
  recency. You can also group by project, agent or date. Unread dots, and toasts when a
  background chat needs you.
- **Fast fuzzy project finder** (`Ctrl+P`). Uses `nucleo` over a parallel scan of your project
  roots plus Claude/Codex history, with highlighted matches, git branch and stack tags. Projects
  can be pinned and recents are tracked.
- **Limits and usage**:
  - Claude 5-hour, weekly and per-model weekly windows (OAuth usage API, plus live
    `rate_limit_event`s)
  - Codex 5h and weekly (from rollout logs)
  - Copilot AI credits (AIC), also per chat
  - Hermes, via `hermes usage --json`
  - context-window usage and cost per chat
  - warnings at a threshold you configure
  - all of it lives behind one title-bar button: a sparkle plus a progress ring per agent
    account (with the agent's logo, turning amber/red near a limit). Hover it for a card with
    every account's windows and reset times; click to pin the card open.
- **Claude Remote Control.** A per-chat toggle that hands the session to claude.ai / the Claude
  app.
- **Mothr Remote.** Controls the whole app from a phone or another computer. It's a
  token-protected web client (installable as a PWA) with live streaming, approvals, answers and
  new chats. It can be reached on the LAN, directly over your **Tailscale** tailnet (no
  extra setup beyond being signed in), or through **Dev Tunnels**. The app shows a QR code for pairing.
- **Mothr mobile** (`apps/mobile`). A native Android/iOS remote built on the same Rust session
  model. It stays connected in the background and sends a notification when an agent needs a
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
- **Projects.** A page in the sidebar (Ctrl+Shift+P) listing every indexed project, pinned and
  recent first, with fuzzy search over names and paths. Open one to read it without leaving
  Mothr:
  - folders with their README rendered underneath; `.gitignore`d files and `.git` stay hidden
  - Markdown rendered for reading, with Preview / Source, links that open other files of the
    project, and relative images
  - code with highlighting and line numbers, images shown as pictures
  - **Full screen** (F11): the app sidebar steps aside for a tree of the whole project, a fuzzy
    **Go to file** (Ctrl+P), and an "On this page" outline for Markdown
  - pin the project or start a chat in it from its header

  The phone app has the same page: the folder button on the chat list. Only folders Mothr knows
  as projects can be browsed remotely, and paths can't leave the project.

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
  mothr-remote     axum server + embedded web client, tunnels, QR
  mothr-client     remote-API client shared with the mobile app (UniFFI)
mothr-design/      Mothr's own design system, written in its Design format
```

- **One state model, three places.** Every view of a session (hub, desktop, web) applies the same
  `SessionEvent`s through the same reducer (`SessionState::apply`). The web client uses a JS port
  of it, tested for equivalence.
- **Transports**:
  - Claude Code uses `claude -p --input-format stream-json --permission-prompt-tool stdio`. That
    gives permission prompts, rate-limit events, permission-mode and model switching, and Remote
    Control.
  - Copilot (`copilot --acp`), OpenCode (`opencode acp`), Hermes (`hermes acp`) and Codex (the
    `codex-acp` adapter) use the Agent Client Protocol.
- **Data locations.** Transcripts go to `~/.local/share/mothr/sessions/*.json` (or the
  platform equivalent) and settings to `~/.config/mothr/settings.toml`. Set `MOTHR_HOME`
  to relocate both. Mothr was called Shepherd before; on first launch it moves the old
  `shepherd` config and data folders over, and Design mode still reads a project's old
  `shepherd-design/` folder.

## Running

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

Linux needs the usual GPUI dependencies (Vulkan, xkbcommon, wayland/x11 dev libraries).

| Shortcut (Cmd on macOS) | Action                  |
| ----------------------- | ----------------------- |
| `Ctrl+N`                | New chat                |
| `Ctrl+Shift+N`          | New design              |
| `Ctrl+Shift+G`          | New general chat        |
| `Ctrl+P` / `Ctrl+K`     | Find project            |
| `Ctrl+B`                | Toggle sidebar          |
| `Ctrl+L`                | Focus composer          |
| `Ctrl+,`                | Settings                |
| `Enter` / `Shift+Enter` | Send / newline          |

### Developer hooks

- `MOTHR_HOME=<dir>`: isolated config and data.
- `MOTHR_OPEN=settings|remote|picker|usage|latest|changes|designs|design|subagent|projects`: open
  a view at startup (`MOTHR_OPEN_DELAY_MS` delays it). `changes` opens the newest chat's Changes
  panel; `design` opens the newest design. `project:<path>[:<file>[:full]]` opens a project, a
  file in it, and full screen. `image:<path>` opens the image viewer.
- `MOTHR_DESIGN_PAGE=overview|color|type|space|elevation|component:<n>|screen:<n>`: the page a
  design opens on. `MOTHR_DESIGN_EXPORT=menu|html|storybook` opens the export menu or exports.
- `MOTHR_DEV_REVIEW=accept|reject`: answer the first waiting edits from the Changes panel.
  `MOTHR_DIFF_MODE=split` starts the panel in split view.
- `MOTHR_TRACE=<file>`: append Claude Code's raw protocol lines to a file.
- `MOTHR_PREFILL=<text>`: type text into the open chat's composer at startup.
- `MOTHR_DEV_PROMPT="<agent_id>|<mode>|<model>|<chat|design>|<prompt>"`: start a real chat in
  the current directory.
- `MOTHR_EXPORT_DIR=<dir>`: Design exports skip the save dialogs and write into this folder.
- `cargo run -p mothr-app --example export_design -- <project> <out.html> [storybook-dir] [--build]`:
  export a project's design without opening the app.
- `cargo run -p mothr-agents --example drive -- <agent_id> <cwd> "<prompt>" [model]`: headless
  driver smoke test.
- `cargo run -p mothr-remote --example demo`: the remote web client against a fake hub.

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
