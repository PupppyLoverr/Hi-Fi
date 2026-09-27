//! Agent chat surface: a transcript of the task (user turns, tool steps,
//! streamed replies) over a reply composer, backed by a real CLI harness via
//! [`crate::agent_runner`]. The transcript persists per chat under
//! `chats/<tab>.json` so it survives restarts.

use gpui::{
    Context, Entity, Focusable, FontWeight, MouseButton, PathPromptOptions, ScrollHandle,
    SharedString, StyledText, Window, div, prelude::*, px,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::StreamExt;

use crate::agent_runner::{self, AgentEvent, HARNESSES, RunConfig};
use crate::assets::icons;
use crate::store::Store;
use crate::text_input::{TextField, TextFieldEvent};
use crate::theme::{Palette, Theme};
use crate::views::glyph;
use hifi_core::Guard;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
enum Item {
    User {
        text: String,
    },
    Step {
        id: String,
        tool: String,
        title: String,
        #[serde(default)]
        output: String,
        #[serde(default)]
        done: bool,
        #[serde(default)]
        error: Option<String>,
    },
    Text {
        id: String,
        text: String,
    },
    Note {
        text: String,
    },
}

pub struct AgentChatView {
    store: Entity<Store>,
    tab_id: String,
    input: Entity<TextField>,
    items: Vec<Item>,
    /// Bumped on every run/stop so a superseded reader stops applying events.
    run: u64,
    active: Option<agent_runner::Run>,
    started: Option<Instant>,
    scroll: ScrollHandle,
    picker_open: bool,
    /// Steps whose output is expanded.
    expanded: Vec<String>,
}

impl AgentChatView {
    pub fn new(
        store: Entity<Store>,
        tab_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings_id = tab_id.clone();
        store.update(cx, |s, cx| s.hydrate_agent_settings(&settings_id, cx));
        let input = cx.new(|cx| TextField::new("Reply, @ for context", cx));
        cx.subscribe(&input, |me: &mut AgentChatView, input, event, cx| {
            let TextFieldEvent::Submitted(text) = event else {
                return;
            };
            let text = text.trim().to_string();
            if text.is_empty() || me.active.is_some() {
                return;
            }
            input.update(cx, |i, cx| i.reset(cx));
            me.send(text, cx);
        })
        .detach();
        window.focus(&input.read(cx).focus_handle(cx), cx);
        let (prompt, live) = {
            let s = store.read(cx);
            (
                s.state
                    .tab(&tab_id)
                    .map(|t| t.prompt.clone())
                    .unwrap_or_default(),
                s.live_agents.contains(&tab_id),
            )
        };
        let mut me = Self {
            store,
            tab_id,
            input,
            items: Vec::new(),
            run: 0,
            active: None,
            started: None,
            scroll: ScrollHandle::new(),
            picker_open: false,
            expanded: Vec::new(),
        };
        me.items = me.load(cx);
        let has_turns = me.items.iter().any(|i| matches!(i, Item::User { .. }));
        if live && !has_turns && !prompt.trim().is_empty() {
            me.send(prompt, cx);
        }
        me
    }

    fn chat_file(&self, cx: &Context<Self>) -> std::path::PathBuf {
        self.store
            .read(cx)
            .paths
            .chats_dir()
            .join(format!("{}.json", self.tab_id))
    }

    fn load(&self, cx: &Context<Self>) -> Vec<Item> {
        std::fs::read_to_string(self.chat_file(cx))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn persist(&self, cx: &Context<Self>) {
        let path = self.chat_file(cx);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string(&self.items) {
            let _ = std::fs::write(path, s);
        }
    }

    fn tab(&self, cx: &Context<Self>) -> Option<hifi_core::Tab> {
        self.store.read(cx).state.tab(&self.tab_id).cloned()
    }

    fn send(&mut self, text: String, cx: &mut Context<Self>) {
        let first = !self.items.iter().any(|i| matches!(i, Item::User { .. }));
        let tab = self.tab(cx);
        let harness = tab
            .as_ref()
            .map(|t| t.command.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| self.store.read(cx).state.settings.agent_harness.clone());
        if first {
            let id = self.tab_id.clone();
            let t = text.clone();
            self.store
                .update(cx, |s, cx| s.start_agent_in(&id, &harness, &t, cx));
        }
        let tab = self.tab(cx);
        self.items.push(Item::User { text: text.clone() });
        self.picker_open = false;
        let cfg = RunConfig {
            harness,
            model: tab.as_ref().map(|t| t.model.clone()).unwrap_or_default(),
            cwd: tab.as_ref().map(|t| t.cwd.clone()).unwrap_or_default(),
            guard: tab.as_ref().and_then(|t| t.guard).unwrap_or_default(),
            session: tab.as_ref().map(|t| t.session.clone()).unwrap_or_default(),
            prompt: text,
            tab_id: self.tab_id.clone(),
            first_turn: first || tab.as_ref().is_none_or(|t| t.session.is_empty()),
        };
        self.run += 1;
        let run = self.run;
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<AgentEvent>();
        match agent_runner::spawn(cfg, tx) {
            Ok(handle) => {
                self.active = Some(handle);
                self.started = Some(Instant::now());
            }
            Err(e) => {
                self.items.push(Item::Note { text: e });
                self.persist(cx);
                cx.notify();
                return;
            }
        }
        self.persist(cx);
        self.scroll.scroll_to_bottom();
        cx.notify();
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.next().await {
                let alive = this
                    .update(cx, |v: &mut AgentChatView, cx| {
                        if v.run != run {
                            return false;
                        }
                        v.apply(ev, cx);
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    return;
                }
            }
        })
        .detach();
        // Tick the "Working for Ns" label while running.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let go = this
                    .update(cx, |v: &mut AgentChatView, cx| {
                        let go = v.active.is_some() && v.run == run;
                        if go {
                            cx.notify();
                        }
                        go
                    })
                    .unwrap_or(false);
                if !go {
                    return;
                }
            }
        })
        .detach();
    }

    fn apply(&mut self, ev: AgentEvent, cx: &mut Context<Self>) {
        match ev {
            AgentEvent::Session(sid) => {
                let id = self.tab_id.clone();
                self.store
                    .update(cx, |s, cx| s.set_agent_session(&id, &sid, cx));
                return;
            }
            AgentEvent::ToolStart { id, tool, title } => {
                if !self
                    .items
                    .iter()
                    .any(|i| matches!(i, Item::Step { id: sid, .. } if *sid == id))
                {
                    self.items.push(Item::Step {
                        id,
                        tool,
                        title,
                        output: String::new(),
                        done: false,
                        error: None,
                    });
                }
            }
            AgentEvent::ToolDone {
                id,
                tool,
                title,
                output,
                error,
            } => {
                let existing = self
                    .items
                    .iter_mut()
                    .find(|i| matches!(i, Item::Step { id: sid, .. } if *sid == id));
                let output: String = output.chars().take(4000).collect();
                match existing {
                    Some(Item::Step {
                        title: t,
                        output: o,
                        done,
                        error: e,
                        ..
                    }) => {
                        *t = title;
                        *o = output;
                        *done = true;
                        *e = error;
                    }
                    _ => self.items.push(Item::Step {
                        id,
                        tool,
                        title,
                        output,
                        done: true,
                        error,
                    }),
                }
                self.refresh_agent_tabs(cx);
            }
            AgentEvent::Text { id, text } => {
                let existing = self
                    .items
                    .iter_mut()
                    .find(|i| matches!(i, Item::Text { id: tid, .. } if *tid == id));
                match existing {
                    Some(Item::Text { text: t, .. }) => *t = text,
                    _ => self.items.push(Item::Text { id, text }),
                }
            }
            AgentEvent::Done { error } => {
                if self.active.is_none() {
                    return;
                }
                if let Some(e) = error {
                    self.items.push(Item::Note {
                        text: format!("The harness stopped with an error: {}", short(&e, 600)),
                    });
                }
                self.active = None;
                self.started = None;
            }
        }
        self.persist(cx);
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Browser tools mutate the store from the IPC thread; poke it so the tab
    /// strip picks up new agent tabs promptly.
    fn refresh_agent_tabs(&self, cx: &mut Context<Self>) {
        self.store.update(cx, |_, cx| cx.notify());
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.run += 1;
        if let Some(run) = self.active.take() {
            run.stop();
        }
        self.started = None;
        self.items.push(Item::Note {
            text: "Stopped".into(),
        });
        self.persist(cx);
        cx.notify();
    }

    fn pick_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Use folder".into()),
        });
        let store = self.store.clone();
        let id = self.tab_id.clone();
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(p) = paths.first()
            {
                let cwd = p.display().to_string();
                cx.update(|cx| {
                    store.update(cx, |s, cx| s.set_agent_cwd(&id, &cwd, cx));
                });
                let _ = this.update(cx, |_, cx| cx.notify());
            }
        })
        .detach();
    }
}

