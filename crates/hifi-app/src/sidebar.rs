//! Sidebar — cosmos's rail on Hi-Fi's model: quick actions, a Pinned
//! disclosure, one disclosure section per group (a split renders as one
//! segmented row), and the spaces/settings foot. It paints no fill of its
//! own; the shell lays the cosmos sidebar tone under it.

use std::{collections::HashSet, time::Duration};

use gpui::{
    Animation, AnimationExt, AnyElement, ClickEvent, Context, Entity, MouseButton, ScrollHandle,
    SharedString, Window, div, hsla, img, point, prelude::*, px,
};
use std::path::PathBuf;

use crate::assets::icons;
use crate::store::Store;
use crate::theme::{Palette, Theme};
use crate::views::glyph;
use hifi_core::{SplitNode, Tab, TabId, TabKind};

pub struct SpaceTooltip(pub String);

impl gpui::Render for SpaceTooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(cx).palette;
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(p.card)
            .border_1()
            .border_color(p.border)
            .text_size(px(12.))
            .text_color(p.text)
            .child(self.0.clone())
    }
}

/// Width of the rail in compact mode — wide enough to clear the traffic lights.
pub const COMPACT_WIDTH: f32 = 76.;
/// cosmos `shell/spaces.rs` section metrics.
const SIDEBAR_SECTION_GAP: f32 = 12.0;
#[allow(dead_code)]
const SIDEBAR_DISCLOSURE_BODY_INSET: f32 = 4.0;
const PINNED_KEY: &str = "__pinned";

pub struct Sidebar {
    store: Entity<Store>,
    scroll: ScrollHandle,
    /// Disclosure sections the user folded (group ids, or `PINNED_KEY`).
    collapsed: HashSet<String>,
    expanded: HashSet<String>,
    running_agents: HashSet<TabId>,
    failed_agents: HashSet<TabId>,
}

impl Sidebar {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self {
            store,
            scroll: ScrollHandle::new(),
            collapsed: HashSet::new(),
            expanded: HashSet::new(),
            running_agents: HashSet::new(),
            failed_agents: HashSet::new(),
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
    } else if c.contains("hermes") {
        icons::HERMES_MARK
    } else {
        icons::TERMINAL
    }
}

pub fn kind_icon(tab: &Tab) -> &'static str {
    if tab.kind == TabKind::Preview && hifi_docs::DocKind::from_path(&tab.url).is_some() {
        return icons::DOCUMENT;
    }
    match tab.kind {
        TabKind::Web => icons::GLOBAL,
        TabKind::NewTab => icons::MAGNIFER,
        TabKind::Terminal | TabKind::Agent => mark_icon(&tab.command),
        TabKind::Diff => icons::GIT_BRANCH,
        TabKind::Preview => icons::EYE,
        TabKind::Settings => icons::SETTINGS,
        TabKind::Notes => icons::PEN_NEW_SQUARE,
    }
}

pub fn tab_badge(tab: &Tab, size: f32, p: &Palette) -> gpui::AnyElement {
    if tab.kind != TabKind::Web {
        return glyph(kind_icon(tab), size, p.muted).into_any_element();
    }
    if let Some(path) = favicon_path(tab) {
        return img(path).size(px(size)).rounded(px(3.)).into_any_element();
    }
    glyph(icons::GLOBAL, size, p.muted).into_any_element()
}

pub fn favicon_path(tab: &Tab) -> Option<PathBuf> {
    if tab.kind != TabKind::Web {
        return None;
    }
    favicon_file(&tab.favicon_url)
}

pub fn favicon_file(path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path);
    (path.is_file()
        && matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "svg")
        ))
    .then_some(path)
}

/// Leaf stacks of a split tree, in visual order.
#[allow(dead_code)]
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

