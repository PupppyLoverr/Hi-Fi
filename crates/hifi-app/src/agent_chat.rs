//! Agent chat surface: a transcript of the task (user turns, tool steps, page
//! previews, subagent fan-out, streamed replies) over a reply composer.
//!
//! Replies come from a built-in scripted agent so the surface is usable
//! without a harness installed; "Run in terminal" hands the same prompt to the
//! real CLI harness.

use gpui::{
    Context, Entity, Focusable, MouseButton, ScrollHandle, SharedString, Window, div, prelude::*,
    px,
};
use std::time::{Duration, Instant};

use crate::assets::icons;
use crate::store::Store;
use crate::text_input::{TextField, TextFieldEvent};
use crate::theme::{Palette, Theme};
use crate::views::glyph;

#[derive(Clone)]
enum Item {
    User(String),
    Step {
        icon: &'static str,
        text: String,
        link: Option<String>,
    },
    Page {
        host: String,
        title: String,
    },
    Text(String),
    Spawn(Vec<(&'static str, String, &'static str)>),
}

pub struct AgentChatView {
    store: Entity<Store>,
    tab_id: String,
    input: Entity<TextField>,
    items: Vec<Item>,
    /// Bumped on every run/stop so a superseded script stops emitting.
    run: u64,
    running: bool,
    started: Option<Instant>,
    scroll: ScrollHandle,
}

const MODELS: &[(&str, &str)] = &[
    ("Claude Code", "claude"),
    ("Codex", "codex"),
    ("Devin", "devin"),
    ("OpenCode", "opencode"),
    ("Amp", "amp"),
];

impl AgentChatView {
    pub fn new(
        store: Entity<Store>,
        tab_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextField::new("Reply, @ for context", cx));
        cx.subscribe(&input, |me: &mut AgentChatView, input, event, cx| {
            let TextFieldEvent::Submitted(text) = event else {
                return;
            };
            let text = text.trim().to_string();
            if text.is_empty() {
                return;
            }
            input.update(cx, |i, cx| i.reset(cx));
            me.send(text, cx);
        })
        .detach();
        window.focus(&input.read(cx).focus_handle(cx), cx);
        let prompt = store
            .read(cx)
            .state
            .tabs
            .iter()
            .find(|t| t.id == tab_id)
            .map(|t| t.prompt.clone())
            .unwrap_or_default();
        let mut me = Self {
            store,
            tab_id,
            input,
            items: Vec::new(),
            run: 0,
            running: false,
            started: None,
            scroll: ScrollHandle::new(),
        };
        let live = me.store.read(cx).live_agents.contains(&me.tab_id);
        if live && !prompt.trim().is_empty() {
            me.send(prompt, cx);
        } else if !prompt.trim().is_empty() {
            me.items.push(Item::User(prompt.clone()));
            me.items.extend(first_turn(&prompt));
        }
        me
    }

    fn harness(&self, cx: &Context<Self>) -> String {
        self.store
            .read(cx)
            .state
            .tabs
            .iter()
            .find(|t| t.id == self.tab_id)
            .map(|t| t.command.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "claude".into())
    }

    fn send(&mut self, text: String, cx: &mut Context<Self>) {
        let first = !self.items.iter().any(|i| matches!(i, Item::User(_)));
        if first {
            let id = self.tab_id.clone();
            let harness = self.harness(cx);
            let t = text.clone();
            self.store
                .update(cx, |s, cx| s.start_agent_in(&id, &harness, &t, cx));
        }
        self.items.push(Item::User(text.clone()));
        let script = if first {
            first_turn(&text)
        } else {
            follow_up(&text)
        };
        self.play(script, cx);
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.run += 1;
        self.running = false;
        self.started = None;
        self.items.push(Item::Step {
            icon: icons::STOP,
            text: "Stopped".into(),
            link: None,
        });
        cx.notify();
    }

    /// Emit each scripted item after a short "work" delay; text streams in
    /// word by word.
    fn play(&mut self, script: Vec<Item>, cx: &mut Context<Self>) {
        self.run += 1;
        let run = self.run;
        self.running = true;
        self.started = Some(Instant::now());
        self.scroll.scroll_to_bottom();
        cx.notify();
        cx.spawn(async move |this, cx| {
            for item in script {
                let pause = match item {
                    Item::Text(_) => 350,
                    Item::Page { .. } => 900,
                    Item::Spawn(_) => 1100,
                    _ => 700,
                };
                cx.background_executor()
                    .timer(Duration::from_millis(pause))
                    .await;
                let alive = this
                    .update(cx, |v: &mut AgentChatView, _| v.run == run)
                    .unwrap_or(false);
                if !alive {
                    return;
                }
                if let Item::Text(full) = item {
                    let words: Vec<&str> = full.split_inclusive(' ').collect();
                    let _ = this.update(cx, |v: &mut AgentChatView, cx| {
                        v.items.push(Item::Text(String::new()));
                        cx.notify();
                    });
                    let mut shown = String::new();
                    for w in words {
                        cx.background_executor()
                            .timer(Duration::from_millis(28))
                            .await;
                        shown.push_str(w);
                        let ok = this
                            .update(cx, |v: &mut AgentChatView, cx| {
                                if v.run != run {
                                    return false;
                                }
                                if let Some(Item::Text(t)) = v.items.last_mut() {
                                    t.clone_from(&shown);
                                }
                                v.scroll.scroll_to_bottom();
                                cx.notify();
                                true
                            })
                            .unwrap_or(false);
                        if !ok {
                            return;
                        }
                    }
                } else {
                    let _ = this.update(cx, |v: &mut AgentChatView, cx| {
                        v.items.push(item);
                        v.scroll.scroll_to_bottom();
                        cx.notify();
                    });
                }
            }
            let _ = this.update(cx, |v: &mut AgentChatView, cx| {
                if v.run == run {
                    v.running = false;
                    v.started = None;
                    cx.notify();
                }
            });
        })
        .detach();
        // Tick the "Working for Ns" label while running.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let go = this
                    .update(cx, |v: &mut AgentChatView, cx| {
                        cx.notify();
                        v.running && v.run == run
                    })
                    .unwrap_or(false);
                if !go {
                    return;
                }
            }
        })
        .detach();
    }
}

