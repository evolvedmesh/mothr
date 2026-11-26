## Description

<!-- What this changes and why. -->

Fixes #<!-- issue number, closes it on merge -->

<!--
The PR title becomes the commit message on merge and drives the release: use a Conventional
Commit, e.g. `feat(hub): …`, `fix(desktop): …`, `docs: …`. The scope is the crate without the
`mothr-` prefix (`desktop` / `mobile` for the apps). `feat` releases a minor version, `fix` and
`perf` a patch; mark breaking changes with `!` or a `BREAKING CHANGE:` footer.
Feature branches target `dev`.
-->

## Type of change

<!-- Delete what doesn't apply. -->
- [ ] Bug fix
- [ ] New feature
- [ ] Breaking change
- [ ] Refactor (no behavior change)
- [ ] Documentation
- [ ] CI / build

## Where it applies

- [ ] Desktop app (`apps/desktop`)
- [ ] Phone app (`apps/mobile`)
- [ ] Web client (`crates/mothr-remote/assets`)
- [ ] Agent drivers (`crates/mothr-agents`)
- [ ] Hub, core or other crates

## How has this been tested?

<!-- What you ran and what you checked by hand. Say which agents you tried it with. -->

- [ ] `bun run test`
- [ ] `bun run lint` (clippy `-D warnings` + mobile lint)
- [ ] Ran the app against an isolated profile (`MOTHR_HOME=$(mktemp -d) cargo run`)

## Screenshots

<!-- For UI changes: before / after, light and dark if colors changed. -->

## Checklist

- [ ] Changes to `SessionEvent` / `SessionState::apply` are mirrored in the web client's JS reducer
- [ ] Protocol specifics stay in `mothr-agents`; per-agent features are gated by `AgentKind::supports_*`
- [ ] UI sizes go through `tokens::sz()` and colors through the palette; follows DESIGN.md
- [ ] Docs updated (README, AGENTS.md, apps/mobile/README.md) where behavior or setup changed
- [ ] No `target/`, `.codegraph/` or real `MOTHR_HOME` data (transcripts, settings, tokens) committed
