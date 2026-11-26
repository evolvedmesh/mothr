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
    /// Chrome: the sidebar, which is also the window frame.
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
    /// The terminal's 16 ANSI colors: black, red, green, yellow, blue, magenta, cyan, white,
    /// then their bright versions.
    pub ansi: [Hsla; 16],
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
            ansi: [
                c(0x2B2C35), c(0xF2555A), c(0x3DD68C), c(0xF5B544), c(0x5AA9FF), c(0xB48CF6), c(0x4CC9D8), c(0xC9CAD3),
                c(0x60626E), c(0xFF7A7E), c(0x6BE6A9), c(0xFFCB6B), c(0x84C0FF), c(0xC9A8FF), c(0x7ADDE8), c(0xECECF1),
            ],
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
            ansi: [
                c(0x15161A), c(0xC8323A), c(0x0E8A52), c(0xA36D00), c(0x2364C7), c(0x7C4DDB), c(0x0B8494), c(0xB9BAC4),
                c(0x5F6170), c(0xDC3B40), c(0x14A363), c(0xC7860A), c(0x2F7FE0), c(0x9466F0), c(0x1499AB), c(0x9698A4),
            ],
        }
    }

    /// The palette in use: the base one with your tone and accent.
    pub fn for_mode(dark: bool) -> Self {
        Self::themed(dark, &appearance())
    }

    /// A base palette with a tone and an accent applied.
    pub fn themed(dark: bool, a: &mothr_core::Appearance) -> Self {
        let mut p = if dark { Self::dark() } else { Self::light() };
        p.apply_tone(a.tone);
        p.apply_accent(a.accent_hex(dark));
        p
    }

    fn apply_tone(&mut self, tone: mothr_core::Tone) {
        use mothr_core::Tone;
        // bg, chrome, surface, surface_hover, raised, border, border_strong, code_bg
        let set: [u32; 8] = match (tone, self.dark) {
            (Tone::Graphite, _) => return,
            (Tone::Slate, true) => [0x0A0D13, 0x0D1119, 0x121722, 0x19202C, 0x151B26, 0x1D2431, 0x293243, 0x0E121A],
            (Tone::Slate, false) => [0xFAFBFD, 0xF1F4F9, 0xFFFFFF, 0xEBEFF6, 0xFFFFFF, 0xE1E6EF, 0xD0D8E4, 0xF3F6FA],
            (Tone::Warm, true) => [0x0F0D0B, 0x13110E, 0x191613, 0x201C18, 0x1C1915, 0x28231E, 0x36302A, 0x14120F],
            (Tone::Warm, false) => [0xFCFBF9, 0xF6F3EF, 0xFFFFFF, 0xF1EDE7, 0xFFFFFF, 0xE8E2DA, 0xD9D1C6, 0xF7F4EF],
            (Tone::Black, true) => [0x000000, 0x050506, 0x0C0C0E, 0x141417, 0x111113, 0x1A1A1E, 0x27272C, 0x08080A],
            (Tone::Black, false) => return,
        };
        self.bg = c(set[0]);
        self.chrome = c(set[1]);
        self.surface = c(set[2]);
        self.surface_hover = c(set[3]);
        self.raised = c(set[4]);
        self.border = c(set[5]);
        self.border_strong = c(set[6]);
        self.code_bg = c(set[7]);
    }

    fn apply_accent(&mut self, hex: u32) {
        let accent = c(hex);
        self.brand = accent;
        let mut hover = accent;
        hover.l = if self.dark { (hover.l + 0.06).min(0.97) } else { (hover.l - 0.05).max(0.05) };
        self.brand_hover = hover;
        self.brand_soft = accent.opacity(if self.dark { 0.14 } else { 0.10 });
        // Text on the accent stays readable whatever its lightness.
        self.on_brand = if luminance(hex) > 0.5 { c(0x111216) } else { c(0xFFFFFF) };
    }
}

/// Relative luminance (0 black … 1 white) of 0xRRGGBB.
pub fn luminance(hex: u32) -> f32 {
    let ch = |v: u32| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * ch((hex >> 16) & 0xFF) + 0.7152 * ch((hex >> 8) & 0xFF) + 0.0722 * ch(hex & 0xFF)
}

static APPEARANCE: std::sync::RwLock<Option<mothr_core::Appearance>> = std::sync::RwLock::new(None);

/// Set the accent, tone and layout the palette and the workspace follow. Re-apply the theme
/// (`theme::tune`) afterwards for colors to change.
pub fn set_appearance(a: mothr_core::Appearance) {
    *APPEARANCE.write().unwrap_or_else(|e| e.into_inner()) = Some(a);
}

pub fn appearance() -> mothr_core::Appearance {
    APPEARANCE.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
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
