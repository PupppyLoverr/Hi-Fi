//! Sidebar — the frosted rail, laid out like Radius: window controls row,
//! pinned tiles, group cards (a split renders as one segmented row), a
//! New Tab row, and the space switcher at the bottom. Painted translucent
//! so the window's behind-window blur frosts through.

use gpui::{
    Context, Entity, Hsla, MouseButton, ScrollHandle, SharedString, Window, WindowControlArea, div,
    hsla, prelude::*, px,
};

use crate::assets::icons;
use crate::store::Store;
use crate::theme::{Palette, Theme, radius};
use crate::views::glyph;
use hifi_core::{SplitNode, Tab, TabId, TabKind};

/// Width of the rail in compact mode — wide enough to clear the traffic lights.
pub const COMPACT_WIDTH: f32 = 76.;
/// Height of the rail's top row; the traffic lights are centred in it.
pub const TOP_ROW: f32 = 46.;

pub struct Sidebar {
    store: Entity<Store>,
    scroll: ScrollHandle,
}

impl Sidebar {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self {
            store,
            scroll: ScrollHandle::new(),
        }
    }
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

pub fn kind_icon(tab: &Tab) -> &'static str {
    match tab.kind {
        TabKind::Web => icons::GLOBE,
        TabKind::NewTab => icons::HOME,
        TabKind::Terminal | TabKind::Agent => mark_icon(&tab.command),
        TabKind::Diff => icons::GIT_BRANCH,
        TabKind::Preview => icons::EYE,
        TabKind::Settings => icons::SETTINGS,
        TabKind::Notes => icons::PEN_NEW_SQUARE,
    }
}

/// Stable per-host hue so a site keeps its colour across restarts.
fn host_hue(host: &str) -> f32 {
    let h = host.bytes().fold(2166136261u32, |acc, b| {
        (acc ^ u32::from(b)).wrapping_mul(16777619)
    });
    (h % 360) as f32 / 360.
}

/// Site monogram standing in for a favicon — no image decoding or network
/// fetch, so it costs nothing to paint.
pub fn tab_badge(tab: &Tab, size: f32, p: &Palette) -> gpui::AnyElement {
    if tab.kind != TabKind::Web {
        return glyph(kind_icon(tab), size, p.muted).into_any_element();
    }
    let host = hifi_core::host_of(&tab.url);
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    let letter: String = host
        .chars()
        .find(|c| c.is_alphanumeric())
        .unwrap_or('•')
        .to_uppercase()
        .collect();
    let hue = host_hue(&host);
    let (bg, fg): (Hsla, Hsla) = if p.is_dark {
        (hsla(hue, 0.45, 0.28, 1.), hsla(hue, 0.9, 0.86, 1.))
    } else {
        (hsla(hue, 0.6, 0.9, 1.), hsla(hue, 0.6, 0.32, 1.))
    };
    div()
        .size(px(size + 1.))
        .flex_none()
        .rounded(px(4.))
        .bg(bg)
        .text_color(fg)
        .text_size(px(size * 0.66))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(letter)
        .into_any_element()
}

/// Leaf stacks of a split tree, in visual order.
fn stacks(node: &SplitNode, out: &mut Vec<Vec<TabId>>) {
    match node {
        SplitNode::Leaf { tabs } => out.push(tabs.clone()),
        SplitNode::Split { first, second, .. } => {
            stacks(first, out);
            stacks(second, out);
        }
    }
}

