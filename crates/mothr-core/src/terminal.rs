//! Terminals beside chats, as remote clients see them: a styled snapshot of the screen.
//!
//! The desktop paints the terminal from the emulator directly; the web client and the phone get
//! [`TerminalScreen`]s (pushed as [`crate::HubEvent::Terminal`]) and send keystrokes back as
//! [`crate::AppCommand::TerminalInput`].

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// A terminal's key: the chat it belongs to (or a new-chat draft on the desktop).
pub type TerminalId = Uuid;

/// A color in a terminal cell: `0..=255` is the 256-color table (0–15 are the theme's ANSI
/// colors), and `RGB_FLAG | 0xRRGGBB` a true color. `None` on a span is the default color.
pub type TermColor = u32;

/// Set on a [`TermColor`] that is a true color rather than a table index.
pub const RGB_FLAG: u32 = 1 << 24;

/// A run of cells with the same style.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TermSpan {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<TermColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<TermColor>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub underline: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dim: bool,
    /// Foreground and background swapped (the default colors included).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inverse: bool,
}

/// What a terminal shows now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TerminalScreen {
    pub id: TerminalId,
    /// Where the shell started (the chat's primary repo).
    pub cwd: PathBuf,
    /// The title the shell set, if any.
    #[serde(default)]
    pub title: Option<String>,
    pub cols: u16,
    pub rows: u16,
    /// One entry per row, top to bottom; trailing blank cells are left out.
    pub lines: Vec<Vec<TermSpan>>,
    /// `[row, column]` of the cursor, when it's shown.
    #[serde(default)]
    pub cursor: Option<[u16; 2]>,
    /// Why the shell isn't running (it exited, or couldn't start); None while it runs.
    #[serde(default)]
    pub exited: Option<String>,
    /// The program asked for application cursor keys: arrows send `ESC O A` … instead of
    /// `ESC [ A` (full-screen programs like vim and less).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub app_cursor: bool,
}

impl TerminalScreen {
    /// The screen as plain text, one line per row.
    pub fn text(&self) -> String {
        self.lines.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }
}
