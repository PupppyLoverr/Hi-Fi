//! Hi-Fi theme — the cosmos design tokens, ported 1:1.
//!
//! Dark:  bg/card #0b0b0c / shell #141416 / dialog #161616 / raised #343438 /
//!        text #e8e8ea / muted #a9a9ae / faint #85858a
//! Light: bg #ffffff / shell #eef3f8 / card #ffffff / text #303035 /
//!        muted #62626a / faint #797981
//! Radii: 12 / 10 / 9 / 6. Fonts: platform UI font + Geist Mono (code/terminal).

use gpui::{App, BoxShadow, Global, Hsla, hsla, px};
use hifi_core::{Appearance, Settings};

pub const FONT_MONO: &str = "Geist Mono";
pub const CARD_INSET: f32 = 8.0;
pub const CARD_RADIUS: f32 = 12.0;
pub const TOOLBAR_H: f32 = 44.0;
pub const PANE_STRIP_H: f32 = 30.0;
pub const ROW_H: f32 = 32.0;
pub const ROW_RADIUS: f32 = 9.0;
pub const SECTION_HEADER_H: f32 = 28.0;
pub const SIDEBAR_WIDTH: f32 = 248.0;

pub mod space {
    use gpui::{Pixels, px};
    pub const SM: Pixels = px(8.);
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
    pub border: Hsla,
    pub border_strong: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    pub is_dark: bool,
}

pub fn color(h: u32) -> Hsla {
    gpui::rgb(h).into()
}

/// Accent families — name → (dark value, light value).
pub const ACCENTS: &[(&str, u32, u32)] = &[
    ("mono", 0xe8e8ea, 0x1d1d20),
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

/// Cosmos `GLASS_ALPHA`: the shell tint over the blurred desktop. Linux has
/// no compositor blur contract, so the shell stays opaque there.
pub const GLASS_ALPHA: f32 = if cfg!(any(target_os = "macos", target_os = "windows")) {
    0.80
} else {
    1.0
};
const INK_FILL_SCALE: f32 = 0.7;
const INK_HAIRLINE_SCALE: f32 = 1.4;

impl Palette {
    /// Translucent fill ink (cosmos `ink`): soft-white on dark, soft-black on
    /// light, alphas quoted in dark-mode terms.
    pub fn ink(&self, alpha: f32) -> Hsla {
        if self.is_dark {
            hsla(0., 0., 1., alpha)
        } else {
            hsla(0., 0., 0., alpha * INK_FILL_SCALE)
        }
    }

    /// Hairline ink (cosmos `hairline`).
    pub fn hairline(&self, alpha: f32) -> Hsla {
        if self.is_dark {
            hsla(0., 0., 1., alpha)
        } else {
            hsla(0., 0., 0., (alpha * INK_HAIRLINE_SCALE).min(0.5))
        }
    }

    /// Interactive-state wash (cosmos `wash`).
    pub fn wash(&self, alpha: f32) -> Hsla {
        if self.is_dark {
            hsla(0., 0., 0.92, alpha)
        } else {
            hsla(0., 0., 0.10, alpha * INK_FILL_SCALE)
        }
    }

    /// Row hover plate (cosmos `glass_hover`).
    pub fn glass_hover(&self) -> Hsla {
        self.wash(0.08)
    }

    pub fn diff_add(&self) -> Hsla {
        self.success.opacity(0.10)
    }

    pub fn diff_del(&self) -> Hsla {
        self.danger.opacity(0.10)
    }

    /// Selected row plate (cosmos `glass_selected_bg`).
    pub fn selected(&self) -> Hsla {
        self.wash(if self.is_dark { 0.11 } else { 0.06 })
    }

    /// Selected sidebar pill: a white card on light glass, the cosmos plate
    /// on dark.
    pub fn pill(&self) -> Hsla {
        if self.is_dark {
            self.selected()
        } else {
            hsla(0., 0., 1., 0.92)
        }
    }

    /// Shell tint over the compositor-blurred desktop.
    pub fn glass(&self) -> Hsla {
        self.shell.opacity(GLASS_ALPHA)
    }

    /// Floating popover card over the frost (cosmos `glass_overlay`).
    pub fn glass_overlay(&self) -> Hsla {
        if Theme::is_frost() {
            self.dialog.opacity(0.86)
        } else {
            self.dialog
        }
    }

    pub fn shadow_card(&self) -> Vec<BoxShadow> {
        if self.is_dark {
            return Vec::new();
        }
        vec![
            BoxShadow::new(px(0.), px(1.), self.ink(0.04)).blur_radius(px(2.)),
            BoxShadow::new(px(0.), px(8.), self.ink(0.06)).blur_radius(px(24.)),
        ]
    }

    pub fn dark(accent: Hsla) -> Self {
        Self {
            bg: color(0x0b0b0c),
            shell: color(0x141416),
            dialog: color(0x161616),
            raised: color(0x343438),
            card: color(0x0b0b0c),
            text: color(0xe8e8ea),
            muted: color(0xa9a9ae),
            faint: color(0x85858a),
            accent,
            border: hsla(0., 0., 1., 0.07),
            border_strong: hsla(0., 0., 1., 0.13),
            danger: color(0xef6a6a),
            success: color(0x4ade80),
            is_dark: true,
        }
    }

    pub fn light(accent: Hsla) -> Self {
        Self {
            bg: color(0xffffff),
            shell: color(0xeef3f8),
            dialog: color(0xffffff),
            raised: color(0xdadae0),
            card: color(0xffffff),
            text: color(0x303035),
            muted: color(0x62626a),
            faint: color(0x797981),
            accent,
            border: hsla(0., 0., 0., 0.07),
            border_strong: hsla(0., 0., 0., 0.13),
            danger: color(0xd13d3d),
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
    pub const SPACE_SM: f32 = 8.0;
    pub const SIDEBAR_MIN: f32 = 200.0;
    pub const SIDEBAR_MAX: f32 = 320.0;
    pub const TOOLBAR_BTN: f32 = 24.0;
    pub const ADDRESS_H: f32 = 28.0;
    pub const ICON_OPTICAL: f32 = 15.0;

    /// Whether surfaces sit on the compositor-blurred glass shell.
    pub const fn is_frost() -> bool {
        GLASS_ALPHA < 1.0
    }

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
