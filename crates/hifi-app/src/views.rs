//! Internal surfaces: the glass new-tab page, settings, diff viewer, file
//! preview — pure GPUI, no native views.

use gpui::{
    Context, Entity, Focusable, Hsla, MouseButton, ScrollHandle, SharedString,
    Window, div, img, prelude::*, px, rems, svg,
};
use std::path::PathBuf;

use crate::assets::icons;
use crate::store::Store;
use crate::text_input::{TextField, TextFieldEvent};
use crate::theme::{Palette, Theme, radius, space};

/// Render a Solar icon sized + tinted.
pub fn glyph(path: &'static str, size: f32, color: Hsla) -> gpui::Svg {
    svg()
        .path(path)
        .size(px(size))
        .text_color(color)
}

// ---------------------------------------------------------------------------
// New tab
// ---------------------------------------------------------------------------

pub struct NewTabView {
    store: Entity<Store>,
    input: Entity<TextField>,
    now: String,
    _tick: gpui::Task<()>, 
}

impl NewTabView {
    pub fn new(store: Entity<Store>, _tab_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut t = TextField::new("Search or enter address", cx);
            t.placeholder_color = None;
            t
        });
        let store_for_submit = store.clone();
        let own_tab = _tab_id.clone();
        cx.subscribe(&input, move |_me, _i, event, cx| {
            if let TextFieldEvent::Submitted(text) = event {
                let tab = own_tab.clone();
                store_for_submit.update(cx, |store, cx| {
                    let url = resolve_input(store, &text);
                    store.navigate(&tab, &url, cx);
                });
            }
        })
        .detach();
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(30))
                    .await;
                if this
                    .update(cx, |v: &mut NewTabView, cx| {
                        v.now = now_string();
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        window.focus(&input.read(cx).focus_handle(cx), cx);
        Self {
            store,
            input,
            now: now_string(),
            _tick: tick,
        }
    }
}

fn now_string() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

fn greeting() -> &'static str {
    let h = chrono::Local::now().format("%H").to_string();
    match h.parse::<u32>().unwrap_or(12) {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        17..=21 => "Good evening",
        _ => "Good night",
    }
}

fn resolve_input(store: &Store, text: &str) -> String {
    match hifi_core::route(text) {
        hifi_core::RoutedUrl::Internal(_) | hifi_core::RoutedUrl::Preview(_) => text.to_string(),
        hifi_core::RoutedUrl::External(u) => {
            if let Some(q) = u.strip_prefix("search:") {
                hifi_core::schemes::resolve_search(&store.state.settings.search_engine, q)
            } else {
                u
            }
        }
    }
}

/// Glass tile used by new-tab sections.
fn glass_tile(p: &Palette) -> gpui::Div {
    div()
        .bg(gpui::white().opacity(if p.is_dark { 0.05 } else { 0.55 }))
        .rounded(radius::BUBBLE)
        .border_1()
        .border_color(gpui::white().opacity(if p.is_dark { 0.09 } else { 0.4 }))
        .shadow_sm()
}

