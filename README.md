# Shepherd

A native, GPU-rendered desktop manager for coding-agent chats. Shepherd runs **Claude Code,
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
  - model switching
- **Interactive agents.** Permission requests appear as inline Allow / Always allow / Deny cards.
  Agent questions (`AskUserQuestion`, ACP elicitation) appear as option chips with a free-text
  "Other" field. Plan approval (`ExitPlanMode`) can switch the chat to accept-edits.
- **Unified permission modes**: Ask, Accept edits, Auto, Plan and Allow all. Each is mapped onto
  the agent's own modes and also enforced on Shepherd's side for ACP agents. Allow all is shown
  in red.
- **Orchestrator chats and subagents.** When an agent spawns subagents (Claude Code's
  Agent/Task tool), the chat is marked as an orchestrator:
  - the sidebar shows a running/total pill, with each subagent nested beneath it
  - the chat header shows a live subagent strip
  - every subagent opens as a normal chat with its own transcript; there you can approve its
    tool requests, answer its questions, or stop it
- **Working with messages**:
  - **Copy:** any message, from the hover actions.
  - **Edit & resend** (Claude Code): edit a message you sent. Shepherd reverts the agent's file
    changes back to that point (Claude's file checkpoints), rewinds the conversation with a
    truncating resume, and sends the new text.
  - **Queue:** messages sent while the agent is busy wait in a visible queue and send
    themselves when the turn ends. Each queued item has "Send now" and remove.
  - **Steer** (Claude Code): inject a message into the running turn; the agent reads it at its
    next step.
  - **Send now:** stops the current turn and sends immediately.
- **Slash commands.** Type `/` to list the agent's own commands, with descriptions and argument
  hints: Claude Code's commands and skills, and ACP agents' `available_commands`.
- **General chats.** These aren't tied to any project. They're for questions and quick tasks
  anywhere on the system, such as editing a config file. They run from your home directory.
  Start one with the globe button, the "General" chip, the first row of the project finder, or
  `Ctrl+Shift+G`.
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
  - Copilot AI credits (AIC)
  - Hermes, via `hermes usage --json`
  - context-window usage and cost per chat
  - warnings at a threshold you configure
- **Claude Remote Control.** A per-chat toggle that hands the session to claude.ai / the Claude
  app.
- **Shepherd Remote.** Controls the whole app from a phone or another computer. It's a
  token-protected web client (installable as a PWA) with live streaming, approvals, answers and
  new chats. It can be exposed on the LAN, on your **Tailscale** tailnet, publicly through
  **Tailscale Funnel**, or through **Dev Tunnels**. The app shows a QR code for pairing.
- **Design mode.** A chat variant where the agent writes a design system and mockup screens as
  JSON in `shepherd-design/`. Shepherd renders them natively and live:
  - color, type, spacing, radius and shadow tokens
  - a component gallery with variants
  - device artboards with zoom
  - hot reload on every agent edit

## Architecture

```
crates/
  shepherd-core      domain model: agents, sessions, transcript entries, events + reducer,
                     usage model, settings, the HubApi trait
  shepherd-agents    detection + drivers: Claude stream-json control protocol, ACP JSON-RPC
  shepherd-hub       orchestrator: sessions, persistence, usage polling, discovery; implements HubApi
  shepherd-projects  project discovery, fuzzy search, pins/recents
  shepherd-usage     account limits for Claude / Codex / Copilot / Hermes
  shepherd-design    Design-mode document model, loader, file watcher, agent instructions
  shepherd-remote    axum server + embedded web client, tunnels, QR
  shepherd-app       the GPUI desktop app (binary: `shepherd`)
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
- **Data locations.** Transcripts go to `~/.local/share/shepherd/sessions/*.json` (or the
  platform equivalent) and settings to `~/.config/shepherd/settings.toml`. Set `SHEPHERD_HOME`
  to relocate both.

## Running

```sh
cargo run --release            # builds and launches `shepherd`
cargo test --workspace         # 66 tests
```

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

- `SHEPHERD_HOME=<dir>`: isolated config and data.
- `SHEPHERD_OPEN=settings|remote|picker|usage|latest|design|subagent`: open a view at startup
  (`SHEPHERD_OPEN_DELAY_MS` delays it).
- `SHEPHERD_TRACE=<file>`: append Claude Code's raw protocol lines to a file.
- `SHEPHERD_PREFILL=<text>`: type text into the open chat's composer at startup.
- `SHEPHERD_DEV_PROMPT="<agent_id>|<mode>|<model>|<chat|design>|<prompt>"`: start a real chat in
  the current directory.
- `cargo run -p shepherd-agents --example drive -- <agent_id> <cwd> "<prompt>" [model]`: headless
  driver smoke test.
- `cargo run -p shepherd-remote --example demo`: the remote web client against a fake hub.

## Known gaps

- Verification so far:
  - **Claude Code and Copilot** were verified end to end: streaming, tools, permissions,
    questions, usage.
  - **Hermes** connects, creates sessions, and lists its models and modes. A full turn wasn't
    verified because its configured model endpoint wasn't responding on the development machine.
  - **Codex and OpenCode** weren't installed there, so they're untested against real binaries.
- Edit & resend and Steer need Claude Code. Other agents can queue messages or interrupt and
  send, but they can't rewind or take input mid-turn.
- Subagent tracking needs Claude Code. The ACP agents don't expose subagents in a form Shepherd
  reads yet.
- The ACP driver doesn't offer client-side terminals (`terminal: false`). Agents run commands
  themselves.
- Design mode renders a JSON DSL rather than HTML/React. Export to code is not implemented yet.
