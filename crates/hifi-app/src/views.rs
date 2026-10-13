//! Internal surfaces: the glass new-tab page, settings, diff viewer, file
//! preview — pure GPUI, no native views.

use gpui::{
    Context, Entity, Focusable, Hsla, MouseButton, ScrollHandle, SharedString, Window, div, img,
    prelude::*, px, svg,
};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::assets::icons;
use crate::store::Store;
use crate::text_input::{TextField, TextFieldEvent};
use crate::theme::{Palette, Theme, space};

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
    picker_open: bool,
}

const ASK_PLACEHOLDER: &str = "Ask AI a task, @ for context";
const ASK_COMPACT_PLACEHOLDER: &str = "Ask AI a task, @ for context";
const SEARCH_PLACEHOLDER: &str = "Search or type a URL";

impl NewTabView {
    pub fn new(
        store: Entity<Store>,
        tab_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextField::new(ASK_PLACEHOLDER, cx));
        input.update(cx, |input, cx| {
            input.set_compact_placeholder(ASK_COMPACT_PLACEHOLDER, cx);
        });
        cx.subscribe(&input, move |me: &mut NewTabView, input, event, cx| {
            let TextFieldEvent::Submitted(text) = event else {
                return;
            };
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            let tab = tab_id.clone();
            let ask = me.ask;
            me.store.update(cx, |store, cx| {
                let harness = store.state.settings.agent_harness.clone();
                if ask && !looks_like_address(&text) {
                    store.start_agent_in(&tab, &harness, &text, cx);
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
            picker_open: false,
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
    let label: SharedString = label.into();
    let icon_only = label.is_empty();
    div()
        .id(id.into())
        .flex()
        .items_center()
        .gap(px(5.))
        .h(px(28.))
        .px(px(8.))
        .when(icon_only, |d| {
            d.w(px(28.)).px(px(0.)).justify_center().gap(px(0.))
        })
        .rounded(px(8.))
        .cursor_pointer()
        .text_size(px(12.5))
        .text_color(p.muted)
        .hover(|s| s.bg(p.glass_hover()).text_color(p.text))
        .child(glyph(icon, 13., p.muted))
        .when(!icon_only, |d| {
            d.child(
                div()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(label),
            )
        })
        .when(chevron && !icon_only, |d| {
            d.child(glyph(icons::ALT_ARROW_DOWN, 10., p.faint))
        })
}

struct TaskCard {
    id: Option<String>,
    url: String,
    title: String,
    preview: String,
    time: String,
    running: bool,
}

fn relative_time(timestamp: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(timestamp);
    let elapsed = now.saturating_sub(timestamp);
    match elapsed {
        0..=59 => "Now".into(),
        60..=3_599 => format!("{}m ago", elapsed / 60),
        3_600..=86_399 => format!("{}h ago", elapsed / 3_600),
        86_400..=172_799 => "Yesterday".into(),
        _ => format!("{}d ago", elapsed / 86_400),
    }
}

fn modified_timestamp(path: &std::path::Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
}

impl gpui::Render for NewTabView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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

        // Recent sessions first, then recently visited pages.
        let mut recent_keys = HashSet::new();
        let mut cards = Vec::with_capacity(3);
        for tab in store.state.tabs.iter().rev() {
            let kind = match tab.kind {
                hifi_core::TabKind::Agent => "agent",
                hifi_core::TabKind::Terminal => "terminal",
                _ => continue,
            };
            let title = if !tab.prompt.is_empty() {
                tab.prompt.clone()
            } else {
                tab.display_title()
            };
            if !recent_keys.insert((kind, title.clone())) {
                continue;
            }
            let place = if tab.cwd.is_empty() {
                "Local".to_string()
            } else {
                basename_of(&tab.cwd)
            };
            let chat_path = store.paths.chats_dir().join(format!("{}.json", tab.id));
            cards.push(TaskCard {
                id: Some(tab.id.clone()),
                url: String::new(),
                title,
                preview: if kind == "agent" {
                    crate::agent_chat::latest_assistant_preview(&chat_path)
                        .unwrap_or_else(|| place.clone())
                } else {
                    place
                },
                time: modified_timestamp(&chat_path)
                    .map(relative_time)
                    .unwrap_or_else(|| "Today".into()),
                running: store.running_agents.contains(&tab.id),
            });
            if cards.len() == 3 {
                break;
            }
        }
        for entry in store.history.iter().rev() {
            if cards.len() >= 3 {
                break;
            }
            if cards.iter().any(|c| c.url == entry.url) {
                continue;
            }
            let host = hifi_core::host_of(&entry.url);
            let title = if entry.title.is_empty() {
                host.clone()
            } else {
                entry.title.clone()
            };
            if !recent_keys.insert(("web", title.clone())) {
                continue;
            }
            cards.push(TaskCard {
                id: None,
                url: entry.url.clone(),
                title,
                preview: host,
                time: relative_time(entry.visited_at),
                running: false,
            });
        }

        let (harness, model, guard, cwd) = {
            let st = &self.store.read(cx).state.settings;
            (
                st.agent_harness.clone(),
                st.agent_model.clone(),
                st.agent_guard,
                st.agent_cwd.clone(),
            )
        };
        let h_icon = crate::sidebar::mark_icon(&harness);
        let ask = self.ask;
        let compact_width = f32::from(window.viewport_size().width) < 760.;

        let mode_seg = |label: &'static str, on: bool, id: &'static str| {
            div()
                .id(id)
                .h(px(28.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(12.5))
                .when(on, |d| {
                    if p.is_dark {
                        d.bg(p.raised).text_color(p.text)
                    } else {
                        d.bg(p.pill()).text_color(p.text).shadow(vec![
                            gpui::BoxShadow::new(px(0.), px(1.), p.ink(0.06)).blur_radius(px(2.)),
                        ])
                    }
                })
                .when(!on, |d| {
                    d.text_color(p.muted).hover(|s| s.text_color(p.text))
                })
                .child(label)
        };

        let composer = div()
            .id("composer")
            .w_full()
            .max_w(px(680.))
            .min_h(px(52.))
            .pl(px(16.))
            .pr(px(6.))
            .py(px(6.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .rounded(px(14.))
            .bg(if has_image {
                gpui::black().opacity(0.35)
            } else {
                p.card
            })
            .border_1()
            .border_color(if self.input.read(cx).focus_handle(cx).is_focused(window) {
                p.border_strong
            } else {
                p.border
            })
            .shadow(p.shadow_card())
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
                    .min_w(px(180.))
                    .text_size(px(14.))
                    .text_color(fg)
                    .child(self.input.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .p(px(3.))
                    .gap(px(2.))
                    .h(px(28.))
                    .rounded(px(8.))
                    .bg(p.ink(0.04))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .px(px(6.))
                            .text_size(px(11.))
                            .text_color(p.faint)
                            .when(!compact_width, |d| {
                                d.child(
                                    div()
                                        .px(px(4.))
                                        .rounded(px(4.))
                                        .border_1()
                                        .border_color(p.border)
                                        .child("Tab"),
                                )
                                .child("to switch")
                            }),
                    )
                    .child(mode_seg("Search", !ask, "mode-search").on_click(
                        cx.listener(|this, _: &gpui::ClickEvent, _, cx| this.set_mode(false, cx)),
                    ))
                    .child(mode_seg("Ask", ask, "mode-ask").on_click(
                        cx.listener(|this, _: &gpui::ClickEvent, _, cx| this.set_mode(true, cx)),
                    )),
            );

        let store_ctx = self.store.clone();
        let store_cwd = self.store.clone();
        let store_guard = self.store.clone();
        let store_model = self.store.clone();
        let this_m = cx.entity().downgrade();
        let folder = if cwd.is_empty() {
            "Home".to_string()
        } else {
            std::path::Path::new(&cwd)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(cwd.clone())
        };
        let menu = (ask && self.picker_open).then(|| {
            div()
                .absolute()
                .bottom(px(28.))
                .right(px(4.))
                .child(crate::agent_chat::model_menu(
                    &harness,
                    &model,
                    380.,
                    &p,
                    move |h, m, cx| {
                        store_model.update(cx, |s, cx| {
                            s.update_settings(
                                |st| {
                                    st.agent_harness = h.to_string();
                                    st.agent_model = m.to_string();
                                },
                                cx,
                            )
                        });
                        let _ = this_m.update(cx, |v: &mut NewTabView, cx| {
                            v.picker_open = false;
                            cx.notify();
                        });
                    },
                ))
        });
        let controls = div()
            .relative()
            .w_full()
            .min_w(px(0.))
            .max_w(px(680.))
            .px(px(4.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(
                composer_pill("ctl-add", icons::PLUS, "", false, &p).on_click(move |_, _, cx| {
                    store_ctx.update(cx, |s, cx| s.dock_open("hifi://notes", cx));
                }),
            )
            .child(
                composer_pill(
                    "ctl-local",
                    icons::FOLDER,
                    if compact_width { String::new() } else { folder },
                    true,
                    &p,
                )
                .on_click(move |_, _, cx| {
                    let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                        files: false,
                        directories: true,
                        multiple: false,
                        prompt: Some("Use folder".into()),
                    });
                    let store = store_cwd.clone();
                    cx.spawn(async move |cx| {
                        if let Ok(Ok(Some(paths))) = rx.await
                            && let Some(path) = paths.first()
                        {
                            let cwd = path.display().to_string();
                            cx.update(|cx| {
                                store.update(cx, |s, cx| {
                                    s.update_settings(|st| st.agent_cwd = cwd, cx)
                                })
                            });
                        }
                    })
                    .detach();
                }),
            )
            .child(
                composer_pill(
                    "ctl-guard",
                    icons::SHIELD,
                    if compact_width {
                        String::new()
                    } else {
                        format!("Guard · {}", guard.label())
                    },
                    true,
                    &p,
                )
                .on_click(move |_, _, cx| {
                    let i = hifi_core::Guard::ALL
                        .iter()
                        .position(|g| *g == guard)
                        .unwrap_or(0);
                    let next = hifi_core::Guard::ALL[(i + 1) % hifi_core::Guard::ALL.len()];
                    store_guard.update(cx, |s, cx| {
                        s.update_settings(|st| st.agent_guard = next, cx)
                    });
                }),
            )
            .when(ask, |d| {
                d.child(
                    composer_pill(
                        "ctl-model",
                        h_icon,
                        if compact_width {
                            String::new()
                        } else {
                            crate::agent_runner::model_short(&model).to_string()
                        },
                        true,
                        &p,
                    )
                    .ml_auto()
                    .max_w(px(160.))
                    .overflow_hidden()
                    .on_click(cx.listener(
                        |this, _: &gpui::ClickEvent, _, cx| {
                            this.picker_open = !this.picker_open;
                            cx.notify();
                        },
                    )),
                )
            })
            .children(menu.map(|m| gpui::deferred(m).with_priority(1)));

        let mut hero = div()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .w_full()
            .max_w(px(680.))
            .px(space::LG)
            .gap(px(10.))
            .child(div().mb(px(18.)).child(glyph(
                icons::HIFI_MARK,
                40.,
                fg.opacity(if has_image { 0.9 } else { 0.28 }),
            )))
            .child(composer)
            .child(controls);

        if !cards.is_empty() {
            let mut row = div().w_full().flex().items_stretch().gap(px(12.));
            for (i, card) in cards.into_iter().enumerate() {
                let store = self.store.clone();
                let TaskCard {
                    id,
                    url,
                    title,
                    preview,
                    time,
                    running,
                } = card;
                row = row.child(
                    div()
                        .id(SharedString::from(format!("task-{i}")))
                        .flex_1()
                        .min_w(px(0.))
                        .min_h(px(112.))
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .p(px(14.))
                        .rounded(px(12.))
                        .bg(if has_image {
                            gpui::black().opacity(0.35)
                        } else {
                            p.card
                        })
                        .border_1()
                        .border_color(p.border)
                        .cursor_pointer()
                        .hover(|s| s.bg(p.glass_hover()))
                        .child(
                            div()
                                .w_full()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(div().text_size(px(12.)).text_color(p.faint).child(time))
                                .child(div().flex_1())
                                .when(running, |d| {
                                    d.child(div().size(px(6.)).rounded_full().bg(p.text))
                                }),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .overflow_hidden()
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(fg)
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(title),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h(px(0.))
                                .overflow_hidden()
                                .text_size(px(12.5))
                                .text_color(if has_image { fg.opacity(0.7) } else { p.muted })
                                .line_clamp(2)
                                .child(preview),
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
            hero = hero.child(div().w_full().mt(px(12.)).flex().flex_col().child(row));
        }

        root.child(div().h(gpui::relative(0.30)).min_h(px(48.)).flex_none())
            .child(hero)
            .child(div().h(px(32.)).flex_none())
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

pub struct SettingsView {
    store: Entity<Store>,
    bg_field: Entity<TextField>,
    composio_field: Entity<TextField>,
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
        let composio_key = store.read(cx).state.settings.composio_api_key.clone();
        let composio_field = cx.new(|cx| {
            let mut field = TextField::new("Composio API key", cx);
            field.set_secret(true, cx);
            field
        });
        composio_field.update(cx, |field, cx| field.set_value(composio_key, cx));
        let store2 = store.clone();
        cx.subscribe(&bg_field, move |_me, _f, event, cx| {
            if let TextFieldEvent::Submitted(path) = event {
                store2.update(cx, |s, cx| {
                    s.update_settings(|st| st.background_image = path.clone(), cx);
                });
            }
        })
        .detach();
        let store2 = store.clone();
        cx.subscribe(&composio_field, move |_me, _field, event, cx| {
            if let TextFieldEvent::Submitted(key) = event {
                store2.update(cx, |s, cx| {
                    s.update_settings(|settings| settings.composio_api_key = key.clone(), cx);
                });
            }
        })
        .detach();
        Self {
            store,
            bg_field,
            composio_field,
        }
    }
}

fn settings_section(
    title: &str,
    p: &Palette,
    rows: impl IntoIterator<Item = gpui::AnyElement>,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(p.text)
                .child(title.to_string()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .overflow_hidden()
                .rounded(px(12.))
                .bg(p.card)
                .border_1()
                .border_color(p.border)
                .children(rows),
        )
}

fn settings_row(
    title: &str,
    detail: Option<&str>,
    control: impl IntoElement,
    p: &Palette,
    last: bool,
    wide_control: bool,
    natural_width: bool,
) -> gpui::Div {
    let mut control_side = div()
        .min_w(px(0.))
        .max_w(gpui::relative(1.))
        .flex_none()
        .flex()
        .items_center();
    if wide_control && !natural_width {
        control_side = control_side.w(px(210.));
    }

    div()
        .w_full()
        .min_w(px(0.))
        .min_h(px(if detail.is_some() { 64. } else { 52. }))
        .px(px(16.))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(16.))
        .when(!last, |d| d.border_b_1().border_color(p.hairline(0.12)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex_basis(px(108.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_size(px(13.5))
                        .text_color(p.text)
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(title.to_string()),
                )
                .when_some(detail, |d, detail| {
                    d.child(
                        div()
                            .text_size(px(12.5))
                            .text_color(p.faint)
                            .child(detail.to_string()),
                    )
                }),
        )
        .child(control_side.child(control))
}

fn settings_switch(id: &str, on: bool, p: &Palette) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id.to_string()))
        .w(px(36.))
        .h(px(22.))
        .px(px(3.))
        .flex()
        .items_center()
        .justify_end()
        .when(!on, |d| d.justify_start())
        .rounded_full()
        .cursor_pointer()
        .bg(if on { p.text } else { p.ink(0.15) })
        .child(div().size(px(16.)).rounded_full().bg(p.bg))
}

fn settings_segment(
    id: &str,
    title: &str,
    selected: bool,
    p: &Palette,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id.to_string()))
        .h(px(30.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(px(8.))
        .cursor_pointer()
        .text_size(px(12.5))
        .text_color(if selected { p.text } else { p.muted })
        .when(selected, |d| {
            if p.is_dark {
                d.bg(p.raised)
            } else {
                d.bg(p.pill()).shadow(vec![
                    gpui::BoxShadow::new(px(0.), px(1.), p.ink(0.06)).blur_radius(px(2.)),
                ])
            }
        })
        .child(title.to_string())
}

impl gpui::Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(cx).palette;
        let settings = self.store.read(cx).state.settings.clone();
        let store = self.store.clone();
        let mut page = div()
            .w_full()
            .max_w(px(720.))
            .px(px(40.))
            .pt(px(40.))
            .pb(px(40.))
            .flex()
            .flex_col()
            .gap(px(24.))
            .text_color(p.text)
            .child(
                div()
                    .text_size(px(22.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Settings"),
            );

        let appearance_store = store.clone();
        let appearance_row = settings_row(
            "Theme",
            None,
            div()
                .p(px(3.))
                .flex()
                .gap(px(2.))
                .rounded(px(8.))
                .bg(p.ink(0.05))
                .children(
                    [
                        ("System", hifi_core::Appearance::System),
                        ("Light", hifi_core::Appearance::Light),
                        ("Dark", hifi_core::Appearance::Dark),
                    ]
                    .into_iter()
                    .map(|(name, value)| {
                        let store = appearance_store.clone();
                        settings_segment(name, name, settings.appearance == value, &p)
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                store.update(cx, |s, cx| {
                                    s.update_settings(|st| st.appearance = value, cx)
                                })
                            })
                    }),
                ),
            &p,
            true,
            false,
            false,
        )
        .into_any_element();
        page = page.child(settings_section("Appearance", &p, [appearance_row]));

        let accent = settings.accent.clone();
        let accent_store = store.clone();
        let accent_control = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(6.))
            .children(crate::theme::ACCENTS.iter().map(|(name, dark_v, light_v)| {
                let store = accent_store.clone();
                let selected = accent == *name;
                let swatch = crate::theme::color(if p.is_dark { *dark_v } else { *light_v });
                div()
                    .id(SharedString::from(format!("acc-{name}")))
                    .size(px(16.))
                    .rounded_full()
                    .bg(swatch)
                    .cursor_pointer()
                    .border_1()
                    .border_color(if selected { p.text } else { p.hairline(0.12) })
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        store.update(cx, |s, cx| {
                            s.update_settings(|st| st.accent = name.to_string(), cx)
                        })
                    })
                    .into_any_element()
            }));
        page = page.child(settings_section(
            "Accent",
            &p,
            [
                settings_row("Accent color", None, accent_control, &p, true, true, true)
                    .into_any_element(),
            ],
        ));

        let store_bg = store.clone();
        let mut bg_control = div()
            .w_full()
            .min_w(px(0.))
            .h(px(30.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .rounded(px(8.))
            .bg(p.ink(0.03))
            .border_1()
            .border_color(p.border_strong)
            .child(div().flex_1().min_w(px(0.)).child(self.bg_field.clone()));
        if !settings.background_image.is_empty() {
            bg_control = bg_control.child(
                div()
                    .id("clear-bg")
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(p.muted)
                    .hover(|s| s.bg(p.glass_hover()))
                    .child("Clear")
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        store_bg.update(cx, |s, cx| {
                            s.update_settings(|st| st.background_image.clear(), cx)
                        })
                    }),
            );
        }
        page = page.child(settings_section(
            "New tab background",
            &p,
            [settings_row(
                "Image path",
                Some("Local path · press Enter to apply"),
                bg_control,
                &p,
                true,
                true,
                false,
            )
            .into_any_element()],
        ));

        let compact = settings.compact_sidebar;
        let restore = settings.restore_session;
        let compact_store = store.clone();
        let restore_store = store.clone();
        let behavior = [
            settings_row(
                "Compact sidebar",
                None,
                settings_switch("compact", compact, &p).on_mouse_down(
                    MouseButton::Left,
                    move |_, _, cx| {
                        compact_store.update(cx, |s, cx| {
                            s.update_settings(|st| st.compact_sidebar = !st.compact_sidebar, cx)
                        })
                    },
                ),
                &p,
                false,
                false,
                false,
            )
            .into_any_element(),
            settings_row(
                "Restore session on launch",
                None,
                settings_switch("restore", restore, &p).on_mouse_down(
                    MouseButton::Left,
                    move |_, _, cx| {
                        restore_store.update(cx, |s, cx| {
                            s.update_settings(|st| st.restore_session = !st.restore_session, cx)
                        })
                    },
                ),
                &p,
                true,
                false,
                false,
            )
            .into_any_element(),
        ];
        page = page.child(settings_section("Behavior", &p, behavior));

        let composio_field = self.composio_field.clone();
        let composio_key = div()
            .w(px(210.))
            .max_w(gpui::relative(1.))
            .flex_none()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(5.))
            .text_size(px(13.))
            .child(
                div()
                    .w_full()
                    .h(px(30.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(p.border_strong)
                    .bg(if p.is_dark { p.card } else { p.ink(0.02) })
                    .child(composio_field),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(p.faint)
                    .cursor_pointer()
                    .hover(|d| d.text_color(p.text))
                    .child("Get a key")
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.open_url("https://composio.dev");
                    }),
            );
        let driver_path = PathBuf::from(&settings.cua_driver_path)
            .is_file()
            .then(|| PathBuf::from(&settings.cua_driver_path))
            .or_else(|| crate::agent_runner::find_binary("cua-driver"))
            .map(|path| path.display().to_string());
        let computer_use_store = store.clone();
        let computer_use = settings.computer_use;
        let driver_status = driver_path
            .map(|path| format!("Installed · {path}"))
            .unwrap_or_else(|| "Not installed".into());
        page = page.child(settings_section(
            "Integrations",
            &p,
            [
                settings_row(
                    "Composio API key",
                    Some("Press Return to save"),
                    composio_key,
                    &p,
                    false,
                    true,
                    false,
                )
                .into_any_element(),
                settings_row(
                    "Computer use",
                    None,
                    settings_switch("computer-use", computer_use, &p).on_mouse_down(
                        MouseButton::Left,
                        move |_, _, cx| {
                            computer_use_store.update(cx, |s, cx| {
                                s.update_settings(
                                    |settings| settings.computer_use = !settings.computer_use,
                                    cx,
                                )
                            })
                        },
                    ),
                    &p,
                    false,
                    false,
                    true,
                )
                .into_any_element(),
                settings_row(
                    "CUA driver",
                    None,
                    div()
                        .min_w(px(0.))
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .truncate()
                        .child(driver_status),
                    &p,
                    true,
                    false,
                    true,
                )
                .into_any_element(),
            ],
        ));

        let data_path = store.read(cx).paths.root.display().to_string();
        let state_path = store.read(cx).paths.state_file().display().to_string();
        let cli_path = dirs::home_dir()
            .map(|home| home.join(".cargo").join("bin").join("hifi"))
            .unwrap_or_else(|| PathBuf::from("hifi"))
            .display()
            .to_string();
        let path_control = |id: &'static str, path: String| {
            let copy = path.clone();
            let tooltip = path.clone();
            div()
                .id(SharedString::from(format!("{id}-path")))
                .w_full()
                .min_w(px(0.))
                .flex()
                .items_center()
                .gap(px(7.))
                .tooltip(move |_, cx| {
                    cx.new(|_| crate::sidebar::SpaceTooltip(tooltip.clone()))
                        .into()
                })
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .font_family(crate::theme::FONT_MONO)
                        .text_size(px(10.5))
                        .text_color(p.faint)
                        .child(path),
                )
                .child(
                    div()
                        .id(id)
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .cursor_pointer()
                        .text_color(p.muted)
                        .hover(|s| s.bg(p.glass_hover()))
                        .child(glyph(icons::COPY, 12., p.muted))
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy.clone()));
                        }),
                )
                .into_any_element()
        };
        page = page.child(settings_section(
            "Data & CLI",
            &p,
            [
                settings_row(
                    "Data directory",
                    None,
                    path_control("copy-data", data_path),
                    &p,
                    false,
                    true,
                    false,
                )
                .into_any_element(),
                settings_row(
                    "State file",
                    None,
                    path_control("copy-state", state_path),
                    &p,
                    false,
                    true,
                    false,
                )
                .into_any_element(),
                settings_row(
                    "CLI executable",
                    None,
                    path_control("copy-cli", cli_path),
                    &p,
                    true,
                    true,
                    false,
                )
                .into_any_element(),
            ],
        ));
        page = page.child(
            div()
                .text_size(px(11.))
                .text_color(p.faint)
                .child("Hi-Fi · GPUI shell · Solar Icons (CC BY 4.0, 480 Design)"),
        );

        div()
            .id("settings-scroll")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .bg(p.bg)
            .child(page)
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