fn icon_button(
    id: impl Into<SharedString>,
    icon: &'static str,
    p: Palette,
    on_click: impl Fn(&mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .size(px(26.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(radius::ROUND)
        .cursor_pointer()
        .hover(|s| s.bg(p.raised.opacity(0.45)))
        .child(glyph(icon, 15., p.muted))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            on_click(cx)
        })
}

fn section_header(label: &'static str, p: Palette) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .h(px(26.))
        .px(px(8.))
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(p.faint.opacity(0.85))
        .child(div().flex_1().child(label))
}

impl Sidebar {
    fn tab_row(&self, tab: &Tab, active: bool, compact: bool, p: Palette) -> gpui::AnyElement {
        let store_f = self.store.clone();
        let store_m = self.store.clone();
        let store_x = self.store.clone();
        let (tid, tid_m, tid_x) = (tab.id.clone(), tab.id.clone(), tab.id.clone());
        div()
            .id(SharedString::from(format!("tab-{}", tab.id)))
            .group("sb-row")
            .h(px(30.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .rounded(px(8.))
            .cursor_pointer()
            .when(compact, |d| d.justify_center())
            .when(active, |d| d.bg(p.raised.opacity(0.7)))
            .when(!active, |d| d.hover(|s| s.bg(p.raised.opacity(0.35))))
            .child(tab_badge(tab, 15., &p))
            .when(!compact, |d| {
                d.child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(13.))
                        .text_color(if active { p.text } else { p.muted })
                        .child(tab.display_title()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("close-{}", tab.id)))
                        .size(px(18.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .opacity(0.)
                        .group_hover("sb-row", |s| s.opacity(1.))
                        .hover(|s| s.bg(p.raised))
                        .child(glyph(icons::CLOSE, 10., p.muted))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            store_x.update(cx, |s, cx| s.close_tab(&tid_x, cx));
                        }),
                )
            })
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                store_f.update(cx, |s, cx| s.focus_tab(&tid, cx));
            })
            .on_mouse_down(MouseButton::Middle, move |_, _, cx| {
                store_m.update(cx, |s, cx| s.close_tab(&tid_m, cx));
            })
            .into_any_element()
    }

    /// Radius shows a split as one row: a segment per visible pane.
    fn split_row(
        &self,
        gid: &str,
        shown: &[Tab],
        active: Option<&TabId>,
        p: Palette,
    ) -> gpui::AnyElement {
        let mut row = div()
            .id(SharedString::from(format!("split-{gid}")))
            .h(px(30.))
            .flex()
            .flex_none()
            .items_center()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(8.))
            .bg(p.raised.opacity(0.22))
            .border_1()
            .border_color(p.border);
        for tab in shown {
            let on = active == Some(&tab.id);
            let store = self.store.clone();
            let tid = tab.id.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("seg-{}", tab.id)))
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(6.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(on, |d| d.bg(p.raised.opacity(0.8)))
                    .when(!on, |d| d.hover(|s| s.bg(p.raised.opacity(0.4))))
                    .child(tab_badge(tab, 13., &p))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .text_color(if on { p.text } else { p.muted })
                            .child(tab.display_title()),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        store.update(cx, |s, cx| s.focus_tab(&tid, cx));
                    }),
            );
        }
        row.into_any_element()
    }
}

