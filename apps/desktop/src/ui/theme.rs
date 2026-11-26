//! Applies the design-system tokens to the component theme and manages light/dark mode.

use super::tokens::{self, Palette};
use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::*;
use super::tokens::sz;
use mothr_core::ThemePreference;
use std::borrow::Cow;

/// One bundled font file.
pub struct BundledFont {
    pub family: &'static str,
    pub weight: u16,
    pub file_name: &'static str,
    pub data: &'static [u8],
}

macro_rules! font {
    ($family:literal, $weight:literal, $file:literal) => {
        BundledFont { family: $family, weight: $weight, file_name: $file, data: include_bytes!(concat!("../../assets/fonts/", $file)) }
    };
}

/// The Geist families bundled with the app (SIL OFL). Design exports embed them too.
pub const FONTS: &[BundledFont] = &[
    font!("Geist", 400, "Geist-400.ttf"),
    font!("Geist", 500, "Geist-500.ttf"),
    font!("Geist", 600, "Geist-600.ttf"),
    font!("Geist", 700, "Geist-700.ttf"),
    font!("Geist Mono", 400, "GeistMono-400.ttf"),
    font!("Geist Mono", 500, "GeistMono-500.ttf"),
    font!("Geist Mono", 600, "GeistMono-600.ttf"),
];

/// Register the bundled Geist families. Call once at startup.
pub fn load_fonts(cx: &mut App) {
    let fonts: Vec<Cow<'static, [u8]>> = FONTS.iter().map(|f| Cow::Borrowed(f.data)).collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        tracing::warn!("failed to load bundled fonts: {e}");
    }
}

/// Resolve a preference to a concrete mode for this window.
pub fn resolve(pref: ThemePreference, window: Option<&Window>) -> ThemeMode {
    match pref {
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
        ThemePreference::System => match window.map(|w| w.appearance()) {
            Some(WindowAppearance::Light | WindowAppearance::VibrantLight) => ThemeMode::Light,
            // Before a window exists GPUI can't know; dark is Mothr's home turf.
            _ => ThemeMode::Dark,
        },
    }
}

/// Switch mode and re-apply Mothr's tokens on top of the component theme.
pub fn apply_mode(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    Theme::change(mode, window, cx);
    tune(cx);
    cx.refresh_windows();
}

pub fn apply_preference(pref: ThemePreference, window: &mut Window, cx: &mut App) {
    let mode = resolve(pref, Some(window));
    apply_mode(mode, Some(window), cx);
}

/// Map the palette onto every component-library color.
pub fn tune(cx: &mut App) {
    let dark = cx.theme().mode.is_dark();
    let pal = Palette::for_mode(dark);
    cx.set_global(pal);
    Theme::update(cx, move |theme| {
        theme.font_family = tokens::FONT_UI.into();
        theme.mono_font_family = tokens::FONT_MONO.into();
        theme.font_size = sz(tokens::text::BASE);
        theme.mono_font_size = sz(12.5);
        theme.radius = sz(tokens::radius::MD);
        theme.radius_lg = sz(tokens::radius::LG);
        let c = &mut theme.colors;
        c.background = pal.bg;
        c.foreground = pal.text;
        c.border = pal.border;
        c.muted = pal.surface_hover;
        c.muted_foreground = pal.text_muted;
        c.accent = pal.surface_hover;
        c.accent_foreground = pal.text;
        c.secondary = pal.surface_hover;
        c.secondary_hover = pal.border;
        c.secondary_active = pal.border_strong;
        c.secondary_foreground = pal.text;
        c.popover = pal.raised;
        c.popover_foreground = pal.text;
        c.input = pal.border_strong;
        c.ring = pal.brand.opacity(0.55);
        c.selection = pal.brand.opacity(0.30);
        c.caret = pal.brand;
        c.link = pal.brand;
        c.link_hover = pal.brand_hover;
        c.link_active = pal.brand;

        c.primary = pal.brand;
        c.primary_hover = pal.brand_hover;
        c.primary_active = pal.brand.opacity(0.85);
        c.primary_foreground = pal.on_brand;
        c.button_primary = pal.brand;
        c.button_primary_hover = pal.brand_hover;
        c.button_primary_active = pal.brand.opacity(0.85);
        c.button_primary_foreground = pal.on_brand;

        c.success = pal.success;
        c.warning = pal.warning;
        c.danger = pal.danger;
        c.danger_hover = pal.danger.opacity(0.9);
        c.danger_active = pal.danger.opacity(0.8);
        c.info = pal.info;

        c.sidebar = pal.chrome;
        c.sidebar_foreground = pal.text;
        c.sidebar_border = pal.border;
        c.sidebar_accent = pal.surface_hover;
        c.sidebar_accent_foreground = pal.text;
        c.title_bar = pal.chrome;
        c.title_bar_border = pal.border;
        c.status_bar = pal.chrome;
        c.status_bar_border = pal.border;

        c.list = pal.bg;
        c.list_hover = pal.surface_hover;
        c.list_active = pal.brand_soft;
        c.list_active_border = pal.brand.opacity(0.4);
        c.list_even = pal.bg;
        c.list_head = pal.chrome;

        c.switch = pal.brand;
        c.switch_thumb = gpui_kit::white();
        c.progress_bar = pal.brand;
        c.slider_bar = pal.brand;
        c.slider_thumb = pal.on_brand;
        c.scrollbar = gpui_kit::transparent_black();
        c.scrollbar_thumb = pal.text_faint.opacity(0.35);
        c.scrollbar_thumb_hover = pal.text_faint.opacity(0.6);
        c.skeleton = pal.surface_hover;
        c.tab_bar = pal.chrome;
        c.tab = pal.chrome;
        c.tab_active = pal.bg;
        c.tab_active_foreground = pal.text;
        c.tab_foreground = pal.text_muted;
        c.overlay = pal.scrim;
        c.window_border = pal.border;
        c.drag_border = pal.brand;
        c.drop_target = pal.brand_soft;
    });
}

/// Change the accent, tone or layout: saved to settings and applied at once.
pub fn edit_appearance(store: &Entity<crate::store::Store>, f: impl FnOnce(&mut mothr_core::Appearance), cx: &mut App) {
    let hub = store.read(cx).hub.clone();
    let mut s = hub.settings();
    f(&mut s.appearance);
    tokens::set_appearance(s.appearance.clone());
    hub.update_settings(s);
    tune(cx);
    cx.refresh_windows();
}

/// Elevated card surface (composer, tool cards, settings groups).
pub fn card(cx: &App) -> Div {
    let pal = tokens::p(cx);
    div()
        .rounded(sz(tokens::radius::LG))
        .border_1()
        .border_color(pal.border)
        .bg(pal.surface)
}
