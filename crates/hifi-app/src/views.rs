//! Internal surfaces: the glass new-tab page, settings, diff viewer, file
//! preview — pure GPUI, no native views.

use gpui::{
    Context, Entity, Focusable, Hsla, MouseButton, ScrollHandle, SharedString, Window, div, img,
    prelude::*, px, rems, svg,
};
use std::path::PathBuf;

use crate::assets::icons;
use crate::store::Store;
use crate::text_input::{TextField, TextFieldEvent};
use crate::theme::{Palette, Theme, radius, space};

/// Render a Solar icon sized + tinted.
pub fn glyph(path: &'static str, size: f32, color: Hsla) -> gpui::Svg {
    svg().path(path).size(px(size)).text_color(color)
}

// ---------------------------------------------------------------------------
// New tab
// ---------------------------------------------------------------------------

pub struct NewTabView {
    store: Entity<Store>,
    input: Entity<TextField>,
    /// Ask = start an agent task; Search = navigate / web search.
    ask: bool,
    harness: usize,
}

/// Agent harnesses the composer can start: label, command, brand mark.
const HARNESSES: &[(&str, &str, &str)] = &[
    ("Claude Code", "claude", icons::CLAUDE_MARK),
    ("Codex", "codex", icons::OPENAI_MARK),
    ("Devin", "devin", icons::DEVIN_MARK),
    ("OpenCode", "opencode", icons::OPENCODE_MARK),
    ("Amp", "amp", icons::AMP_MARK),
];

const ASK_PLACEHOLDER: &str = "Ask AI a task, @ for context";
const SEARCH_PLACEHOLDER: &str = "Search Google or type a URL";

impl NewTabView {
    pub fn new(
        store: Entity<Store>,
        tab_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextField::new(ASK_PLACEHOLDER, cx));
        cx.subscribe(&input, move |me: &mut NewTabView, input, event, cx| {
            let TextFieldEvent::Submitted(text) = event else {
                return;
            };
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            let tab = tab_id.clone();
            let (ask, harness) = (me.ask, HARNESSES[me.harness].1);
            me.store.update(cx, |store, cx| {
                if ask && !looks_like_address(&text) {
                    store.start_agent_in(&tab, harness, &text, cx);
                } else {
                    let url = resolve_input(store, &text);
                    store.navigate(&tab, &url, cx);
                }
            });
            input.update(cx, |i, cx| i.reset(cx));
        })
        .detach();
        window.focus(&input.read(cx).focus_handle(cx), cx);
        Self {
            store,
            input,
            ask: true,
            harness: 0,
        }
    }

    fn set_mode(&mut self, ask: bool, cx: &mut Context<Self>) {
        self.ask = ask;
        self.input.update(cx, |i, cx| {
            i.set_placeholder(
                if ask {
                    ASK_PLACEHOLDER
                } else {
                    SEARCH_PLACEHOLDER
                },
                cx,
            )
        });
        cx.notify();
    }
}

