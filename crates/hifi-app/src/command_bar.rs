//! `⌘T` command bar — Spotlight-style launcher floating above content
//! (deferred element, so it composites over native webviews).

use gpui::{Context, Entity, KeyDownEvent, MouseButton, SharedString, Window, div, prelude::*, px};

use crate::assets::icons;
use crate::store::Store;
use crate::text_input::{TextField, TextFieldEvent};
use crate::theme::{Theme, radius, space};
use crate::views::glyph;

#[derive(Clone)]
enum Candidate {
    OpenUrl(String, String),   // display, url
    SwitchTab(String, String), // display, tab id
    Command(String, Command),
}

#[derive(Clone)]
enum Command {
    NewTab,
    NewTerminal,
    NewAgent,
    NewDiff,
    Settings,
    ToggleSidebar,
    Reload,
    Pin,
    NewGroup,
}

pub struct CommandBar {
    store: Entity<Store>,
    pub input: Entity<TextField>,
    candidates: Vec<Candidate>,
    selected: usize,
}

impl CommandBar {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut t = TextField::new("Type a URL, search, or command…", cx);
            t.placeholder_color = None;
            t
        });
        let store_for_events = store.clone();
        cx.subscribe(&input, move |_me, _i, event, cx| {
            match event {
                TextFieldEvent::Changed => cx.notify(),
                TextFieldEvent::Submitted(text) => {
                    // Enter without selection = open as URL/search.
                    store_for_events.update(cx, |s, cx| s.submit_address(text, cx));
                }
                TextFieldEvent::Escaped => {
                    store_for_events.update(cx, |s, cx| {
                        s.command_bar_open = false;
                        s.address_target = None;
                        cx.notify();
                    });
                }
            }
        })
        .detach();
        Self {
            store,
            input,
            candidates: vec![],
            selected: 0,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().to_string();
        let store = self.store.read(cx);
        let mut out: Vec<Candidate> = Vec::new();
        let q = query.to_lowercase();

        if !q.is_empty() {
            // Fuzzy-match open tabs first.
            for tab in &store.state.tabs {
                let hay = format!("{} {}", tab.title, tab.url).to_lowercase();
                if hay.contains(&q) {
                    out.push(Candidate::SwitchTab(
                        format!("{} — {}", tab.display_title(), hifi_core::host_of(&tab.url)),
                        tab.id.clone(),
                    ));
                }
            }
            // Then URL/search.
            let url = match hifi_core::route(&query) {
                hifi_core::RoutedUrl::External(u) => {
                    if let Some(qq) = u.strip_prefix("search:") {
                        hifi_core::schemes::resolve_search(&store.state.settings.search_engine, qq)
                    } else {
                        u
                    }
                }
                _ => query.clone(),
            };
            out.insert(
                0,
                Candidate::OpenUrl(
                    if url.starts_with("http") || url.starts_with("hifi") {
                        format!("Open {url}")
                    } else {
                        format!("Open {query}")
                    },
                    url,
                ),
            );
        } else {
            // Empty query: command palette mode.
            out.extend([
                Candidate::Command("New tab".into(), Command::NewTab),
                Candidate::Command("New terminal".into(), Command::NewTerminal),
                Candidate::Command("New agent".into(), Command::NewAgent),
                Candidate::Command("Open diff".into(), Command::NewDiff),
                Candidate::Command("Toggle sidebar".into(), Command::ToggleSidebar),
                Candidate::Command("Toggle pin".into(), Command::Pin),
                Candidate::Command("Reload".into(), Command::Reload),
                Candidate::Command("New group".into(), Command::NewGroup),
                Candidate::Command("Settings".into(), Command::Settings),
            ]);
            // Recent tabs.
            for tab in store.state.tabs.iter().rev().take(6) {
                out.push(Candidate::SwitchTab(
                    format!("{} — {}", tab.display_title(), hifi_core::host_of(&tab.url)),
                    tab.id.clone(),
                ));
            }
        }
        self.candidates = out;
        self.selected = self.selected.min(self.candidates.len().saturating_sub(1));
    }

    fn execute(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.candidates.get(self.selected).cloned() else {
            return;
        };
        self.store.update(cx, |s, cx| {
            if let Candidate::OpenUrl(_, url) = &c {
                s.submit_address(url, cx);
                return;
            }
            s.command_bar_open = false;
            s.address_target = None;
            match c {
                Candidate::OpenUrl(..) => {}
                Candidate::SwitchTab(_, id) => s.focus_tab(&id, cx),
                Candidate::Command(_, cmd) => match cmd {
                    Command::NewTab => {
                        s.open_tab("hifi://newtab", None, None, cx);
                    }
                    Command::NewTerminal => {
                        s.open_tab("hifi://terminal", None, None, cx);
                    }
                    Command::NewAgent => {
                        s.open_tab("hifi://agent", None, None, cx);
                    }
                    Command::NewDiff => {
                        s.open_tab("hifi://diff", None, None, cx);
                    }
                    Command::Settings => {
                        s.open_tab("hifi://settings", None, None, cx);
                    }
                    Command::ToggleSidebar => {
                        s.sidebar_collapsed = !s.sidebar_collapsed;
                        cx.notify();
                    }
                    Command::Pin => {
                        if let Some(id) = s.active_tab_id() {
                            s.toggle_pin(&id, cx);
                        }
                    }
                    Command::Reload => {
                        if let Some(id) = s.active_tab_id() {
                            s.pending_reload.push(id);
                            cx.notify();
                        }
                    }
                    Command::NewGroup => {
                        s.create_group("Group", None, cx);
                    }
                },
            }
        });
        self.input.update(cx, |i, cx| i.reset(cx));
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, _w: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "down" => {
                self.selected = (self.selected + 1).min(self.candidates.len().saturating_sub(1));
                cx.notify();
            }
            "up" => {
                self.selected = self.selected.saturating_sub(1);
                cx.notify();
            }
            _ => {}
        }
    }
}

