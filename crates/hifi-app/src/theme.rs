//! Hi-Fi theme — the cosmos design tokens, ported 1:1.
//!
//! Dark:  bg #060606 / shell #0d0d0d / dialog #161616 / raised #343438 /
//!        card #0e0e0e / text #e8e8ea / muted #a9a9ae / faint #85858a
//! Light: bg #ffffff / shell #f3f3f5 / card #ffffff / text #303035 /
//!        muted #62626a / faint #797981
//! Radii: 16 / 10 / 6. Fonts: Geist (UI) + Geist Mono (code/terminal).

use gpui::{App, Global, Hsla, hsla};
use hifi_core::{Appearance, Settings};

#[allow(dead_code)] // UI font is GPUI's default; kept for explicit styling later
pub const FONT_UI: &str = "Geist";
pub const FONT_MONO: &str = "Geist Mono";

/// Corner radii.
pub mod radius {
    use gpui::{Pixels, px};
    pub const BUBBLE: Pixels = px(16.);
    pub const CARD: Pixels = px(10.);
    pub const ROUND: Pixels = px(6.);
}

pub mod space {
    use gpui::{Pixels, px};
    pub const XS: Pixels = px(4.);
    pub const SM: Pixels = px(8.);
    pub const MD: Pixels = px(12.);
    pub const LG: Pixels = px(16.);
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Hsla,
    pub shell: Hsla,
    pub dialog: Hsla,
    pub raised: Hsla,
    pub card: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub accent: Hsla,
    #[allow(dead_code)]
    pub accent_soft: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub danger: Hsla,
    #[allow(dead_code)]
    pub success: Hsla,
    pub is_dark: bool,
}

pub fn color(h: u32) -> Hsla {
    gpui::rgb(h).into()
}

/// Accent families — name → (dark value, light value).
pub const ACCENTS: &[(&str, u32, u32)] = &[
    ("violet", 0x8b7cf6, 0x5b43e8),
    ("indigo", 0x818cf8, 0x4f46e5),
    ("blue", 0x60a5fa, 0x2563eb),
    ("cyan", 0x22d3ee, 0x0891b2),
    ("teal", 0x2dd4bf, 0x0d9488),
    ("green", 0x4ade80, 0x16a34a),
    ("amber", 0xfbbf24, 0xd97706),
    ("orange", 0xfb923c, 0xea580c),
    ("rose", 0xfb7185, 0xe11d48),
];

pub fn accent_value(name: &str, dark: bool) -> Hsla {
    let (_, d, l) = ACCENTS
        .iter()
        .find(|(n, _, _)| *n == name)
        .copied()
        .unwrap_or(ACCENTS[0]);
    color(if dark { d } else { l })
}

impl Palette {
    pub fn dark(accent: Hsla) -> Self {
        Self {
            bg: color(0x060606),
            shell: color(0x0d0d0d),
            dialog: color(0x161616),
            raised: color(0x343438),
            card: color(0x0e0e0e),
            text: color(0xe8e8ea),
            muted: color(0xa9a9ae),
            faint: color(0x85858a),
            accent,
            accent_soft: accent.opacity(0.16),
            border: hsla(0., 0., 1., 0.07),
            border_strong: hsla(0., 0., 1., 0.13),
            danger: color(0xf87171),
            success: color(0x4ade80),
            is_dark: true,
        }
    }

    pub fn light(accent: Hsla) -> Self {
        Self {
            bg: color(0xffffff),
            shell: color(0xf3f3f5),
            dialog: color(0xffffff),
            raised: color(0xdadae0),
            card: color(0xffffff),
            text: color(0x303035),
            muted: color(0x62626a),
            faint: color(0x797981),
            accent,
            accent_soft: accent.opacity(0.14),
            border: hsla(0., 0., 0., 0.07),
            border_strong: hsla(0., 0., 0., 0.13),
            danger: color(0xdc2626),
            success: color(0x16a34a),
            is_dark: false,
        }
    }
}

/// Global theme handle — views read `Theme::of(cx)`.
#[derive(Clone)]
pub struct Theme {
    pub palette: Palette,
}

impl Global for Theme {}

impl Theme {
    pub fn of(cx: &App) -> Self {
        cx.global::<Theme>().clone()
    }

    pub fn install(settings: &Settings, system_dark: bool, cx: &mut App) {
        let dark = match settings.appearance {
            Appearance::Dark => true,
            Appearance::Light => false,
            Appearance::System => system_dark,
        };
        let accent = accent_value(&settings.accent, dark);
        cx.set_global(Theme {
            palette: if dark {
                Palette::dark(accent)
            } else {
                Palette::light(accent)
            },
        });
    }
}
