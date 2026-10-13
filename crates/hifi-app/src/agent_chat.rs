//! Agent chat surface: a transcript of the task (user turns, tool steps,
//! streamed replies) over a reply composer, backed by a real CLI harness via
//! [`crate::agent_runner`]. The transcript persists per chat under
//! `chats/<tab>.json` so it survives restarts.

use gpui::{
    Animation, AnimationExt, Context, Entity, Focusable, FontWeight, MouseButton,
    PathPromptOptions, ScrollHandle, SharedString, StyledText, Window, div, img, prelude::*, px,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::{Path, PathBuf},
};

use futures::StreamExt;

use crate::agent_runner::{self, AgentEvent, HARNESSES, RunConfig};
use crate::assets::icons;
use crate::store::Store;
use crate::text_input::{TextField, TextFieldEvent};
use crate::theme::{FONT_MONO, Palette, Theme};
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
    expanded: HashSet<String>,
    expanded_groups: HashSet<String>,
    artifacts: HashMap<String, PathBuf>,
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
            expanded: HashSet::new(),
            expanded_groups: HashSet::new(),
            artifacts: HashMap::new(),
        };
        me.items = me.load(cx);
        let has_turns = me.items.iter().any(|i| matches!(i, Item::User { .. }));
        if live && !has_turns && !prompt.trim().is_empty() {
            me.send(prompt, cx);
        } else if me.items.is_empty() && !prompt.trim().is_empty() {
            me.items = vec![
                Item::User { text: prompt },
                Item::Note {
                    text: "Earlier activity from this chat wasn't saved.".into(),
                },
            ];
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
        let settings = self.store.read(cx).state.settings.clone();
        let harness = tab
            .as_ref()
            .map(|t| t.command.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| settings.agent_harness.clone());
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
            composio_api_key: settings.composio_api_key,
            computer_use: settings.computer_use,
            cua_driver_path: Path::new(&settings.cua_driver_path)
                .is_file()
                .then(|| settings.cua_driver_path.clone())
                .or_else(|| {
                    agent_runner::find_binary("cua-driver").map(|path| path.display().to_string())
                })
                .unwrap_or_default(),
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
                let id = self.tab_id.clone();
                self.store
                    .update(cx, |s, cx| s.set_agent_running(&id, true, false, cx));
            }
            Err(e) => {
                let id = self.tab_id.clone();
                self.store
                    .update(cx, |s, cx| s.set_agent_running(&id, false, true, cx));
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
                let failed = error.is_some();
                let id = self.tab_id.clone();
                self.store
                    .update(cx, |s, cx| s.set_agent_running(&id, false, failed, cx));
                if let Some(e) = error {
                    self.items.push(Item::Note {
                        text: format!("Stopped — {e}"),
                    });
                }
                let artifacts = self
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        Item::Text { text, .. } => Some(parse_markdown(text)),
                        _ => None,
                    })
                    .flatten()
                    .filter_map(|block| match block {
                        MarkdownBlock::Artifact { kind, content } => Some((kind, content)),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                for (kind, content) in artifacts {
                    self.open_artifact(&kind, &content, cx);
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
        let id = self.tab_id.clone();
        self.store
            .update(cx, |s, cx| s.set_agent_running(&id, false, false, cx));
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

    fn open_artifact(&mut self, kind: &str, content: &str, cx: &mut Context<Self>) {
        let key = format!("{kind}:{content}");
        let root = self
            .store
            .read(cx)
            .paths
            .root
            .join("artifacts")
            .join(&self.tab_id);
        let path = self.artifacts.get(&key).cloned().unwrap_or_else(|| {
            let _ = std::fs::create_dir_all(&root);
            let mut number = 1;
            while root.join(format!("{number}.html")).exists() {
                number += 1;
            }
            root.join(format!("{number}.html"))
        });
        if let Some(html) = artifact_html(kind, content, Theme::of(cx).palette.is_dark) {
            if std::fs::write(&path, html).is_err() {
                return;
            }
            self.artifacts.insert(key, path.clone());
            let url = format!("hifi://preview/{}", path.display());
            let path_text = path.to_string_lossy().to_string();
            let chat_id = self.tab_id.clone();
            self.store.update(cx, |store, cx| {
                if let Some(existing) = store
                    .state
                    .tabs
                    .iter()
                    .find(|tab| tab.url == path_text)
                    .map(|tab| tab.id.clone())
                {
                    store.focus_tab(&existing, cx);
                } else {
                    let group = store.state.tab(&chat_id).map(|tab| tab.group_id.clone());
                    store.open_tab(
                        &url,
                        group,
                        Some((chat_id, hifi_core::SplitSide::Right)),
                        cx,
                    );
                }
            });
        }
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct InlineStyle {
    range: Range<usize>,
    bold: bool,
    code: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InlineText {
    text: String,
    styles: Vec<InlineStyle>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MarkdownBlock {
    Text(InlineText),
    Bullet(InlineText),
    Heading { level: usize, text: InlineText },
    Code(String),
    Image { alt: String, src: String },
    Artifact { kind: String, content: String },
    Blank,
}

fn push_inline(text: &str, bold: bool, code: bool, inline: &mut InlineText) {
    if text.is_empty() {
        return;
    }
    let start = inline.text.len();
    inline.text.push_str(text);
    let end = inline.text.len();
    if let Some(last) = inline.styles.last_mut()
        && last.range.end == start
        && last.bold == bold
        && last.code == code
    {
        last.range.end = end;
    } else {
        inline.styles.push(InlineStyle {
            range: start..end,
            bold,
            code,
        });
    }
}

fn parse_inline_into(text: &str, bold: bool, inline: &mut InlineText) {
    let mut index = 0;
    while index < text.len() {
        let rest = &text[index..];
        if let Some(after_tick) = rest.strip_prefix('`') {
            if let Some(end) = after_tick.find('`') {
                let end = index + 1 + end;
                push_inline(&text[index + 1..end], bold, true, inline);
                index = end + 1;
            } else {
                push_inline(rest, bold, false, inline);
                break;
            }
        } else if let Some(rest) = rest.strip_prefix("**") {
            if let Some(end) = rest.find("**") {
                let end = index + 2 + end;
                parse_inline_into(&text[index + 2..end], true, inline);
                index = end + 2;
            } else {
                push_inline("**", bold, false, inline);
                index += 2;
            }
        } else {
            let ch = rest.chars().next().expect("index is within the string");
            let end = index + ch.len_utf8();
            push_inline(&text[index..end], bold, false, inline);
            index = end;
        }
    }
}

fn parse_inline(text: &str) -> InlineText {
    let mut inline = InlineText {
        text: String::new(),
        styles: Vec::new(),
    };
    parse_inline_into(text, false, &mut inline);
    inline
}

fn heading_line(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=3).contains(&level) && line.as_bytes().get(level) == Some(&b' ') {
        Some((level, &line[level + 1..]))
    } else {
        None
    }
}

fn parse_markdown(text: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut code_lines = Vec::new();
    let mut math_lines = Vec::new();
    let mut in_code = false;
    let mut in_math = false;
    let mut code_kind = String::new();

    for line in text.lines() {
        if !in_code && line.trim() == "$$" {
            if in_math {
                blocks.push(MarkdownBlock::Artifact {
                    kind: "math".into(),
                    content: math_lines.join("\n"),
                });
                math_lines.clear();
                in_math = false;
            } else {
                in_math = true;
            }
            continue;
        }
        if in_math {
            math_lines.push(line.to_string());
            continue;
        }
        if line.trim_start().starts_with("```") {
            if in_code {
                let code = code_lines.join("\n");
                if matches!(code_kind.as_str(), "mermaid" | "math" | "html" | "hifi-ui") {
                    blocks.push(MarkdownBlock::Artifact {
                        kind: code_kind.clone(),
                        content: code,
                    });
                } else {
                    blocks.push(MarkdownBlock::Code(code));
                }
                code_lines.clear();
                in_code = false;
            } else {
                in_code = true;
                code_kind = line
                    .trim_start()
                    .trim_start_matches('`')
                    .trim()
                    .to_ascii_lowercase();
            }
            continue;
        }
        if in_code {
            code_lines.push(line.to_string());
        } else if line.is_empty() {
            blocks.push(MarkdownBlock::Blank);
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            blocks.push(MarkdownBlock::Bullet(parse_inline(item)));
        } else if let Some((alt, src)) = parse_image_line(line) {
            blocks.push(MarkdownBlock::Image { alt, src });
        } else if let Some(math) = line
            .trim()
            .strip_prefix("$$")
            .and_then(|value| value.strip_suffix("$$"))
        {
            blocks.push(MarkdownBlock::Artifact {
                kind: "math".into(),
                content: math.trim().into(),
            });
        } else if let Some((level, heading)) = heading_line(line) {
            blocks.push(MarkdownBlock::Heading {
                level,
                text: parse_inline(heading),
            });
        } else {
            blocks.push(MarkdownBlock::Text(parse_inline(line)));
        }
    }
    if in_code {
        let code = code_lines.join("\n");
        if matches!(code_kind.as_str(), "mermaid" | "math" | "html" | "hifi-ui") {
            blocks.push(MarkdownBlock::Artifact {
                kind: code_kind,
                content: code,
            });
        } else {
            blocks.push(MarkdownBlock::Code(code));
        }
    }
    if in_math {
        blocks.push(MarkdownBlock::Artifact {
            kind: "math".into(),
            content: math_lines.join("\n"),
        });
    }
    blocks
}

fn parse_image_line(line: &str) -> Option<(String, String)> {
    let inner = line.strip_prefix("![")?;
    let (alt, rest) = inner.split_once("](")?;
    let src = rest.strip_suffix(')')?;
    let absolute = src.starts_with("http://")
        || src.starts_with("https://")
        || Path::new(src).is_absolute()
        || (src.as_bytes().get(1) == Some(&b':')
            && src.as_bytes().first().is_some_and(u8::is_ascii_alphabetic));
    absolute.then(|| (alt.to_string(), src.to_string()))
}

fn step_group_end(items: &[Item], start: usize) -> Option<usize> {
    if !matches!(items.get(start), Some(Item::Step { .. })) {
        return None;
    }
    let end = items[start..]
        .iter()
        .position(|item| !matches!(item, Item::Step { .. }))
        .map_or(items.len(), |offset| start + offset);
    (end - start >= 3).then_some(end)
}

fn looks_like_diff(output: &str) -> bool {
    output
        .lines()
        .any(|line| line.starts_with("@@") || line.starts_with("+++") || line.starts_with("---"))
}

fn diff_summary(output: &str) -> (usize, usize) {
    output.lines().fold((0, 0), |(added, deleted), line| {
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("@@") {
            (added, deleted)
        } else if line.starts_with('+') {
            (added + 1, deleted)
        } else if line.starts_with('-') {
            (added, deleted + 1)
        } else {
            (added, deleted)
        }
    })
}

fn artifact_html(kind: &str, content: &str, dark: bool) -> Option<String> {
    let palette = if dark {
        "color-scheme:dark;--bg:#111;--fg:#eee;--muted:#aaa;--border:#333"
    } else {
        "color-scheme:light;--bg:#fff;--fg:#171717;--muted:#666;--border:#ddd"
    };
    let style = format!(
        "<meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><style>:root{{{palette}}}*{{box-sizing:border-box}}html,body{{margin:0;min-height:100%;background:var(--bg);color:var(--fg);font:15px -apple-system,BlinkMacSystemFont,sans-serif}}body{{padding:32px}}a{{color:inherit}}button,input,textarea{{font:inherit;color:inherit;background:var(--bg);border:1px solid var(--border);border-radius:6px}}pre{{white-space:pre-wrap}}.mermaid{{display:flex;justify-content:center}}</style>"
    );
    let body = match kind {
        "mermaid" => format!(
            "<pre class=\"mermaid\">{}</pre><script>{}</script><script>mermaid.initialize({{startOnLoad:true,theme:'neutral'}})</script>",
            escape_html(content),
            include_str!("../assets/vendor/mermaid-11.4.1.min.js"),
        ),
        "math" => format!(
            "<style>{}</style><div id=\"math\"></div><script>{}</script><script>katex.render({},document.getElementById('math'),{{displayMode:true,throwOnError:false}})</script>",
            include_str!("../assets/vendor/katex-0.16.11.min.css"),
            include_str!("../assets/vendor/katex-0.16.11.min.js"),
            serde_json::to_string(content).ok()?,
        ),
        "html" | "hifi-ui" => content.to_string(),
        _ => return None,
    };
    let bridge = "<script>window.hifi={send:text=>window.ipc.postMessage(JSON.stringify({type:'ui_send',text:String(text)}))}</script>";
    Some(format!(
        "<!doctype html><html><head>{style}</head><body>{body}{bridge}</body></html>"
    ))
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn inline_element(inline: &InlineText) -> StyledText {
    let highlights = inline.styles.iter().map(|style| {
        (
            style.range.clone(),
            if style.bold {
                gpui::HighlightStyle::from(FontWeight::BOLD)
            } else {
                gpui::HighlightStyle::default()
            },
        )
    });
    let mono_ranges = inline
        .styles
        .iter()
        .filter(|style| style.code)
        .map(|style| (style.range.clone(), SharedString::from(FONT_MONO)));
    StyledText::new(inline.text.clone())
        .with_highlights(highlights)
        .with_font_family_overrides(mono_ranges)
}

fn markdown_text(
    text: &str,
    p: Palette,
    mut open_artifact: impl FnMut(String, String) -> gpui::AnyElement,
) -> gpui::AnyElement {
    let mut content = div().w_full().flex().flex_col().gap(px(6.));
    for block in parse_markdown(text) {
        let element = match block {
            MarkdownBlock::Text(inline) => div()
                .text_size(px(13.5))
                .line_height(px(21.))
                .child(inline_element(&inline)),
            MarkdownBlock::Bullet(inline) => div()
                .flex()
                .items_start()
                .gap(px(6.))
                .child(div().w(px(10.)).flex_none().text_color(p.faint).child("•"))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(13.5))
                        .line_height(px(21.))
                        .child(inline_element(&inline)),
                ),
            MarkdownBlock::Heading { level, text } => div()
                .text_size(px(match level {
                    1 => 16.,
                    2 => 14.5,
                    _ => 13.5,
                }))
                .line_height(px(21.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(inline_element(&text)),
            MarkdownBlock::Code(code) => div()
                .w_full()
                .px(px(12.))
                .py(px(8.))
                .rounded(px(8.))
                .bg(p.ink(0.04))
                .font_family(FONT_MONO)
                .text_size(px(12.))
                .line_height(px(18.))
                .child(code),
            MarkdownBlock::Image { alt, src } => div()
                .w_full()
                .max_w(px(720.))
                .max_h(px(320.))
                .overflow_hidden()
                .rounded(px(10.))
                .child(img(src).max_w(px(720.)).max_h(px(320.)).rounded(px(10.)))
                .child(div().text_size(px(10.)).text_color(p.faint).child(alt)),
            MarkdownBlock::Artifact { kind, content } => div().child(open_artifact(kind, content)),
            MarkdownBlock::Blank => div().h(px(4.)).flex_none(),
        };
        content = content.child(element.flex_none());
    }
    content.into_any_element()
}

fn compact_error(error: &str) -> String {
    let mut chars = error.chars();
    let mut visible: String = chars.by_ref().take(180).collect();
    if chars.next().is_some() {
        visible.push('…');
    }
    visible
}

fn error_label(error: &str) -> &'static str {
    if ["rule", "denied", "permission"]
        .iter()
        .any(|needle| error.to_ascii_lowercase().contains(needle))
    {
        "Denied"
    } else {
        "Error"
    }
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
    if tool.contains("web_search") || tool == "search" {
        return "Searched".into();
    }
    if tool.contains("browser_open") || tool.contains("navigate") {
        let url = serde_json::from_str::<serde_json::Value>(output)
            .ok()
            .and_then(|value| {
                value
                    .get("url")
                    .or_else(|| value.get("href"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| title.to_string());
        let host = hifi_core::host_of(&url);
        return if host.is_empty() {
            "Opened page".into()
        } else {
            format!("Opened {host}")
        };
    }
    if tool.contains("browser_click") || tool.contains("browser_type") {
        let verb = if tool.contains("browser_click") {
            "Clicked"
        } else {
            "Typed"
        };
        if let Ok(result) = serde_json::from_str::<serde_json::Value>(output)
            && let Some(label) = result.get("label").and_then(serde_json::Value::as_str)
        {
            return format!("{verb} {label}");
        }
        if let Some(index) = title
            .strip_prefix("[data-hifi=\"")
            .and_then(|target| target.strip_suffix("\"]"))
            .filter(|index| index.chars().all(|c| c.is_ascii_digit()))
        {
            return format!("{verb} element #{index}");
        }
        return verb.into();
    }
    if tool.contains("browser_read")
        || tool.contains("browser_snapshot")
        || tool == "read"
        || tool.contains("snapshot")
    {
        return "Read".into();
    }
    if tool.starts_with("doc_") {
        let file = Path::new(title)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| title.to_string());
        return format!("Created {file}");
    }
    if tool.contains("bash") || tool.contains("shell") || tool.contains("command") {
        return "Ran".into();
    }
    if tool.contains("write") || tool.contains("edit") || tool.contains("patch") {
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
        let file = if let Ok(relative) = resolved.strip_prefix(&base) {
            relative.display().to_string()
        } else {
            resolved.display().to_string()
        };
        return format!("Edited {file}");
    }
    if tool.contains("read") {
        return "Read".into();
    }
    if tool.contains("browser") {
        return "Read".into();
    }
    if title.is_empty() {
        return title.to_string();
    }
    title.to_string()
}

fn tool_icon(tool: &str) -> &'static str {
    let t = tool.to_lowercase();
    if t.contains("web_search") || t == "search" {
        icons::MAGNIFER
    } else if t.contains("browser_open") || t.contains("navigate") {
        icons::GLOBE
    } else if t.contains("browser_click") || t.contains("browser_type") {
        icons::CURSOR_MARK
    } else if t.contains("browser_read") || t.contains("browser_snapshot") || t.contains("read") {
        icons::EYE
    } else if t.contains("bash") || t.contains("shell") || t.contains("command") {
        icons::TERMINAL
    } else if t.starts_with("doc_") {
        icons::DOCUMENT
    } else if t.contains("edit") || t.contains("write") || t.contains("patch") {
        icons::PEN_NEW_SQUARE
    } else if t == "glob" || t == "grep" || t == "list" || t.contains("search") {
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
        let installed = agent_runner::installed(h);
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
                .child(glyph(crate::sidebar::mark_icon(h.id), 11., p.faint))
                .child(h.label.to_uppercase())
                .when(!installed, |d| {
                    d.child(
                        div()
                            .ml_auto()
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_color(p.muted)
                            .child("Not installed"),
                    )
                }),
        );
        for m in h.models {
            let selected = h.id == current_harness && *m == current_model;
            let on_pick = on_pick.clone();
            let (hc, mm) = (h.id, *m);
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
    id: &str,
    icon: &'static str,
    title: String,
    done: bool,
    failed: bool,
    p: &Palette,
) -> gpui::Div {
    let status = if failed {
        glyph(icons::CLOSE, 12., p.faint).into_any_element()
    } else if done {
        glyph(icons::CHECK, 12., p.faint).into_any_element()
    } else {
        glyph(icons::RESTART, 13., p.muted)
            .with_animation(
                SharedString::from(format!("tool-spinner-{id}")),
                Animation::new(Duration::from_millis(900)).repeat(),
                |svg, delta| {
                    svg.with_transformation(gpui::Transformation::rotate(gpui::radians(
                        delta * std::f32::consts::TAU,
                    )))
                },
            )
            .into_any_element()
    };
    div()
        .w_full()
        .h(px(26.))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(glyph(icon, 14., p.muted))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_family(crate::theme::FONT_MONO)
                .text_size(px(11.5))
                .text_color(p.muted)
                .child(title),
        )
        .child(div().ml_auto().flex_none().child(status))
}

impl gpui::Render for AgentChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.active.is_none() {
            let id = self.tab_id.clone();
            let prompt = self
                .store
                .update(cx, |store, _| store.take_agent_prompt(&id));
            if let Some(prompt) = prompt {
                self.send(prompt, cx);
            }
        }
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

        let mut i = 0;
        while i < self.items.len() {
            if let Some(end) = step_group_end(&self.items, i) {
                let first_id = match &self.items[i] {
                    Item::Step { id, .. } => id.clone(),
                    _ => unreachable!(),
                };
                let running = self.items[i..end]
                    .iter()
                    .any(|item| matches!(item, Item::Step { done: false, .. }));
                let last_title = match &self.items[end - 1] {
                    Item::Step {
                        tool,
                        title,
                        output,
                        ..
                    } => display_tool_title(tool, title, output, &cwd),
                    _ => String::new(),
                };
                let expanded = running || self.expanded_groups.contains(&first_id);
                let count = end - i;
                let group_key = first_id.clone();
                let group_row = div()
                    .id(SharedString::from(format!("step-group-{first_id}")))
                    .w_full()
                    .h(px(30.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .hover(|d| d.bg(p.wash(0.04)))
                    .child(glyph(
                        if expanded {
                            icons::ALT_ARROW_DOWN
                        } else {
                            icons::ALT_ARROW_RIGHT
                        },
                        11.,
                        p.faint,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_size(px(11.5))
                            .text_color(p.muted)
                            .truncate()
                            .child(format!("{count} steps · {last_title}")),
                    )
                    .when(running, |d| {
                        d.child(glyph(icons::RESTART, 12., p.muted).with_animation(
                            SharedString::from(format!("step-group-spinner-{first_id}")),
                            Animation::new(Duration::from_millis(900)).repeat(),
                            |svg, delta| {
                                svg.with_transformation(gpui::Transformation::rotate(
                                    gpui::radians(delta * std::f32::consts::TAU),
                                ))
                            },
                        ))
                    })
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                        if !this.expanded_groups.remove(&group_key) {
                            this.expanded_groups.insert(group_key.clone());
                        }
                        cx.notify();
                    }));
                transcript = transcript.child(group_row);
                if !expanded {
                    i = end;
                    continue;
                }
            }
            let item = &self.items[i];
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
                            .rounded(px(12.))
                            .bg(p.ink(0.07))
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
                    let has_error = error.as_ref().is_some_and(|text| !text.trim().is_empty());
                    let row = step_row(
                        id,
                        tool_icon(tool),
                        display_tool_title(tool, title, output, &cwd),
                        *done,
                        error.is_some(),
                        &p,
                    );
                    let key = id.clone();
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .id(SharedString::from(format!("step-{id}")))
                                .when(has_output || has_error, |d| d.cursor_pointer())
                                .child(row)
                                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                                    if !this.expanded.remove(&key) {
                                        this.expanded.insert(key.clone());
                                    }
                                    cx.notify();
                                })),
                        )
                        .when_some(error.as_deref(), |d, error| {
                            let key = id.clone();
                            let text = if expanded {
                                error.to_string()
                            } else {
                                compact_error(error)
                            };
                            d.child(
                                div()
                                    .id(SharedString::from(format!("step-error-{id}")))
                                    .min_w(px(0.))
                                    .ml(px(22.))
                                    .mb(px(6.))
                                    .px(px(10.))
                                    .py(px(8.))
                                    .rounded(px(8.))
                                    .cursor_pointer()
                                    .bg(p.ink(0.04))
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(p.danger)
                                            .child(error_label(error)),
                                    )
                                    .child(
                                        div()
                                            .font_family(FONT_MONO)
                                            .text_size(px(11.5))
                                            .line_height(px(16.))
                                            .text_color(p.muted)
                                            .when(!expanded, |d| d.line_clamp(3).text_ellipsis())
                                            .child(text),
                                    )
                                    .on_click(cx.listener(
                                        move |this, _: &gpui::ClickEvent, _, cx| {
                                            if !this.expanded.remove(&key) {
                                                this.expanded.insert(key.clone());
                                            }
                                            cx.notify();
                                        },
                                    )),
                            )
                        })
                        .when(expanded && has_output, |d| {
                            let output_view = if looks_like_diff(output) {
                                let (added, deleted) = diff_summary(output);
                                let mut diff = div()
                                    .ml(px(23.))
                                    .mb(px(6.))
                                    .p(px(8.))
                                    .rounded(px(8.))
                                    .bg(p.wash(0.04))
                                    .flex()
                                    .flex_col()
                                    .gap(px(1.))
                                    .child(
                                        div()
                                            .px(px(4.))
                                            .pb(px(4.))
                                            .text_size(px(10.5))
                                            .text_color(p.muted)
                                            .child(format!("+{added} −{deleted}")),
                                    );
                                for line in output.lines().take(150) {
                                    let wash = if line.starts_with('+') && !line.starts_with("+++")
                                    {
                                        Some(p.diff_add())
                                    } else if line.starts_with('-') && !line.starts_with("---") {
                                        Some(p.diff_del())
                                    } else {
                                        None
                                    };
                                    diff = diff.child(
                                        div()
                                            .w_full()
                                            .px(px(4.))
                                            .when_some(wash, |d, wash| d.bg(wash))
                                            .font_family(FONT_MONO)
                                            .text_size(px(11.))
                                            .line_height(px(16.))
                                            .text_color(p.muted)
                                            .child(line.to_string()),
                                    );
                                }
                                diff
                            } else {
                                div()
                                    .ml(px(23.))
                                    .mb(px(6.))
                                    .p(px(10.))
                                    .rounded(px(8.))
                                    .bg(p.wash(0.05))
                                    .font_family(FONT_MONO)
                                    .text_size(px(11.))
                                    .line_height(px(16.))
                                    .text_color(p.muted)
                                    .child(short(output, 1500))
                            };
                            d.child(output_view)
                        })
                }
                Item::Text { text, .. } => {
                    let view = cx.entity().downgrade();
                    div().py(px(8.)).text_color(p.text).child(markdown_text(
                        text,
                        p,
                        move |kind, content| {
                            let icon = match kind.as_str() {
                                "mermaid" => icons::GIT_BRANCH,
                                "math" => icons::PI_MARK,
                                _ => icons::DOCUMENT,
                            };
                            let heading = content
                                .lines()
                                .next()
                                .unwrap_or(&kind)
                                .trim()
                                .chars()
                                .take(56)
                                .collect::<String>();
                            let artifact_kind = kind.clone();
                            let artifact_content = content.clone();
                            let view = view.clone();
                            div()
                                .w_full()
                                .p(px(12.))
                                .rounded(px(9.))
                                .border_1()
                                .border_color(p.border)
                                .bg(p.ink(0.04))
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .child(glyph(icon, 15., p.muted))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .text_size(px(12.))
                                        .text_color(p.text)
                                        .truncate()
                                        .child(if heading.is_empty() {
                                            kind.to_uppercase()
                                        } else {
                                            heading
                                        }),
                                )
                                .child(
                                    div()
                                        .px(px(9.))
                                        .py(px(5.))
                                        .rounded(px(6.))
                                        .bg(p.wash(0.07))
                                        .text_size(px(11.))
                                        .text_color(p.muted)
                                        .child("Open"),
                                )
                                .cursor_pointer()
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    let kind = artifact_kind.clone();
                                    let content = artifact_content.clone();
                                    let _ = view.update(cx, |view, cx| {
                                        view.open_artifact(&kind, &content, cx);
                                    });
                                })
                                .into_any_element()
                        },
                    ))
                }
                Item::Note { text } => div()
                    .w_full()
                    .min_w(px(0.))
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
            i += 1;
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
                    .child(div().size(px(7.)).rounded_full().bg(p.text))
                    .child(format!("Working for {}s", t.elapsed().as_secs())),
            );
        }

        let running = self.active.is_some();
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

        let can_send = !self.input.read(cx).value().trim().is_empty();
        let send_btn = div()
            .id("chat-send")
            .flex_none()
            .size(px(30.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .when(can_send || running, |d| d.cursor_pointer())
            .bg(if running {
                p.danger
            } else if can_send {
                p.text
            } else {
                p.ink(0.08)
            })
            .child(glyph(
                if running {
                    icons::STOP
                } else {
                    icons::ARROW_UP
                },
                14.,
                if can_send || running { p.bg } else { p.faint },
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
                        p.border_strong
                    } else {
                        p.hairline(0.22)
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
                    .w_full()
                    .min_w(px(0.))
                    .px(px(4.))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(2.))
                    .child({
                        let session_hint = if session.is_empty() {
                            cwd.clone()
                        } else {
                            format!("{cwd}\nSession: {session}")
                        };
                        chip("chat-local", icons::FOLDER, folder_label(&cwd), true, &p)
                            .max_w(px(150.))
                            .tooltip(move |_, cx| {
                                cx.new(|_| crate::sidebar::SpaceTooltip(session_hint.clone()))
                                    .into()
                            })
                            .on_click(
                                cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                                    this.pick_folder(cx)
                                }),
                            )
                    })
                    .child(
                        chip(
                            "chat-guard",
                            icons::SHIELD,
                            format!("Guard · {}", guard.label()),
                            true,
                            &p,
                        )
                        .max_w(px(130.))
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
                    .child(
                        chip(
                            "chat-model",
                            h_icon,
                            agent_runner::model_short(&model).to_string(),
                            true,
                            &p,
                        )
                        .max_w(px(150.))
                        .ml_auto()
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

#[cfg(test)]
mod tests {
    use super::{
        Item, MarkdownBlock, compact_error, diff_summary, looks_like_diff, parse_image_line,
        parse_inline, parse_markdown, step_group_end,
    };

    fn step(id: &str) -> Item {
        Item::Step {
            id: id.into(),
            tool: "read".into(),
            title: id.into(),
            output: String::new(),
            done: true,
            error: None,
        }
    }

    #[test]
    fn compact_errors_truncate_after_180_characters() {
        let error = "x".repeat(181);

        assert_eq!(compact_error(&error), format!("{}…", "x".repeat(180)));
    }

    #[test]
    fn parses_bullet_lines() {
        let blocks = parse_markdown("- first\n* second");

        assert!(matches!(&blocks[0], MarkdownBlock::Bullet(item) if item.text == "first"));
        assert!(matches!(&blocks[1], MarkdownBlock::Bullet(item) if item.text == "second"));
    }

    #[test]
    fn parses_headings_without_hashes() {
        let blocks = parse_markdown("# Title\n## Subtitle\n### Detail");

        assert!(
            matches!(&blocks[0], MarkdownBlock::Heading { level: 1, text } if text.text == "Title")
        );
        assert!(
            matches!(&blocks[1], MarkdownBlock::Heading { level: 2, text } if text.text == "Subtitle")
        );
        assert!(
            matches!(&blocks[2], MarkdownBlock::Heading { level: 3, text } if text.text == "Detail")
        );
    }

    #[test]
    fn combines_bold_and_inline_code_styles() {
        let inline = parse_inline("**bold and `code`**");

        assert_eq!(inline.text, "bold and code");
        assert!(
            inline
                .styles
                .iter()
                .any(|style| { style.range == (0..9) && style.bold && !style.code })
        );
        assert!(
            inline
                .styles
                .iter()
                .any(|style| { style.range == (9..13) && style.bold && style.code })
        );
    }

    #[test]
    fn leaves_unbalanced_backticks_unchanged() {
        let inline = parse_inline("keep `literal");

        assert_eq!(inline.text, "keep `literal");
        assert!(inline.styles.iter().all(|style| !style.code));
    }

    #[test]
    fn parses_fenced_code_as_one_block() {
        let blocks = parse_markdown("```rust\nlet x = 1;\nprintln!(\"{x}\");\n```");

        assert_eq!(
            blocks,
            vec![MarkdownBlock::Code("let x = 1;\nprintln!(\"{x}\");".into())]
        );
    }

    #[test]
    fn groups_only_three_or_more_consecutive_steps() {
        let two = vec![step("a"), step("b")];
        let three = vec![step("a"), step("b"), step("c")];

        assert_eq!(step_group_end(&two, 0), None);
        assert_eq!(step_group_end(&three, 0), Some(3));
    }

    #[test]
    fn detects_and_summarizes_unified_diffs() {
        let diff = "@@ -1,1 +1,1 @@\n-old\n+new\n context\n";

        assert!(looks_like_diff(diff));
        assert_eq!(diff_summary(diff), (1, 1));
        assert!(!looks_like_diff("plain output"));
    }

    #[test]
    fn parses_only_absolute_or_remote_markdown_images() {
        assert_eq!(
            parse_image_line("![diagram](https://example.com/diagram.png)"),
            Some(("diagram".into(), "https://example.com/diagram.png".into()))
        );
        assert_eq!(
            parse_image_line("![local](/tmp/local.png)"),
            Some(("local".into(), "/tmp/local.png".into()))
        );
        assert_eq!(parse_image_line("![relative](images/diagram.png)"), None);
    }

    #[test]
    fn parses_all_supported_artifact_blocks() {
        let blocks = parse_markdown(
            "```mermaid\ngraph LR\nA-->B\n```\n```math\nx^2\n```\n$$y^2$$\n```html\n<h1>HTML</h1>\n```\n```hifi-ui\n<button>Reply</button>\n```",
        );
        let artifacts = blocks
            .into_iter()
            .filter_map(|block| match block {
                MarkdownBlock::Artifact { kind, content } => Some((kind, content)),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            artifacts,
            vec![
                ("mermaid".into(), "graph LR\nA-->B".into()),
                ("math".into(), "x^2".into()),
                ("math".into(), "y^2".into()),
                ("html".into(), "<h1>HTML</h1>".into()),
                ("hifi-ui".into(), "<button>Reply</button>".into()),
            ]
        );
    }
}