impl gpui::Render for NewTabView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(cx).palette;
        let store = self.store.read(cx);
        let settings = &store.state.settings;
        let fg = if settings.background_image.is_empty() {
            p.text
        } else {
            gpui::white().into()
        };

        // Background: custom image or accent aurora wash.
        let mut root = div().size_full().flex().flex_col().items_center();
        if !settings.background_image.is_empty() {
            let path = settings.background_image.clone();
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .child(
                        img(PathBuf::from(path))
                            .size_full()
                            .object_fit(gpui::ObjectFit::Cover),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .bg(gpui::black().opacity(0.3 + settings.background_dim)),
                    ),
            );
        } else {
            root = root
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .bg(p.bg)
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .right_0()
                                .w(rems(24.))
                                .h(rems(24.))
                                .rounded_full()
                                .bg(p.accent.opacity(0.10)),
                        )
                        .child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left_0()
                                .w(rems(28.))
                                .h(rems(28.))
                                .rounded_full()
                                .bg(gpui::rgb(0x4a8ef7).opacity(0.08)),
                        ),
                );
        }

        let date = chrono::Local::now().format("%A, %B %-d").to_string();
        let pinned: Vec<_> = store
            .state
            .active_space()
            .map(|s| {
                s.groups
                    .iter()
                    .flat_map(|g| g.tab_ids())
                    .filter_map(|id| store.state.tab(&id))
                    .filter(|t| t.pinned)
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let recents: Vec<_> = store.history.iter().rev().take(10).cloned().collect();

        let store2 = self.store.clone();
        let store3 = self.store.clone();
        let store4 = self.store.clone();
        let mut col = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(space::LG)
            .w_full()
            .max_w(rems(46.))
            .mx_auto()
            .mt(rems(9.))
            .px(space::LG)
            .text_color(fg);

        col = col
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(space::XS)
                    .child(
                        div()
                            .text_size(px(64.))
                            .font_weight(gpui::FontWeight::EXTRA_LIGHT)
                            .child(self.now.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(15.))
                            .text_color(fg.opacity(0.72))
                            .child(format!("{date} · {}", greeting())),
                    ),
            )
            .child(
                // Search field — glass
                glass_tile(&p)
                    .w_full()
                    .h(px(52.))
                    .px(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(glyph(icons::MAGNIFER, 18., fg.opacity(0.55)))
                    .child(div().flex_1().child(self.input.clone())),
            )
            .child(
                // Action chips
                div()
                    .flex()
                    .flex_row()
                    .gap(space::SM)
                    .children([
                        ("New tab", icons::PLUS, "hifi://newtab"),
                        ("Terminal", icons::TERMINAL, "hifi://terminal"),
                        ("Agent", icons::BOT, "hifi://agent"),
                        ("Diff", icons::GIT_BRANCH, "hifi://diff"),
                        ("Settings", icons::SETTINGS, "hifi://settings"),
                    ]
                    .into_iter()
                    .map(|(label, ic, url)| {
                        let store = store2.clone();
                        div()
                            .id(label)
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .px(px(14.))
                            .h(px(34.))
                            .rounded_full()
                            .bg(gpui::white().opacity(if p.is_dark { 0.07 } else { 0.5 }))
                            .border_1()
                            .border_color(gpui::white().opacity(if p.is_dark { 0.08 } else { 0.35 }))
                            .cursor_pointer()
                            .hover(|s| s.bg(gpui::white().opacity(if p.is_dark { 0.12 } else { 0.7 })))
                            .child(glyph(ic, 15., fg.opacity(0.8)))
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .text_color(fg.opacity(0.9))
                                    .child(label),
                            )
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                store.update(cx, |store, cx| {
                                    store.open_tab(url, None, None, cx);
                                });
                            })
                    })
                    .collect::<Vec<_>>()),
            );

        if !pinned.is_empty() {
            let mut dial = div().flex().flex_row().gap(space::MD).flex_wrap().justify_center();
            for tab in pinned {
                let store = store3.clone();
                let id = tab.id.clone();
                let host = hifi_core::host_of(&tab.url);
                let letter = host.chars().next().unwrap_or('•').to_uppercase().to_string();
                dial = dial.child(
                    div()
                        .id(SharedString::from(format!("pin-{id}")))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(px(5.))
                        .w(px(76.))
                        .h(px(70.))
                        .rounded(radius::CARD)
                        .bg(gpui::white().opacity(if p.is_dark { 0.06 } else { 0.55 }))
                        .border_1()
                        .border_color(gpui::white().opacity(if p.is_dark { 0.08 } else { 0.35 }))
                        .cursor_pointer()
                        .hover(|s| s.bg(gpui::white().opacity(if p.is_dark { 0.12 } else { 0.75 })))
                        .child(
                            div()
                                .size(px(26.))
                                .rounded(px(7.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(p.accent.opacity(0.25))
                                .text_color(p.accent)
                                .text_size(px(12.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(letter),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(fg.opacity(0.75))
                                .child(host),
                        )
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            store.update(cx, |store, cx| store.focus_tab(&id, cx));
                        }),
                );
            }
            col = col.child(dial);
        }

        if !recents.is_empty() {
            let mut strip = div()
                .flex()
                .flex_row()
                .gap(space::SM)
                .flex_wrap()
                .justify_center();
            for entry in recents {
                let store = store4.clone();
                let url = entry.url.clone();
                strip = strip.child(
                    div()
                        .id(SharedString::from(format!("rec-{url}")))
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .px(px(12.))
                        .h(px(30.))
                        .rounded_full()
                        .bg(gpui::white().opacity(if p.is_dark { 0.06 } else { 0.5 }))
                        .border_1()
                        .border_color(gpui::white().opacity(if p.is_dark { 0.07 } else { 0.3 }))
                        .cursor_pointer()
                        .hover(|s| s.bg(gpui::white().opacity(if p.is_dark { 0.11 } else { 0.7 })))
                        .child(glyph(icons::GLOBE, 13., fg.opacity(0.6)))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(fg.opacity(0.85))
                                .child(if entry.title.is_empty() {
                                    hifi_core::host_of(&entry.url)
                                } else {
                                    entry.title.clone()
                                }),
                        )
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            store.update(cx, |s, cx| {
                                s.open_tab(&url, None, None, cx);
                            });
                        }),
                );
            }
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(space::SM)
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(fg.opacity(0.5))
                            .child("JUMP BACK IN"),
                    )
                    .child(strip),
            );
        }

        root.child(div().flex_1().child(col)).child(
            // kbd hints
            div()
                .flex()
                .flex_row()
                .gap(space::LG)
                .pb(space::LG)
                .children(
                    [("⌘T", "Command bar"), ("⌘L", "Address"), ("⌘W", "Close tab")]
                        .iter()
                        .map(|(kbd, label)| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .px(px(6.))
                                        .h(px(20.))
                                        .rounded(px(5.))
                                        .bg(gpui::white().opacity(if p.is_dark { 0.08 } else { 0.5 }))
                                        .border_1()
                                        .border_color(gpui::white().opacity(if p.is_dark {
                                            0.1
                                        } else {
                                            0.35
                                        }))
                                        .text_size(px(10.))
                                        .text_color(fg.opacity(0.7))
                                        .child(*kbd),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(fg.opacity(0.5))
                                        .child(*label),
                                )
                        })
                        .collect::<Vec<_>>(),
                ),
        )
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

