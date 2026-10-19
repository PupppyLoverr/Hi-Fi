//! Shared metrics for surface toolbars and their controls — cosmos
//! `surface_chrome.rs`, ported with the palette passed explicitly.

use gpui::{Div, div, prelude::*, px};

use crate::theme::{Palette, TOOLBAR_H, Theme};

pub const HEADER_HEIGHT: f32 = TOOLBAR_H;
pub const CONTROL_SIZE: f32 = Theme::TOOLBAR_BTN;
pub const CONTROL_RADIUS: f32 = 6.0;
pub const ICON_SIZE: f32 = Theme::ICON_OPTICAL;
pub const CONTROL_GAP: f32 = 8.0;
pub const EDGE_INSET: f32 = 8.0;

/// Shared field treatment for the browser address and search controls.
pub fn input(p: &Palette) -> Div {
    div()
        .h(px(Theme::ADDRESS_H))
        .min_w_0()
        .flex_1()
        .px(px(8.0))
        .rounded(px(8.))
        .bg(p.bg.opacity(0.))
        .flex()
        .items_center()
        .gap(px(8.0))
        .text_size(px(13.))
}

pub fn toolbar(p: &Palette) -> Div {
    div()
        .h(px(HEADER_HEIGHT))
        .w_full()
        .flex_none()
        .px(px(EDGE_INSET))
        .flex()
        .items_center()
        .gap(px(CONTROL_GAP))
        .border_b_1()
        .border_color(p.border)
}