impl gpui::Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(cx).palette;
        let store = self.store.read(cx);
        let compact = store.state.settings.compact_sidebar;
        let width = if compact {
            COMPACT_WIDTH
        } else {
            store.state.settings.sidebar_width
        };
        let Some(space) = store.state.active_space() else {
            return div().id("sidebar-empty");
        };
        let space_id = space.id.clone();
        let spaces: Vec<(String, String)> = store
            .state
            .spaces
            .iter()
            .map(|s| (s.id.clone(), s.name.clone()))
            .collect();
        let groups = space.groups.clone();
        let active_group = space
            .active_group
            .clone()
            .or_else(|| groups.first().map(|g| g.id.clone()));
        let tab_of = |id: &TabId| store.state.tab(id).cloned();
        let pinned: Vec<Tab> = groups
            .iter()
            .flat_map(|g| g.tab_ids())
            .filter_map(|id| tab_of(&id))
            .filter(|t| t.pinned)
            .collect();
        let active_tab = store.active_tab_id();

        let mut bar = div()
            .id("sidebar")
            .w(px(width))
            .h_full()
            .flex()
            .flex_col()
            .bg(p.shell.opacity(if p.is_dark { 0.55 } else { 0.72 }));

        // Window controls row — the traffic lights sit in its left 76px.
        let store_t = self.store.clone();
        let store_n = self.store.clone();
        bar = bar.child(
            div()
                .id("sidebar-top")
                .h(px(TOP_ROW))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(2.))
                .pl(px(COMPACT_WIDTH))
                .pr(px(8.))
                .window_control_area(WindowControlArea::Drag)
                .child(div().flex_1())
                .when(!compact, |d| {
                    d.child(icon_button(
                        "sb-toggle",
                        icons::SIDEBAR_LEFT,
                        p,
                        move |cx| {
                            store_t.update(cx, |s, cx| {
                                s.sidebar_collapsed = true;
                                cx.notify();
                            });
                        },
                    ))
                    .child(icon_button(
                        "sb-newtab",
                        icons::PLUS,
                        p,
                        move |cx| {
                            store_n.update(cx, |s, cx| {
                                s.open_tab("hifi://newtab", None, None, cx);
                            });
                        },
                    ))
                }),
        );

        // Pinned tiles — Radius's favicon grid above the tree.
        if !pinned.is_empty() {
            let cols = if compact { 1. } else { 4. };
            let gap = 6.;
            let tile_w = ((width - 16. - gap * (cols - 1.)) / cols).floor();
            let mut grid = div()
                .flex()
                .flex_none()
                .flex_wrap()
                .gap(px(gap))
                .px(px(8.))
                .pb(px(8.));
            for tab in &pinned {
                let on = active_tab.as_ref() == Some(&tab.id);
                let store = self.store.clone();
                let tid = tab.id.clone();
                grid = grid.child(
                    div()
                        .id(SharedString::from(format!("pin-{}", tab.id)))
                        .w(px(tile_w))
                        .h(px(38.))
                        .rounded(px(10.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .bg(p.raised.opacity(if on { 0.75 } else { 0.3 }))
                        .border_1()
                        .border_color(if on { p.border_strong } else { p.border })
                        .hover(|s| s.bg(p.raised.opacity(0.55)))
                        .child(tab_badge(tab, 16., &p))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            store.update(cx, |s, cx| s.focus_tab(&tid, cx));
                        }),
                );
            }
            bar = bar.child(grid);
        }

        let mut list = div()
            .id("sidebar-scroll")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .gap(px(4.))
            .px(px(8.));

        if !compact {
            let store_g = self.store.clone();
            list = list.child(section_header("Groups", p).child(icon_button(
                "sb-newgroup",
                icons::FOLDER_WITH_FILES,
                p,
                move |cx| {
                    store_g.update(cx, |s, cx| {
                        s.create_group("Group", None, cx);
                    });
                },
            )));
        }

        for group in &groups {
            let gid = group.id.clone();
            let g_active = active_group.as_ref() == Some(&gid);
            let active_in_group = if g_active {
                group.active_tab.clone()
            } else {
                None
            };

            let mut rows: Vec<gpui::AnyElement> = Vec::new();
            let mut all = Vec::new();
            if let Some(root) = &group.root {
                stacks(root, &mut all);
            }
            let mut shown_ids: Vec<TabId> = Vec::new();
            if all.len() > 1 {
                let shown: Vec<Tab> = all
                    .iter()
                    .filter_map(|stack| {
                        stack
                            .iter()
                            .find(|t| group.active_tab.as_ref() == Some(*t))
                            .or_else(|| stack.last())
                            .and_then(tab_of)
                    })
                    .collect();
                shown_ids = shown.iter().map(|t| t.id.clone()).collect();
                if compact {
                    for t in &shown {
                        rows.push(self.tab_row(
                            t,
                            active_in_group.as_ref() == Some(&t.id),
                            true,
                            p,
                        ));
                    }
                } else {
                    rows.push(self.split_row(&gid, &shown, active_in_group.as_ref(), p));
                }
            }
            for id in all.iter().flatten() {
                if shown_ids.contains(id) {
                    continue;
                }
                let Some(tab) = tab_of(id) else { continue };
                if tab.pinned {
                    continue;
                }
                rows.push(self.tab_row(&tab, active_in_group.as_ref() == Some(id), compact, p));
            }

            if compact {
                list = list.children(rows);
                list = list.child(div().h(px(1.)).mx(px(10.)).my(px(2.)).bg(p.border));
                continue;
            }

            let store_h = self.store.clone();
            let store_plus = self.store.clone();
            let (gid_h, gid_plus) = (gid.clone(), gid.clone());
            let count = group.tab_ids().len();
            let header = div()
                .id(SharedString::from(format!("group-{gid}")))
                .group("sb-group")
                .h(px(28.))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .px(px(6.))
                .rounded(px(7.))
                .cursor_pointer()
                .hover(|s| s.bg(p.raised.opacity(0.25)))
                .child(glyph(
                    if g_active {
                        icons::FOLDER_WITH_FILES
                    } else {
                        icons::FOLDER
                    },
                    14.,
                    if g_active { p.text } else { p.faint },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(12.5))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(if g_active { p.text } else { p.muted })
                        .child(group.name.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.faint.opacity(0.7))
                        .group_hover("sb-group", |s| s.opacity(0.))
                        .child(count.to_string()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("group-plus-{gid}")))
                        .size(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .opacity(0.)
                        .group_hover("sb-group", |s| s.opacity(1.))
                        .hover(|s| s.bg(p.raised))
                        .child(glyph(icons::PLUS, 11., p.muted))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            store_plus.update(cx, |s, cx| {
                                s.open_tab("hifi://newtab", Some(gid_plus.clone()), None, cx);
                            });
                        }),
                )
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    store_h.update(cx, |s, cx| s.set_active_group(&gid_h, cx));
                });

            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(px(2.))
                    .p(px(4.))
                    .rounded(radius::CARD)
                    .when(g_active, |d| {
                        d.bg(p.raised.opacity(if p.is_dark { 0.18 } else { 0.28 }))
                            .border_1()
                            .border_color(p.border)
                    })
                    .child(header)
                    .children(rows),
            );
        }

        // "+ New Tab" — Radius's trailing row.
        let store_nt = self.store.clone();
        list = list.child(
            div()
                .id("sb-newtab-row")
                .h(px(30.))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .rounded(px(8.))
                .cursor_pointer()
                .when(compact, |d| d.justify_center().px(px(8.)))
                .text_size(px(13.))
                .text_color(p.faint)
                .hover(|s| s.bg(p.raised.opacity(0.3)).text_color(p.muted))
                .child(glyph(icons::PLUS, 14., p.faint))
                .when(!compact, |d| d.child("New Tab"))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    store_nt.update(cx, |s, cx| {
                        s.open_tab("hifi://newtab", None, None, cx);
                    });
                }),
        );
        bar = bar.child(list);

        // Bottom bar: spaces (Arc-style monograms) + settings.
        let mut bottom = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .h(px(44.))
            .when(compact, |d| d.flex_col().h_auto().py(px(8.)));
        for (id, name) in &spaces {
            let on = *id == space_id;
            let store = self.store.clone();
            let sid = id.clone();
            let letter: String = name.chars().next().unwrap_or('S').to_uppercase().collect();
            bottom = bottom.child(
                div()
                    .id(SharedString::from(format!("space-{id}")))
                    .size(px(24.))
                    .flex_none()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_size(px(10.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .bg(if on {
                        p.accent.opacity(0.22)
                    } else {
                        p.raised.opacity(0.2)
                    })
                    .text_color(if on { p.accent } else { p.faint })
                    .border_1()
                    .border_color(if on { p.accent.opacity(0.45) } else { p.border })
                    .hover(|s| s.bg(p.raised.opacity(0.45)))
                    .child(letter)
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        store.update(cx, |s, cx| s.switch_space(&sid, cx));
                    }),
            );
        }
        let store_sp = self.store.clone();
        let store_st = self.store.clone();
        bottom = bottom
            .child(icon_button("space-add", icons::PLUS, p, move |cx| {
                store_sp.update(cx, |s, cx| s.create_space("Space", cx));
            }))
            .when(!compact, |d| d.child(div().flex_1()))
            .child(icon_button(
                "open-settings",
                icons::SETTINGS,
                p,
                move |cx| {
                    store_st.update(cx, |s, cx| {
                        s.open_tab("hifi://settings", None, None, cx);
                    });
                },
            ));
        bar.child(bottom)
    }
}
