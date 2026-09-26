//! Sidebar — the frosted rail: spaces switcher, groups, tab rows.
//! Painted translucent so the window's behind-window blur frosts through.

use gpui::{
    Context, Entity, MouseButton, ScrollHandle, SharedString, Window, div, prelude::*, px,
};

use crate::assets::icons;
use crate::store::Store;
use crate::theme::{Theme, radius, space};
use crate::views::glyph;
use hifi_core::TabKind;

pub struct Sidebar {
    store: Entity<Store>,
    scroll: ScrollHandle,
}

impl Sidebar {
    pub fn new(store: Entity<Store>, _cx: &mut Context<Self>) -> Self {
        Self {
            store,
            scroll: ScrollHandle::new(),
        }
    }
}

fn tab_icon(tab: &hifi_core::Tab, p: &crate::theme::Palette) -> gpui::Svg {
    let (path, color) = match tab.kind {
        TabKind::NewTab => (icons::HOME, p.faint),
        TabKind::Terminal => (icons::TERMINAL, p.faint),
        TabKind::Agent => (icons::BOT, p.faint),
        TabKind::Diff => (icons::GIT_BRANCH, p.faint),
        TabKind::Preview => (icons::EYE, p.faint),
        TabKind::Settings => (icons::SETTINGS, p.faint),
        TabKind::Notes => (icons::PEN_NEW_SQUARE, p.faint),
        TabKind::Web => (icons::GLOBE, p.faint),
    };
    glyph(path, 15., color)
}

/// Harness brand mark for agent/terminal commands (Radius-style header icon).
pub fn mark_icon(command: &str) -> &'static str {
    let c = command.to_lowercase();
    if c.contains("claude") {
        icons::CLAUDE_MARK
    } else if c.contains("codex") || c.contains("openai") {
        icons::OPENAI_MARK
    } else if c.contains("cursor") {
        icons::CURSOR_MARK
    } else if c.contains("grok") {
        icons::GROK_MARK
    } else if c.contains("amp") {
        icons::AMP_MARK
    } else if c.contains("devin") {
        icons::DEVIN_MARK
    } else if c.contains("opencode") {
        icons::OPENCODE_MARK
    } else if c.contains("kimi") {
        icons::KIMI_MARK
    } else if c.contains("pi") && !c.contains("pipe") {
        icons::PI_MARK
    } else if c.contains("antigravity") {
        icons::ANTIGRAVITY_MARK
    } else {
        icons::TERMINAL
    }
}

