//! `⌘T` command bar — Spotlight-style launcher floating above content
//! (deferred element, so it composites over native webviews).

use gpui::{
    Context, Entity, KeyDownEvent, MouseButton, SharedString, Window, div, img, prelude::*, px,
};

use crate::assets::icons;
use crate::store::Store;
use crate::text_input::{TextField, TextFieldEvent};
use crate::theme::{Theme, radius, space};
use crate::views::glyph;

#[derive(Clone)]
enum Candidate {
    OpenUrl(String, String),                         // display, url
    SwitchTab(String, String, &'static str, String), // display, tab id, icon, favicon
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
        cx.subscribe(&input, move |me: &mut CommandBar, _i, event, cx| {
            match event {
                TextFieldEvent::Changed => cx.notify(),
                TextFieldEvent::Submitted(_) => {
                    // Enter runs the highlighted candidate (the URL/search
                    // row by default, the command list when empty).
                    me.refresh(cx);
                    me.execute(cx);
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
                        crate::sidebar::kind_icon(tab),
                        tab.favicon_url.clone(),
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
                    crate::sidebar::kind_icon(tab),
                    tab.favicon_url.clone(),
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
                Candidate::SwitchTab(_, id, _, _) => s.focus_tab(&id, cx),
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
        let mut commands_heading = false;
        let mut tabs_heading = false;
        for (i, cand) in self.candidates.iter().enumerate() {
            if matches!(cand, Candidate::Command(..)) && !commands_heading {
                commands_heading = true;
                list = list.child(
                    div()
                        .h(px(20.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .text_size(px(10.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(p.faint)
                        .child("Commands"),
                );
            }
            if matches!(cand, Candidate::SwitchTab(..)) && !tabs_heading {
                tabs_heading = true;
                list = list.child(
                    div()
                        .h(px(20.))
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .text_size(px(10.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(p.faint)
                        .child("Open tabs"),
                );
            }
            let (icon, label, shortcut, favicon) = match cand {
                Candidate::OpenUrl(l, _) => (icons::GLOBAL, l.clone(), None, None),
                Candidate::SwitchTab(l, _, icon, favicon) => {
                    (*icon, l.clone(), None, Some(favicon.clone()))
                }
                Candidate::Command(l, command) => {
                    let (icon, shortcut) = match command {
                        Command::NewTab => (icons::PLUS, None),
                        Command::NewTerminal => (icons::TERMINAL, None),
                        Command::NewAgent => (icons::CHAT_ROUND_LINE, None),
                        Command::NewDiff => (icons::GIT_BRANCH, None),
                        Command::Settings => (icons::SETTINGS, Some("⌘,")),
                        Command::ToggleSidebar => (icons::SIDEBAR_LEFT, Some("⌘B")),
                        Command::Reload => (icons::REFRESH, Some("⌘R")),
                        Command::Pin => (icons::PIN, None),
                        Command::NewGroup => (icons::FOLDER, Some("⌘⇧\\")),
                    };
                    (icon, l.clone(), shortcut, None)
                }
            };
            let active = i == self.selected;
            let me = cx.entity();
            let icon_element = favicon
                .and_then(|path| crate::sidebar::favicon_file(&path))
                .map(|path| img(path).size(px(16.)).rounded(px(3.)).into_any_element())
                .unwrap_or_else(|| {
                    glyph(icon, 14., if active { p.text } else { p.faint }).into_any_element()
                });
            list = list.child(
                div()
                    .id(SharedString::from(format!("cb-{i}")))
                    .flex()
                    .items_center()
                    .gap(space::SM)
                    .px(px(12.))
                    .h(px(30.))
                    .rounded(radius::ROUND)
                    .cursor_pointer()
                    .bg(if active {
                        p.wash(0.06)
                    } else {
                        gpui::transparent_white()
                    })
                    .when(!active, |d| d.hover(|s| s.bg(p.glass_hover())))
                    .child(icon_element)
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(if active { p.text } else { p.muted })
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(label),
                    )
                    .when_some(shortcut, |d, shortcut| {
                        d.child(
                            div()
                                .px(px(5.))
                                .py(px(2.))
                                .rounded(px(4.))
                                .bg(p.ink(0.04))
                                .border_1()
                                .border_color(p.hairline(0.08))
                                .text_size(px(10.))
                                .text_color(p.faint)
                                .child(shortcut),
                        )
                    })
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
            .bg(p.dialog)
            .border_1()
            .border_color(p.hairline(0.10))
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
