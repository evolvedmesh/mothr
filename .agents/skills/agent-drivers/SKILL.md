---
name: agent-drivers
description: Debug or extend Mothr's agent drivers (Claude Code stream-json in claude.rs, ACP JSON-RPC in acp.rs), trace their raw protocol, run headless turns, list past chats, or add a new agent or capability. Use for bugs where an agent misbehaves in Mothr, protocol changes in an agent CLI, or adding agent support.
---

# Agent drivers

Everything protocol-specific lives in `crates/mothr-agents`. A driver owns one agent process
and translates its wire protocol to `SessionEvent`s and from `SessionCommand`s. The hub and UI
never see the protocol.

| File | What it does |
| --- | --- |
| `detect.rs` | finds each agent's binary (PATH + known install dirs) and picks its `Transport` |
| `claude.rs` | Claude Code over `claude -p --input-format stream-json --output-format stream-json --permission-prompt-tool stdio …` |
| `acp.rs` | ACP JSON-RPC 2.0 over stdio for Codex (`codex-acp`), Copilot (`copilot --acp`), OpenCode (`opencode acp`), Hermes (`hermes acp`); works on `serde_json::Value` to tolerate per-agent schema differences |
| `ask_mcp.rs` | a loopback MCP server registered with ACP agents in `session/new`, giving them a question tool (ACP has none) |
| `policy.rs` | Mothr's permission policy on top of what each agent supports (per `PermissionMode`) |
| `edits.rs` | turns Claude Code's edit tool calls into `FileDiff`s for the Changes panel |
| `history.rs` | past chats: Claude Code's JSONL under `~/.claude/projects/<dir>/`; ACP `session/list` / `session/load` |

## See the raw protocol

`MOTHR_TRACE=<file>` appends every line in and out of the agent (both drivers) to a file:

```sh
MOTHR_TRACE=/tmp/mothr-trace.jsonl MOTHR_HOME=$(mktemp -d) cargo run
```

Reproduce the bug, then read the trace next to the `SessionEvent`s you expected. Most driver
bugs are a protocol message the driver didn't expect (a new field, a renamed type, a different
ordering) rather than Mothr logic.

## Run a turn without the UI

```sh
cargo run -p mothr-agents --example drive -- <agent_id> <cwd> "<prompt>" [model]
```

It prints the detected install and every `SessionEvent`. Knobs:

- `DRIVE_PERMISSION=allow|always|reject`: how permission requests are answered (default allow).
- `DRIVE_SET_MODE=<mode id>` (`allow_all`, `accept_edits`, …): switch to that mode when the
  first permission request arrives instead of answering it; the driver should answer the
  requests the new mode allows. Ask for a command the agent asks about (`touch x.txt`).
- `DRIVE_ANSWER_DELAY=<secs>`: answer late, to exercise timeouts.
- `DRIVE_RESUME=<agent session id>`: resume a conversation and show its history.
- `DRIVE_ATTACH=<image>`, `DRIVE_EFFORT=…`, `DRIVE_CONTEXT=…`.

Past chats as the resume picker sees them:
`cargo run -p mothr-agents --example past -- <agent_id> [session id]`.

Agent ids: `claude_code`, `codex`, `copilot`, `open_code`, `hermes`. Use a scratch directory as
`<cwd>`, never the user's repo, if the prompt makes the agent edit files. Combine with
`MOTHR_TRACE` to capture both sides.

Real agents cost the user's quota. Keep prompts tiny and don't loop them.

## Changing a driver

- Keep the hub/UI contract: emit existing `SessionEvent`s where they fit. If you need a new
  one, follow the `protocol-change` skill.
- Fix a protocol difference for every agent it affects. In `acp.rs`, prefer tolerant reads
  (`v["x"].as_str().unwrap_or(…)`) over strict structs, as the rest of the file does.
- A feature only some agents have gets a capability on `AgentKind` (`supports_edit`,
  `supports_steering`, `supports_remote_control`, `supported_modes`), and the UI checks the
  capability, not the kind.
- In your report, say which agents you ran it against. Claude Code and Copilot have been
  verified end to end; Codex and OpenCode haven't been run against real binaries (README →
  Known gaps).

## Adding an agent

1. `crates/mothr-core/src/agent.rs`: the `AgentKind` variant, `ALL`, `id`/`from_id`,
   `display_name`, `label`, `supported_modes`, `supports_*`.
2. `crates/mothr-agents/src/detect.rs`: binary names and install dirs, and the `Transport`
   (usually `Transport::Acp { program, args, env }`).
3. `history.rs`, if it can list past chats.
4. Logo: `apps/desktop/assets/logos/<id>.svg`, registered in `apps/desktop/src/assets.rs`,
   credited in `assets/logos/NOTICE.md`. Brand tint in DESIGN.md → Principles.
5. Web client: the `AGENTS` table in `crates/mothr-remote/assets/index.html` (name, short
   name, letter, color, modes; modes must match `supported_modes`), and `PROVIDER_AGENT` if it
   has usage limits.
6. Usage limits, if any: a provider in `crates/mothr-usage`.
7. `crates/mothr-remote/examples/demo.rs` and the config tests that enumerate agents.

Then `cargo test --workspace` and a `drive` run against the real binary.
