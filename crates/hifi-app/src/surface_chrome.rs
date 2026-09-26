//! Shared metrics for surface toolbars and their controls — cosmos
//! `surface_chrome.rs`, ported with the palette passed explicitly.

use gpui::{Div, div, prelude::*, px};

use crate::theme::{Palette, Theme};

pub const HEADER_HEIGHT: f32 = Theme::TITLEBAR_HEIGHT;
pub const CONTROL_SIZE: f32 = 24.0;
pub const CONTROL_RADIUS: f32 = 6.0;
pub const ICON_SIZE: f32 = 14.0;
pub const CONTROL_GAP: f32 = 4.0;
pub const EDGE_INSET: f32 = 8.0;

/// Shared field treatment for the browser address and search controls.
pub fn input(p: &Palette) -> Div {
    div()
        .h(px(CONTROL_SIZE))
        .min_w_0()
        .flex_1()
        .px(px(8.0))
        .rounded(px(CONTROL_RADIUS))
        .bg(p.ink(0.035))
        .flex()
        .items_center()
        .gap(px(6.0))
        .text_size(px(11.5))
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
        .border_t_1()
        .border_b_1()
        .border_color(p.border)
        .bg(if Theme::is_frost() {
            p.shell.opacity(0.26)
        } else {
            p.shell
        })
}