fn short(text: &str, n: usize) -> String {
    let t: String = text.chars().take(n).collect();
    if text.chars().count() > n {
        format!("{t}…")
    } else {
        t
    }
}

fn markdown_text(text: &str) -> StyledText {
    let parts: Vec<&str> = text.split("**").collect();
    if parts.len() < 3 || parts.len().is_multiple_of(2) {
        return StyledText::new(text.replace("**", ""));
    }
    let mut plain = String::new();
    let mut highlights = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let start = plain.len();
        plain.push_str(part);
        if index % 2 == 1 && start < plain.len() {
            highlights.push((start..plain.len(), FontWeight::BOLD.into()));
        }
    }
    StyledText::new(plain).with_highlights(highlights)
}

fn short_error(text: &str) -> String {
    let mut result = text.lines().take(3).collect::<Vec<_>>().join("\n");
    if result.chars().count() > 300 {
        result = result.chars().take(300).collect();
        result.push('…');
    }
    result
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn display_tool_title(tool: &str, title: &str, output: &str, cwd: &str) -> String {
    let tool = tool.to_ascii_lowercase();
    if tool.contains("browser_click") || tool.contains("browser_type") {
        let verb = if tool.contains("browser_click") {
            "Click"
        } else {
            "Type into"
        };
        if let Ok(result) = serde_json::from_str::<serde_json::Value>(output)
            && let Some(label) = result.get("label").and_then(serde_json::Value::as_str)
        {
            return if verb == "Click" {
                format!("{verb} \"{label}\"")
            } else {
                format!("{verb} <{label}>")
            };
        }
        if let Some(index) = title
            .strip_prefix("[data-hifi=\"")
            .and_then(|target| target.strip_suffix("\"]"))
            .filter(|index| index.chars().all(|c| c.is_ascii_digit()))
        {
            return format!("{verb} element #{index}");
        }
    }
    if !(tool.contains("write") || tool.contains("edit") || tool.contains("patch")) {
        return title.to_string();
    }
    let base = normalize_path(Path::new(cwd));
    let path = Path::new(title);
    let resolved = normalize_path(
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        }
        .as_path(),
    );
    if let Ok(relative) = resolved.strip_prefix(&base) {
        relative.display().to_string()
    } else {
        resolved.display().to_string()
    }
}