fn host_in(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '/' && c != ':'))
        .find(|w| w.contains('.') && !w.ends_with('.') && w.len() > 3)
        .map(|w| {
            hifi_core::host_of(&if w.contains("://") {
                w.to_string()
            } else {
                format!("https://{w}")
            })
        })
        .filter(|h| !h.is_empty())
}

fn short(text: &str, n: usize) -> String {
    let t: String = text.chars().take(n).collect();
    if text.chars().count() > n {
        format!("{t}…")
    } else {
        t
    }
}

fn first_turn(prompt: &str) -> Vec<Item> {
    let host = host_in(prompt).unwrap_or_else(|| "www.google.com".into());
    let topic = short(prompt, 48);
    vec![
        Item::Step {
            icon: icons::MAGNIFER,
            text: format!("Search the web for “{topic}”"),
            link: None,
        },
        Item::Step {
            icon: icons::GLOBE,
            text: format!("Open {host}"),
            link: None,
        },
        Item::Page {
            title: format!("{topic} — results"),
            host: host.clone(),
        },
        Item::Step {
            icon: icons::KEY_MINIMALISTIC,
            text: "Use saved sign-in for".into(),
            link: Some(host.clone()),
        },
        Item::Step {
            icon: icons::EYE,
            text: "Read the page and extract the relevant sections".into(),
            link: None,
        },
        Item::Text(format!(
            "I found three good sources for “{topic}”. Next, I'll check each of them in parallel so we can compare before I write anything down."
        )),
        Item::Spawn(vec![
            (icons::CLAUDE_MARK, "Summarise the top result".into(), "Opus 4.8"),
            (icons::OPENAI_MARK, "Cross-check facts and dates".into(), "GPT-6 Astra"),
            (icons::DEVIN_MARK, "Collect links into a page".into(), "Devin"),
        ]),
        Item::Step {
            icon: icons::DOCUMENT_ADD,
            text: "Save findings to Pages".into(),
            link: Some("Untitled".into()),
        },
        Item::Text(
            "Done. The three subagents agree on the main points, and I saved the summary with sources to a new page. Want me to turn it into a checklist or keep digging?"
                .into(),
        ),
    ]
}

fn follow_up(text: &str) -> Vec<Item> {
    vec![
        Item::Step {
            icon: icons::CHAT_ROUND_LINE,
            text: "Reading your reply".into(),
            link: None,
        },
        Item::Step {
            icon: icons::CHECKLIST,
            text: format!("Plan: {}", short(text, 56)),
            link: None,
        },
        Item::Text(format!(
            "Got it — “{}”. I've updated the plan and I'll keep the tabs I opened in this chat so you can follow along.",
            short(text, 80)
        )),
    ]
}

