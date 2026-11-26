# Security policy

## Reporting a vulnerability

Please report security problems **privately**, not in a public issue or pull request:

1. Open the repository's **Security** tab.
2. Choose **Report a vulnerability** and describe the problem.

Only the maintainers see the report. Include what you found, how to reproduce it, which
version of Mothr (Settings → Updates shows it) and which platform (Linux desktop, Android, web
client), and what an attacker could do with it. We'll confirm we got it, keep you posted while
we fix it, and credit you in the release notes unless you'd rather not be named.

Please don't test against other people's computers or pairing links, and don't share details
publicly until a fixed release is out.

## Supported versions

Only the **latest release** gets security fixes. The desktop app and the phone app tell you
when a new release is out (Settings → Updates on the desktop, the banner above your chats on
the phone).

## What's in scope

Mothr runs coding agents on your computer and can be controlled remotely, so these matter
most:

- **Mothr Remote**: the server that lets the phone app and the web client control the desktop.
  For example, reaching it without the pairing token, leaking the token, or reaching it from
  outside the network or tailnet it was set to.
- **Pairing**: links and QR codes carry a secret token; anything that exposes or weakens it.
- **Agent permissions**: anything that lets an agent run a command or edit a file that the
  chat's permission mode should have stopped or asked about.
- **Repo browsing over Remote**: reading files outside the repos Mothr knows.
- **Installing and updating**: `install.sh` and the desktop and phone updaters, which only
  install files that match the release's `SHA256SUMS` (and, on Android, the release signing
  key). For example, getting them to install something else.

Out of scope: problems in the agents themselves (Claude Code, Copilot, Codex, OpenCode,
Hermes; report those to their makers), and anything that needs someone to already control
your account or computer.
