//! Sidebar — cosmos's rail on Hi-Fi's model: quick actions, a Pinned
//! disclosure, one disclosure section per group (a split renders as one
//! segmented row), and the spaces/settings foot. It paints no fill of its
//! own; the shell lays the cosmos sidebar tone under it.

use std::collections::HashSet;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, Hsla, MouseButton, ScrollHandle, SharedString, Window,
    div, hsla, prelude::*, px,
};

use crate::assets::icons;
use crate::store::Store;
use crate::theme::{Palette, Theme};
use crate::views::glyph;
use hifi_core::{SplitNode, Tab, TabId, TabKind};

/// Width of the rail in compact mode — wide enough to clear the traffic lights.
pub const COMPACT_WIDTH: f32 = 76.;
/// cosmos `shell/spaces.rs` section metrics.
const SIDEBAR_SECTION_GAP: f32 = 12.0;
const SIDEBAR_DISCLOSURE_HEADER_HEIGHT: f32 = 28.0;
const SIDEBAR_DISCLOSURE_BODY_INSET: f32 = 4.0;
const PINNED_KEY: &str = "__pinned";

pub struct Sidebar {
    store: Entity<Store>,
    scroll: ScrollHandle,
    /// Disclosure sections the user folded (group ids, or `PINNED_KEY`).
    collapsed: HashSet<String>,
}

impl Sidebar {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self {
            store,
            scroll: ScrollHandle::new(),
            collapsed: HashSet::new(),
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
        .size(px(24.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|s| s.bg(p.glass_hover()))
        .child(glyph(icon, 15., p.muted))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            on_click(cx)
        })
}

/// A cosmos sidebar action row: icon + label at rest text strength.
fn action_row(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    compact: bool,
    p: Palette,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(30.))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .rounded(px(8.))
        .cursor_pointer()
        .text_size(px(13.))
        .text_color(p.text.opacity(0.8))
        .hover(|s| s.bg(p.glass_hover()).text_color(p.text))
        .when(compact, |d| d.justify_center())
        .child(glyph(icon, 15., p.muted))
        .when(!compact, |d| d.child(label))
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
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .py(px(6.))
            .min_h(px(30.))
            .rounded(px(8.))
            .cursor_pointer()
            .when(compact, |d| d.justify_center())
            .when(active, |d| {
                d.bg(p.pill()).when(!p.is_dark, |d| {
                    d.shadow_xs().border_1().border_color(p.hairline(0.04))
                })
            })
            .when(!active, |d| d.hover(|s| s.bg(p.glass_hover())))
            .child(tab_badge(tab, 15., &p))
            .when(!compact, |d| {
                let sub = row_subtitle(tab);
                d.child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(13.))
                                .line_height(px(17.))
                                .text_color(if active { p.text } else { p.text.opacity(0.8) })
                                .child(row_title(tab)),
                        )
                        .when_some(sub, |d, sub| {
                            d.child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(11.))
                                    .line_height(px(14.))
                                    .text_color(p.faint)
                                    .child(sub),
                            )
                        }),
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
                        .hover(|s| s.bg(p.wash(0.14)))
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
            .bg(p.ink(0.03))
            .border_1()
            .border_color(p.hairline(0.06));
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
                    .when(on, |d| d.bg(p.pill()).when(!p.is_dark, |d| d.shadow_xs()))
                    .when(!on, |d| d.hover(|s| s.bg(p.glass_hover())))
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
    /// cosmos `sidebar_disclosure_header`: a 28px muted label row with a
    /// chevron; clicking folds the section.
    #[allow(clippy::too_many_arguments)]
    fn disclosure(
        &self,
        id: impl Into<SharedString>,
        key: &str,
        label: impl Into<SharedString>,
        open: bool,
        trailing: Option<AnyElement>,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = key.to_string();
        let chevron = glyph(
            if open {
                icons::ALT_ARROW_DOWN
            } else {
                icons::ALT_ARROW_RIGHT
            },
            12.,
            p.faint.opacity(0.7),
        );
        let store = self.store.clone();
        div()
            .id(id.into())
            .group("sb-disclosure")
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .h(px(SIDEBAR_DISCLOSURE_HEADER_HEIGHT))
            .px(px(Theme::SPACE_SM))
            .cursor_pointer()
            .child(
                div()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(p.muted.opacity(0.7))
                    .child(label.into()),
            )
            .child(div().flex_1())
            .children(trailing)
            .child(chevron)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if !this.collapsed.remove(&key) {
                    this.collapsed.insert(key.clone());
                }
                if key != PINNED_KEY {
                    store.update(cx, |s, cx| s.set_active_group(&key, cx));
                }
                cx.notify();
            }))
            .into_any_element()
    }
}