/// A typed URL/host always navigates, even in Ask mode.
fn looks_like_address(text: &str) -> bool {
    !text.contains(char::is_whitespace)
        && (text.contains("://") || text.starts_with("localhost") || text.contains('.'))
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

fn basename_of(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_string()
}

/// A compact composer control: icon + label + chevron, text-weight only.
fn composer_pill(
    id: impl Into<SharedString>,
    icon: &'static str,
    label: impl Into<SharedString>,
    chevron: bool,
    p: &Palette,
) -> gpui::Stateful<gpui::Div> {
    let p = *p;
    div()
        .id(id.into())
        .flex()
        .items_center()
        .gap(px(5.))
        .h(px(24.))
        .px(px(6.))
        .rounded(px(6.))
        .cursor_pointer()
        .text_size(px(12.))
        .text_color(p.muted)
        .hover(|s| s.bg(p.glass_hover()).text_color(p.text))
        .child(glyph(icon, 13., p.muted))
        .child(label.into())
        .when(chevron, |d| {
            d.child(glyph(icons::ALT_ARROW_DOWN, 10., p.faint))
        })
}

struct TaskCard {
    id: Option<String>,
    url: String,
    status: String,
    title: String,
    body: String,
    icon: &'static str,
    live: bool,
}

impl gpui::Render for NewTabView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(cx).palette;
        let store = self.store.read(cx);
        let settings = &store.state.settings;
        let has_image = !settings.background_image.is_empty();
        let fg = if has_image { gpui::white() } else { p.text };

        let mut root = div()
            .id("newtab")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .overflow_y_scroll();
        if has_image {
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
        }

        // Recent tasks: agent sessions first (newest last in the model), then
        // recent pages to fill the row.
        let mut cards: Vec<TaskCard> = store
            .state
            .tabs
            .iter()
            .rev()
            .filter(|t| {
                matches!(
                    t.kind,
                    hifi_core::TabKind::Agent | hifi_core::TabKind::Terminal
                )
            })
            .take(3)
            .map(|t| {
                let harness = if t.command.is_empty() {
                    "claude".to_string()
                } else {
                    t.command.clone()
                };
                let place = if t.cwd.is_empty() {
                    "Local".to_string()
                } else {
                    basename_of(&t.cwd)
                };
                TaskCard {
                    id: Some(t.id.clone()),
                    url: String::new(),
                    status: if t.kind == hifi_core::TabKind::Agent {
                        format!("Agent · {harness}")
                    } else {
                        "Terminal".into()
                    },
                    title: if !t.prompt.is_empty() {
                        t.prompt.clone()
                    } else if t.title.is_empty() {
                        t.display_title()
                    } else {
                        t.title.clone()
                    },
                    body: if t.branch.is_empty() {
                        place
                    } else {
                        format!("{place} · {}", t.branch)
                    },
                    icon: crate::sidebar::kind_icon(t),
                    live: true,
                }
            })
            .collect();
        for entry in store.history.iter().rev() {
            if cards.len() >= 3 {
                break;
            }
            if cards.iter().any(|c| c.url == entry.url) {
                continue;
            }
            let host = hifi_core::host_of(&entry.url);
            cards.push(TaskCard {
                id: None,
                url: entry.url.clone(),
                status: "Recently visited".into(),
                title: if entry.title.is_empty() {
                    host.clone()
                } else {
                    entry.title.clone()
                },
                body: host,
                icon: icons::GLOBE,
                live: false,
            });
        }

        let (h_label, _, h_icon) = HARNESSES[self.harness];
        let ask = self.ask;

        let mode_seg = |label: &'static str, on: bool, id: &'static str| {
            div()
                .id(id)
                .h(px(26.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded(px(7.))
                .cursor_pointer()
                .text_size(px(12.5))
                .when(on, |d| {
                    d.bg(if p.is_dark { p.raised } else { gpui::white() })
                        .text_color(p.text)
                        .shadow_xs()
                        .border_1()
                        .border_color(p.hairline(0.06))
                })
                .when(!on, |d| {
                    d.text_color(p.muted).hover(|s| s.text_color(p.text))
                })
                .child(label)
        };

        let composer = div()
            .id("composer")
            .w_full()
            .max_w(px(620.))
            .h(px(46.))
            .pl(px(14.))
            .pr(px(4.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(12.))
            .bg(if p.is_dark {
                p.dialog.opacity(0.9)
            } else {
                gpui::white().opacity(0.96)
            })
            .border_1()
            .border_color(p.hairline(0.07))
            .shadow_md()
            .capture_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, _, cx| {
                let k = &e.keystroke;
                if k.key == "tab" && !k.modifiers.modified() {
                    this.set_mode(!this.ask, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex_1()
                    .min_w(px(60.))
                    .overflow_hidden()
                    .text_size(px(14.))
                    .text_color(p.text)
                    .child(self.input.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink(1.)
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .items_center()
                    .gap(px(5.))
                    .text_size(px(11.5))
                    .text_color(p.faint)
                    .child(
                        div()
                            .px(px(5.))
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .rounded(px(4.))
                            .bg(p.ink(0.06))
                            .text_size(px(10.5))
                            .text_color(p.muted)
                            .child("Tab"),
                    )
                    .child("to switch"),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .p(px(3.))
                    .gap(px(2.))
                    .rounded(px(9.))
                    .bg(p.ink(0.05))
                    .child(mode_seg("Search", !ask, "mode-search").on_click(
                        cx.listener(|this, _: &gpui::ClickEvent, _, cx| this.set_mode(false, cx)),
                    ))
                    .child(mode_seg("Ask", ask, "mode-ask").on_click(
                        cx.listener(|this, _: &gpui::ClickEvent, _, cx| this.set_mode(true, cx)),
                    )),
            );

        let store_ctx = self.store.clone();
        let store_term = self.store.clone();
        let controls = div()
            .w_full()
            .max_w(px(620.))
            .px(px(4.))
            .flex()
            .items_center()
            .gap(px(2.))
            .child(
                composer_pill("ctl-add", icons::PAPERCLIP, "", false, &p).on_click(
                    move |_, _, cx| {
                        store_ctx.update(cx, |s, cx| s.dock_open("hifi://notes", cx));
                    },
                ),
            )
            .child(
                composer_pill("ctl-local", icons::LAPTOP, "Local", false, &p).on_click(
                    move |_, _, cx| {
                        store_term.update(cx, |s, cx| {
                            s.open_tab("hifi://terminal", None, None, cx);
                        });
                    },
                ),
            )
            .child(composer_pill(
                "ctl-guard",
                icons::SHIELD,
                "Guard",
                false,
                &p,
            ))
            .child(div().flex_1())
            .when(ask, |d| {
                d.child(
                    composer_pill("ctl-harness", h_icon, h_label, true, &p).on_click(cx.listener(
                        |this, _: &gpui::ClickEvent, _, cx| {
                            this.harness = (this.harness + 1) % HARNESSES.len();
                            cx.notify();
                        },
                    )),
                )
            });

        let mut hero = div()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .w_full()
            .max_w(px(820.))
            .px(space::LG)
            .pt(rems(11.))
            .gap(px(10.))
            .child(div().mb(px(18.)).child(glyph(
                icons::HIFI_MARK,
                44.,
                fg.opacity(if has_image { 0.9 } else { 0.28 }),
            )))
            .child(composer)
            .child(controls);

        if !cards.is_empty() {
            let mut row = div().w_full().flex().flex_wrap().gap(px(10.));
            for (i, card) in cards.into_iter().enumerate() {
                let store = self.store.clone();
                let TaskCard {
                    id,
                    url,
                    status,
                    title,
                    body,
                    icon,
                    live,
                } = card;
                row = row.child(
                    div()
                        .id(SharedString::from(format!("task-{i}")))
                        .flex_1()
                        .min_w(px(180.))
                        .h(px(236.))
                        .flex()
                        .flex_col()
                        .rounded(px(12.))
                        .overflow_hidden()
                        .bg(if p.is_dark {
                            p.dialog.opacity(0.85)
                        } else {
                            gpui::white().opacity(0.92)
                        })
                        .border_1()
                        .border_color(p.hairline(0.07))
                        .shadow_sm()
                        .cursor_pointer()
                        .hover(|s| s.border_color(p.hairline(0.14)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .px(px(12.))
                                .pt(px(10.))
                                .pb(px(8.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .text_size(px(11.5))
                                        .text_color(p.faint)
                                        .child(div().flex_1().min_w_0().truncate().child(status))
                                        .when(live, |d| {
                                            d.child(div().size(px(6.)).rounded_full().bg(p.accent))
                                        }),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(13.5))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(p.text)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(12.))
                                        .text_color(p.muted)
                                        .child(body),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .mx(px(8.))
                                .mb(px(8.))
                                .rounded(px(8.))
                                .bg(p.accent.opacity(if p.is_dark { 0.10 } else { 0.07 }))
                                .border_1()
                                .border_color(p.hairline(0.04))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(glyph(icon, 30., p.accent.opacity(0.55))),
                        )
                        .on_click(move |_, _, cx| {
                            store.update(cx, |s, cx| match &id {
                                Some(id) => s.focus_tab(id, cx),
                                None => {
                                    s.open_tab(&url, None, None, cx);
                                }
                            });
                        }),
                );
            }
            hero = hero.child(
                div()
                    .w_full()
                    .mt(rems(4.5))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .px(px(14.))
                            .text_size(px(14.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child("Recent tasks"),
                    )
                    .child(row),
            );
        }

        root.child(hero).child(div().h(px(32.)).flex_none())
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
    pub fn new(
        store: Entity<Store>,
        _tab: String,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
    div().flex().flex_col().gap(space::SM).child(
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
    div()
        .text_size(px(13.))
        .text_color(p.text)
        .child(text.to_string())
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
        .border_color(if active {
            p.accent.opacity(0.5)
        } else {
            p.border
        })
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
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Settings"),
                    ),
            );

        // Appearance
        let appearance = settings.appearance;
        let store_a = store.clone();
        page = page.child(
            section("Appearance", &p).child(
                div().flex().gap(space::SM).children(
                    [
                        ("System", hifi_core::Appearance::System),
                        ("Light", hifi_core::Appearance::Light),
                        ("Dark", hifi_core::Appearance::Dark),
                    ]
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
            ),
        );

        // Accent
        let accent = settings.accent.clone();
        let store_b = store.clone();
        page = page.child(
            section("Accent", &p).child(div().flex().gap(space::SM).flex_wrap().children(
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
                        .border_color(if on {
                            p.text
                        } else {
                            gpui::transparent_white()
                        })
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            store.update(cx, |s, cx| {
                                s.update_settings(|st| st.accent = name.to_string(), cx)
                            })
                        })
                }),
            )),
        );

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
                .child(sub(
                    &p,
                    "Local file — rendered behind the glass cards with a dim overlay.",
                )),
        );

        // Sidebar / session
        let store_d = store.clone();
        let compact = settings.compact_sidebar;
        let restore = settings.restore_session;
        page = page.child(
            section("Behavior", &p)
                .child(row().child(label(&p, "Compact sidebar")).child({
                    let store = store_d.clone();
                    chip_button("compact", if compact { "On" } else { "Off" }, compact, &p)
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            store.update(cx, |s, cx| {
                                s.update_settings(|st| st.compact_sidebar = !st.compact_sidebar, cx)
                            })
                        })
                }))
                .child(row().child(label(&p, "Restore session on launch")).child({
                    let store = store_d.clone();
                    chip_button("restore", if restore { "On" } else { "Off" }, restore, &p)
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            store.update(cx, |s, cx| {
                                s.update_settings(|st| st.restore_session = !st.restore_session, cx)
                            })
                        })
                })),
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

        page.child(div().mt(px(6.)).child(sub(
            &p,
            "Hi-Fi — GPUI shell · WKWebView engine · Solar Icons (CC BY 4.0, 480 Design)",
        )))
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
            let git = move |args: Vec<&'static str>| {
                let path = path.clone();
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(path)
                    .args(args)
                    .output()
            };
            let (out, numstat, branch) = cx
                .background_executor()
                .spawn(async move {
                    (
                        git(vec!["diff", "HEAD", "--", "."]),
                        git(vec!["diff", "--numstat", "HEAD"]),
                        git(vec!["branch", "--show-current"]),
                    )
                })
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
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|v, _, _, cx| {
                                    v.refresh(cx);
                                }),
                            ),
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
                    .children(
                        self.lines
                            .iter()
                            .cloned()
                            .map(|(c, l)| div().text_color(c).whitespace_nowrap().child(l)),
                    ),
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
