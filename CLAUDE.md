# CLAUDE.md

Project instructions for Claude Code. The shared guidance for all coding agents is in
AGENTS.md and is imported here:

@AGENTS.md

## Claude Code specifics

- **CodeGraph first.** This repo has a `.codegraph/` index. Before grep/find/Read for code
  questions, call the `codegraph_explore` MCP tool with `projectPath` set to this repo (or run
  `codegraph explore "<query>"` in the shell). If `.codegraph/codegraph.db` is missing after a
  fresh clone, run `codegraph init -y`. The web client (`crates/mothr-remote/assets/index.html`)
  and Kotlin resources aren't covered well; grep those.
- **Skills.** `.claude/skills` links to `.agents/skills`: `run-mothr`, `desktop-ui` (headless
  gamescope screenshot loop), `protocol-change`, `agent-drivers`, `release`. Load the matching
  one before starting that kind of task.
- **Verify visually.** For any desktop UI change, run `desktop-ui`'s `ui-shot.sh`, read the
  PNG with the Read tool, and iterate until it matches the intent, in dark and light. Keep
  screenshots in the session scratchpad; they can show the user's repos.
- **Long builds go to the background.** A cold `cargo build`, any `--release` build, and every
  Gradle build take minutes: run them with `run_in_background` and wait for the notification.
- **Isolated profile, always.** Set `MOTHR_HOME` (a temp dir) for every launch, test and
  example; never start the app against the user's real profile.
- **Mothr drives Claude Code itself** (`crates/mothr-agents/src/claude.rs`). When changing that
  driver, `MOTHR_TRACE=<file>` records the raw stream-json protocol, and
  `cargo run -p mothr-agents --example drive -- claude_code <cwd> "<prompt>"` runs a headless
  turn. Real turns use the user's quota: keep prompts tiny, use a scratch `<cwd>`.
- **Input injection.** Never use `ydotool` or anything uinput-based: it types into the user's
  real session. Inside the headless gamescope use `xdotool` on its own `DISPLAY`; on the
  Android emulator use `adb shell input`.
- **Commits.** Conventional Commits with crate scopes (AGENTS.md → Git workflow); they decide
  the next release. Only commit or push when asked, and never on `main` directly.