fn tool_icon(tool: &str) -> &'static str {
    let t = tool.to_lowercase();
    if t.contains("browser_open") || t.contains("navigate") || t.contains("webfetch") {
        icons::GLOBE
    } else if t.contains("browser_click") || t.contains("browser_type") {
        icons::KEYBOARD
    } else if t.contains("browser") {
        icons::EYE
    } else if t == "bash" {
        icons::TERMINAL
    } else if t == "edit" || t == "write" || t == "patch" {
        icons::PEN_NEW_SQUARE
    } else if t == "read" {
        icons::DOCUMENT
    } else if t == "glob" || t == "grep" || t == "list" {
        icons::MAGNIFER
    } else if t == "todowrite" || t == "todoread" {
        icons::CHECKLIST
    } else if t == "task" {
        icons::BOT
    } else {
        icons::SETTINGS
    }
}

fn folder_label(cwd: &str) -> String {
    if cwd.is_empty() {
        return "Home".into();
    }
    std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string())
}

/// Composer control: icon + label, text-weight only. Hi-Fi's own square-ish
/// pill with a soft wash rather than a bordered capsule.
pub fn chip(
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
        .gap(px(6.))
        .h(px(24.))
        .pl(px(7.))
        .pr(if chevron { px(5.) } else { px(8.) })
        .rounded(px(8.))
        .text_size(px(11.5))
        .text_color(p.muted)
        .cursor_pointer()
        .hover(|s| s.bg(p.wash(0.06)).text_color(p.text))
        .child(glyph(icon, 13., p.text.opacity(0.8)))
        .child(div().min_w(px(0.)).truncate().child(label.into()))
        .when(chevron, |d| {
            d.child(glyph(icons::ALT_ARROW_DOWN, 9., p.faint))
        })
}