pub struct SettingsView {
    store: Entity<Store>,
    bg_field: Entity<TextField>,
}

impl SettingsView {
    pub fn new(store: Entity<Store>, _tab: String, _w: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial = store.read(cx).state.settings.background_image.clone();
        let bg_field = cx.new(|cx| TextField::new("~/Pictures/…", cx));
        bg_field.update(cx, |f, cx| f.set_value(initial, cx));
        let store2 = store.clone();
        cx.subscribe(&bg_field, move |_me, _f, event, cx| {
            if let TextFieldEvent::Changed = event {
                // applied via the Apply button
                let _ = &store2;
            }
            if let TextFieldEvent::Submitted(path) = event {
                store2.update(cx, |s, cx| {
                    s.update_settings(|st| st.background_image = path.clone(), cx);
                });
            }
        })
        .detach();
        Self { store, bg_field }
    }
}

fn section(title: &str, p: &Palette) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(space::SM)
        .child(
            div()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(p.faint)
                .child(title.to_uppercase()),
        )
}

fn row() -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .py(px(2.))
}

fn label(p: &Palette, text: &str) -> gpui::Div {
    div().text_size(px(13.)).text_color(p.text).child(text.to_string())
}

fn sub(p: &Palette, text: &str) -> gpui::Div {
    div()
        .text_size(px(11.))
        .text_color(p.faint)
        .child(text.to_string())
}

fn chip_button(id: &str, text: &str, active: bool, p: &Palette) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id.to_string()))
        .px(px(12.))
        .h(px(28.))
        .flex()
        .items_center()
        .rounded_full()
        .cursor_pointer()
        .text_size(px(12.))
        .bg(if active {
            p.accent.opacity(0.22)
        } else {
            p.card.opacity(0.6)
        })
        .text_color(if active { p.accent } else { p.muted })
        .border_1()
        .border_color(if active { p.accent.opacity(0.5) } else { p.border })
        .child(text.to_string())
}

