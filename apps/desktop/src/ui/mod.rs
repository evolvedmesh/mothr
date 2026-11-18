//! Mothr's GPUI interface.

pub mod changes;
pub mod chat;
pub mod design;
pub mod diff;
pub mod home;
pub mod kit;
pub mod picker;
pub mod settings;
pub mod sidebar;
pub mod theme;
pub mod tokens;
pub mod usage;
pub mod workspace;

pub fn init(cx: &mut gpui_kit::App) {
    picker::init(cx);
}
