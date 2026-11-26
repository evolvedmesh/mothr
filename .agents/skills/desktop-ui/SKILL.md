---
name: desktop-ui
description: Build, restyle or fix Mothr's GPUI desktop views (apps/desktop/src/ui) with the design tokens, and check them visually through a headless gamescope screenshot loop (look, fix, look again) without touching the user's screen. Use for any change to how the desktop app looks or lays out, and to verify UI work before calling it done.
---

# Desktop UI

## Where things are

| | |
| --- | --- |
| `ui/tokens.rs` | the design system: `Palette` (dark + light, per tone), `sz()` (UI scale, default 1.15), type scale, radii, spacing, motion. Mirrors `DESIGN.md` and `mothr-design/system.json` |
| `ui/theme.rs` | applies tokens to gpui-kit's component theme (`tune`), bundled fonts, light/dark |
| `ui/kit.rs` | shared pieces: `icon("name")` (Lucide), `agent_avatar`, `agent_logo`, `status_dot`, `pulse_dot`, `meter`, `section_label`, `empty_state`, `segment`, `appear`, `kbd_hint` |
| `ui/workspace.rs` | the window shell: sidebar + current page, navigation, dev-hook targets |
| `ui/chat.rs`, `sidebar.rs`, `home.rs`, `settings.rs`, `changes.rs`, `diff.rs`, `terminal.rs`, `skills.rs`, `workspaces.rs`, `projects/`, `design/` | the pages |
| `store.rs` | the views' data: mirrors hub state, `dispatch(AppCommand)` |

Reuse what's in `kit.rs` and gpui-kit's components before writing a new element. Find
similar UI first: `codegraph explore "<a view that already does this>"`.

## Rules

- Sizes: `sz(n)`, never `px(n)`, so Settings → Interface size scales everything.
- Colors: `let pal = tokens::p(cx);` then `pal.surface`, `pal.text_muted`, `pal.brand`, …
  Never a hex literal in a view. A new semantic color goes into `Palette` (both appearances,
  every tone) and, for gpui-kit components, `theme.rs::tune`.
- DESIGN.md decides the look: quiet chrome (surfaces a few percent apart, hairline borders),
  violet only for live state, selection and the next step, agents' tints only on their avatar
  and selected agent cards, no gradients/glows/sparkles in the chrome, motion 120/200/320 ms
  ease-out, nothing bounces. Read its Components table before designing a card or row.
- Copy: short and plain, the real agent/command/path, never overstate state (DESIGN.md → Voice).
- Dark is the primary look, but light must be equally right: check both.
- If the change should also show on the web client (`crates/mothr-remote/assets/index.html`,
  CSS vars in `:root`) or the phone (`apps/mobile/.../Theme.kt`, Compose screens), update them
  too, or say in your report that you didn't.

## The screenshot loop (headless, via gamescope)

`scripts/ui-shot.sh` runs Mothr inside a **headless gamescope**: its own Xwayland, no window
on the user's screen, a throwaway `MOTHR_HOME`, and `gamescopectl screenshot` to capture what
gamescope composites. It needs gamescope 3.14+ (`gamescope --backend headless`,
`gamescopectl`).

One shot (builds the debug binary first, waits `UI_SHOT_WAIT`=6 s, captures, cleans up):

```sh
S=.agents/skills/desktop-ui/scripts/ui-shot.sh
$S shot /tmp/mothr-settings.png MOTHR_OPEN=settings
```

Then **look at the PNG** (open it with your image-reading tool) and compare it with what you
meant to build. Fix, run again, look again. Iterate until it's right. Don't call UI work done
from a compile alone.

Several shots of one running instance (e.g. waiting for a live chat to stream):

```sh
$S start MOTHR_DEV_PROMPT="claude_code|ask||chat|Say hi"
$S snap /tmp/a.png        # repeat as the screen changes; the first frame can still be blank
$S stop                   # always stop: it kills gamescope + Mothr and deletes the profile
```

Get to the screen you're working on with Mothr's dev hooks instead of clicks:

| Need | Pass |
| --- | --- |
| a page | `MOTHR_OPEN=settings\|remote\|usage\|skills\|workspaces\|projects\|designs\|resume\|picker\|terminal` |
| a repo / file | `MOTHR_OPEN=project:<path>[:<file>[:full]]` |
| a real chat | `MOTHR_DEV_PROMPT="<agent>\|<mode>\|<model>\|chat\|<prompt>"` (costs the user's agent quota; keep it tiny) |
| text in the composer | `MOTHR_PREFILL=<text>` |
| the Changes panel | `MOTHR_OPEN=changes`, `MOTHR_DIFF_MODE=split`, `MOTHR_DEV_REVIEW=accept\|reject` |
| a design page | `MOTHR_DESIGN_PAGE=overview\|color\|type\|component:<n>\|screen:<n>` |

Knobs (environment of `ui-shot.sh`):

- `UI_SHOT_SETTINGS=<file>` seeds `settings.toml` (all fields optional), e.g.
  `printf 'theme = "light"\nproject_roots = []\n' > /tmp/light.toml` for the light theme
  without scanning the user's repos.
- `UI_SHOT_SIZE=1280x800` (default 1440x900; windows are forced to fill it).
- `UI_SHOT_NO_BUILD=1` to reuse the last build; `UI_SHOT_BIN=<path>` for another binary.

Clicks and typing: `DISPLAY=$($S display) xdotool mousemove X Y click 1` works against the
nested X server if `xdotool` is installed. Screenshot pixels map 1:1 to its coordinates. Never
use `ydotool`: it injects through uinput into the user's real session.

Notes:

- `WAYLAND_DISPLAY` is unset for Mothr so GPUI takes gamescope's X11 display instead of the
  user's compositor. That's why nothing shows on screen.
- The log is `<MOTHR_HOME>/gamescope.log` (printed by `start`); GPUI warnings about
  `.SystemUIFont` are harmless.
- Screenshots can show the user's repo names, agent skills and usage. Keep them in your
  scratch space; don't commit or publish them.
- Without gamescope (macOS, other setups), run `MOTHR_HOME=$(mktemp -d) cargo run` with the
  same hooks and take a screenshot with the platform's tool (`run-mothr` skill).

## Before you finish

```sh
cargo clippy -p mothr-app --all-targets -- -D warnings
cargo test -p mothr-app
```

And attach (or describe) the final screenshots, dark and light, in your report.
