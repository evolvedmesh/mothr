//! Workspaces: named groups of chats you switch between, like a browser's spaces.
//!
//! Every chat belongs to exactly one workspace. There is always at least one, and at most
//! [`MAX_WORKSPACES`].

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type WorkspaceId = Uuid;

/// How many workspaces you can have: one per number key, 1 … 0.
pub const MAX_WORKSPACES: usize = 10;

/// What the first workspace is called.
pub const DEFAULT_WORKSPACE_NAME: &str = "Main";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    /// One of [`WORKSPACE_ICONS`]; None draws a plain dot.
    #[serde(default)]
    pub icon: Option<String>,
}

impl Workspace {
    pub fn new(name: impl Into<String>) -> Self {
        Self { id: Uuid::new_v4(), name: name.into(), icon: None }
    }

    /// The icon to draw, if it's one Mothr offers.
    pub fn icon_name(&self) -> Option<&'static str> {
        let icon = self.icon.as_deref()?;
        WORKSPACE_ICONS.iter().copied().find(|i| *i == icon)
    }
}

/// Icons a workspace can wear (Lucide names).
pub const WORKSPACE_ICONS: &[&str] = &[
    "briefcase", "house", "flask-conical", "rocket", "book-open", "code", "bug", "palette", "sparkles", "graduation-cap",
    "heart", "star", "coffee", "gamepad-2", "music", "globe", "leaf", "flame", "shield", "zap",
];

/// A workspace name that isn't taken: "Workspace 2", "Workspace 3" …
pub fn next_workspace_name(existing: &[Workspace]) -> String {
    (2..)
        .map(|n| format!("Workspace {n}"))
        .find(|name| !existing.iter().any(|w| w.name.eq_ignore_ascii_case(name)))
        .unwrap_or_else(|| "Workspace".into())
}

/// Clean up a name typed by the user; None when nothing is left.
pub fn clean_workspace_name(name: &str) -> Option<String> {
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name: String = name.chars().take(40).collect();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_name_skips_taken_ones() {
        let ws = vec![Workspace::new("Main"), Workspace::new("Workspace 2")];
        assert_eq!(next_workspace_name(&ws), "Workspace 3");
    }

    #[test]
    fn names_are_trimmed_and_capped() {
        assert_eq!(clean_workspace_name("  Side   projects "), Some("Side projects".into()));
        assert_eq!(clean_workspace_name("   "), None);
        assert_eq!(clean_workspace_name(&"x".repeat(80)).map(|n| n.len()), Some(40));
    }

    #[test]
    fn unknown_icons_are_ignored() {
        let mut w = Workspace::new("A");
        w.icon = Some("rocket".into());
        assert_eq!(w.icon_name(), Some("rocket"));
        w.icon = Some("not-an-icon".into());
        assert_eq!(w.icon_name(), None);
    }
}