fn step_row(icon: &'static str, text: String, link: Option<String>, p: &Palette) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(9.))
        .py(px(4.))
        .text_size(px(13.))
        .text_color(p.muted)
        .child(glyph(icon, 14., p.faint))
        .child(div().min_w_0().truncate().child(text))
        .when_some(link, |d, l| d.child(div().text_color(p.accent).child(l)))
}

fn model_pill(label: &'static str, p: &Palette) -> gpui::Div {
    div()
        .flex_none()
        .h(px(20.))
        .px(px(8.))
        .flex()
        .items_center()
        .rounded_full()
        .bg(p.ink(0.05))
        .border_1()
        .border_color(p.hairline(0.06))
        .text_size(px(11.))
        .text_color(p.muted)
        .child(label)
}

fn page_card(host: &str, title: &str, p: &Palette) -> gpui::Div {
    let p = *p;
    let bar = |w: f32, o: f32| div().h(px(6.)).w(px(w)).rounded(px(3.)).bg(p.ink(o));
    div()
        .ml(px(23.))
        .my(px(4.))
        .w(px(300.))
        .rounded(px(10.))
        .overflow_hidden()
        .border_1()
        .border_color(p.hairline(0.08))
        .bg(if p.is_dark { p.dialog } else { gpui::white() })
        .shadow_sm()
        .child(
            div()
                .h(px(26.))
                .px(px(9.))
                .flex()
                .items_center()
                .gap(px(6.))
                .bg(p.ink(0.035))
                .border_b_1()
                .border_color(p.hairline(0.06))
                .child(div().size(px(6.)).rounded_full().bg(p.ink(0.18)))
                .child(div().size(px(6.)).rounded_full().bg(p.ink(0.18)))
                .child(
                    div()
                        .ml(px(4.))
                        .flex_1()
                        .h(px(16.))
                        .px(px(6.))
                        .flex()
                        .items_center()
                        .rounded(px(4.))
                        .bg(p.ink(0.05))
                        .text_size(px(10.))
                        .text_color(p.faint)
                        .child(host.to_string()),
                ),
        )
        .child(
            div()
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(7.))
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(p.text)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .h(px(54.))
                        .rounded(px(6.))
                        .bg(p.accent.opacity(if p.is_dark { 0.16 } else { 0.10 })),
                )
                .child(bar(250., 0.08))
                .child(bar(210., 0.06))
                .child(bar(160., 0.05)),
        )
}

