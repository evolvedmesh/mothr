# CLAUDE.md

Project instructions for Claude Code. The shared guidance for all coding agents is in
AGENTS.md and is imported here:

@AGENTS.md

## Claude Code specifics

- **CodeGraph first.** This repo has a `.codegraph/` index. Before grep/find/Read for code
  questions, call the `codegraph_explore` MCP tool with `projectPath` set to this repo (or run
  `codegraph explore "<query>"` in the shell). If `.codegraph/codegraph.db` is missing after a
  fresh clone, run `codegraph init -y`.
- Mothr drives Claude Code itself (`crates/mothr-agents/src/claude.rs`). When changing that
  driver, `MOTHR_TRACE=<file>` records the raw stream-json protocol for debugging, and
  `cargo run -p mothr-agents --example drive -- claude_code <cwd> "<prompt>"` runs a headless
  turn.
- Don't launch the app against the user's real profile for experiments; set `MOTHR_HOME`.