/// Model menu shared by the chat composer and the New Tab composer: every
/// harness with its models; picking one sets harness + model together.
pub fn model_menu(
    current_harness: &str,
    current_model: &str,
    max_h: f32,
    p: &Palette,
    on_pick: impl Fn(&'static str, &'static str, &mut gpui::App) + Clone + 'static,
) -> gpui::Stateful<gpui::Div> {
    let p = *p;
    let mut menu = div()
        .id("model-menu")
        .w(px(250.))
        .max_h(px(max_h))
        .overflow_y_scroll()
        .p(px(6.))
        .rounded(px(12.))
        .bg(p.glass_overlay())
        .border_1()
        .border_color(p.hairline(0.10))
        .shadow_lg()
        .flex()
        .flex_col();
    for h in HARNESSES {
        let installed = agent_runner::find_binary(h.command).is_some();
        menu = menu.child(
            div()
                .px(px(8.))
                .pt(px(8.))
                .pb(px(4.))
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(10.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(p.faint)
                .child(glyph(crate::sidebar::mark_icon(h.command), 11., p.faint))
                .child(h.label.to_uppercase())
                .when(!installed, |d| {
                    d.child(
                        div()
                            .ml_auto()
                            .font_weight(gpui::FontWeight::NORMAL)
                            .child("not installed"),
                    )
                }),
        );
        for m in h.models {
            let selected = h.command == current_harness && *m == current_model;
            let on_pick = on_pick.clone();
            let (hc, mm) = (h.command, *m);
            menu = menu.child(
                div()
                    .id(SharedString::from(format!("model-{hc}-{mm}")))
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(7.))
                    .text_size(px(12.5))
                    .text_color(if selected { p.text } else { p.muted })
                    .when(selected, |d| d.bg(p.wash(0.08)))
                    .hover(|s| s.bg(p.wash(0.06)).text_color(p.text))
                    .cursor_pointer()
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .child(agent_runner::model_short(m).to_string()),
                    )
                    .when(m.ends_with("-free") || *m == "opencode/big-pickle", |d| {
                        d.child(
                            div()
                                .px(px(5.))
                                .rounded(px(4.))
                                .bg(p.accent.opacity(0.14))
                                .text_size(px(9.5))
                                .text_color(p.accent)
                                .child("free"),
                        )
                    })
                    .when(selected, |d| d.child(glyph(icons::CHECK, 12., p.accent)))
                    .on_click(move |_, _, cx| on_pick(hc, mm, cx)),
            );
        }
    }
    menu
}

fn step_row(
    icon: &'static str,
    title: String,
    done: bool,
    error: Option<&str>,
    p: &Palette,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(9.))
        .py(px(4.))
        .text_size(px(13.))
        .text_color(if error.is_some() { p.danger } else { p.muted })
        .child(if done {
            glyph(icon, 14., if error.is_some() { p.danger } else { p.faint })
        } else {
            glyph(icon, 14., p.accent)
        })
        .child(div().min_w_0().truncate().child(title))
        .when_some(error, |d, e| {
            d.child(
                div()
                    .min_w_0()
                    .text_size(px(11.5))
                    .line_clamp(3)
                    .text_color(p.danger.opacity(0.8))
                    .child(short_error(e)),
            )
        })
}

