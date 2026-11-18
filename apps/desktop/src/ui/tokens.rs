//! Mothr design system tokens.
//!
//! Everything visual derives from here: the two palettes (dark is the primary experience,
//! light is its mirror), the type scale, radii, spacing, elevation and motion. The same
//! system is documented in `DESIGN.md` and authored as `mothr-design/system.json`, so it
//! can be browsed in Mothr's own Design canvas.
//!
//! Principles:
//! - **Quiet chrome, loud content.** Surfaces step in tiny luminance increments; only the
//!   brand violet and semantic colors carry saturation.
//! - **One accent.** Violet means "you can act here" (primary actions, focus, selection).
//! - **Agents keep their identity** through their brand tints, never through chrome.
//! - **Calm motion.** 120–320 ms, ease-out, no bounces except success feedback.

#![allow(dead_code)] // Full scales are defined even where not every step is used yet.

use gpui_kit::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Interface scale (1.0 = 100%). Everything Mothr draws goes through [`sz`]; gpui-kit
/// components follow via the window's rem size.
static SCALE: AtomicU32 = AtomicU32::new(0x3F933333); // 1.15

pub fn set_scale(scale: f32) {
    SCALE.store(scale.clamp(0.8, 1.8).to_bits(), Ordering::Relaxed);
}

pub fn scale() -> f32 {
    f32::from_bits(SCALE.load(Ordering::Relaxed))
}

/// A scaled length: use instead of `px()` for anything in the UI.
pub fn sz(v: f32) -> Pixels {
    px(v * scale())
}

/// Semantic colors for one appearance.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    /// Window background (main content area).
    pub bg: Hsla,
    /// Chrome: sidebar and title bar.
    pub chrome: Hsla,
    /// Cards, composer, tool cards.
    pub surface: Hsla,
    /// Hovered surfaces and quiet fills (inputs, chips).
    pub surface_hover: Hsla,
    /// Popovers, menus and dialogs.
    pub raised: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub brand: Hsla,
    pub brand_hover: Hsla,
    /// Translucent brand fill for selection and soft emphasis.
    pub brand_soft: Hsla,
    /// Second gradient stop for brand marks.
    pub brand_2: Hsla,
    pub on_brand: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub info: Hsla,
    /// Code blocks and terminal output.
    pub code_bg: Hsla,
    /// Backdrop behind modals.
    pub scrim: Hsla,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn ca(hex: u32, alpha: f32) -> Hsla {
    c(hex).opacity(alpha)
}

impl Palette {
    pub fn dark() -> Self {
        Self {
            dark: true,
            bg: c(0x0B0C0F),
            chrome: c(0x0F1014),
            surface: c(0x14151A),
            surface_hover: c(0x1A1B21),
            raised: c(0x17181E),
            border: c(0x1F2027),
            border_strong: c(0x2B2C35),
            text: c(0xECECF1),
            text_muted: c(0x9B9DAA),
            text_faint: c(0x60626E),
            brand: c(0x8B7CF6),
            brand_hover: c(0x9C8FF8),
            brand_soft: ca(0x8B7CF6, 0.14),
            brand_2: c(0x4CB3FF),
            on_brand: c(0xFFFFFF),
            success: c(0x3DD68C),
            warning: c(0xF5B544),
            danger: c(0xF2555A),
            info: c(0x5AA9FF),
            code_bg: c(0x101116),
            scrim: ca(0x000000, 0.55),
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            bg: c(0xFCFCFD),
            chrome: c(0xF5F5F8),
            surface: c(0xFFFFFF),
            surface_hover: c(0xF1F1F5),
            raised: c(0xFFFFFF),
            border: c(0xE7E7EC),
            border_strong: c(0xD6D6DE),
            text: c(0x15161A),
            text_muted: c(0x5F6170),
            text_faint: c(0x9698A4),
            brand: c(0x6D5BEF),
            brand_hover: c(0x5E4BE6),
            brand_soft: ca(0x6D5BEF, 0.10),
            brand_2: c(0x2F9BEF),
            on_brand: c(0xFFFFFF),
            success: c(0x14A363),
            warning: c(0xC7860A),
            danger: c(0xDC3B40),
            info: c(0x2F7FE0),
            code_bg: c(0xF6F6F9),
            scrim: ca(0x0B0C10, 0.28),
        }
    }

