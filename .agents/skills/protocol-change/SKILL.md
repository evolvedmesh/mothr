---
name: protocol-change
description: Checklist for adding or changing a SessionEvent, SessionCommand, AppCommand or HubEvent (or an Entry variant) in Mothr so every layer stays in sync - core reducer, JS web-client port, drivers, hub, desktop store, mothr-client and the Kotlin app, and persisted transcripts. Use whenever a change touches crates/mothr-core/src/session.rs or hub.rs.
---

# Change the session protocol

Four message types cross Mothr's layers, all in `crates/mothr-core`:

| Type | Direction | Defined in |
| --- | --- | --- |
| `SessionEvent` (`#[serde(tag = "event")]`) | driver → hub → every view | `session.rs` |
| `SessionCommand` (`#[serde(tag = "command")]`) | view → hub → driver | `session.rs` |
| `AppCommand` (`#[serde(tag = "type")]`) | view → hub (`HubApi::dispatch`) | `hub.rs` |
| `HubEvent` | hub → views (desktop `Store`, remote WS clients) | `hub.rs` |

They're serialized as JSON to the web client and the phone, and `SessionState` (built by the
reducer) is what transcripts on disk contain. A change that compiles in Rust can still break
the web client (plain JS) or old transcripts, so walk the whole list.

Start with `codegraph explore "<the type> <variant you're touching>"` to see every match arm.

## SessionEvent (or Entry)

1. **Core:** add the variant and handle it in `SessionState::apply` (`session.rs`). `apply`
   returns whether anything changed; keep that accurate. Add a unit test beside the existing
   ones in `session.rs`.
2. **Web client:** port the same semantics to `applyEvent` in
   `crates/mothr-remote/assets/index.html` ("JS port of mothr_core::SessionState::apply (keep
   identical semantics)"), and render it if it's visible. No test compares the two reducers.
3. **Driver(s):** emit it from `crates/mothr-agents/src/claude.rs` and/or `acp.rs`. If only
   one agent can produce it, that's fine; don't fake it for the others.
4. **Desktop:** render it in `apps/desktop/src/ui/chat.rs` (or the view that owns it). The
   `Store` already applies session events through the reducer.
5. **Phone:** if users should see it there, map it in `crates/mothr-client/src/model.rs`
   (`entry_view`) into a `views.rs` uniffi record/enum, then render it in
   `apps/mobile/composeApp/src/commonMain/.../ChatScreen.kt`. New uniffi types regenerate the
   Kotlin bindings on the next Gradle build.
6. **Demo hub:** if it helps testing, emit it from `crates/mothr-remote/examples/demo.rs`.

## SessionCommand

1. **Core:** add it to `SessionCommand`. Give optional fields `#[serde(default)]`.
2. **Drivers:** handle it in both `claude.rs` and `acp.rs`, or gate it behind an
   `AgentKind::supports_*` capability (`agent.rs`) and hide the control where unsupported.
3. **Hub:** check whether `mothr-hub/src/lib.rs` intercepts it (queueing, edit & resend,
   persistence) before forwarding to the driver.
4. **Callers:** desktop (`ChatView::dispatch` → `AppCommand::Session`), the web client's
   sender in `index.html`, and `crates/mothr-client/src/lib.rs` (a method the Kotlin
   `AppModel` calls).

## AppCommand

1. **Core:** add it to `AppCommand` in `hub.rs`.
2. **Hub:** handle it in `Hub::dispatch` (`crates/mothr-hub/src/lib.rs`). Return an error
   with a plain message for bad input; it reaches the user as a toast or a failed reply.
3. **Remote:** nothing to wire: `POST /api/command` deserializes `AppCommand` directly. Make
   sure that's acceptable for a remote caller (path checks: remote browsing can't leave a
   known repo).
4. **Callers:** desktop through `Store::dispatch`; web client / `mothr-client` if remote users
   need it.

## HubEvent

1. **Core:** add it to `HubEvent`, and to `HubSnapshot` too if a late joiner needs the
   current value (the desktop resyncs from a snapshot when it lags).
2. **Hub:** broadcast it.
3. **Consumers:** `apps/desktop/src/store.rs::on_event`, the web client's WS handler, and
   `crates/mothr-client/src/model.rs`.

## Persisted data

- Transcripts are `SessionState` as JSON in `sessions/<id>.json`; settings are `Settings` as
  TOML. Any new field on a persisted struct needs `#[serde(default)]`, so a file written by an
  older version still loads. `crates/mothr-hub/tests/workspaces.rs` shows the pattern: it
  writes a chat in the old shape, starts a hub and checks it loads.
- Renaming a variant or field breaks old files and older phone apps talking to a newer
  desktop. Prefer adding over renaming; if you must rename, accept the old name with
  `#[serde(alias = "...")]`.

## Verify

```sh
cargo test -p mothr-core           # reducer
cargo test -p mothr-remote -p mothr-client -p mothr-hub
cargo clippy --workspace --all-targets -- -D warnings
```

Then watch it end to end: the desktop app and the web client side by side (`run-mothr` skill)
on the same chat, and the phone app if you touched `mothr-client`.
