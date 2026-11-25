# Mothr design system

Mothr's design system lives in three places that stay in step:

| Where | What it's for |
| --- | --- |
| `apps/desktop/src/ui/tokens.rs` | The source of truth in code: palettes, type scale, radii, spacing, elevation, motion. |
| `apps/desktop/src/ui/theme.rs` | Maps the tokens onto every gpui-kit component color and registers the fonts. |
| `mothr-design/` | The same system written in Mothr's own design format. Pin or open this repo in Mothr and it appears on the **Design** page. |

## Principles

1. **Quiet chrome, loud content.**
   - Surfaces differ by only a few percent in brightness. Borders are hairlines.
   - The conversation, code and agent output carry the visual weight; the frame around them
     doesn't.
2. **One accent, used sparingly.**
   - Violet (`brand`) marks live state and selection: working status, orchestration, the
     active row's edge.
   - Primary controls (Send, the logo) are monochrome: `text` on `bg`.
   - No gradients, glows or sparkles in the chrome.
3. **Agents keep their identity.**
   - Each agent has a brand tint, used only on its avatar tile and on selected agent cards:
     Claude `#D97757`, Codex `#10A37F`, Copilot `#8B5CF6`, OpenCode `#3B82F6`,
     Hermes `#F59E0B`.
4. **State is always visible.**
   - Working pulses violet, Needs you pulses amber, Error is red, Ready is green, Ended is grey.
   - A chat that's waiting on you is shown in the "Needs you" sidebar group, with a pulsing
     status dot, and as an amber card in the transcript.
5. **Calm motion.**
   - Durations are 120 ms (feedback), 200 ms (navigation) and 320 ms (hero), ease-out.
   - Content fades and rises a few pixels. Nothing bounces.
6. **Dark first, light equal.**
   - Dark is the primary experience. Every token has a light counterpart, and "System" follows
     the OS live.

## Color