    pub fn for_mode(dark: bool) -> Self {
        if dark { Self::dark() } else { Self::light() }
    }
}

impl Global for Palette {}

/// The active palette.
pub fn p(cx: &App) -> Palette {
    cx.try_global::<Palette>().copied().unwrap_or_else(Palette::dark)
}

/// Font families bundled with the app (SIL OFL, see assets/fonts/OFL.txt).
pub const FONT_UI: &str = "Geist";
pub const FONT_MONO: &str = "Geist Mono";

/// Type scale (px). Base UI text is 13.5; reading text in chats is 14.5.
pub mod text {
    pub const XS: f32 = 11.5;
    pub const SM: f32 = 12.5;
    pub const BASE: f32 = 13.5;
    pub const READ: f32 = 14.5;
    pub const LG: f32 = 16.0;
    pub const XL: f32 = 19.0;
    pub const XXL: f32 = 24.0;
    pub const DISPLAY: f32 = 34.0;
    /// Line height for reading text.
    pub const READ_LEADING: f32 = 1.65;
}

/// Corner radii (px).
pub mod radius {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
    pub const XL: f32 = 16.0;
    pub const XXL: f32 = 20.0;
}

/// Spacing scale (px) on a 4px grid.
pub mod space {
    pub const S1: f32 = 4.0;
    pub const S2: f32 = 8.0;
    pub const S3: f32 = 12.0;
    pub const S4: f32 = 16.0;
    pub const S5: f32 = 20.0;
    pub const S6: f32 = 24.0;
    pub const S8: f32 = 32.0;
    pub const S10: f32 = 40.0;
}

/// Layout constants.
pub mod layout {
    pub const SIDEBAR_W: f32 = 272.0;
    pub const READ_MAX_W: f32 = 780.0;
    pub const TITLEBAR_H: f32 = 40.0;
}

/// Motion durations.
pub mod motion {
    use super::Duration;
    pub const FAST: Duration = Duration::from_millis(120);
    pub const BASE: Duration = Duration::from_millis(200);
    pub const SLOW: Duration = Duration::from_millis(320);
}

/// Elevation: a soft drop shadow plus (in dark mode) a 1px inner highlight via border.
pub fn shadow_sm(pal: &Palette) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: gpui_kit::black().opacity(if pal.dark { 0.35 } else { 0.06 }),
        offset: point(sz(0.), sz(1.)),
        blur_radius: sz(2.),
        spread_radius: sz(0.),
        inset: false,
    }]
}

pub fn shadow_md(pal: &Palette) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: gpui_kit::black().opacity(if pal.dark { 0.45 } else { 0.07 }),
            offset: point(sz(0.), sz(8.)),
            blur_radius: sz(24.),
            spread_radius: sz(-4.),
            inset: false,
        },
        BoxShadow {
            color: gpui_kit::black().opacity(if pal.dark { 0.3 } else { 0.04 }),
            offset: point(sz(0.), sz(1.)),
            blur_radius: sz(3.),
            spread_radius: sz(0.),
            inset: false,
        },
    ]
}

pub fn shadow_lg(pal: &Palette) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: gpui_kit::black().opacity(if pal.dark { 0.6 } else { 0.14 }),
            offset: point(sz(0.), sz(24.)),
            blur_radius: sz(64.),
            spread_radius: sz(-12.),
            inset: false,
        },
        BoxShadow {
            color: gpui_kit::black().opacity(if pal.dark { 0.35 } else { 0.05 }),
            offset: point(sz(0.), sz(2.)),
            blur_radius: sz(6.),
            spread_radius: sz(0.),
            inset: false,
        },
    ]
}

/// A colored glow, used for focus and the hero halo.
pub fn glow(color: Hsla, blur: f32, spread: f32) -> Vec<BoxShadow> {
    vec![BoxShadow { color, offset: point(sz(0.), sz(0.)), blur_radius: sz(blur), spread_radius: sz(spread), inset: false }]
}

/// Brand gradient (diagonal violet → sky).
pub fn brand_gradient(pal: &Palette) -> Background {
    linear_gradient(135., linear_color_stop(pal.brand, 0.), linear_color_stop(pal.brand_2, 1.))
}