impl gpui::Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(cx).palette;
        let store = self.store.read(cx);
        let compact = store.state.settings.compact_sidebar;
        let sb_width = store.state.settings.sidebar_width;
        let Some(space_state) = store.state.active_space() else {
            return div().id("sidebar-empty");
        };
        let space_id = space_state.id.clone();
        let space_name = space_state.name.clone();
        let spaces: Vec<_> = store.state.spaces.clone();
        let groups: Vec<_> = space_state.groups.clone();
        let active_group = space_state.active_group.clone();
        let store_e = self.store.clone();

        let mut bar = div()
            .id("sidebar")
            .w(if compact { px(56.) } else { px(sb_width) })
            .h_full()
            .flex()
            .flex_col()
            // Translucent shell tint — the behind-window blur frosts through.
            .bg(p.shell.opacity(if p.is_dark { 0.45 } else { 0.35 }))
            .border_r_1()
            .border_color(p.border)
            .pt(px(38.)) // clear of the titlebar
            .pb(px(8.));

        // Space switcher.
        let mut space_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(space::XS)
            .px(space::SM)
            .pb(space::SM);
        for s in &spaces {
            let store = store_e.clone();
            let id = s.id.clone();
            let on = s.id == space_id;
            let letter: String = s.name.chars().next().unwrap_or('S').to_uppercase().collect();
            space_row = space_row.child(
                div()
                    .id(SharedString::from(format!("space-{id}")))
                    .size(px(26.))
                    .rounded(radius::ROUND)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .bg(if on { p.accent.opacity(0.25) } else { gpui::transparent_white() })
                    .text_color(if on { p.accent } else { p.muted })
                    .border_1()
                    .border_color(if on { p.accent.opacity(0.4) } else { gpui::transparent_white() })
                    .hover(|s| s.bg(p.raised.opacity(0.35)))
                    .child(letter)
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        store.update(cx, |s, cx| s.switch_space(&id, cx));
                    }),
            );
        }
        {
            let store = store_e.clone();
            space_row = space_row.child(
                div()
                    .id("space-add")
                    .size(px(26.))
                    .rounded(radius::ROUND)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(p.faint)
                    .hover(|s| s.bg(p.raised.opacity(0.35)).text_color(p.muted))
                    .child(glyph(icons::PLUS, 13., p.faint))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        store.update(cx, |s, cx| s.create_space("Space", cx));
                    }),
            );
        }
        bar = bar.child(space_row);

        if !compact {
            bar = bar.child(
                div()
                    .px(space::SM)
                    .pb(px(2.))
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(p.faint)
                    .child(space_name),
            );
        }

        // Groups + tab rows.
        let mut list = div()
            .id("sidebar-scroll")
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(space::SM);

        for group in groups {
            let gid = group.id.clone();
            let g_active = active_group.as_ref() == Some(&gid);
            // Group header — cosmos's section-header style: 11px medium,
            // muted-60%, px(8) pt(12) pb(4), with a quiet + on the right.
            let store_g = store_e.clone();
            let store_g2 = store_e.clone();
            let mut header = div()
                .id(SharedString::from(format!("group-{gid}")))
                .flex()
                .items_center()
                .gap(space::XS)
                .px(space::SM)
                .pt(px(12.))
                .pb(px(4.))
                .cursor_pointer()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(if g_active {
                    p.muted.opacity(0.9)
                } else {
                    p.faint.opacity(0.6)
                });
            if !compact {
                header = header
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(group.name.clone()),
                    )
                    .child({
                        let gid2 = gid.clone();
                        div()
                            .id(SharedString::from(format!("group-new-{gid}")))
                            .cursor_pointer()
                            .rounded(px(4.))
                            .hover(|s| s.bg(p.raised.opacity(0.5)))
                            .child(glyph(icons::PLUS, 11., p.faint))
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                store_g2.update(cx, |s, cx| {
                                    s.open_tab("hifi://newtab", Some(gid2.clone()), None, cx);
                                });
                            })
                    });
            } else {
                header = header.child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(group.name.chars().next().unwrap_or('G').to_uppercase().collect::<String>()),
                );
            }
            header = header.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                store_g.update(cx, |s, cx| s.set_active_group(&gid, cx));
            });
            list = list.child(header);

            if compact {
                continue;
            }

            // Tab rows (pinned first, Radius's Kind/Title labels).
            let mut tab_ids = group.tab_ids();
            tab_ids.sort_by_key(|id| {
                store.state.tab(id).map(|t| !t.pinned).unwrap_or(true)
            });
            for tid in tab_ids {
                let Some(tab) = store.state.tab(&tid) else { continue };
                let is_active = group.active_tab.as_ref() == Some(&tid) && g_active;
                let store_t = store_e.clone();
                let store_t2 = store_e.clone();
                let tid2 = tid.clone();
                // Cosmos row geometry: rounded(8) px(8) py(6) gap(8),
                // 13px text, 15px icon; selected = raised bg + medium weight.
                let mut row = div()
                    .id(SharedString::from(format!("tab-{tid}")))
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .px(space::SM)
                    .py(px(6.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .bg(if is_active {
                        p.raised
                    } else {
                        gpui::transparent_white()
                    })
                    .hover(|s| {
                        if is_active {
                            s
                        } else {
                            s.bg(p.raised.opacity(0.5))
                        }
                    })
                    .child(tab_icon(tab, &p));
                if tab.pinned {
                    row = row.child(
                        div()
                            .w(px(2.))
                            .h(px(12.))
                            .rounded_full()
                            .bg(p.accent),
                    );
                }
                row = row
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(13.))
                            .when(is_active, |d| d.font_weight(gpui::FontWeight::MEDIUM))
                            .text_color(if is_active { p.text } else { p.muted })
                            .child(tab.display_title()),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("close-{tid}")))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(16.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .child(glyph(icons::CLOSE, 10., p.faint))
                            .hover(|s| s.bg(p.danger.opacity(0.2)))
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                store_t2.update(cx, |s, cx| s.close_tab(&tid2, cx));
                            }),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        store_t.update(cx, |s, cx| s.focus_tab(&tid, cx));
                    });
                list = list.child(row);
            }
        }

        bar = bar.child(list);

        // Bottom bar: new group + settings.
        let store_b = store_e.clone();
        bar = bar.child(
            div()
                .flex()
                .items_center()
                .gap(space::XS)
                .px(space::SM)
                .pt(space::XS)
                .border_t_1()
                .border_color(p.border)
                .child({
                    let store = store_b.clone();
                    div()
                        .id("new-group")
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(10.))
                        .h(px(30.))
                        .rounded(radius::ROUND)
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .hover(|s| s.bg(p.raised.opacity(0.35)).text_color(p.text))
                        .child(glyph(icons::FOLDER_WITH_FILES, 13., p.faint))
                        .when(!compact, |d| d.child("New group"))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            store.update(cx, |s, cx| {
                                s.create_group("Group", None, cx);
                            });
                        })
                })
                .child(div().flex_1())
                .child({
                    let store = store_b.clone();
                    div()
                        .id("open-settings")
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(radius::ROUND)
                        .cursor_pointer()
                        .hover(|s| s.bg(p.raised.opacity(0.35)))
                        .child(glyph(icons::SETTINGS, 15., p.muted))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            store.update(cx, |s, cx| {
                                s.open_tab("hifi://settings", None, None, cx);
                            });
                        })
                }),
        );

        bar
    }
}