impl gpui::Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(cx).palette;
        let settings = self.store.read(cx).state.settings.clone();
        let store = self.store.clone();

        let mut page = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg.opacity(0.35))
            .p(px(28.))
            .gap(px(22.))
            .text_color(p.text)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .child(glyph(icons::SETTINGS, 18., p.accent))
                    .child(div().text_size(px(20.)).font_weight(gpui::FontWeight::SEMIBOLD).child("Settings")),
            );

        // Appearance
        let appearance = settings.appearance;
        let store_a = store.clone();
        page = page.child(section("Appearance", &p).child(
            div().flex().gap(space::SM).children(
                [("System", hifi_core::Appearance::System), ("Light", hifi_core::Appearance::Light), ("Dark", hifi_core::Appearance::Dark)]
                    .into_iter()
                    .map(|(name, value)| {
                        let store = store_a.clone();
                        chip_button(name, name, appearance == value, &p).on_mouse_down(
                            MouseButton::Left,
                            move |_, _, cx| {
                                store.update(cx, |s, cx| {
                                    s.update_settings(|st| st.appearance = value, cx)
                                })
                            },
                        )
                    }),
            ),
        ));

        // Accent
        let accent = settings.accent.clone();
        let store_b = store.clone();
        page = page.child(section("Accent", &p).child(
            div().flex().gap(space::SM).flex_wrap().children(
                crate::theme::ACCENTS.iter().map(|(name, dark_v, light_v)| {
                    let store = store_b.clone();
                    let on = accent == *name;
                    let swatch = crate::theme::color(if p.is_dark { *dark_v } else { *light_v });
                    div()
                        .id(SharedString::from(format!("acc-{name}")))
                        .size(px(26.))
                        .rounded_full()
                        .bg(swatch)
                        .cursor_pointer()
                        .border_2()
                        .border_color(if on { p.text } else { gpui::transparent_white() })
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            store.update(cx, |s, cx| {
                                s.update_settings(|st| st.accent = name.to_string(), cx)
                            })
                        })
                }),
            ),
        ));

        // Background
        let store_c = store.clone();
        let has_bg = !settings.background_image.is_empty();
        page = page.child(
            section("New tab background", &p)
                .child(row().child(label(&p, "Image path")))
                .child(
                    div()
                        .flex()
                        .gap(space::SM)
                        .child(
                            div()
                                .flex_1()
                                .h(px(34.))
                                .px(px(12.))
                                .rounded(radius::ROUND)
                                .bg(p.card)
                                .border_1()
                                .border_color(p.border)
                                .flex()
                                .items_center()
                                .child(self.bg_field.clone()),
                        )
                        .child(if has_bg {
                            let store = store_c.clone();
                            chip_button("clear-bg", "Clear", false, &p).on_mouse_down(
                                MouseButton::Left,
                                move |_, _, cx| {
                                    store.update(cx, |s, cx| {
                                        s.update_settings(|st| st.background_image.clear(), cx)
                                    })
                                },
                            )
                        } else {
                            chip_button("bg-hint", "Enter applies", false, &p)
                        }),
                )
                .child(sub(&p, "Local file — rendered behind the glass cards with a dim overlay.")),
        );

        // Sidebar / session
        let store_d = store.clone();
        let compact = settings.compact_sidebar;
        let restore = settings.restore_session;
        page = page.child(
            section("Behavior", &p)
                .child(
                    row()
                        .child(label(&p, "Compact sidebar"))
                        .child({
                            let store = store_d.clone();
                            chip_button("compact", if compact { "On" } else { "Off" }, compact, &p)
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    store.update(cx, |s, cx| {
                                        s.update_settings(|st| st.compact_sidebar = !st.compact_sidebar, cx)
                                    })
                                })
                        }),
                )
                .child(
                    row()
                        .child(label(&p, "Restore session on launch"))
                        .child({
                            let store = store_d.clone();
                            chip_button("restore", if restore { "On" } else { "Off" }, restore, &p)
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    store.update(cx, |s, cx| {
                                        s.update_settings(|st| st.restore_session = !st.restore_session, cx)
                                    })
                                })
                        }),
                ),
        );

        // Data + CLI
        page = page.child(
            section("Data & CLI", &p)
                .child(
                    row()
                        .child(label(&p, "Session store"))
                        .child(sub(&p, "~/Library/Application Support/HiFi/state.json")),
                )
                .child(
                    row()
                        .child(label(&p, "IPC socket"))
                        .child(sub(&p, "~/Library/Application Support/HiFi/ipc.sock")),
                )
                .child(sub(
                    &p,
                    "Install the CLI: `cargo install --path crates/hifi-cli` — then `hifi ping`, `hifi tab open <url>`",
                )),
        );

        page.child(
            div().mt(px(6.)).child(sub(
                &p,
                "Hi-Fi — GPUI shell · WKWebView engine · Solar Icons (CC BY 4.0, 480 Design)",
            )),
        )
    }
}

