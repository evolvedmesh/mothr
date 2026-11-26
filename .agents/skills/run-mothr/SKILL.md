---
name: run-mothr
description: Launch Mothr to see a change working - the desktop app on an isolated profile, the web client and phone app against the demo hub, or an Android build on an emulator. Use when asked to run, start, try or screenshot Mothr, or before reporting a UI or remote change as done.
---

# Run Mothr

Never run against the user's real profile (`~/.config/mothr`, `~/.local/share/mothr`): it holds
their chats, settings and remote tokens. Every recipe below uses a throwaway `MOTHR_HOME`.
Agents still use their own logins (`~/.claude`, `~/.codex`, …), so real chats work.

Builds take a while (first debug build several minutes, Gradle + Rust for Android a few
more). Run them in the background and wait for them to finish; don't poll.

## Desktop app

```sh
export MOTHR_HOME=$(mktemp -d)
cargo run                       # binary `mothr` (default member: apps/desktop)
```

Jump straight to what you changed instead of clicking there:

| Hook | Effect |
| --- | --- |
| `MOTHR_OPEN=settings\|remote\|picker\|resume\|terminal\|usage\|latest\|changes\|designs\|design\|subagent\|projects\|workspaces\|skills` | open a view at startup (`MOTHR_OPEN_DELAY_MS` delays it) |
| `MOTHR_OPEN=project:<path>[:<file>[:full]]`, `image:<path>`, `resume:<agent>:<session id>`, `skills:discover[:<owner/repo>]`, `skills:search:<q>` | deeper targets |
| `MOTHR_DEV_PROMPT="<agent_id>\|<mode>\|<model>\|<chat\|design>\|<prompt>"` | start a real chat in the current directory |
| `MOTHR_PREFILL=<text>` | type into the open chat's composer |
| `MOTHR_DEV_REVIEW=accept\|reject`, `MOTHR_DIFF_MODE=split` | drive the Changes panel |
| `MOTHR_DESIGN_PAGE=…`, `MOTHR_DESIGN_EXPORT=menu\|html\|storybook`, `MOTHR_EXPORT_DIR=<dir>` | Design mode |
| `MOTHR_TRACE=<file>` | append every agent protocol line (Claude Code and ACP) to a file |

Agent ids: `claude_code`, `codex`, `copilot`, `open_code`, `hermes`. Modes: `ask`,
`accept_edits`, `auto`, `plan`, `allow_all` (not every agent has every mode; see
`AgentKind::supported_modes`).

To check a visual change, use the headless screenshot loop in the `desktop-ui` skill
(`.agents/skills/desktop-ui/scripts/ui-shot.sh shot out.png MOTHR_OPEN=…`): it runs Mothr in a
headless gamescope, so nothing opens on the user's screen, and hands you a PNG to look at.
Without gamescope, run the app as above and use the platform's screenshot tool. Check light
and dark if you touched colors.

## Web client and phone, without real agents

The demo hub is a fake `HubApi` with canned chats; no agents or desktop app needed:

```sh
MOTHR_HOME=$(mktemp -d) MOTHR_DEMO_TOKEN=devtoken-devtoken-devtoken MOTHR_DEMO_PORT=7979 \
  cargo run -p mothr-remote --example demo
```

With `MOTHR_HOME` set, the "Wire up remote control" chat's picture is written there and served
(without it the chat shows only the image's name). That chat also reports an unlisted model id
with reasoning levels on the "default" entry, like Claude Code does.

If the phone app is already installed from a release, `adb install -r` refuses the debug build
(`INSTALL_FAILED_VERSION_DOWNGRADE`); add `-d`.

- Web client: open `http://127.0.0.1:7979/?token=devtoken-devtoken-devtoken`. It is
  `crates/mothr-remote/assets/index.html`, compiled into the server with `include_str!`, so
  restart the example after editing it.
- Make an agent stop for you: start a chat whose prompt contains "permission" or "question";
  add "later" to delay it 8 s (time to background the phone app and see the notification):

  ```sh
  curl -H 'Authorization: Bearer devtoken-devtoken-devtoken' -H 'content-type: application/json' \
    http://127.0.0.1:7979/api/command \
    -d '{"type":"new_session","agent":"codex","kind":"chat","project":"/tmp/x","mode":"ask","prompt":"Pick a db, question later"}'
  ```

## Phone app on an Android emulator

```sh
cd apps/mobile
# Gobley 0.3.7 can't read Cargo 1.100+'s build dir; unset ANDROID_SDK_ROOT if it points
# at a different SDK than ANDROID_HOME.
RUSTUP_TOOLCHAIN=1.93.0 env -u ANDROID_SDK_ROOT ./gradlew :composeApp:installDebug --console=plain
```

- Start an emulator first if none is running: `emulator -list-avds`, then
  `emulator -avd <name> -no-window -no-audio -no-snapshot-save` (background), and wait with
  `adb wait-for-device shell 'while [ "$(getprop sys.boot_completed)" != 1 ]; do sleep 2; done'`.
- To test the R8 release build instead: `assembleRelease -Pmothr.abis=x86_64` (emulator ABI),
  then `adb install -r composeApp/build/outputs/apk/release/composeApp-release.apk`.
- Launch: `adb shell am start -W -n dev.mothr.mobile/.MainActivity`.
- Pair with the demo hub, which the emulator reaches at `10.0.2.2`. There is no deep link, so
  drive the UI with adb:
  1. `adb shell uiautomator dump /sdcard/ui.xml && adb shell cat /sdcard/ui.xml` lists
     Compose nodes with `text` and `bounds`.
  2. Tap the `EditText` under "Pairing link" (`adb shell input tap X Y`), then
     `adb shell input text 'http://10.0.2.2:7979/?token=devtoken-devtoken-devtoken'`.
  3. Dump again (closing the keyboard moves the layout) and tap "Connect".
  4. The chat list ("Chats", "Needs you", …) means the Rust client is connected.
- Crashes: `adb logcat -d | grep -E 'FATAL|AndroidRuntime|UnsatisfiedLink|NoSuchMethod|ClassNotFound'`.
  A missing class or method in a release build means R8 stripped it; fix
  `composeApp/proguard-rules.pro`.

## Headless agent turn

```sh
cargo run -p mothr-agents --example drive -- claude_code "$PWD" "List the files here" [model]
```

See the `agent-drivers` skill for its env knobs.

## Clean up

Stop what you started: the app, the demo hub, the emulator (`adb emu kill`). Delete the
throwaway `MOTHR_HOME` if you made one.
