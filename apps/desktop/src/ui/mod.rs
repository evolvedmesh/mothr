//! Mothr's GPUI interface.

pub mod changes;
pub mod chat;
pub mod clip;
pub mod design;
pub mod diff;
pub mod home;
pub mod kit;
pub mod lightbox;
pub mod options;
pub mod picker;
pub mod projects;
pub mod settings;
pub mod sidebar;
pub mod skills;
pub mod theme;
pub mod tokens;
pub mod usage;
pub mod workspace;
pub mod workspaces;

pub fn init(cx: &mut gpui_kit::App) {
    picker::init(cx);
    projects::init(cx);
}