impl gpui::Render for AgentChatView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(cx).palette;
        let harness = self.harness(cx);
        let (h_label, h_icon) = MODELS
            .iter()
            .find(|(_, c)| *c == harness)
            .map(|(l, c)| (*l, crate::sidebar::mark_icon(c)))
            .unwrap_or(("Claude Code", icons::CLAUDE_MARK));
        let title = self
            .items
            .iter()
            .find_map(|i| match i {
                Item::User(t) => Some(short(t, 90)),
                _ => None,
            })
            .unwrap_or_else(|| "New chat".into());

        let mut transcript = div()
            .w_full()
            .max_w(px(700.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(24.))
            .pt(px(8.))
            .pb(px(24.));

        if self.items.is_empty() {
            transcript = transcript.child(
                div()
                    .pt(px(120.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .child(glyph(icons::HIFI_MARK, 40., p.text.opacity(0.25)))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(p.text)
                            .child("What should we do?"),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(p.faint)
                            .child("The agent browses, reads and writes pages for you."),
                    ),
            );
        }

        for (i, item) in self.items.iter().enumerate() {
            let el = match item.clone() {
                Item::User(t) => div()
                    .w_full()
                    .flex()
                    .justify_end()
                    .when(i > 0, |d| d.mt(px(18.)))
                    .mb(px(10.))
                    .child(
                        div()
                            .max_w(px(460.))
                            .px(px(14.))
                            .py(px(9.))
                            .rounded(px(14.))
                            .bg(if p.is_dark { p.raised } else { p.ink(0.055) })
                            .text_size(px(13.5))
                            .text_color(p.text)
                            .child(t),
                    ),
                Item::Step { icon, text, link } => step_row(icon, text, link, &p),
                Item::Page { host, title } => page_card(&host, &title, &p),
                Item::Text(t) => div()
                    .py(px(8.))
                    .text_size(px(13.5))
                    .line_height(px(21.))
                    .text_color(p.text)
                    .child(t),
                Item::Spawn(children) => {
                    let mut col = div().flex().flex_col().child(step_row(
                        icons::SPLIT_COLUMNS,
                        "Spawning subagents in parallel".into(),
                        None,
                        &p,
                    ));
                    for (icon, task, model) in children {
                        col = col.child(
                            div()
                                .ml(px(6.))
                                .pl(px(17.))
                                .border_l_1()
                                .border_color(p.hairline(0.10))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .py(px(4.))
                                .text_size(px(13.))
                                .child(glyph(icons::BOT, 13., p.faint))
                                .child(div().text_color(p.faint).child("Spawned"))
                                .child(glyph(icon, 13., p.text))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(p.text)
                                        .child(task),
                                )
                                .child(model_pill(model, &p)),
                        );
                    }
                    col
                }
            };
            transcript = transcript.child(el.flex_none());
        }

        if let Some(t) = self.started {
            transcript = transcript.child(
                div()
                    .mt(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .text_color(p.faint)
                    .child(div().size(px(7.)).rounded_full().bg(p.accent))
                    .child(format!("Working for {}s", t.elapsed().as_secs())),
            );
        }

        let running = self.running;
        let store_t = self.store.clone();
        let prompt = self
            .items
            .iter()
            .find_map(|i| match i {
                Item::User(t) => Some(t.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let harness_t = harness.clone();

        let header = div()
            .flex_none()
            .h(px(40.))
            .px(px(18.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(glyph(icons::PEN_NEW_SQUARE, 14., p.muted))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(p.text)
                    .child(title),
            )
            .child(glyph(icons::ELLIPSIS, 14., p.faint))
            .child(div().flex_1())
            .child(
                div()
                    .id("chat-run-terminal")
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .h(px(24.))
                    .px(px(8.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .hover(|s| s.bg(p.glass_hover()).text_color(p.text))
                    .child(glyph(icons::TERMINAL, 13., p.muted))
                    .child("Run in terminal")
                    .on_click(move |_, _, cx| {
                        store_t.update(cx, |s, cx| s.open_harness(&harness_t, &prompt, cx));
                    }),
            );

        let send_btn = div()
            .id("chat-send")
            .flex_none()
            .size(px(30.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(if p.is_dark { p.text } else { gpui::black() })
            .child(glyph(
                if running {
                    icons::STOP
                } else {
                    icons::ARROW_UP
                },
                14.,
                if p.is_dark {
                    gpui::black()
                } else {
                    gpui::white()
                },
            ))
            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                if this.running {
                    this.stop(cx);
                } else {
                    let text = this.input.read(cx).value().trim().to_string();
                    if !text.is_empty() {
                        this.input.update(cx, |i, cx| i.reset(cx));
                        this.send(text, cx);
                    }
                }
            }));

        let pill = |id: &'static str, icon: &'static str, label: &'static str, chev: bool| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(5.))
                .h(px(22.))
                .px(px(6.))
                .rounded(px(6.))
                .text_size(px(11.5))
                .text_color(p.muted)
                .cursor_pointer()
                .hover(|s| s.bg(p.glass_hover()).text_color(p.text))
                .child(glyph(icon, 12., p.muted))
                .child(label)
                .when(chev, |d| d.child(glyph(icons::ALT_ARROW_DOWN, 9., p.faint)))
        };

        let composer = div()
            .w_full()
            .max_w(px(700.))
            .px(px(20.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .h(px(46.))
                    .pl(px(8.))
                    .pr(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(23.))
                    .bg(if p.is_dark {
                        p.dialog.opacity(0.95)
                    } else {
                        gpui::white()
                    })
                    .border_1()
                    .border_color(p.hairline(0.08))
                    .shadow_md()
                    .child(
                        div()
                            .size(px(28.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(p.glass_hover()))
                            .child(glyph(icons::PLUS, 15., p.muted)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(40.))
                            .overflow_hidden()
                            .text_size(px(13.5))
                            .text_color(p.text)
                            .child(self.input.clone()),
                    )
                    .child(send_btn),
            )
            .child(
                div()
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(pill("chat-local", icons::LAPTOP, "Local", true))
                    .child(pill("chat-guard", icons::SHIELD, "Guard", true))
                    .child(div().flex_1())
                    .child(pill("chat-model", h_icon, h_label, false))
                    .child(pill("chat-effort", icons::GAUGE, "High", true)),
            );

        let focus = self.input.read(cx).focus_handle(cx);
        div()
            .id(SharedString::from(format!("agent-chat-{}", self.tab_id)))
            .size_full()
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.focus(&focus, cx);
            })
            .child(header)
            .child(
                div()
                    .id("chat-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(transcript.flex_none()),
            )
            .child(div().flex_none().flex().justify_center().child(composer))
    }
}