fn basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Row title: the page title for web/pages, the harness for sessions.
fn row_title(tab: &Tab) -> String {
    match tab.kind {
        TabKind::Agent if !tab.prompt.is_empty() => tab.prompt.clone(),
        TabKind::Notes if tab.title.is_empty() || tab.title == "Notes" => "Untitled".into(),
        TabKind::Notes => tab.title.clone(),
        TabKind::Terminal | TabKind::Agent | TabKind::Diff => {
            let t = tab.display_title();
            t.split_once('/').map_or(t.clone(), |(k, _)| k.to_string())
        }
        _ => tab.display_title(),
    }
}

/// cosmos's two-line session rows carry the workspace underneath.
fn row_subtitle(tab: &Tab) -> Option<String> {
    match tab.kind {
        TabKind::Terminal | TabKind::Agent | TabKind::Diff if !tab.cwd.is_empty() => {
            let mut s = basename(&tab.cwd);
            if !tab.branch.is_empty() {
                s = format!("{s} · {}", tab.branch);
            }
            Some(s)
        }
        _ => None,
    }
}

impl gpui::Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(cx).palette;
        let active_tab = self.store.read(cx).active_tab_id();
        let state = self.store.read(cx).state.clone();
        let compact = state.settings.compact_sidebar;
        let width = if compact {
            COMPACT_WIDTH
        } else {
            state.settings.sidebar_width
        };
        let Some(space) = state.active_space() else {
            return div().id("sidebar-empty");
        };
        let space_id = space.id.clone();
        let spaces: Vec<(String, String)> = state
            .spaces
            .iter()
            .map(|s| (s.id.clone(), s.name.clone()))
            .collect();
        let groups = space.groups.clone();
        let active_group = space
            .active_group
            .clone()
            .or_else(|| groups.first().map(|g| g.id.clone()));
        let tab_of = |id: &TabId| state.tab(id).cloned();
        let pinned: Vec<Tab> = groups
            .iter()
            .flat_map(|g| g.tab_ids())
            .filter_map(|id| tab_of(&id))
            .filter(|t| t.pinned)
            .collect();

        let mut bar = div().id("sidebar").w(px(width)).h_full().flex().flex_col();

        let mut list = div()
            .id("sidebar-scroll")
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .px(px(Theme::SPACE_SM))
            .pt(px(4.));

        // Workspace header: the space switcher (cycles spaces).
        let space_name = space.name.clone();
        let next_space = spaces
            .iter()
            .skip_while(|(id, _)| *id != space_id)
            .nth(1)
            .or_else(|| spaces.first())
            .map(|(id, _)| id.clone());
        let store_hdr = self.store.clone();
        let initial: String = space_name
            .chars()
            .next()
            .unwrap_or('S')
            .to_uppercase()
            .collect();
        list = list.child(
            div()
                .id("sb-space-header")
                .h(px(32.))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .px(px(6.))
                .rounded(px(8.))
                .cursor_pointer()
                .when(compact, |d| d.justify_center())
                .hover(|s| s.bg(p.glass_hover()))
                .child(
                    div()
                        .size(px(20.))
                        .flex_none()
                        .rounded_full()
                        .bg(p.accent)
                        .text_color(gpui::white())
                        .text_size(px(10.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(initial),
                )
                .when(!compact, |d| {
                    d.child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(p.text)
                            .child(space_name),
                    )
                    .child(glyph(icons::ALT_ARROW_DOWN, 11., p.faint))
                })
                .on_click(move |_, _, cx| {
                    if let Some(id) = next_space.clone() {
                        store_hdr.update(cx, |s, cx| s.switch_space(&id, cx));
                    }
                }),
        );

        // Favorites: pinned tabs as a 3-up tile grid.
        if !pinned.is_empty() {
            let mut grid = div().mt(px(8.)).flex().flex_wrap().flex_none().gap(px(6.));
            for tab in &pinned {
                let on = active_tab.as_ref() == Some(&tab.id);
                let store = self.store.clone();
                let tid = tab.id.clone();
                grid = grid.child(
                    div()
                        .id(SharedString::from(format!("tile-{}", tab.id)))
                        .h(px(38.))
                        .when(compact, |d| d.w_full())
                        .when(!compact, |d| {
                            d.w(px(((width - 2. * Theme::SPACE_SM - 12.) / 3.).floor()))
                        })
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(10.))
                        .cursor_pointer()
                        .bg(if p.is_dark {
                            p.ink(0.05)
                        } else {
                            hsla(0., 0., 1., 0.5)
                        })
                        .border_1()
                        .border_color(if on {
                            p.accent.opacity(0.6)
                        } else {
                            p.hairline(0.04)
                        })
                        .hover(|s| s.bg(p.pill()))
                        .child(tab_badge(tab, 16., &p))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            store.update(cx, |s, cx| s.focus_tab(&tid, cx));
                        }),
                );
            }
            list = list.child(grid);
        }

        // Chats (agent sessions) and Pages (Notion-style docs), space-wide.
        let of_kind = |kind: TabKind| -> Vec<Tab> {
            groups
                .iter()
                .flat_map(|g| g.tab_ids())
                .filter_map(|id| tab_of(&id))
                .filter(|t| t.kind == kind && !t.pinned)
                .collect()
        };
        let sections: [(
            &str,
            &str,
            &'static str,
            &'static str,
            &'static str,
            TabKind,
            bool,
        ); 2] = [
            (
                "sb-chats",
                "Chats",
                "New Chat",
                icons::PEN_NEW_SQUARE,
                "hifi://agent",
                TabKind::Agent,
                true,
            ),
            (
                "sb-pages",
                "Pages",
                "New Page",
                icons::DOCUMENT_ADD,
                "hifi://notes",
                TabKind::Notes,
                false,
            ),
        ];
        for (id, title, new_label, new_icon, url, kind, dock) in sections {
            let open = !self.collapsed.contains(id);
            list = list.child(div().h(px(SIDEBAR_SECTION_GAP)).flex_none());
            if !compact {
                list = list.child(self.disclosure(id, id, title, open, None, p, cx));
            }
            if !(open || compact) {
                continue;
            }
            let store = self.store.clone();
            let mut col = div().flex().flex_col().flex_none().gap(px(1.)).child(
                action_row(id, new_icon, new_label, compact, p)
                    .text_color(p.muted)
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        store.update(cx, |s, cx| {
                            if dock {
                                s.dock_open(url, cx);
                            } else {
                                s.open_tab(url, None, None, cx);
                            }
                        });
                    }),
            );
            for tab in of_kind(kind) {
                col =
                    col.child(self.tab_row(&tab, active_tab.as_ref() == Some(&tab.id), compact, p));
            }
            list = list.child(col);
        }

        // Tabs: the browser's own section, then one disclosure per group.
        list = list.child(div().h(px(SIDEBAR_SECTION_GAP)).flex_none());
        let store_nt = self.store.clone();
        list = list.child(
            action_row("sb-new-tab", icons::PLUS, "New Tab", compact, p)
                .text_color(p.muted)
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    store_nt.update(cx, |s, cx| {
                        s.open_tab("hifi://newtab", None, None, cx);
                    });
                }),
        );

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
                if tab.pinned || matches!(tab.kind, TabKind::Agent | TabKind::Notes) {
                    continue;
                }
                rows.push(self.tab_row(&tab, active_in_group.as_ref() == Some(id), compact, p));
            }

            list = list.child(div().h(px(SIDEBAR_SECTION_GAP)).flex_none());
            if compact {
                list = list.child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .gap(px(1.))
                        .children(rows),
                );
                continue;
            }
            let open = !self.collapsed.contains(&gid);
            let store_plus = self.store.clone();
            let gid_plus = gid.clone();
            let plus = div()
                .id(SharedString::from(format!("group-plus-{gid}")))
                .size(px(18.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .opacity(0.)
                .group_hover("sb-disclosure", |s| s.opacity(1.))
                .hover(|s| s.bg(p.wash(0.14)))
                .child(glyph(icons::PLUS, 11., p.muted))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    store_plus.update(cx, |s, cx| {
                        s.open_tab("hifi://newtab", Some(gid_plus.clone()), None, cx);
                    });
                })
                .into_any_element();
            let label = if g_active {
                group.name.clone()
            } else {
                format!("{}  ·  {}", group.name, group.tab_ids().len())
            };
            list = list.child(self.disclosure(
                SharedString::from(format!("group-{gid}")),
                &gid,
                label,
                open,
                Some(plus),
                p,
                cx,
            ));
            if open {
                list = list.child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .gap(px(1.))
                        .pb(px(SIDEBAR_DISCLOSURE_BODY_INSET))
                        .children(rows),
                );
            }
        }
        if !compact {
            let store_g = self.store.clone();
            list = list
                .child(div().h(px(SIDEBAR_SECTION_GAP)).flex_none())
                .child(
                    action_row(
                        "sb-newgroup",
                        icons::FOLDER_WITH_FILES,
                        "New Group",
                        false,
                        p,
                    )
                    .text_color(p.faint)
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        store_g.update(cx, |s, cx| {
                            s.create_group("Group", None, cx);
                        });
                    }),
                );
        }
        list = list.child(div().h(px(12.)).flex_none());
        bar = bar.child(list);

        // Footer: spaces + settings, above a hairline (cosmos sidebar foot).
        let mut bottom = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .px(px(Theme::SPACE_SM))
            .h(px(44.))
            .border_t_1()
            .border_color(p.hairline(0.05))
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
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .when(on, |d| d.bg(p.selected()).text_color(p.text))
                    .when(!on, |d| {
                        d.text_color(p.faint).hover(|s| s.bg(p.glass_hover()))
                    })
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