// ---------------------------------------------------------------------------
// Diff pane — `git diff` of a project's worktree, colorized.
// ---------------------------------------------------------------------------

pub struct DiffView {
    path: String,
    lines: Vec<(Hsla, SharedString)>,
    stat: String,
    scroll: ScrollHandle,
}

impl DiffView {
    pub fn new(path: String, _tab: String, _w: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut v = Self {
            path,
            lines: vec![],
            stat: "…".into(),
            scroll: ScrollHandle::new(),
        };
        v.refresh(cx);
        v
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        cx.spawn(async move |this, cx| {
            let out = tokio::process::Command::new("git")
                .args(["-C", &path, "diff", "HEAD", "--", "."])
                .output()
                .await;
            let numstat = tokio::process::Command::new("git")
                .args(["-C", &path, "diff", "--numstat", "HEAD"])
                .output()
                .await;
            let branch = tokio::process::Command::new("git")
                .args(["-C", &path, "branch", "--show-current"])
                .output()
                .await;
            let _ = this.update(cx, |v: &mut DiffView, cx| {
                let branch = branch
                    .ok()
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "HEAD".into());
                let mut adds = 0u32;
                let mut dels = 0u32;
                let mut files = 0u32;
                if let Ok(o) = numstat {
                    for line in String::from_utf8_lossy(&o.stdout).lines() {
                        let mut it = line.split('\t');
                        if let (Some(a), Some(d), Some(_)) = (it.next(), it.next(), it.next()) {
                            adds += a.parse::<u32>().unwrap_or(0);
                            dels += d.parse::<u32>().unwrap_or(0);
                            files += 1;
                        }
                    }
                }
                v.stat = if files == 0 {
                    format!("clean · {branch}")
                } else {
                    format!("{files} changed · +{adds} −{dels} · {branch}")
                };
                let red = crate::theme::color(0xf26d6d);
                let green = crate::theme::color(0x59c56f);
                let cyan = crate::theme::color(0x5ad4e6);
                let faint = crate::theme::color(0x85858a);
                v.lines = out
                    .ok()
                    .map(|o| {
                        String::from_utf8_lossy(&o.stdout)
                            .lines()
                            .map(|l| {
                                let c = if l.starts_with('+') && !l.starts_with("+++") {
                                    green
                                } else if l.starts_with('-') && !l.starts_with("---") {
                                    red
                                } else if l.starts_with("@@") || l.starts_with("diff ") {
                                    cyan
                                } else {
                                    faint
                                };
                                (c, SharedString::from(l.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                cx.notify();
            });
        })
        .detach();
    }
}

impl gpui::Render for DiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(cx).palette;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg.opacity(0.4))
            .child(
                div()
                    .h(px(36.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .border_b_1()
                    .border_color(p.border)
                    .child(glyph(icons::GIT_BRANCH, 14., p.accent))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_family(crate::theme::FONT_MONO)
                            .text_color(p.text)
                            .child(self.stat.clone()),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("diff-refresh")
                            .cursor_pointer()
                            .child(glyph(icons::REFRESH, 14., p.muted))
                            .on_mouse_down(MouseButton::Left, cx.listener(|v, _, _, cx| {
                                v.refresh(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .id("diff-scroll")
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p(px(12.))
                    .flex()
                    .flex_col()
                    .font_family(crate::theme::FONT_MONO)
                    .text_size(px(11.5))
                    .children(self.lines.iter().cloned().map(|(c, l)| {
                        div()
                            .text_color(c)
                            .whitespace_nowrap()
                            .child(l)
                    })),
            )
    }
}

// ---------------------------------------------------------------------------
// Preview pane — markdown / html / image files.
// ---------------------------------------------------------------------------

pub struct PreviewView {
    path: String,
}

impl PreviewView {
    pub fn new(path: String, _tab: String, _w: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self { path }
    }
}

impl gpui::Render for PreviewView {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(cx).palette;
        let content = std::fs::read_to_string(&self.path).unwrap_or_default();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg.opacity(0.4))
            .p(px(24.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(p.text)
                    .child(self.path.clone()),
            )
            .child(
                div()
                    .mt(px(12.))
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .font_family(crate::theme::FONT_MONO)
                    .child(content),
            )
    }
}