| Token | Dark | Light | Use |
| --- | --- | --- | --- |
| `bg` | `#0B0C0F` | `#FCFCFD` | Main content area |
| `chrome` | `#0F1014` | `#F5F5F8` | Sidebar (the window's frame) |
| `surface` | `#14151A` | `#FFFFFF` | Cards, composer, tool cards |
| `surface_hover` | `#1A1B21` | `#F1F1F5` | Hover, quiet fills, user bubbles |
| `raised` | `#17181E` | `#FFFFFF` | Menus, popovers, dialogs |
| `border` | `#1F2027` | `#E7E7EC` | Hairlines |
| `border_strong` | `#2B2C35` | `#D6D6DE` | Inputs, outlined buttons |
| `text` | `#ECECF1` | `#15161A` | Primary text |
| `text_muted` | `#9B9DAA` | `#5F6170` | Secondary text |
| `text_faint` | `#60626E` | `#9698A4` | Meta, timestamps, hints |
| `brand` | `#8B7CF6` | `#6D5BEF` | Primary actions, focus, selection |
| `brand_2` | `#4CB3FF` | `#2F9BEF` | Second stop of the logo gradient only |
| `success` | `#3DD68C` | `#14A363` | Ready, allowed, completed, "+" diff lines |
| `warning` | `#F5B544` | `#C7860A` | Needs you, permission cards, limits ≥ 75% |
| `danger` | `#F2555A` | `#DC3B40` | Errors, deny, limits ≥ 90%, Allow-all mode |

The thresholds for limit meters are: green below 75%, amber from 75%, red from 90%, and
exhausted at 100%.

### Making it yours

The user can recolor and rearrange Mothr (Settings → Colors and layout; on the phone, Chats →
⋯ → Appearance). The principles hold whatever they pick: there is still exactly one accent.

- **Accent**: Violet (default), Blue, Teal, Green, Orange, Rose, Pink, Mono, or any `#RRGGBB`.
  It replaces `brand` everywhere; `brand_hover`, `brand_soft` and `on_brand` (dark or white text,
  by the accent's luminance) follow from it.
- **Tone**: Graphite (default), Slate (cool), Warm, or Black (pure black in dark mode). A tone
  swaps the surface ramp (`bg` … `border_strong`, `code_bg`); text and semantic colors stay.
- **Layout**: the sidebar (and with it the window's controls) on the left or right.

The presets live in `mothr_core::{Accent, Tone}` and are mirrored in the phone app's
`Theme.kt`; change both together.

## Typography

Fonts are **Geist** and **Geist Mono**, bundled under the SIL OFL (`assets/fonts`), so the app
looks the same on every platform.

| Role | Size | Weight | Leading |
| --- | --- | --- | --- |
| Display (hero greeting) | 34 | 600 | 1.15 |
| Title | 19 | 600 | 1.3 |
| Heading | 16 | 600 | 1.4 |
| Reading (chat messages) | 14.5 | 400 | 1.65 |
| Body (UI) | 13.5 | 400 | 1.5 |
| Label | 13.5 | 500 | 1.4 |
| Caption (meta, groups) | 11.5 | 500 | 1.4 |
| Mono (commands, paths, code) | 12.5 | 400 | 1.55 |

## Scale

Every size goes through `tokens::sz()`, which multiplies by the interface scale (Settings →
Appearance: 100 / 115 / 130 / 145%, default 115%). gpui-kit components follow through the
window's rem size, so text, buttons, menus and spacing grow together.

## Space, shape, elevation

- **Spacing** follows a 4 px grid: 4 · 8 · 12 · 16 · 20 · 24 · 32 · 40.
- **Radii**:
  - xs 4: tags
  - sm 6: chips
  - md 8: buttons and rows
  - lg 12: cards
  - xl 16: composer and user bubbles
  - full: pills
- **Elevation**:
  - `sm` for the selected segment
  - `md` for the composer and cards
  - `lg` for modals and the project finder
  - `glow` is reserved for design previews; the app's chrome doesn't use it
- **Layout**:
  - the sidebar is 272 px
  - the reading column is capped at 780 px
  - the Changes panel takes 52% of a chat's width
  - the Design viewer has a 228 px page list and a 420 px chat; a narrow window drops the list
    while the chat is open

## Components

| Component | Notes |
| --- | --- |
| Button / Primary | Violet with a glow. Use one per view; it's the next step. |
| Button / Quiet | Outline or ghost, for secondary actions. Never filled. |
| Pill / Status | A dot plus a label. The dot pulses while live. The orchestrator variant shows running/total subagents. |
| Session row | Agent tile with a status badge, title, and one line of context. The active row has a 2.5 px violet edge. Subagents nest beneath their orchestrator with tree guides. |
| Tool card | Collapsed: icon, action, target, outcome, and the +/- line count. Expanded: output or a highlighted diff. Subagent cards link to the subagent's chat. |
| Permission card | Amber frame, the exact command or path, and the Allow / Always allow / Deny choices. When resolved it shrinks to a single line. |
| Edit review card | A permission card for a file edit: amber frame, the diff, and Reject / Accept. Replaces the tool card while it waits. |
| Changes panel | Every edit in a chat, waiting ones first, with a state per file. Rejected and failed edits start collapsed. The footer says what the permission mode means for edits. |
| Design card | A design introduced by its own tokens: its surface, type, a real component, palette and first screen. |
| Design viewer | Storybook layout: page list, a page per foundation, component and screen, Export at the top right, chat on demand. |
| Question card | Violet frame, option chips (single or multi select), plus a free-text "Other" field. |
| Composer | Raised surface. On focus its border turns violet and it glows. Shows agent, model, project and mode pickers; Enter sends. |
| Subagent strip | Below an orchestrator's header: one chip per subagent with live status and tool and token counts. |
| Usage meter | 5 px track, colored by usage level, with a reset countdown. |
| Sidebar frame | There is no separate title bar; the sidebar is the window's frame. Header (drag to move, double-click to maximize, window buttons when Mothr draws them): the Mothr mark, then find a project, back/forward and collapse. Above the footer, while any chat works or waits on you, a live activity row (pulsing; violet working, amber needs you) that opens the first. Footer: the limits rings, Mothr Remote while it's on, and light/dark and Settings in one hairline island. Collapsed, it becomes a 52 px rail of the same controls. |
| Limits button | At the foot of the sidebar: one 20 px progress ring per agent account with its logo inside. Rings use the agent's tint until 75%, then amber/red. Hover shows the limits card, opening upwards; click pins it beside the sidebar. |
| Home | A time-of-day greeting over a typed headline that cycles short phrases ("Fix a bug", "Ship it"), its caret in the accent and blinking while a phrase rests; the agent and project under it; the composer docked at the bottom with the project picker below. |
| Agent avatar | The agent's real logo on a soft tile of its brand tint (logos in `apps/desktop/assets/logos`). |

## Voice

- **Short and plain.** "Needs you", "Waiting for you", "Remote on".
- **Name the actual thing.** Use the agent's real name, the real command, the real path.
- **Never overstate state.** Something is "Ended" only after the process exits. A subagent is
  "running" only while its task is live.