impl gpui::Render for CommandBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh(cx);
        let p = Theme::of(cx).palette;
        let mut list = div().flex().flex_col().max_h(px(320.)).overflow_y_hidden();
        for (i, cand) in self.candidates.iter().enumerate() {
            let (icon, label, sub) = match cand {
                Candidate::OpenUrl(l, _) => (icons::GLOBE, l.clone(), "open".to_string()),
                Candidate::SwitchTab(l, _) => (icons::LIST, l.clone(), "switch".into()),
                Candidate::Command(l, _) => (icons::COMMAND, l.clone(), "command".into()),
            };
            let active = i == self.selected;
            let me = cx.entity();
            list = list.child(
                div()
                    .id(SharedString::from(format!("cb-{i}")))
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .px(px(12.))
                    .h(px(34.))
                    .rounded(radius::ROUND)
                    .cursor_pointer()
                    .bg(if active {
                        p.accent.opacity(0.16)
                    } else {
                        gpui::transparent_white()
                    })
                    .child(glyph(icon, 14., if active { p.accent } else { p.faint }))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(if active { p.text } else { p.muted })
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(p.faint.opacity(0.7))
                            .child(sub),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        me.update(cx, |bar, cx| {
                            bar.selected = i;
                            bar.execute(cx);
                        });
                    }),
            );
        }

        div()
            .w(px(640.))
            .rounded(radius::BUBBLE)
            .bg(p.dialog.opacity(0.92))
            .border_1()
            .border_color(p.border_strong)
            .shadow_lg()
            .flex()
            .flex_col()
            .p(px(8.))
            .gap(px(4.))
            .on_key_down(cx.listener(Self::key))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .px(px(8.))
                    .h(px(40.))
                    .child(glyph(icons::COMMAND, 16., p.accent))
                    .child(div().flex_1().child(self.input.clone())),
            )
            .child(list)
    }
}