impl gpui::Render for AgentChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(cx).palette;
        let tab = self.tab(cx);
        let harness = tab
            .as_ref()
            .map(|t| t.command.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| self.store.read(cx).state.settings.agent_harness.clone());
        let model = tab
            .as_ref()
            .map(|t| t.model.clone())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| self.store.read(cx).state.settings.agent_model.clone());
        let guard = tab
            .as_ref()
            .and_then(|t| t.guard)
            .unwrap_or(self.store.read(cx).state.settings.agent_guard);
        let cwd = tab
            .as_ref()
            .map(|t| t.cwd.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| self.store.read(cx).state.settings.agent_cwd.clone());
        let session = tab.as_ref().map(|t| t.session.clone()).unwrap_or_default();
        let h_icon = crate::sidebar::mark_icon(&harness);
        let title = self
            .items
            .iter()
            .find_map(|i| match i {
                Item::User { text } => Some(short(text, 90)),
                _ => None,
            })
            .unwrap_or_else(|| "New chat".into());

        let mut transcript = div()
            .w_full()
            .max_w(px(720.))
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
                            .child("The agent browses in its own tabs, edits files and runs commands in your folder."),
                    ),
            );
        }

        for (i, item) in self.items.iter().enumerate() {
            let el = match item {
                Item::User { text } => div()
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
                            .rounded_br(px(4.))
                            .bg(p.accent.opacity(if p.is_dark { 0.22 } else { 0.12 }))
                            .text_size(px(13.5))
                            .text_color(p.text)
                            .child(text.clone()),
                    ),
                Item::Step {
                    id,
                    tool,
                    title,
                    output,
                    done,
                    error,
                } => {
                    let expanded = self.expanded.contains(id);
                    let has_output = !output.trim().is_empty();
                    let row = step_row(
                        tool_icon(tool),
                        display_tool_title(tool, title, output, &cwd),
                        *done,
                        error.as_deref(),
                        &p,
                    );
                    let key = id.clone();
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .id(SharedString::from(format!("step-{id}")))
                                .when(has_output, |d| d.cursor_pointer())
                                .child(row)
                                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                                    if let Some(i) = this.expanded.iter().position(|k| *k == key) {
                                        this.expanded.remove(i);
                                    } else {
                                        this.expanded.push(key.clone());
                                    }
                                    cx.notify();
                                })),
                        )
                        .when(expanded && has_output, |d| {
                            d.child(
                                div()
                                    .ml(px(23.))
                                    .mb(px(6.))
                                    .p(px(10.))
                                    .rounded(px(8.))
                                    .bg(p.wash(0.05))
                                    .font_family("Geist Mono")
                                    .text_size(px(11.))
                                    .line_height(px(16.))
                                    .text_color(p.muted)
                                    .child(short(output, 1500)),
                            )
                        })
                }
                Item::Text { text, .. } => div()
                    .py(px(8.))
                    .text_size(px(13.5))
                    .line_height(px(21.))
                    .text_color(p.text)
                    .child(markdown_text(text)),
                Item::Note { text } => div()
                    .my(px(6.))
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(8.))
                    .bg(p.wash(0.05))
                    .border_l_2()
                    .border_color(p.accent.opacity(0.6))
                    .text_size(px(12.5))
                    .text_color(p.muted)
                    .child(text.clone()),
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

        let running = self.active.is_some();
        let store_t = self.store.clone();
        let prompt = self
            .items
            .iter()
            .find_map(|i| match i {
                Item::User { text } => Some(text.clone()),
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
            .child(glyph(h_icon, 14., p.muted))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.5))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(p.text)
                    .child(title),
            )
            .child(div().flex_1())
            .child(
                chip(
                    "chat-run-terminal",
                    icons::TERMINAL,
                    "Open in terminal",
                    false,
                    &p,
                )
                .on_click(move |_, _, cx| {
                    store_t.update(cx, |s, cx| s.open_harness(&harness_t, &prompt, cx));
                }),
            );

        // Tabs the agent opened for itself — visible, one click to show.
        let agent_tabs: Vec<(String, String, bool)> = self
            .store
            .read(cx)
            .agent_tabs(&self.tab_id)
            .iter()
            .map(|t| {
                (
                    t.id.clone(),
                    if t.title.is_empty() {
                        hifi_core::host_of(&t.url)
                    } else {
                        t.title.clone()
                    },
                    t.loading,
                )
            })
            .collect();
        let tab_strip = (!agent_tabs.is_empty()).then(|| {
            let mut strip = div()
                .px(px(6.))
                .pb(px(6.))
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(p.faint)
                        .mr(px(4.))
                        .child(format!("Agent tabs · {}", agent_tabs.len())),
                );
            for (id, title, loading) in agent_tabs {
                let store = self.store.clone();
                let tid = id.clone();
                strip = strip.child(
                    div()
                        .id(SharedString::from(format!("agent-tab-{id}")))
                        .h(px(22.))
                        .px(px(7.))
                        .max_w(px(180.))
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .rounded(px(7.))
                        .bg(p.wash(0.05))
                        .text_size(px(11.))
                        .text_color(p.muted)
                        .cursor_pointer()
                        .hover(|s| s.bg(p.wash(0.09)).text_color(p.text))
                        .child(glyph(
                            icons::GLOBE,
                            11.,
                            if loading { p.accent } else { p.faint },
                        ))
                        .child(div().truncate().child(title))
                        .on_click(move |_, _, cx| {
                            store.update(cx, |s, cx| s.show_agent_tab(&tid, cx));
                        }),
                );
            }
            strip
        });

        let send_btn = div()
            .id("chat-send")
            .flex_none()
            .size(px(30.))
            .rounded(px(10.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(if running { p.danger } else { p.accent })
            .child(glyph(
                if running {
                    icons::STOP
                } else {
                    icons::ARROW_UP
                },
                14.,
                gpui::white(),
            ))
            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                if this.active.is_some() {
                    this.stop(cx);
                } else {
                    let text = this.input.read(cx).value().trim().to_string();
                    if !text.is_empty() {
                        this.input.update(cx, |i, cx| i.reset(cx));
                        this.send(text, cx);
                    }
                }
            }));

        let store_m = self.store.clone();
        let tab_m = self.tab_id.clone();
        let this_m = cx.entity().downgrade();
        let menu = self.picker_open.then(|| {
            div()
                .absolute()
                .bottom(px(30.))
                .right(px(0.))
                .child(model_menu(&harness, &model, 300., &p, move |h, m, cx| {
                    store_m.update(cx, |s, cx| s.set_agent_model(&tab_m, h, m, cx));
                    let _ = this_m.update(cx, |v: &mut AgentChatView, cx| {
                        v.picker_open = false;
                        cx.notify();
                    });
                }))
        });

        let composer = div()
            .w_full()
            .max_w(px(720.))
            .px(px(20.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(tab_strip)
            .child(
                div()
                    .h(px(48.))
                    .pl(px(8.))
                    .pr(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(16.))
                    .bg(p.glass_overlay())
                    .border_1()
                    .border_color(if self.input.read(cx).focus_handle(cx).is_focused(window) {
                        p.accent
                    } else {
                        p.hairline(0.10)
                    })
                    .shadow_md()
                    .child(
                        div()
                            .size(px(28.))
                            .rounded(px(9.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(p.wash(0.06)))
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
                    .relative()
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(
                        chip("chat-local", icons::FOLDER, folder_label(&cwd), true, &p).on_click(
                            cx.listener(|this, _: &gpui::ClickEvent, _, cx| this.pick_folder(cx)),
                        ),
                    )
                    .child(
                        chip(
                            "chat-guard",
                            icons::SHIELD,
                            format!("Guard · {}", guard.label()),
                            true,
                            &p,
                        )
                        .on_click(cx.listener(
                            move |this, _: &gpui::ClickEvent, _, cx| {
                                let i = Guard::ALL.iter().position(|g| *g == guard).unwrap_or(0);
                                let next = Guard::ALL[(i + 1) % Guard::ALL.len()];
                                let id = this.tab_id.clone();
                                this.store
                                    .update(cx, |s, cx| s.set_agent_guard(&id, next, cx));
                                cx.notify();
                            },
                        )),
                    )
                    .when(!session.is_empty(), |d| {
                        d.child(chip(
                            "chat-session",
                            icons::LINK,
                            format!("Session · {}", short(&session, 12)),
                            false,
                            &p,
                        ))
                    })
                    .child(div().flex_1())
                    .child(
                        chip(
                            "chat-model",
                            h_icon,
                            agent_runner::model_short(&model).to_string(),
                            true,
                            &p,
                        )
                        .on_click(cx.listener(
                            |this, _: &gpui::ClickEvent, _, cx| {
                                this.picker_open = !this.picker_open;
                                cx.notify();
                            },
                        )),
                    )
                    .children(menu.map(|m| gpui::deferred(m).with_priority(1))),
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
