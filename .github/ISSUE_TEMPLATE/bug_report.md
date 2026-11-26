---
name: Bug report
about: Something in Mothr (desktop, phone app or web client) doesn't work as it should
title: ''
labels: bug
assignees: ''

---

**Describe the bug**
What went wrong, in a sentence or two.

**To reproduce**
1. Start a chat with '...' (agent, mode, model)
2. Do '...'
3. See '...'

**Expected behavior**
What you expected to happen instead.

**Where it happens**
- [ ] Desktop app
- [ ] Phone app (Android / iOS)
- [ ] Web client (remote, in a browser)

**Agent**
- Agent: [e.g. Claude Code, Codex, Copilot, OpenCode, Hermes]
- Agent version: [e.g. output of `claude --version`]
- Mode: [Ask, Accept edits, Auto or Plan]

**Environment**
- Mothr version: [e.g. 0.1.0, or the commit if built from source]
- OS: [e.g. Ubuntu 24.04, Arch]
- Desktop session: [Wayland / X11, compositor or DE]
- Phone: [e.g. Pixel 8, Android 16], if it's about the phone app
- Remote connection: [same Wi-Fi / Tailscale / Dev Tunnels], if it's about remote use

**Screenshots or logs**
Add screenshots if the bug is visible. For Claude Code protocol problems, run Mothr with
`MOTHR_TRACE=/tmp/mothr-trace.jsonl` and attach the relevant lines.

Don't paste pairing links, tokens, API keys or private transcripts. Pairing links carry a
secret token.

**Additional context**
Anything else that might help, e.g. whether it started after an update.