impl Sidebar {
    #[allow(clippy::too_many_arguments)]
    fn capped_tab_list(
        &self,
        tabs: &[Tab],
        key: &str,
        limit: usize,
        active_tab: Option<&TabId>,
        compact: bool,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active_index = active_tab.and_then(|active| tabs.iter().position(|t| &t.id == active));
        let rows = if compact {
            LimitedRows {
                visible: tabs.to_vec(),
                more: 0,
                show_less: false,
            }
        } else {
            limit_rows(tabs, active_index, limit, self.expanded.contains(key))
        };
        let mut list = div().flex().flex_col().flex_none().gap(px(1.));
        for tab in rows.visible {
            list = list.child(self.tab_row(&tab, active_tab == Some(&tab.id), compact, p));
        }

        if !compact && (rows.more > 0 || rows.show_less) {
            let key = key.to_string();
            let show_more = rows.more > 0;
            let label = if show_more {
                format!("Show {} more", rows.more)
            } else {
                "Show less".into()
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!(
                        "sb-list-action-{key}-{}",
                        if show_more { "more" } else { "less" }
                    )))
                    .h(px(Theme::ROW_H))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pl(px(46.))
                    .pr(px(8.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(p.glass_hover()))
                    .text_size(px(12.))
                    .text_color(p.faint)
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if show_more {
                            this.expanded.insert(key.clone());
                        } else {
                            this.expanded.remove(&key);
                        }
                        cx.notify();
                    })),
            );
        }
        list.into_any_element()
    }

    fn tab_row(&self, tab: &Tab, active: bool, compact: bool, p: Palette) -> gpui::AnyElement {
        let store_f = self.store.clone();
        let store_m = self.store.clone();
        let store_x = self.store.clone();
        let store_r = self.store.clone();
        let (tid, tid_m, tid_x) = (tab.id.clone(), tab.id.clone(), tab.id.clone());
        let tid_r = tab.id.clone();
        let status = if self.failed_agents.contains(&tab.id) {
            glyph(icons::DANGER_TRIANGLE, 12., p.danger).into_any_element()
        } else if self.running_agents.contains(&tab.id) {
            div()
                .size(px(6.))
                .rounded_full()
                .bg(p.text)
                .with_animation(
                    SharedString::from(format!("agent-pulse-{}", tab.id)),
                    Animation::new(Duration::from_millis(1200)).repeat(),
                    |dot, delta| dot.opacity(0.4 + 0.6 * delta),
                )
                .into_any_element()
        } else {
            div().size(px(6.)).flex_none().into_any_element()
        };
        div()
            .id(SharedString::from(format!("tab-{}", tab.id)))
            .group("sb-row")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .h(px(Theme::ROW_H))
            .rounded(px(Theme::ROW_RADIUS))
            .cursor_pointer()
            .when(compact, |d| d.justify_center())
            .when(active, |d| {
                d.bg(p.pill()).when(!p.is_dark, |d| {
                    d.shadow_xs().border_1().border_color(p.hairline(0.04))
                })
            })
            .when(!active, |d| d.hover(|s| s.bg(p.glass_hover())))
            .child(tab_badge(tab, 16., &p))
            .child(status)
            .when(!compact, |d| {
                let sub = row_subtitle(tab);
                d.child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(13.))
                                .line_height(px(17.))
                                .text_color(
                                    if tab.kind == TabKind::Notes
                                        && (tab.title.is_empty() || tab.title == "Untitled")
                                    {
                                        p.muted
                                    } else if active {
                                        p.text
                                    } else {
                                        p.text.opacity(0.8)
                                    },
                                )
                                .child(row_title(tab)),
                        )
                        .when_some(sub, |d, sub| {
                            d.child(
                                div()
                                    .max_w(px(90.))
                                    .flex_none()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(10.5))
                                    .text_color(p.faint)
                                    .opacity(0.)
                                    .group_hover("sb-row", |s| s.opacity(1.))
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
            .on_mouse_down(MouseButton::Right, move |event, _, cx| {
                cx.stop_propagation();
                store_r.update(cx, |s, cx| {
                    s.pending_context_menu = Some((tid_r.clone(), event.position));
                    cx.notify();
                });
            })
            .into_any_element()
    }

    /// Radius shows a split as one row: a segment per visible pane.
    #[allow(dead_code)]
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
        activate_group: bool,
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
            .h(px(Theme::SECTION_HEADER_H))
            .px(px(Theme::SPACE_SM))
            .cursor_pointer()
            .child(
                div()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(10.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(p.muted.opacity(0.7))
                    .child(label.into()),
            )
            .child(div().flex_1())
            .children(trailing)
            .child(
                div()
                    .opacity(0.)
                    .group_hover("sb-disclosure", |s| s.opacity(1.))
                    .child(chevron),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if !this.collapsed.remove(&key) {
                    this.collapsed.insert(key.clone());
                }
                if activate_group {
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

struct LimitedRows<T> {
    visible: Vec<T>,
    more: usize,
    show_less: bool,
}

fn limit_rows<T: Clone>(
    rows: &[T],
    active_index: Option<usize>,
    limit: usize,
    expanded: bool,
) -> LimitedRows<T> {
    if expanded {
        return LimitedRows {
            visible: rows.to_vec(),
            more: 0,
            show_less: rows.len() > limit,
        };
    }

    let mut visible: Vec<T> = rows.iter().take(limit).cloned().collect();
    if let Some(index) = active_index.filter(|index| *index >= limit && *index < rows.len()) {
        visible.push(rows[index].clone());
    }
    let more = rows.len().saturating_sub(visible.len());
    LimitedRows {
        visible,
        more,
        show_less: false,
    }
}

/// Row title: the page title for web/pages, the harness for sessions.
fn row_title(tab: &Tab) -> String {
    match tab.kind {
        TabKind::Agent if !tab.prompt.is_empty() => clean_prompt(&tab.display_title(), 48),
        TabKind::Notes if tab.title.is_empty() || tab.title == "Notes" => "Untitled".into(),
        TabKind::Notes => tab.title.clone(),
        _ => tab.display_title(),
    }
}

fn clean_prompt(prompt: &str, max_chars: usize) -> String {
    let cleaned = prompt
        .replace(['*', '`', '#', '_'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = cleaned.chars();
    let short: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else if short.is_empty() {
        "Agent".into()
    } else {
        short
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
        self.running_agents = self.store.read(cx).running_agents.clone();
        self.failed_agents = self.store.read(cx).failed_agents.clone();
        let compact = state.settings.compact_sidebar;
        let width = if compact {
            COMPACT_WIDTH
        } else {
            if state.settings.sidebar_width > 0. {
                state
                    .settings
                    .sidebar_width
                    .clamp(Theme::SIDEBAR_MIN, Theme::SIDEBAR_MAX)
            } else {
                Theme::SIDEBAR_WIDTH
            }
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

        // Workspace header: the space switcher menu.
        let space_name = space.name.clone();
        let store_hdr = self.store.clone();
        list = list.child(
            div()
                .id("sb-space-header")
                .h(px(Theme::SPACE_SWITCHER_H))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .px(px(6.))
                .rounded(px(8.))
                .cursor_pointer()
                .when(compact, |d| d.justify_center())
                .hover(|s| s.bg(p.glass_hover()))
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
                .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                    cx.stop_propagation();
                    store_hdr.update(cx, |s, cx| {
                        s.pending_space_menu = Some(event.position);
                        cx.notify();
                    });
                }),
        );
        let store_search = self.store.clone();
        list = list.child(
            div()
                .id("sb-search")
                .group("sb-search-row")
                .h(px(28.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(7.))
                .rounded(px(8.))
                .bg(p.ink(0.04))
                .border_1()
                .border_color(p.hairline(0.08))
                .cursor_pointer()
                .hover(|s| s.bg(p.ink(0.08)))
                .child(glyph(icons::MAGNIFER, 13., p.muted))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child("Search or jump…"),
                )
                .when(!compact, |d| {
                    d.child(div().text_size(px(10.)).text_color(p.faint).child("⌘K"))
                })
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    store_search.update(cx, |s, cx| s.toggle_command_bar(cx));
                }),
        );

        // Favorites: pinned tabs as a 3-up tile grid.
        if !pinned.is_empty() {
            list = list.child(div().h(px(SIDEBAR_SECTION_GAP)).flex_none());
            list = list.child(self.disclosure(
                "sb-pinned",
                PINNED_KEY,
                "PINNED",
                !self.collapsed.contains(PINNED_KEY),
                None,
                false,
                p,
                cx,
            ));
            let mut grid = div().mt(px(8.)).flex().flex_wrap().flex_none().gap(px(6.));
            if !self.collapsed.contains(PINNED_KEY) {
                for tab in &pinned {
                    let on = active_tab.as_ref() == Some(&tab.id);
                    let store = self.store.clone();
                    let tid = tab.id.clone();
                    grid = grid.child(
                        div()
                            .id(SharedString::from(format!("tile-{}", tab.id)))
                            .w(px(44.))
                            .h(px(36.))
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
                                p.text.opacity(0.6)
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
            }
            list = list.child(grid);
        }

        let space_tabs: Vec<Tab> = groups
            .iter()
            .flat_map(|g| g.tab_ids().into_iter().chain(g.dock.tabs.iter().cloned()))
            .filter_map(|id| tab_of(&id))
            .filter(|t| !t.pinned)
            .collect();
        let today_groups: Vec<(String, String, Vec<Tab>)> = groups
            .iter()
            .map(|group| {
                let tabs = group
                    .tab_ids()
                    .into_iter()
                    .chain(group.dock.tabs.iter().cloned())
                    .filter_map(|id| tab_of(&id))
                    .filter(|tab| {
                        !tab.pinned
                            && matches!(
                                tab.kind,
                                TabKind::Web | TabKind::NewTab | TabKind::Settings
                            )
                    })
                    .collect();
                (group.id.clone(), group.name.clone(), tabs)
            })
            .collect();
        let today_has_rows = today_groups.iter().any(|(_, _, tabs)| !tabs.is_empty());
        if !compact || today_has_rows {
            let store = self.store.clone();
            let action = div()
                .id("sb-today-action")
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .opacity(0.)
                .group_hover("sb-disclosure", |s| s.opacity(1.))
                .hover(|s| s.bg(p.glass_hover()))
                .child(glyph(icons::PLUS, 13., p.muted))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    store.update(cx, |s, cx| s.open_tab("hifi://newtab", None, None, cx));
                })
                .into_any_element();
            list = list.child(div().h(px(SIDEBAR_SECTION_GAP)).flex_none());
            list = list.child(self.disclosure(
                "sb-today",
                "__today",
                "TODAY",
                true,
                Some(action),
                false,
                p,
                cx,
            ));
            if today_groups.len() <= 1 || compact {
                let tabs = today_groups.iter().flat_map(|(_, _, tabs)| tabs.iter());
                let tabs: Vec<Tab> = tabs.cloned().collect();
                list = list.child(self.capped_tab_list(
                    &tabs,
                    "__today",
                    8,
                    active_tab.as_ref(),
                    compact,
                    p,
                    cx,
                ));
            } else {
                for (gid, name, tabs) in &today_groups {
                    if tabs.is_empty() {
                        continue;
                    }
                    let open = !self.collapsed.contains(gid);
                    list = list.child(div().h(px(SIDEBAR_SECTION_GAP)).flex_none());
                    list = list.child(self.disclosure(
                        SharedString::from(format!("today-group-{gid}")),
                        gid,
                        name.clone(),
                        open,
                        None,
                        true,
                        p,
                        cx,
                    ));
                    if open {
                        list = list.child(self.capped_tab_list(
                            tabs,
                            gid,
                            8,
                            active_tab.as_ref(),
                            compact,
                            p,
                            cx,
                        ));
                    }
                }
            }
        }

        // Chats, agents, and pages are space-wide sections. Header actions
        // live in the disclosure row rather than consuming a peer row.
        let sections: [(&str, &str, &'static str, &'static str); 3] = [
            ("sb-chats", "CHATS", "New Chat", "hifi://agent"),
            ("sb-agents", "SESSIONS", "New Agent", "hifi://agent"),
            ("sb-pages", "PAGES", "New Page", "hifi://notes"),
        ];
        for (id, title, _new_label, url) in sections {
            let tabs: Vec<Tab> = space_tabs
                .iter()
                .filter(|tab| match id {
                    "sb-chats" => tab.kind == TabKind::Agent && !tab.prompt.trim().is_empty(),
                    "sb-agents" => {
                        matches!(
                            tab.kind,
                            TabKind::Terminal | TabKind::Diff | TabKind::Preview
                        ) || (tab.kind == TabKind::Agent && tab.prompt.trim().is_empty())
                    }
                    "sb-pages" => tab.kind == TabKind::Notes,
                    _ => false,
                })
                .cloned()
                .collect();
            if tabs.is_empty() {
                continue;
            }
            let open = !self.collapsed.contains(id);
            list = list.child(div().h(px(SIDEBAR_SECTION_GAP)).flex_none());
            if !compact {
                let store = self.store.clone();
                let url = url.to_string();
                let dock = title != "PAGES";
                let action = div()
                    .id(SharedString::from(format!("{id}-action")))
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .opacity(0.)
                    .group_hover("sb-disclosure", |s| s.opacity(1.))
                    .hover(|s| s.bg(p.glass_hover()))
                    .child(glyph(icons::PLUS, 13., p.muted))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        store.update(cx, |s, cx| {
                            if dock {
                                s.dock_open(&url, cx);
                            } else {
                                s.open_tab(&url, None, None, cx);
                            }
                        });
                    })
                    .into_any_element();
                list = list.child(self.disclosure(id, id, title, open, Some(action), false, p, cx));
            }
            if !(open || compact) {
                continue;
            }
            list =
                list.child(self.capped_tab_list(&tabs, id, 5, active_tab.as_ref(), compact, p, cx));
        }

        if !today_has_rows && !compact {
            list = list.child(
                div()
                    .h(px(32.))
                    .px(px(Theme::SPACE_SM))
                    .flex()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(p.faint)
                    .child("No tabs yet — ⌘T"),
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
            .h(px(Theme::SPACE_DOCK_H))
            .border_t_1()
            .border_color(p.hairline(0.05))
            .when(compact, |d| d.flex_col().h_auto().py(px(8.)));
        for (id, name) in &spaces {
            let on = *id == space_id;
            let store = self.store.clone();
            let sid = id.clone();
            let tooltip_name = name.clone();
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
                    .when(on, |d| {
                        d.bg(p.selected())
                            .border_1()
                            .border_color(p.text.opacity(0.8))
                    })
                    .when(!on, |d| d.hover(|s| s.bg(p.glass_hover())))
                    .child(div().size(px(8.)).rounded_full().bg(if on {
                        p.text
                    } else {
                        p.ink(0.25)
                    }))
                    .tooltip(move |_, cx| cx.new(|_| SpaceTooltip(tooltip_name.clone())).into())
                    .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                        store.update(cx, |s, cx| {
                            s.switch_space(&sid, cx);
                            s.pending_space_menu = Some(event.position);
                        });
                    }),
            );
        }
        let store_st = self.store.clone();
        let store_settings = store_st.clone();
        bottom = bottom
            .child(
                icon_button("space-add", icons::PLUS, p, move |cx| {
                    store_st.update(cx, |s, cx| {
                        s.pending_space_menu = Some(point(px(24.), px(24.)));
                        cx.notify();
                    });
                })
                .tooltip(|_, cx| cx.new(|_| SpaceTooltip("New space".into())).into()),
            )
            .when(!compact, |d| d.child(div().flex_1()))
            .child(
                div()
                    .id("open-settings")
                    .size(px(28.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(p.glass_hover()))
                    .child(glyph(icons::SETTINGS, 15., p.muted))
                    .tooltip(|_, cx| cx.new(|_| SpaceTooltip("Settings".into())).into())
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        store_settings.update(cx, |s, cx| {
                            s.open_settings(cx);
                        });
                    }),
            );
        bar.child(bottom)
    }
}

#[cfg(test)]
mod tests {
    use super::limit_rows;

    #[test]
    fn includes_active_row_beyond_limit() {
        let rows: Vec<usize> = (0..10).collect();
        let limited = limit_rows(&rows, Some(9), 5, false);

        assert_eq!(limited.visible, vec![0, 1, 2, 3, 4, 9]);
        assert_eq!(limited.more, 4);
    }

    #[test]
    fn rows_at_limit_need_no_cap_action() {
        let rows: Vec<usize> = (0..5).collect();
        let limited = limit_rows(&rows, None, 5, false);

        assert_eq!(limited.visible, rows);
        assert_eq!(limited.more, 0);
        assert!(!limited.show_less);
    }
}
