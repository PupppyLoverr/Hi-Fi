//! `Shell` — the root view: frosted sidebar, split content area, right dock,
//! deferred overlays (command bar, find bar), and the native-pane registry.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};

use futures::StreamExt;
use gpui::{
    AnyElement, App, Bounds, Context, Entity, Focusable, MouseButton, SharedString, Window,
    WindowControlArea, deferred, div, point, prelude::*, px, relative, size,
};

use crate::assets::icons;
use crate::command_bar::CommandBar;
use crate::sidebar::{self, Sidebar};
use crate::store::Store;
use crate::surface_chrome;
use crate::terminal::TerminalPane;
use crate::text_input::{TextField, TextFieldEvent};
use crate::theme::Theme;
use crate::views::{DiffView, NewTabView, PreviewView, SettingsView, glyph};
use crate::webview::{WakeSender, WebEvent, WebEventSender, WebPaneHost};
use hifi_core::{SplitNode, TabId, TabKind};

pub enum Pane {
    Web(Rc<WebPaneHost>),
    Term(Entity<TerminalPane>),
    View(gpui::AnyView),
}

/// Drag marker for the left sidebar resize handle (cosmos's resize idiom:
/// a marker drag on the handle, `on_drag_move` on the row it sits in).
struct SidebarResize;
/// Drag marker for the right dock resize handle.
struct DockResize;
/// Drag marker for a split divider — `path` addresses the Split node
/// (0 = into `first`, 1 = into `second`).
struct SplitDrag {
    path: Vec<u8>,
    horizontal: bool,
}

/// The drag preview — a near-invisible 1px chip; the live resize is the
/// feedback. It must paint something so GPUI's overlay plane captures the
/// pointer while dragging over native webviews.
struct DragGhost;
impl gpui::Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(px(1.)).bg(gpui::black().opacity(0.02))
    }
}

/// One in-flight IPC request waiting on the UI.
pub struct IpcJob {
    pub request: hifi_core::IpcRequest,
    pub reply: Sender<hifi_core::IpcResponse>,
}

pub struct Shell {
    pub store: Entity<Store>,
    sidebar: Entity<Sidebar>,
    command_bar: Entity<CommandBar>,
    panes: HashMap<TabId, Pane>,
    /// Webview hosts that failed to spawn (error shown instead of a pane).
    pane_errors: HashMap<TabId, String>,
    /// The tab kind each cached pane was built for; a tab whose kind has
    /// since changed (New Tab -> Agent, ...) gets a fresh pane.
    pane_kinds: HashMap<TabId, TabKind>,
    /// Find-bar input, created on first ⌘F.
    find_input: Option<Entity<TextField>>,
    /// WebEvent source; drained each render.
    web_rx: Receiver<WebEvent>,
    web_tx: WebEventSender,
    ipc_rx: Receiver<IpcJob>,
    /// Events drained off the channels by the wake loop, applied on render.
    inbox_web: Vec<WebEvent>,
    inbox_ipc: Vec<IpcJob>,
    wake: WakeSender,
    /// Root focus target so window-level key bindings dispatch when no
    /// input or terminal holds focus.
    focus: gpui::FocusHandle,
    last_window_title: String,
    context_menu: Option<(f32, f32, Vec<crate::menu::MenuItem>)>,
}

impl Shell {
    pub fn new(
        store: Entity<Store>,
        ipc_rx: Receiver<IpcJob>,
        mut wake_rx: futures::channel::mpsc::UnboundedReceiver<()>,
        wake: WakeSender,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let sidebar = cx.new(|cx| Sidebar::new(store.clone(), cx));
        let command_bar = cx.new(|cx| CommandBar::new(store.clone(), cx));
        let find_input = cx.new(|cx| TextField::new("Find in page\u{2026}", cx));
        cx.subscribe(&find_input, |this, input, event, cx| match event {
            TextFieldEvent::Changed | TextFieldEvent::Submitted(_) => {
                let q = input.read(cx).value().to_string();
                this.find_in_page(&q, cx);
            }
            TextFieldEvent::Escaped => {
                this.find_in_page("", cx);
                this.store.update(cx, |s, cx| {
                    s.find_bar_open = false;
                    cx.notify();
                });
            }
        })
        .detach();
        let (web_tx_raw, web_rx) = channel::<WebEvent>();
        let web_tx = WebEventSender::new(web_tx_raw, wake.clone());
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        cx.spawn(async move |this, cx| {
            while wake_rx.next().await.is_some() {
                let alive = this.update(cx, |this, cx| {
                    let before = this.inbox_web.len() + this.inbox_ipc.len();
                    this.inbox_web.extend(this.web_rx.try_iter());
                    this.inbox_ipc.extend(this.ipc_rx.try_iter());
                    let terms: Vec<Entity<TerminalPane>> = this
                        .panes
                        .values()
                        .filter_map(|p| match p {
                            Pane::Term(t) => Some(t.clone()),
                            _ => None,
                        })
                        .collect();
                    for t in terms {
                        t.update(cx, |t, cx| {
                            if t.pump_events(cx) {
                                cx.notify();
                            }
                        });
                    }
                    if this.inbox_web.len() + this.inbox_ipc.len() > before {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        // Restore persisted tabs' titles etc.; panes materialize lazily.
        Self {
            store,
            sidebar,
            command_bar,
            panes: HashMap::new(),
            pane_errors: HashMap::new(),
            pane_kinds: HashMap::new(),
            find_input: Some(find_input),
            web_rx,
            web_tx,
            ipc_rx,
            inbox_web: Vec::new(),
            inbox_ipc: Vec::new(),
            wake,
            focus: cx.focus_handle(),
            last_window_title: String::new(),
            context_menu: None,
        }
    }

    /// Open the command bar as an omnibox. Web and new-tab pages navigate in
    /// place; other surfaces open the result as a new tab.
    fn open_address(&mut self, tab: Option<TabId>, cx: &mut Context<Self>) {
        let (target, prefill) = {
            let s = self.store.read(cx);
            let t = tab
                .or_else(|| s.active_tab_id())
                .and_then(|id| s.state.tab(&id).cloned())
                .filter(|t| matches!(t.kind, TabKind::Web | TabKind::NewTab));
            let prefill = t
                .as_ref()
                .filter(|t| t.kind == TabKind::Web)
                .map(|t| t.url.clone())
                .unwrap_or_default();
            (t.map(|t| t.id), prefill)
        };
        self.store.update(cx, |s, cx| s.open_address(target, cx));
        let input = self.command_bar.read(cx).input.clone();
        input.update(cx, |i, cx| i.set_value_selected(prefill, cx));
    }

    fn open_tab_peek(
        &mut self,
        tab_id: &TabId,
        docked: bool,
        anchor: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        let ids = if docked {
            self.store
                .read(cx)
                .state
                .active_space()
                .and_then(|space| {
                    let gid = space
                        .active_group
                        .clone()
                        .or_else(|| space.groups.first().map(|group| group.id.clone()))?;
                    space
                        .groups
                        .iter()
                        .find(|group| group.id == gid)
                        .map(|group| group.dock.tabs.clone())
                })
                .unwrap_or_else(|| vec![tab_id.clone()])
        } else {
            vec![tab_id.clone()]
        };
        let active = if docked {
            self.store.read(cx).state.active_space().and_then(|space| {
                space
                    .groups
                    .iter()
                    .find_map(|group| group.dock.active.clone())
            })
        } else {
            Some(
                self.store
                    .read(cx)
                    .active_tab_id()
                    .unwrap_or_else(|| tab_id.clone()),
            )
        };
        let store = self.store.clone();
        let shell = cx.entity().downgrade();
        let mut items = Vec::new();
        for id in ids {
            let Some(tab) = self.store.read(cx).state.tab(&id).cloned() else {
                continue;
            };
            let store = store.clone();
            let shell = shell.clone();
            let selected = active.as_deref() == Some(id.as_str());
            items.push(crate::menu::MenuItem {
                id: SharedString::from(format!("peek-{id}")),
                label: tab.display_title().into(),
                icon: Some(sidebar::kind_icon(&tab)),
                action: Rc::new(move |app| {
                    store.update(app, |s, cx| {
                        s.focus_tab(&id, cx);
                        s.dock_focus(&id, cx);
                    });
                    let _ = shell.update(app, |this, _| this.context_menu = None);
                }),
                separator_before: false,
                disabled: false,
                selected,
            });
        }
        self.context_menu = Some((f32::from(anchor.x), f32::from(anchor.y), items));
    }

    /// Run `window.find` on the surface that should be searched: the dock's
    /// active tab when the dock is open, else the group's active tab.
    fn find_in_page(&self, query: &str, cx: &App) {
        let target = {
            let s = self.store.read(cx);
            let docked = s
                .state
                .active_space()
                .and_then(|sp| {
                    let gid = sp
                        .active_group
                        .clone()
                        .or_else(|| sp.groups.first().map(|g| g.id.clone()))?;
                    sp.groups.iter().find(|g| g.id == gid)
                })
                .and_then(|g| {
                    (g.dock.open && g.dock.active.is_some()).then(|| g.dock.active.clone().unwrap())
                });
            docked.or_else(|| s.active_tab_id())
        };
        let Some(id) = target else { return };
        let Some(Pane::Web(h)) = self.panes.get(&id) else {
            return;
        };
        let q = serde_json::to_string(query).unwrap_or_else(|_| "\"\"".into());
        let js = if query.is_empty() {
            "window.getSelection().removeAllRanges()".to_string()
        } else {
            format!("window.find({q}, false, false, true)")
        };
        h.eval(js, |_| {});
    }

    /// Drain webview + IPC events. Called at the top of render — mutations
    /// land on the store which notifies again.
    fn pump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut web = std::mem::take(&mut self.inbox_web);
        web.extend(self.web_rx.try_iter());
        for ev in web {
            match ev {
                WebEvent::Title { tab, title } => {
                    let is_empty_notes_title = self
                        .store
                        .read(cx)
                        .state
                        .tab(&tab)
                        .is_some_and(|tab| tab.kind == TabKind::Notes && title.is_empty());
                    if !is_empty_notes_title {
                        self.store.update(cx, |s, cx| {
                            s.tab_meta_update(&tab, Some(title), None, None, None, None, cx);
                        });
                    }
                }
                WebEvent::Url { tab, url } => {
                    if url.starts_with("hifi://") {
                        self.store.update(cx, |s, cx| {
                            s.open_tab(&url, None, None, cx);
                        });
                    } else {
                        self.store.update(cx, |s, cx| {
                            s.tab_meta_update(&tab, None, Some(url), None, None, None, cx);
                        });
                    }
                }
                WebEvent::Loading { tab, loading } => {
                    self.store.update(cx, |s, cx| {
                        s.tab_meta_update(&tab, None, None, Some(loading), None, None, cx);
                    });
                }
                WebEvent::CanGo { tab, back, forward } => {
                    self.store.update(cx, |s, cx| {
                        s.tab_meta_update(&tab, None, None, None, Some(back), Some(forward), cx);
                    });
                }
                WebEvent::NewTab { url } => {
                    self.store.update(cx, |s, cx| {
                        s.open_tab(&url, None, None, cx);
                    });
                }
                WebEvent::ContextMenu {
                    tab,
                    x,
                    y,
                    href,
                    src,
                    selection,
                } => {
                    let store = self.store.clone();
                    let shell = cx.entity().downgrade();
                    let dismiss: Rc<dyn Fn(&mut App)> = Rc::new(move |app| {
                        let _ = shell.update(app, |this, cx| {
                            this.context_menu = None;
                            cx.notify();
                        });
                    });
                    let action = |id: SharedString,
                                  label: SharedString,
                                  action: Rc<dyn Fn(&mut App)>,
                                  separator_before: bool|
                     -> crate::menu::MenuItem {
                        crate::menu::MenuItem {
                            id,
                            label,
                            icon: None,
                            action,
                            separator_before,
                            disabled: false,
                            selected: false,
                        }
                    };
                    let back_store = store.clone();
                    let back_tab = tab.clone();
                    let back_dismiss = dismiss.clone();
                    let forward_store = store.clone();
                    let forward_tab = tab.clone();
                    let forward_dismiss = dismiss.clone();
                    let reload_store = store.clone();
                    let reload_tab = tab.clone();
                    let reload_dismiss = dismiss.clone();
                    let mut items = vec![
                        action(
                            "ctx-back".into(),
                            "Back".into(),
                            Rc::new(move |app| {
                                back_store.update(app, |s, cx| {
                                    s.pending_back.push(back_tab.clone());
                                    cx.notify();
                                });
                                back_dismiss(app);
                            }),
                            false,
                        ),
                        action(
                            "ctx-forward".into(),
                            "Forward".into(),
                            Rc::new(move |app| {
                                forward_store.update(app, |s, cx| {
                                    s.pending_forward.push(forward_tab.clone());
                                    cx.notify();
                                });
                                forward_dismiss(app);
                            }),
                            false,
                        ),
                        action(
                            "ctx-reload".into(),
                            "Reload".into(),
                            Rc::new(move |app| {
                                reload_store.update(app, |s, cx| {
                                    s.pending_reload.push(reload_tab.clone());
                                    cx.notify();
                                });
                                reload_dismiss(app);
                            }),
                            false,
                        ),
                    ];
                    if !href.is_empty() {
                        let link = href.clone();
                        let link_dismiss = dismiss.clone();
                        items.push(action(
                            "ctx-copy-link".into(),
                            "Copy Link".into(),
                            Rc::new(move |app| {
                                app.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    link.clone(),
                                ));
                                link_dismiss(app);
                            }),
                            true,
                        ));
                        let split_store = store.clone();
                        let split_tab = tab.clone();
                        let split_url = href.clone();
                        let split_dismiss = dismiss.clone();
                        items.push(action(
                            "ctx-open-link".into(),
                            "Open Link in Split Right".into(),
                            Rc::new(move |app| {
                                split_store.update(app, |s, cx| {
                                    s.open_tab(
                                        &split_url,
                                        None,
                                        Some((split_tab.clone(), hifi_core::SplitSide::Right)),
                                        cx,
                                    );
                                });
                                split_dismiss(app);
                            }),
                            false,
                        ));
                    }
                    if !src.is_empty() {
                        let image = src.clone();
                        let image_dismiss = dismiss.clone();
                        items.push(action(
                            "ctx-copy-image".into(),
                            "Copy Image URL".into(),
                            Rc::new(move |app| {
                                app.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    image.clone(),
                                ));
                                image_dismiss(app);
                            }),
                            false,
                        ));
                    }
                    if !selection.is_empty() {
                        let selected = selection.trim().to_string();
                        let label = if selected.chars().count() > 24 {
                            format!(
                                "Copy \"{}…\"",
                                selected.chars().take(24).collect::<String>()
                            )
                        } else {
                            format!("Copy \"{selected}\"")
                        };
                        let selected_dismiss = dismiss.clone();
                        items.push(action(
                            "ctx-copy-selection".into(),
                            label.into(),
                            Rc::new(move |app| {
                                app.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    selected.clone(),
                                ));
                                selected_dismiss(app);
                            }),
                            false,
                        ));
                    }
                    let current_url = store
                        .read(cx)
                        .state
                        .tab(&tab)
                        .map(|current| current.url.clone())
                        .unwrap_or_default();
                    let ask_store = store.clone();
                    let ask_url = current_url.clone();
                    let ask_dismiss = dismiss.clone();
                    items.push(action(
                        "ctx-ask-agent".into(),
                        "Ask the agent about this page".into(),
                        Rc::new(move |app| {
                            ask_store.update(app, |s, cx| {
                                let id = s.open_tab("hifi://agent", None, None, cx);
                                if let Some(agent) = s.state.tab_mut(&id) {
                                    agent.prompt = format!("Summarize {}", ask_url);
                                    agent.title = agent.prompt.clone();
                                }
                                s.save();
                                cx.notify();
                            });
                            ask_dismiss(app);
                        }),
                        true,
                    ));
                    let copy_url_store = current_url.clone();
                    let copy_url_dismiss = dismiss.clone();
                    items.push(action(
                        "ctx-copy-url".into(),
                        "Copy URL".into(),
                        Rc::new(move |app| {
                            app.write_to_clipboard(gpui::ClipboardItem::new_string(
                                copy_url_store.clone(),
                            ));
                            copy_url_dismiss(app);
                        }),
                        true,
                    ));
                    let pin_store = store.clone();
                    let pin_tab = tab.clone();
                    let pin_dismiss = dismiss.clone();
                    let pinned = store
                        .read(cx)
                        .state
                        .tab(&tab)
                        .is_some_and(|current| current.pinned);
                    items.push(action(
                        "ctx-pin".into(),
                        if pinned { "Unpin" } else { "Pin" }.into(),
                        Rc::new(move |app| {
                            pin_store.update(app, |s, cx| s.toggle_pin(&pin_tab, cx));
                            pin_dismiss(app);
                        }),
                        false,
                    ));
                    let spaces = store.read(cx).state.spaces.clone();
                    let current_space = store
                        .read(cx)
                        .state
                        .tab(&tab)
                        .and_then(|current| {
                            store.read(cx).state.spaces.iter().find(|space| {
                                space
                                    .groups
                                    .iter()
                                    .any(|group| group.id == current.group_id)
                            })
                        })
                        .map(|space| space.id.clone());
                    for space in spaces {
                        if current_space.as_deref() == Some(space.id.as_str()) {
                            continue;
                        }
                        let move_store = store.clone();
                        let move_tab = tab.clone();
                        let move_space = space.id.clone();
                        let move_dismiss = dismiss.clone();
                        items.push(action(
                            format!("ctx-move-space-{}", space.id).into(),
                            format!("Move to {}", space.name).into(),
                            Rc::new(move |app| {
                                move_store.update(app, |s, cx| {
                                    s.move_tab_to_space(&move_tab, &move_space, cx)
                                });
                                move_dismiss(app);
                            }),
                            false,
                        ));
                    }
                    let split_right_store = store.clone();
                    let split_right_tab = tab.clone();
                    let split_right_dismiss = dismiss.clone();
                    let split_down_store = store.clone();
                    let split_down_tab = tab.clone();
                    let split_down_dismiss = dismiss.clone();
                    let close_store = store.clone();
                    let close_tab = tab.clone();
                    let close_dismiss = dismiss.clone();
                    items.extend([
                        action(
                            "ctx-split-right".into(),
                            "Split Right".into(),
                            Rc::new(move |app| {
                                split_right_store.update(app, |s, cx| {
                                    s.open_tab(
                                        "hifi://newtab",
                                        None,
                                        Some((
                                            split_right_tab.clone(),
                                            hifi_core::SplitSide::Right,
                                        )),
                                        cx,
                                    );
                                });
                                split_right_dismiss(app);
                            }),
                            true,
                        ),
                        action(
                            "ctx-split-down".into(),
                            "Split Down".into(),
                            Rc::new(move |app| {
                                split_down_store.update(app, |s, cx| {
                                    s.open_tab(
                                        "hifi://newtab",
                                        None,
                                        Some((split_down_tab.clone(), hifi_core::SplitSide::Below)),
                                        cx,
                                    );
                                });
                                split_down_dismiss(app);
                            }),
                            false,
                        ),
                        action(
                            "ctx-close".into(),
                            "Close Pane".into(),
                            Rc::new(move |app| {
                                close_store.update(app, |s, cx| s.close_tab(&close_tab, cx));
                                close_dismiss(app);
                            }),
                            true,
                        ),
                    ]);
                    let origin = window.content_mask().bounds.origin;
                    self.context_menu = Some((
                        f32::from(origin.x) + x as f32,
                        f32::from(origin.y) + y as f32,
                        items,
                    ));
                }
                WebEvent::Keystroke { combo } => {
                    let (key, _tab) = combo.rsplit_once('|').unwrap_or((&combo, ""));
                    if let Ok(ks) = gpui::Keystroke::parse(key) {
                        window.defer(cx, move |window, cx| {
                            window.dispatch_keystroke(ks, cx);
                        });
                    }
                }
                WebEvent::NotesSave { tab, html } => {
                    let page = serde_json::from_str::<crate::notes::PageSave>(&html).ok();
                    self.store.update(cx, |s, cx| match page {
                        Some(page) => {
                            s.notes_save(&tab, &page.title, &page.html);
                            s.set_title(&tab, page.title, cx);
                        }
                        None => s.notes_save(&tab, "", &html),
                    });
                }
                #[cfg(target_os = "linux")]
                WebEvent::Redraw => {}
                #[cfg(target_os = "linux")]
                WebEvent::Clipboard { text } => {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
                WebEvent::Error { tab, message } => {
                    self.store.update(cx, |s, cx| {
                        s.tab_meta_update(
                            &tab,
                            Some(format!("Warning: {message}")),
                            None,
                            Some(false),
                            None,
                            None,
                            cx,
                        );
                    });
                }
            }
        }
        let mut jobs = std::mem::take(&mut self.inbox_ipc);
        jobs.extend(self.ipc_rx.try_iter());
        for job in jobs {
            self.handle_ipc(job, window, cx);
        }

        // Apply queued nav ops to live web hosts.
        let is_dark = Theme::of(cx).palette.is_dark;
        let (navs, reloads, backs, forwards) = self.store.update(cx, |s, _| {
            (
                std::mem::take(&mut s.pending_navs),
                std::mem::take(&mut s.pending_reload),
                std::mem::take(&mut s.pending_back),
                std::mem::take(&mut s.pending_forward),
            )
        });
        for (id, url) in navs {
            match self.panes.get(&id) {
                Some(Pane::Web(h)) => h.load(&url),
                _ => {
                    // View/terminal pane becoming a web tab: replace with a host.
                    if let Some(tab) = self.store.read(cx).state.tab(&id).cloned()
                        && let Some(host) = self.web_host(&tab, is_dark, window, cx)
                    {
                        host.load(&url);
                    }
                }
            }
        }
        for id in reloads {
            if let Some(Pane::Web(h)) = self.panes.get(&id) {
                h.reload();
            }
        }
        for id in backs {
            if let Some(Pane::Web(h)) = self.panes.get(&id) {
                h.back();
            }
        }
        for id in forwards {
            if let Some(Pane::Web(h)) = self.panes.get(&id) {
                h.forward();
            }
        }

        // Prune hosts for closed tabs.
        let live: HashSet<TabId> = self
            .store
            .read(cx)
            .state
            .tabs
            .iter()
            .map(|t| t.id.clone())
            .collect();
        self.panes.retain(|id, pane| {
            let keep = live.contains(id);
            if !keep && let Pane::Web(h) = pane {
                h.hide();
            }
            keep
        });
    }

    fn ipc_eval(&self, req: &hifi_core::IpcRequest) -> Result<String, String> {
        use hifi_core::ipc::methods as m;
        let p = &req.params;
        let get = |k: &str| p.get(k).and_then(|v| v.as_str()).map(String::from);
        let id = get("id").ok_or("id required")?;
        if !self.panes.contains_key(&id) {
            return Err("not a web tab".into());
        }
        let js = match req.method.as_str() {
            m::TAB_EXEC => get("js").unwrap_or_default(),
            m::TAB_SNAPSHOT => crate::jsbridge::SNAPSHOT_JS.to_string(),
            m::TAB_READ => crate::jsbridge::READ_JS.to_string(),
            m::TAB_CLICK => crate::jsbridge::click_js(&get("target").unwrap_or_default()),
            m::TAB_TYPE => crate::jsbridge::type_js(
                &get("target").unwrap_or_default(),
                &get("text").unwrap_or_default(),
                p.get("submit").and_then(|v| v.as_bool()).unwrap_or(false),
            ),
            m::TAB_SCROLL => {
                crate::jsbridge::scroll_js(p.get("dy").and_then(|v| v.as_f64()).unwrap_or(600.0))
            }
            _ => return Err("bad method".into()),
        };
        Ok(js)
    }

    /// Fire an eval-type request without blocking the main thread — the
    /// WKWebView completion handler runs on the main queue, so a blocking
    /// wait here deadlocks it. The reply goes out in the eval callback.
    fn ipc_eval_async(&self, req: &hifi_core::IpcRequest, reply: Sender<hifi_core::IpcResponse>) {
        let id = req.id.clone();
        match self.ipc_eval(req) {
            Ok(js) => {
                let Some(tab_id) = req
                    .params
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                else {
                    let _ = reply.send(hifi_core::IpcResponse::err(id, "id required"));
                    return;
                };
                let Some(Pane::Web(h)) = self.panes.get(&tab_id) else {
                    let _ = reply.send(hifi_core::IpcResponse::err(id, "not a web tab"));
                    return;
                };
                let h = h.clone();
                h.eval(js, move |out| {
                    let _ = reply.send(hifi_core::IpcResponse::ok(
                        id,
                        serde_json::json!(out.unwrap_or_default()),
                    ));
                });
            }
            Err(e) => {
                let _ = reply.send(hifi_core::IpcResponse::err(id, e));
            }
        }
    }

    fn web_host(
        &mut self,
        tab: &hifi_core::Tab,
        is_dark: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Rc<WebPaneHost>> {
        match self.panes.entry(tab.id.clone()) {
            std::collections::hash_map::Entry::Occupied(e) => {
                if let Pane::Web(h) = e.get() {
                    return Some(h.clone());
                }
                // Pane kind changed under the id (e.g. view → web): rebuild.
                e.remove();
            }
            std::collections::hash_map::Entry::Vacant(_) => {}
        }
        let ipc = matches!(tab.kind, TabKind::Web | TabKind::Notes);
        // hifi:// urls never reach the network — the nav delegate would
        // cancel them into WebEvent::Url and open a duplicate tab.
        let load_url = if tab.url.starts_with("hifi://") {
            ""
        } else {
            &tab.url
        };
        match WebPaneHost::new(
            window,
            cx,
            tab.id.clone(),
            load_url,
            self.web_tx.clone(),
            ipc,
        ) {
            Ok(host) => {
                let host = Rc::new(host);
                if tab.kind == TabKind::Notes {
                    let body = self.store.read(cx).notes_body(&tab.id);
                    let saved_title = self.store.read(cx).notes_title(&tab.id);
                    let title = if tab.title.is_empty() || tab.title == "Notes" {
                        saved_title.clone().unwrap_or_else(|| tab.title.clone())
                    } else {
                        tab.title.clone()
                    };
                    if let Some(saved_title) = saved_title
                        && (tab.title.is_empty() || tab.title == "Notes")
                    {
                        let id = tab.id.clone();
                        self.store
                            .update(cx, |s, cx| s.set_title(&id, saved_title, cx));
                    }
                    host.load_html(&crate::notes::editor_html(&body, &title, is_dark));
                }
                self.panes.insert(tab.id.clone(), Pane::Web(host.clone()));
                Some(host)
            }
            Err(e) => {
                self.pane_errors.insert(tab.id.clone(), e);
                None
            }
        }
    }

    fn handle_ipc(&mut self, job: IpcJob, _window: &mut Window, cx: &mut Context<Self>) {
        use hifi_core::ipc::methods as m;
        use serde_json::json;
        let p = &job.request.params;
        let get = |k: &str| p.get(k).and_then(|v| v.as_str()).map(String::from);
        if let Some(agent) = job.request.agent.as_deref() {
            let tab_owner = get("id").and_then(|id| {
                self.store
                    .read(cx)
                    .state
                    .tab(&id)
                    .and_then(|tab| tab.agent_of.as_deref())
            });
            if let Err(error) =
                hifi_core::ipc::agent_may(&job.request.method, Some(agent), tab_owner)
            {
                return self.finish_ipc(job, Err(error.into()));
            }
        }
        let result: Result<serde_json::Value, String> = match job.request.method.as_str() {
            m::PING => Ok(json!({"ok": true, "name": "Hi-Fi"})),
            m::TAB_OPEN => {
                let url = get("url").unwrap_or_else(|| "hifi://newtab".into());
                let group = get("group");
                let kind = get("kind");
                let command = get("command");
                let project_path = get("projectPath");
                // "40%" → 0.40, "0.4" → 0.40 ��� the new leaf's share.
                let size = get("size").and_then(|s| {
                    let s = s.trim().trim_end_matches('%');
                    s.parse::<f32>()
                        .ok()
                        .map(|v| if v > 1.0 { v / 100.0 } else { v })
                });
                let anchor = [
                    ("rightOf", hifi_core::SplitSide::Right),
                    ("leftOf", hifi_core::SplitSide::Left),
                    ("above", hifi_core::SplitSide::Above),
                    ("below", hifi_core::SplitSide::Below),
                ]
                .iter()
                .find_map(|(k, side)| get(k).map(|a| (a, *side)));
                let target_url = if let Some(k) = &kind {
                    format!("hifi://{k}")
                } else {
                    url
                };
                if !hifi_core::schemes::allowed_navigation(&target_url) {
                    return self.finish_ipc(job, Err("navigation scheme is not allowed".into()));
                }
                if let Some(owner) = job.request.agent.clone().or_else(|| get("agentOf")) {
                    let id = self
                        .store
                        .update(cx, |s, cx| s.open_agent_tab(&target_url, &owner, cx));
                    self.ensure_headless_host(&id, _window, cx);
                    return self.finish_ipc(job, Ok(json!({"id": id})));
                }
                Ok(self.store.update(cx, |s, cx| {
                    let id = s.open_tab_sized(&target_url, group, anchor, size, cx);
                    if let Some(t) = s.state.tab_mut(&id) {
                        if let Some(c) = &command {
                            t.command = c.clone();
                        }
                        if let Some(pp) = &project_path {
                            t.cwd = pp.clone();
                        }
                    }
                    json!({"id": id})
                }))
            }
            m::DOCK_OPEN => {
                let url = get("url").unwrap_or_else(|| "hifi://terminal".into());
                let id = self.store.update(cx, |s, cx| s.dock_open(&url, cx));
                Ok(json!({"id": id}))
            }
            m::DOCK_TAB => {
                if let Some(id) = get("id") {
                    self.store.update(cx, |s, cx| s.dock_tab(&id, cx));
                    Ok(json!({"docked": id}))
                } else {
                    Err("id required".into())
                }
            }
            m::DOCK_UNDOCK => {
                if let Some(id) = get("id") {
                    self.store.update(cx, |s, cx| s.undock_tab(&id, cx));
                    Ok(json!({"undocked": id}))
                } else {
                    Err("id required".into())
                }
            }
            m::DOCK_TOGGLE => {
                self.store.update(cx, |s, cx| s.toggle_dock(cx));
                Ok(json!({"ok": true}))
            }
            m::DOCK_LIST => {
                let dock: serde_json::Value = self
                    .store
                    .read(cx)
                    .state
                    .active_space()
                    .and_then(|s| {
                        let gid = s
                            .active_group
                            .clone()
                            .or_else(|| s.groups.first().map(|g| g.id.clone()))?;
                        s.groups.iter().find(|g| g.id == gid)
                    })
                    .map(|g| {
                        json!({"open": g.dock.open, "width": g.dock.width,
                               "active": g.dock.active, "tabs": g.dock.tabs})
                    })
                    .unwrap_or_else(|| json!({"open": false, "tabs": []}));
                Ok(dock)
            }
            m::TAB_CLOSE => {
                if let Some(id) = get("id") {
                    self.panes.remove(&id);
                    self.store.update(cx, |s, cx| s.close_tab(&id, cx));
                    Ok(json!({"closed": id}))
                } else {
                    Err("id required".into())
                }
            }
            m::TAB_LIST => {
                let tabs: Vec<_> = self
                    .store
                    .read(cx)
                    .state
                    .tabs
                    .iter()
                    .filter(|t| {
                        job.request.agent.as_deref().is_none_or(|agent| {
                            t.agent_of.as_deref() == Some(agent)
                        })
                    })
                    .map(|t| {
                        json!({"id": t.id, "kind": t.kind.as_str(), "title": t.title, "url": t.url, "pinned": t.pinned, "groupId": t.group_id, "agentOf": t.agent_of, "loading": t.loading})
                    })
                    .collect();
                Ok(json!(tabs))
            }
            m::TAB_FOCUS => {
                if let Some(id) = get("id") {
                    self.store.update(cx, |s, cx| s.focus_tab(&id, cx));
                    Ok(json!({"focused": id}))
                } else {
                    Err("id required".into())
                }
            }
            m::TAB_NAVIGATE => match (get("id"), get("url")) {
                (Some(id), Some(url)) => {
                    if !hifi_core::schemes::allowed_navigation(&url) {
                        return self
                            .finish_ipc(job, Err("navigation scheme is not allowed".into()));
                    }
                    self.ensure_headless_host(&id, _window, cx);
                    if let Some(Pane::Web(h)) = self.panes.get(&id) {
                        h.load(&url);
                    }
                    self.store.update(cx, |s, cx| s.navigate(&id, &url, cx));
                    Ok(json!({"navigated": id}))
                }
                _ => Err("id+url required".into()),
            },
            m::TAB_RELOAD => {
                if let Some(id) = get("id")
                    && let Some(Pane::Web(h)) = self.panes.get(&id)
                {
                    h.reload();
                }
                Ok(json!({"ok": true}))
            }
            m::TAB_BACK | m::TAB_FORWARD => {
                if let Some(id) = get("id")
                    && let Some(Pane::Web(h)) = self.panes.get(&id)
                {
                    if job.request.method == m::TAB_BACK {
                        h.back();
                    } else {
                        h.forward();
                    }
                }
                Ok(json!({"ok": true}))
            }
            m::TAB_PIN => {
                if let Some(id) = get("id") {
                    self.store.update(cx, |s, cx| s.toggle_pin(&id, cx));
                }
                Ok(json!({"ok": true}))
            }
            m::TAB_EXEC
            | m::TAB_SNAPSHOT
            | m::TAB_CLICK
            | m::TAB_TYPE
            | m::TAB_READ
            | m::TAB_SCROLL => {
                if let Some(id) = get("id") {
                    self.ensure_headless_host(&id, _window, cx);
                }
                self.ipc_eval_async(&job.request, job.reply.clone());
                return;
            }
            m::TAB_SCREENSHOT => {
                let id = get("id");
                if let Some(id) = &id {
                    self.ensure_headless_host(id, _window, cx);
                }
                let host = id
                    .as_ref()
                    .and_then(|id| self.panes.get(id))
                    .map(|p| (id.clone().unwrap(), p));
                match host {
                    Some((id, Pane::Web(h))) => {
                        let path =
                            format!("{}/screenshot-{}.png", std::env::temp_dir().display(), id);
                        h.screenshot(path.clone());
                        Ok(json!({"path": path}))
                    }
                    _ => Err("id required (web tab)".into()),
                }
            }
            m::GROUP_LIST => {
                let groups: Vec<_> = self
                    .store
                    .read(cx)
                    .state
                    .spaces
                    .iter()
                    .flat_map(|s| s.groups.iter())
                    .map(|g| json!({"id": g.id, "name": g.name, "tabs": g.tab_ids()}))
                    .collect();
                Ok(json!(groups))
            }
            m::GROUP_CREATE => {
                let name = get("name").unwrap_or_else(|| "Group".into());
                let gid = self
                    .store
                    .update(cx, |s, cx| s.create_group(&name, get("projectPath"), cx));
                Ok(json!({"id": gid}))
            }
            m::SPACE_LIST => {
                let spaces: Vec<_> = self
                    .store
                    .read(cx)
                    .state
                    .spaces
                    .iter()
                    .map(|s| json!({"id": s.id, "name": s.name}))
                    .collect();
                Ok(json!(spaces))
            }
            m::SPACE_CREATE => {
                let name = get("name").unwrap_or_else(|| "Space".into());
                self.store.update(cx, |s, cx| s.create_space(&name, cx));
                Ok(json!({"ok": true}))
            }
            m::SPACE_SWITCH => {
                if let Some(id) = get("id") {
                    self.store.update(cx, |s, cx| s.switch_space(&id, cx));
                }
                Ok(json!({"ok": true}))
            }
            other => Err(format!("unknown method {other}")),
        };
        self.finish_ipc(job, result);
    }

    fn finish_ipc(&self, job: IpcJob, result: Result<serde_json::Value, String>) {
        let id = job.request.id.clone();
        let response = match result {
            Ok(v) => hifi_core::IpcResponse::ok(id, v),
            Err(e) => hifi_core::IpcResponse::err(id, e),
        };
        let _ = job.reply.send(response);
    }

    /// Agent-owned web tabs live outside the split tree, so nothing renders
    /// them. Give such a tab a hidden, viewport-sized page host so the agent's
    /// snapshot/click/type/read tools work without it ever appearing on
    /// screen.
    fn ensure_headless_host(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.panes.contains_key(id) {
            return;
        }
        let Some(tab) = self.store.read(cx).state.tab(id).cloned() else {
            return;
        };
        if tab.kind != TabKind::Web {
            return;
        }
        let is_dark = Theme::of(cx).palette.is_dark;
        if let Some(host) = self.web_host(&tab, is_dark, window, cx) {
            host.sync_bounds(
                Bounds::new(point(px(0.), px(0.)), size(px(1280.), px(800.))),
                false,
            );
        }
    }

    // ----- layout -----

    /// Route a keystroke to the focused offscreen page unless the app keymap
    /// owns it. No-op where pages take native keyboard focus.
    fn forward_key(
        &mut self,
        stroke: &gpui::Keystroke,
        down: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(target_os = "linux")]
        if self.focus.is_focused(window)
            && let Some(page) = self.focused_page()
        {
            let combo = stroke.unparse();
            if down && crate::webview::is_browser_chord(&combo) {
                page.release_focus();
                return;
            }
            page.key(stroke, down, cx);
            cx.stop_propagation();
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (stroke, down, window, cx);
    }

    /// A webview canvas bound to `host` — syncs the native view to the
    /// painted bounds each frame. Shared by leaves and the dock.
    #[cfg(not(target_os = "linux"))]
    fn web_canvas(host: &Rc<WebPaneHost>) -> gpui::Canvas<()> {
        let host = host.clone();
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, _cx| {
                // Native views paint above GPUI, so clip to the content mask.
                let bounds = bounds.intersect(&window.content_mask().bounds);
                let host = Rc::downgrade(&host);
                window.on_present(move || {
                    if let Some(host) = host.upgrade() {
                        host.sync_bounds(bounds, true);
                    }
                });
            },
        )
    }

    /// Linux pages render offscreen: composite the latest frame directly.
    #[cfg(target_os = "linux")]
    fn web_canvas(host: &Rc<WebPaneHost>) -> gpui::Canvas<()> {
        let host = host.clone();
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, _cx| host.paint(bounds, window),
        )
    }

    #[cfg(not(target_os = "linux"))]
    fn web_surface(host: &Rc<WebPaneHost>, _focus: &gpui::FocusHandle) -> gpui::Div {
        div()
            .size_full()
            .relative()
            .child(Self::web_canvas(host).absolute().inset_0())
    }

    /// Offscreen pages receive input as synthesized events from GPUI.
    #[cfg(target_os = "linux")]
    fn web_surface(host: &Rc<WebPaneHost>, focus: &gpui::FocusHandle) -> gpui::Div {
        use gpui::{MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollWheelEvent};
        let mut surface = div()
            .size_full()
            .relative()
            .child(Self::web_canvas(host).absolute().inset_0());
        for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
            let (down, up, focus) = (host.clone(), host.clone(), focus.clone());
            surface = surface
                .on_mouse_down(button, move |e: &MouseDownEvent, w, cx| {
                    w.focus(&focus, cx);
                    down.pointer("down", e.position, Some(e.button), e.modifiers);
                    cx.stop_propagation();
                })
                .on_mouse_up(button, move |e: &MouseUpEvent, _, cx| {
                    up.pointer("up", e.position, Some(e.button), e.modifiers);
                    cx.stop_propagation();
                });
        }
        let (moved, released, scrolled) = (host.clone(), host.clone(), host.clone());
        surface
            .on_mouse_up_out(MouseButton::Left, move |e: &MouseUpEvent, _, _| {
                released.pointer("up", e.position, Some(e.button), e.modifiers);
            })
            .on_mouse_move(move |e: &MouseMoveEvent, _, cx| {
                if !cx.has_active_drag() {
                    moved.pointer("move", e.position, e.pressed_button, e.modifiers);
                }
            })
            .on_scroll_wheel(move |e: &ScrollWheelEvent, _, cx| {
                scrolled.scroll(e);
                cx.stop_propagation();
            })
    }

    /// The page that owns keyboard input (Linux forwards keys itself).
    #[cfg(target_os = "linux")]
    fn focused_page(&self) -> Option<Rc<WebPaneHost>> {
        self.panes.values().find_map(|p| match p {
            Pane::Web(h) if h.is_focused() => Some(h.clone()),
            _ => None,
        })
    }

    /// The kind-specific body of a pane (no header, no card). Shared by the
    /// split leaves and the right dock surface.
    fn pane_body(
        &mut self,
        tab: &hifi_core::Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = Theme::of(cx).palette;
        let web_kind = |k: TabKind| matches!(k, TabKind::Web | TabKind::Notes);
        match self.pane_kinds.insert(tab.id.clone(), tab.kind) {
            Some(old) if old != tab.kind && !(web_kind(old) && web_kind(tab.kind)) => {
                if let Some(Pane::Web(h)) = self.panes.remove(&tab.id) {
                    h.hide();
                }
                self.pane_errors.remove(&tab.id);
            }
            _ => {}
        }
        match tab.kind {
            TabKind::Web | TabKind::Notes => match self.web_host(tab, p.is_dark, window, cx) {
                Some(host) => Self::web_surface(&host, &self.focus).into_any(),
                None => {
                    let err = self
                        .pane_errors
                        .get(&tab.id)
                        .cloned()
                        .unwrap_or_else(|| "webview failed".into());
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(p.danger)
                        .text_size(px(12.))
                        .child(format!("Warning: {err}"))
                        .into_any()
                }
            },
            TabKind::Agent => {
                let store = self.store.clone();
                let view = self.view_pane(
                    tab.id.clone(),
                    move |tab_id, w, cx| {
                        crate::agent_chat::AgentChatView::new(store.clone(), tab_id, w, cx)
                    },
                    window,
                    cx,
                );
                div().size_full().child(view).into_any()
            }
            TabKind::Terminal => {
                let view = match self.panes.entry(tab.id.clone()) {
                    std::collections::hash_map::Entry::Occupied(e) => match e.get() {
                        Pane::Term(v) => Some(v.clone()),
                        _ => None,
                    },
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let cmd = (!tab.command.is_empty()).then(|| tab.command.clone());
                        let cwd = (!tab.cwd.is_empty()).then(|| tab.cwd.clone());
                        let spawned = match (&cmd, tab.prompt.is_empty()) {
                            (Some(c), false) => TerminalPane::spawn_pty_with_arg_and_wake(
                                c,
                                &tab.prompt,
                                cwd.as_deref(),
                                self.wake.clone(),
                            ),
                            _ => TerminalPane::spawn_pty_with_wake(
                                cmd.as_deref(),
                                cwd.as_deref(),
                                self.wake.clone(),
                            ),
                        };
                        match spawned {
                            Ok(parts) => {
                                let v = cx.new(|cx| TerminalPane::from_parts(parts, cx));
                                e.insert(Pane::Term(v.clone()));
                                Some(v)
                            }
                            Err(err) => {
                                self.pane_errors.insert(tab.id.clone(), err.to_string());
                                None
                            }
                        }
                    }
                };
                match view {
                    Some(v) => div().size_full().child(v).into_any(),
                    None => div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(p.danger)
                        .text_size(px(12.))
                        .child(format!(
                            "Warning: {}",
                            self.pane_errors
                                .get(&tab.id)
                                .cloned()
                                .unwrap_or_else(|| "pty failed".into())
                        ))
                        .into_any(),
                }
            }
            TabKind::NewTab => {
                let store = self.store.clone();
                let view = self.view_pane(
                    tab.id.clone(),
                    move |tab_id, w, cx| NewTabView::new(store.clone(), tab_id, w, cx),
                    window,
                    cx,
                );
                div().size_full().child(view).into_any()
            }
            TabKind::Settings => {
                let store = self.store.clone();
                let view = self.view_pane(
                    tab.id.clone(),
                    move |tab_id, w, cx| SettingsView::new(store.clone(), tab_id, w, cx),
                    window,
                    cx,
                );
                div().size_full().child(view).into_any()
            }
            TabKind::Diff => {
                let project = self
                    .store
                    .read(cx)
                    .state
                    .spaces
                    .iter()
                    .flat_map(|s| &s.groups)
                    .find(|g| g.id == tab.group_id)
                    .map(|g| g.project_path.clone())
                    .unwrap_or_default();
                let path = [tab.cwd.clone(), project]
                    .into_iter()
                    .find(|p| !p.is_empty())
                    .or_else(|| {
                        std::env::current_dir()
                            .ok()
                            .map(|d| d.to_string_lossy().into_owned())
                    })
                    .unwrap_or_default();
                let view = self.view_pane(
                    tab.id.clone(),
                    move |tab_id, w, cx| DiffView::new(path, tab_id, w, cx),
                    window,
                    cx,
                );
                div().size_full().child(view).into_any()
            }
            TabKind::Preview => {
                let view = self.view_pane(
                    tab.id.clone(),
                    |tab_id, w, cx| PreviewView::new(tab.url.clone(), tab_id, w, cx),
                    window,
                    cx,
                );
                div().size_full().child(view).into_any()
            }
        }
    }

    fn render_leaf(
        &mut self,
        tab_id: &TabId,
        focused: bool,
        origin: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab) = self.store.read(cx).state.tab(tab_id).cloned() else {
            return div().into_any();
        };
        let p = Theme::of(cx).palette;
        let _ = origin;
        let header = self.pane_header(&tab, false, focused, cx);
        let body = self.pane_body(&tab, window, cx);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w(px(120.))
            .min_h(px(80.))
            .overflow_hidden()
            .bg(p.bg)
            .child(header)
            .child(div().flex_1().min_h(px(0.)).overflow_hidden().child(body))
            .into_any()
    }

    /// Pane chrome: cosmos's browser toolbar (`surface_chrome::toolbar`) —
    /// back · forward · reload, a rounded address field that doubles as the
    /// omnibox, then the pane actions.
    fn pane_header(
        &mut self,
        tab: &hifi_core::Tab,
        docked: bool,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = Theme::of(cx).palette;
        let store = self.store.clone();
        let id = tab.id.clone();

        let mut header = surface_chrome::toolbar(&p)
            .id(SharedString::from(format!("hdr:{}", tab.id)))
            .relative()
            .when(docked, |d| d.border_t_0());

        let nav =
            |name: &'static str, icon: &'static str, enabled: bool, op: fn(&mut Store, TabId)| {
                let store = store.clone();
                let id = id.clone();
                toolbar_button(name, icon, enabled, p, move |cx| {
                    store.update(cx, |s, cx| {
                        op(s, id.clone());
                        cx.notify();
                    })
                })
            };
        header = header
            .child(nav(
                "nav-back",
                icons::ARROW_LEFT,
                tab.kind == TabKind::Web && tab.can_go_back,
                |s, id| s.pending_back.push(id),
            ))
            .child(nav(
                "nav-fwd",
                icons::ARROW_RIGHT,
                tab.kind == TabKind::Web && tab.can_go_forward,
                |s, id| s.pending_forward.push(id),
            ))
            .child(nav(
                "nav-reload",
                if tab.loading {
                    icons::CLOSE
                } else {
                    icons::REFRESH
                },
                tab.kind == TabKind::Web,
                |s, id| s.pending_reload.push(id),
            ));

        let (lead_icon, primary, secondary): (&'static str, String, String) = match tab.kind {
            TabKind::Web => {
                let rest = tab
                    .url
                    .split_once("://")
                    .map_or(tab.url.as_str(), |(_, r)| r);
                let (host, _) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
                let host = host.strip_prefix("www.").unwrap_or(host).to_string();
                let icon = if tab.url.starts_with("https://") {
                    icons::LOCK
                } else {
                    icons::GLOBE
                };
                (
                    icon,
                    tab.short_title(),
                    if tab.short_title() == host {
                        String::new()
                    } else {
                        host
                    },
                )
            }
            TabKind::Terminal => (sidebar::kind_icon(tab), tab.display_title(), String::new()),
            TabKind::Agent => (sidebar::kind_icon(tab), tab.display_title(), String::new()),
            TabKind::Diff => (icons::GIT_BRANCH, tab.display_title(), String::new()),
            TabKind::Preview => (icons::EYE, tab.display_title(), String::new()),
            TabKind::NewTab => (
                icons::MAGNIFER,
                "Search or enter address".into(),
                String::new(),
            ),
            TabKind::Settings => (icons::SETTINGS, "Settings".into(), String::new()),
            TabKind::Notes => (
                icons::DOCUMENT,
                if tab.title.is_empty() || tab.title == "Notes" {
                    "Untitled".into()
                } else {
                    tab.short_title()
                },
                String::new(),
            ),
        };
        let omnibox = matches!(tab.kind, TabKind::Web | TabKind::NewTab);
        let placeholder = tab.kind == TabKind::NewTab;
        let text_c = if placeholder {
            p.faint
        } else if focused {
            p.text
        } else {
            p.muted
        };
        let label = div()
            .flex_1()
            .flex()
            .min_w(px(0.))
            .overflow_hidden()
            .whitespace_nowrap()
            .gap(px(6.))
            .child(
                div()
                    .flex_shrink(1.)
                    .min_w(px(24.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(text_c)
                    .child(primary),
            )
            .when(!secondary.is_empty(), |d| {
                let sep = if omnibox { "" } else { "  ·  " };
                d.child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(11.))
                        .text_color(p.faint)
                        .child(format!("{sep}{secondary}")),
                )
            });
        if omnibox {
            let addr_id = id.clone();
            header = header.child(
                surface_chrome::input(&p)
                    .id("addr")
                    .cursor_text()
                    .when(focused, |s| {
                        s.bg(p.ink(0.08))
                            .border_1()
                            .border_color(p.accent.opacity(0.35))
                    })
                    .hover(|s| s.bg(p.ink(0.06)))
                    .child(glyph(lead_icon, 12., p.faint))
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.open_address(Some(addr_id.clone()), cx);
                        }),
                    ),
            );
        } else {
            header = header.child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h(px(surface_chrome::CONTROL_SIZE))
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(glyph(lead_icon, surface_chrome::ICON_SIZE, p.muted))
                    .child(label),
            );
        }

        let peek_count = if docked {
            self.store
                .read(cx)
                .state
                .active_space()
                .and_then(|space| {
                    let gid = space
                        .active_group
                        .clone()
                        .or_else(|| space.groups.first().map(|g| g.id.clone()))?;
                    space
                        .groups
                        .iter()
                        .find(|group| group.id == gid)
                        .map(|group| group.dock.tabs.len())
                })
                .unwrap_or(1)
        } else {
            1
        };
        header = header.child(
            div()
                .id("tab-peek")
                .size(px(surface_chrome::CONTROL_SIZE))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(surface_chrome::CONTROL_RADIUS))
                .cursor_pointer()
                .hover(|s| s.bg(p.wash(0.14)))
                .child(glyph(icons::WIDGET, surface_chrome::ICON_SIZE, p.muted))
                .child(
                    div()
                        .absolute()
                        .top(px(-2.))
                        .right(px(-2.))
                        .min_w(px(10.))
                        .h(px(10.))
                        .px(px(2.))
                        .rounded_full()
                        .bg(p.accent)
                        .text_size(px(8.))
                        .text_color(gpui::white())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(peek_count.to_string()),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let id = id.clone();
                        move |this, event: &gpui::MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.open_tab_peek(&id, docked, event.position, cx);
                        }
                    }),
                ),
        );

        let split = |name: &'static str, icon: &'static str, side: hifi_core::SplitSide| {
            let store = store.clone();
            let id = id.clone();
            toolbar_button(name, icon, focused, p, move |cx| {
                store.update(cx, |s, cx| {
                    s.open_tab("hifi://newtab", None, Some((id.clone(), side)), cx);
                });
            })
        };
        header = header
            .child(split(
                "split-r",
                icons::SPLIT_COLUMNS,
                hifi_core::SplitSide::Right,
            ))
            .child(split(
                "split-b",
                icons::FOLD_VERTICAL,
                hifi_core::SplitSide::Below,
            ))
            .child({
                let store = store.clone();
                let id = id.clone();
                toolbar_button(
                    "dock-move",
                    if docked {
                        icons::SIDEBAR_LEFT
                    } else {
                        icons::SIDEBAR_RIGHT
                    },
                    focused,
                    p,
                    move |cx| {
                        store.update(cx, |s, cx| {
                            if docked {
                                s.undock_tab(&id, cx)
                            } else {
                                s.dock_tab(&id, cx)
                            }
                        });
                    },
                )
            })
            .child({
                let store = store.clone();
                let id = id.clone();
                toolbar_button(
                    "pin",
                    icons::PIN,
                    focused,
                    if tab.pinned {
                        crate::theme::Palette {
                            muted: p.accent,
                            ..p
                        }
                    } else {
                        p
                    },
                    move |cx| store.update(cx, |s, cx| s.toggle_pin(&id, cx)),
                )
            });
        header = header.child({
            let store = store.clone();
            let id = id.clone();
            toolbar_button("close", icons::CLOSE, true, p, move |cx| {
                store.update(cx, |s, cx| s.close_tab(&id, cx));
            })
        });
        if tab.loading {
            header = header.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(-1.))
                    .h(px(2.))
                    .bg(p.accent.opacity(0.75)),
            );
        }
        header.into_any()
    }

    fn view_pane<T: gpui::Render + 'static>(
        &mut self,
        id: TabId,
        make: impl FnOnce(String, &mut Window, &mut Context<T>) -> T,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyView {
        match self.panes.entry(id.clone()) {
            std::collections::hash_map::Entry::Occupied(e) => match e.get() {
                Pane::View(v) if v.clone().downcast::<T>().is_ok() => return v.clone(),
                // Kind changed under this id — rebuild below.
                _ => {
                    e.remove();
                }
            },
            std::collections::hash_map::Entry::Vacant(e) => {
                let v: gpui::AnyView = cx.new(|cx| make(id.clone(), window, cx)).into();
                e.insert(Pane::View(v.clone()));
                return v;
            }
        }
        let v: gpui::AnyView = cx.new(|cx| make(id.clone(), window, cx)).into();
        self.panes.insert(id, Pane::View(v.clone()));
        v
    }

    /// Render a split node into nested flexes. `path` addresses this node in
    /// the tree (0 = into `first`, 1 = into `second`) for the divider drags.
    fn render_node(
        &mut self,
        node: &SplitNode,
        active: Option<&TabId>,
        path: &[u8],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            SplitNode::Leaf { tabs } => {
                // The region shows the group's active tab when it's in this
                // stack, otherwise the stack's most recent tab.
                let shown = tabs
                    .iter()
                    .find(|t| Some(*t) == active)
                    .or_else(|| tabs.last());
                // Hide webviews stacked behind the shown tab — their clips are
                // native overlays that would otherwise keep painting on top.
                for t in tabs {
                    if Some(t) != shown
                        && let Some(Pane::Web(h)) = self.panes.get(t)
                    {
                        h.hide();
                    }
                }
                let origin = path.iter().all(|b| *b == 0);
                match shown {
                    Some(tab_id) => {
                        self.render_leaf(tab_id, active == Some(tab_id), origin, window, cx)
                    }
                    None => div().size_full().into_any(),
                }
            }
            SplitNode::Split {
                direction,
                fraction,
                first,
                second,
            } => {
                let f = *fraction;
                let horizontal = *direction == hifi_core::SplitDirection::Horizontal;
                let mut p1 = path.to_vec();
                p1.push(0);
                let mut p2 = path.to_vec();
                p2.push(1);
                let first_el = self.render_node(first, active, &p1, window, cx);
                let second_el = self.render_node(second, active, &p2, window, cx);

                // Draggable divider, cosmos's marker + on_drag_move idiom.
                let p = Theme::of(cx).palette;
                let store = self.store.clone();
                let my_path = path.to_vec();
                let handle_id: SharedString = format!(
                    "split:{}",
                    my_path
                        .iter()
                        .map(u8::to_string)
                        .collect::<Vec<_>>()
                        .join(".")
                )
                .into();
                let reset_path = my_path.clone();
                let reset_store = store.clone();
                let handle = div()
                    .id(handle_id)
                    .group("split-handle")
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(horizontal, |d| d.w(px(33.)).h_full().cursor_col_resize())
                    .when(!horizontal, |d| d.h(px(33.)).w_full().cursor_row_resize())
                    .child(
                        div()
                            .bg(p.border)
                            .when(horizontal, |d| d.w(px(2.)).h(px(32.)))
                            .when(!horizontal, |d| d.h(px(2.)).w(px(32.)))
                            .group_hover("split-handle", |s| s.bg(p.accent.opacity(0.5))),
                    )
                    .occlude()
                    .on_click(move |event, _, cx| {
                        if event.click_count() == 2 {
                            reset_store.update(cx, |s, cx| {
                                s.resize_split(&reset_path, 0.5, cx);
                            });
                        }
                    })
                    .on_drag(
                        SplitDrag {
                            path: my_path.clone(),
                            horizontal,
                        },
                        |_, _point, _, cx| {
                            cx.stop_propagation();
                            cx.new(|_| DragGhost)
                        },
                    );

                let mut row = div()
                    .id(SharedString::from(format!(
                        "splitrow:{}",
                        my_path
                            .iter()
                            .map(u8::to_string)
                            .collect::<Vec<_>>()
                            .join(".")
                    )))
                    .flex()
                    .size_full()
                    .min_h(px(0.))
                    .min_w(px(0.));
                row = if horizontal {
                    row.flex_row()
                } else {
                    row.flex_col()
                };
                let row_path = my_path.clone();
                let (min_first, min_second) = (
                    min_extent(first, horizontal),
                    min_extent(second, horizontal),
                );
                row = row.on_drag_move::<SplitDrag>(move |event, window, cx| {
                    if event.event.pressed_button != Some(gpui::MouseButton::Left) {
                        cx.stop_active_drag(window);
                        return;
                    }
                    let (path, horizontal) = {
                        let drag = event.drag(cx);
                        (drag.path.clone(), drag.horizontal)
                    };
                    // Every split row sees the drag; only the dragged one resizes.
                    if path != row_path {
                        return;
                    }
                    let bounds = event.bounds;
                    let pos = event.event.position;
                    let (frac, span) = if horizontal {
                        (
                            f32::from(pos.x - bounds.left()),
                            f32::from(bounds.size.width),
                        )
                    } else {
                        (
                            f32::from(pos.y - bounds.top()),
                            f32::from(bounds.size.height),
                        )
                    };
                    if span > 1.0 {
                        let lo = (min_first / span).min(0.5);
                        let hi = (1. - min_second / span).max(lo);
                        let mut next = (frac / span).clamp(lo, hi);
                        for snap in [0.333, 0.5, 0.667] {
                            if (next - snap).abs() <= 12. / span {
                                next = snap;
                                break;
                            }
                        }
                        store.update(cx, |s, cx| {
                            s.resize_split(&path, next.clamp(lo, hi), cx);
                        });
                    }
                });
                let min_axis = |d: gpui::Div, m: f32| {
                    if horizontal {
                        d.min_w(px(m)).min_h(px(0.))
                    } else {
                        d.min_h(px(m)).min_w(px(0.))
                    }
                };
                row.child(
                    min_axis(div(), min_first)
                        .flex_basis(relative(f))
                        .flex_grow(1.)
                        .overflow_hidden()
                        .flex()
                        .child(first_el),
                )
                .child(handle)
                .child(
                    min_axis(div(), min_second)
                        .flex_basis(relative(1. - f))
                        .flex_grow(1.)
                        .overflow_hidden()
                        .flex()
                        .child(second_el),
                )
                .into_any()
            }
        }
    }

    /// The cosmos right pane: a flush, left-hairlined glass panel. Its
    /// surface tabs ride a `surface_chrome::toolbar`; with nothing open it
    /// shows cosmos's surface picker.
    fn render_dock(
        &mut self,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = Theme::of(cx).palette;
        let active = self.store.read(cx).state.active_space().and_then(|s| {
            let gid = s
                .active_group
                .clone()
                .or_else(|| s.groups.first().map(|g| g.id.clone()))?;
            s.groups
                .iter()
                .find(|g| g.id == gid)
                .and_then(|g| g.dock.active.clone())
        });
        let body: AnyElement = match active
            .as_ref()
            .and_then(|id| self.store.read(cx).state.tab(id).cloned())
        {
            Some(tab) => {
                let header = Some(self.pane_header(&tab, true, true, cx));
                let body = self.pane_body(&tab, window, cx);
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .children(header)
                    .child(div().flex_1().min_h(px(0.)).child(body))
                    .into_any()
            }
            None => self.render_surface_picker(cx),
        };

        let menu: Option<AnyElement> = self
            .store
            .read(cx)
            .dock_menu_open
            .then(|| self.render_dock_menu(cx));
        div()
            .h_full()
            .w(px(width))
            .flex_none()
            .relative()
            .pl(px(DOCK_GUTTER))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .border_l_1()
                    .border_color(p.border)
                    .bg(if Theme::is_frost() {
                        p.bg.opacity(0.4)
                    } else {
                        p.bg
                    })
                    .child(body),
            )
            .when_some(menu, |d, m| d.child(m))
            .into_any()
    }

    /// The dock's surfaces, in picker order: (id, icon, label, url).
    const SURFACES: &'static [(&'static str, &'static str, &'static str, &'static str)] = &[
        ("agent", icons::BOT, "Agent", "hifi://agent"),
        ("browser", icons::GLOBE, "Browser", "hifi://newtab"),
        ("terminal", icons::TERMINAL, "Terminal", "hifi://terminal"),
        ("page", icons::DOCUMENT, "Page", "hifi://notes"),
        ("changes", icons::GIT_BRANCH, "Changes", "hifi://diff"),
    ];

    /// cosmos `render_surface_picker`: a compact vertical list of surface
    /// rows (icon + label) centred in the empty pane.
    fn render_surface_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = Theme::of(cx).palette;
        let mut col = div()
            .w_full()
            .max_w(px(280.0))
            .flex()
            .flex_col()
            .gap(px(8.0));
        for (id, icon_path, title, url) in Self::SURFACES {
            let store = self.store.clone();
            col = col.child(
                div()
                    .id(SharedString::from(format!("surface-card-{id}")))
                    .w_full()
                    .h(px(44.0))
                    .px(px(14.0))
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.ink(0.02))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.ink(0.05)).border_color(p.border_strong))
                    .child(glyph(icon_path, 15.0, p.muted))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(p.text)
                            .child(*title),
                    )
                    .on_click(move |_, _, cx| {
                        store.update(cx, |s, cx| {
                            s.dock_open(url, cx);
                        });
                    }),
            );
        }
        div()
            .flex_1()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(16.0))
            .child(col)
            .into_any()
    }

    #[allow(dead_code)]
    fn render_dock_overflow_menu(&mut self, ids: &[TabId], cx: &mut Context<Self>) -> AnyElement {
        let p = Theme::of(cx).palette;
        let store = self.store.clone();
        let mut col = div()
            .w(px(220.))
            .max_h(px(260.))
            .overflow_hidden()
            .p(px(4.))
            .flex()
            .flex_col()
            .gap(px(1.))
            .rounded(px(10.))
            .bg(p.glass_overlay())
            .border_1()
            .border_color(p.border)
            .shadow_lg();
        for id in ids {
            let Some(tab) = self.store.read(cx).state.tab(id).cloned() else {
                continue;
            };
            let store = store.clone();
            let tab_id = id.clone();
            col = col.child(
                div()
                    .id(SharedString::from(format!("dock-overflow-{id}")))
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(p.glass_hover()))
                    .child(glyph(sidebar::kind_icon(&tab), 13., p.muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(p.text)
                            .child(tab.display_title()),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        store.update(cx, |s, cx| {
                            s.dock_overflow_open = false;
                            s.dock_focus(&tab_id, cx);
                        });
                    }),
            );
        }
        div()
            .absolute()
            .top(px(surface_chrome::CONTROL_SIZE + 4.))
            .left(px(DOCK_GUTTER + 4.))
            .child(col)
            .into_any()
    }

    /// The `+` menu: open a fresh surface in the dock, or move the active
    /// leaf tab over — a frosted cosmos popover.
    fn render_dock_menu(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = Theme::of(cx).palette;
        let store = self.store.clone();
        let active = self.store.read(cx).active_tab_id();

        let item = |id: SharedString, icon: &'static str, label: &'static str| {
            div()
                .id(id)
                .h(px(28.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(p.glass_hover()))
                .child(glyph(icon, 13., p.muted))
                .child(div().text_size(px(12.5)).text_color(p.text).child(label))
        };

        let mut col = div()
            .w(px(200.))
            .p(px(4.))
            .flex()
            .flex_col()
            .gap(px(1.))
            .rounded(px(10.))
            .bg(p.glass_overlay())
            .border_1()
            .border_color(p.border)
            .shadow_lg();
        for (id, icon_path, label, url) in Self::SURFACES {
            let store = store.clone();
            col = col.child(
                item(format!("dm-{id}").into(), icon_path, label).on_mouse_down(
                    MouseButton::Left,
                    move |_, _, cx| {
                        store.update(cx, |s, cx| {
                            s.dock_menu_open = false;
                            s.dock_open(url, cx);
                        });
                    },
                ),
            );
        }
        if let Some(id) = active {
            let store = store.clone();
            col = col.child(div().h(px(1.)).my(px(3.)).bg(p.border)).child(
                item(
                    "dm-move".into(),
                    icons::SIDEBAR_RIGHT,
                    "Move active tab here",
                )
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    store.update(cx, |s, cx| {
                        s.dock_menu_open = false;
                        s.dock_tab(&id, cx);
                    });
                }),
            );
        }
        deferred(
            div()
                .absolute()
                .top(px(surface_chrome::HEADER_HEIGHT))
                .left(px(8.))
                .child(crate::frost::frosted(10., crate::frost::MENU_BLUR, col)),
        )
        .into_any()
    }

    /// Thin drag strip containing only the sidebar and dock toggles.
    fn render_title_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = Theme::of(cx).palette;
        let store = self.store.clone();
        let (space_name, focused_title, dock_open) = {
            let s = self.store.read(cx);
            let space_name = s
                .state
                .active_space()
                .map(|space| space.name.clone())
                .unwrap_or_else(|| "Hi-Fi".into());
            let tab = s.active_tab_id().and_then(|id| s.state.tab(&id));
            let title = tab.map_or_else(|| "New Tab".into(), |tab| tab.short_title());
            let dock_open = s
                .state
                .active_space()
                .and_then(|sp| {
                    let gid = sp
                        .active_group
                        .clone()
                        .or_else(|| sp.groups.first().map(|g| g.id.clone()))?;
                    sp.groups.iter().find(|g| g.id == gid).map(|g| g.dock.open)
                })
                .unwrap_or(false);
            (space_name, title, dock_open)
        };
        let window_title = format!("{space_name} — {focused_title}");
        if self.last_window_title != window_title {
            window.set_window_title(&window_title);
            self.last_window_title = window_title;
        }
        let store_sb = store.clone();
        let store_dk = store.clone();
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(Theme::TITLEBAR_HEIGHT))
            .pt(px(Theme::TITLEBAR_TOP_PAD))
            .pl(px(TITLEBAR_CLUSTER_PAD))
            .flex()
            .items_center()
            .child(window_control_button(
                "toggle-sidebar",
                icons::SIDEBAR_LEFT,
                p,
                move |cx| {
                    store_sb.update(cx, |s, cx| {
                        s.sidebar_collapsed = !s.sidebar_collapsed;
                        cx.notify();
                    });
                },
            ))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(window_control_button(
                "toggle-dock",
                if dock_open {
                    icons::SIDEBAR_RIGHT
                } else {
                    icons::SIDEBAR
                },
                p,
                move |cx| store_dk.update(cx, |s, cx| s.toggle_dock(cx)),
            ))
            .into_any()
    }
}

/// GPUI-only strip on the dock's leading edge: native webviews swallow
/// mouse events, so the resize handle needs painted space of its own.
const DOCK_GUTTER: f32 = 6.0;

/// Narrowest a split pane can be dragged to; pages lay out badly below it.
const MIN_PANE: f32 = 220.0;

/// The smallest extent a split subtree needs along one axis.
fn min_extent(node: &SplitNode, horizontal: bool) -> f32 {
    match node {
        SplitNode::Leaf { .. } => MIN_PANE,
        SplitNode::Split {
            direction,
            first,
            second,
            ..
        } => {
            let (a, b) = (
                min_extent(first, horizontal),
                min_extent(second, horizontal),
            );
            if (*direction == hifi_core::SplitDirection::Horizontal) == horizontal {
                a + b + 6.
            } else {
                a.max(b)
            }
        }
    }
}

/// Horizontal inset owned by the titlebar control row itself.
const TITLEBAR_CLUSTER_PAD: f32 = 10.0;

/// cosmos `window_control_button`: a 24px glass-hover icon control that
/// occludes the titlebar drag strip beneath it.
fn window_control_button(
    id: &'static str,
    icon_path: &'static str,
    p: crate::theme::Palette,
    on_click: impl Fn(&mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(24.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|s| s.bg(p.glass_hover()))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            on_click(cx)
        })
        .child(glyph(icon_path, 16.0, p.muted))
}

/// cosmos `files::toolbar_button`: a surface-toolbar icon control.
fn toolbar_button(
    id: &'static str,
    icon: &'static str,
    enabled: bool,
    p: crate::theme::Palette,
    on_click: impl Fn(&mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id))
        .size(px(surface_chrome::CONTROL_SIZE))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(surface_chrome::CONTROL_RADIUS))
        .occlude()
        .when(enabled, |d| {
            d.cursor_pointer()
                .hover(|s| s.bg(p.wash(0.14)))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    on_click(cx)
                })
        })
        .child(glyph(
            icon,
            surface_chrome::ICON_SIZE,
            if enabled {
                p.muted
            } else {
                p.faint.opacity(0.45)
            },
        ))
}

impl gpui::Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.pump(window, cx);
        if let Some(anchor) = self.store.update(cx, |s, _| s.pending_space_menu.take()) {
            let store = self.store.clone();
            let shell = cx.entity().downgrade();
            let dismiss: Rc<dyn Fn(&mut App)> = Rc::new(move |app| {
                let _ = shell.update(app, |this, cx| {
                    this.context_menu = None;
                    cx.notify();
                });
            });
            let group_store = store.clone();
            let group_dismiss = dismiss.clone();
            let space_store = store.clone();
            let space_dismiss = dismiss.clone();
            self.context_menu = Some((
                f32::from(anchor.x),
                f32::from(anchor.y),
                vec![
                    crate::menu::MenuItem {
                        id: "space-new-group".into(),
                        label: "New Group".into(),
                        icon: Some(icons::PLUS),
                        action: Rc::new(move |app| {
                            group_store.update(app, |s, cx| {
                                s.create_group("Group", None, cx);
                            });
                            group_dismiss(app);
                        }),
                        separator_before: false,
                        disabled: false,
                        selected: false,
                    },
                    crate::menu::MenuItem {
                        id: "space-new-space".into(),
                        label: "New Space".into(),
                        icon: Some(icons::PLUS),
                        action: Rc::new(move |app| {
                            space_store.update(app, |s, cx| {
                                s.create_space("Space", cx);
                            });
                            space_dismiss(app);
                        }),
                        separator_before: false,
                        disabled: false,
                        selected: false,
                    },
                ],
            ));
        }
        if let Some((tab_id, anchor)) = self.store.update(cx, |s, _| s.pending_context_menu.take())
        {
            let store = self.store.clone();
            let shell = cx.entity().downgrade();
            let close_menu: Rc<dyn Fn(&mut App)> = {
                let shell = shell.clone();
                Rc::new(move |app: &mut App| {
                    let _ = shell.update(app, |this, cx| {
                        this.context_menu = None;
                        cx.notify();
                    });
                })
            };
            let mut items = Vec::new();
            {
                let store = store.clone();
                let id = tab_id.clone();
                let close_menu = close_menu.clone();
                items.push(crate::menu::MenuItem {
                    id: "row-open".into(),
                    label: "Open".into(),
                    icon: Some(icons::ARROW_RIGHT),
                    action: Rc::new(move |app| {
                        store.update(app, |s, cx| s.focus_tab(&id, cx));
                        close_menu(app);
                    }),
                    separator_before: false,
                    disabled: false,
                    selected: false,
                });
            }
            for (label, side) in [
                ("Open in Split Right", hifi_core::SplitSide::Right),
                ("Open in Split Down", hifi_core::SplitSide::Below),
            ] {
                let store = store.clone();
                let id = tab_id.clone();
                let close_menu = close_menu.clone();
                items.push(crate::menu::MenuItem {
                    id: SharedString::from(format!("row-split-{}", label)),
                    label: label.into(),
                    icon: Some(icons::SPLIT_COLUMNS),
                    action: Rc::new(move |app| {
                        store.update(app, |s, cx| {
                            s.open_tab("hifi://newtab", None, Some((id.clone(), side)), cx);
                        });
                        close_menu(app);
                    }),
                    separator_before: false,
                    disabled: false,
                    selected: false,
                });
            }
            {
                let store = store.clone();
                let id = tab_id.clone();
                let close_menu = close_menu.clone();
                items.push(crate::menu::MenuItem {
                    id: "row-pin".into(),
                    label: if store
                        .read(cx)
                        .state
                        .tab(&tab_id)
                        .is_some_and(|tab| tab.pinned)
                    {
                        "Unpin".into()
                    } else {
                        "Pin".into()
                    },
                    icon: Some(icons::PIN),
                    action: Rc::new(move |app| {
                        store.update(app, |s, cx| s.toggle_pin(&id, cx));
                        close_menu(app);
                    }),
                    separator_before: true,
                    disabled: false,
                    selected: false,
                });
            }
            let spaces = self.store.read(cx).state.spaces.clone();
            let current_space = self
                .store
                .read(cx)
                .state
                .tab(&tab_id)
                .and_then(|tab| {
                    self.store
                        .read(cx)
                        .state
                        .spaces
                        .iter()
                        .find(|space| space.groups.iter().any(|group| group.id == tab.group_id))
                })
                .map(|space| space.id.clone());
            for space in spaces {
                if current_space.as_deref() == Some(space.id.as_str()) {
                    continue;
                }
                let store = store.clone();
                let id = tab_id.clone();
                let space_id = space.id.clone();
                let close_menu = close_menu.clone();
                items.push(crate::menu::MenuItem {
                    id: SharedString::from(format!("row-move-{}", space.id)),
                    label: SharedString::from(format!("Move to {}", space.name)),
                    icon: Some(icons::SIDEBAR_RIGHT),
                    action: Rc::new(move |app| {
                        store.update(app, |s, cx| s.move_tab_to_space(&id, &space_id, cx));
                        close_menu(app);
                    }),
                    separator_before: false,
                    disabled: false,
                    selected: false,
                });
            }
            {
                let store = store.clone();
                let id = tab_id.clone();
                items.push(crate::menu::MenuItem {
                    id: "row-close".into(),
                    label: "Close".into(),
                    icon: Some(icons::CLOSE),
                    action: Rc::new(move |app| {
                        store.update(app, |s, cx| s.close_tab(&id, cx));
                    }),
                    separator_before: true,
                    disabled: false,
                    selected: false,
                });
            }
            self.context_menu = Some((f32::from(anchor.x), f32::from(anchor.y), items));
        }
        if window.focused(cx).is_none() {
            window.focus(&self.focus, cx);
        }
        let overlay_open = {
            let s = self.store.read(cx);
            s.command_bar_open || s.find_bar_open
        };
        if !self.store.read(cx).command_bar_open {
            let bar = self.command_bar.read(cx).input.read(cx).focus_handle(cx);
            if bar.is_focused(window) {
                window.focus(&self.focus, cx);
            }
        }
        if overlay_open {
            for pane in self.panes.values() {
                if let Pane::Web(host) = pane {
                    host.release_focus();
                }
            }
        }
        let p = Theme::of(cx).palette;
        let collapsed = self.store.read(cx).sidebar_collapsed;

        // Which web tabs are visible this frame; hide the rest. The dock's
        // tabs live outside the split tree, so union them in when open.
        let (visible_tabs, dock_open) = {
            let group = self.store.read(cx).state.active_space().and_then(|s| {
                let gid = s
                    .active_group
                    .clone()
                    .or_else(|| s.groups.first().map(|g| g.id.clone()))?;
                s.groups.iter().find(|g| g.id == gid)
            });
            match group {
                Some(g) => {
                    let mut v: HashSet<TabId> = g.tab_ids().into_iter().collect();
                    if g.dock.open
                        && let Some(t) = &g.dock.active
                    {
                        v.insert(t.clone());
                    }
                    (v, g.dock.open)
                }
                None => (HashSet::new(), false),
            }
        };
        let menu_open = self.context_menu.is_some();
        for (id, pane) in &self.panes {
            if let Pane::Web(h) = pane {
                if visible_tabs.contains(id) {
                    h.set_input_shield(menu_open || cx.has_active_drag());
                } else {
                    h.set_input_shield(cx.has_active_drag());
                    h.hide();
                }
            }
        }

        let root_node = self.store.read(cx).state.active_space().and_then(|s| {
            let gid = s
                .active_group
                .clone()
                .or_else(|| s.groups.first().map(|g| g.id.clone()))?;
            s.groups
                .iter()
                .find(|g| g.id == gid)
                .and_then(|g| g.root.clone())
        });

        let content: AnyElement = match root_node {
            Some(ref node) => {
                let active = self.store.read(cx).active_tab_id();
                self.render_node(node, active.as_ref(), &[], window, cx)
            }
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(p.faint)
                .text_size(px(13.))
                .child("No tabs - press the new-tab shortcut to open one")
                .into_any(),
        };

        let sb_width = self.store.read(cx).state.settings.sidebar_width;
        let sb_compact = self.store.read(cx).state.settings.compact_sidebar;
        let dock_width = self
            .store
            .read(cx)
            .state
            .active_space()
            .and_then(|s| {
                let gid = s
                    .active_group
                    .clone()
                    .or_else(|| s.groups.first().map(|g| g.id.clone()))?;
                s.groups.iter().find(|g| g.id == gid).map(|g| g.dock.width)
            })
            .unwrap_or(460.);
        let sidebar_now = if collapsed {
            0.
        } else if sb_compact {
            sidebar::COMPACT_WIDTH
        } else {
            sb_width
        };
        let main_min = root_node
            .as_ref()
            .map_or(320., |n| min_extent(n, true).max(320.));
        let dock_max = (f32::from(window.viewport_size().width) - sidebar_now - main_min).max(280.);
        let dock_width = dock_width.max(360.).min(dock_max);
        let mut root = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.glass())
            .text_color(p.text)
            .key_context("Shell")
            .track_focus(&self.focus)
            .capture_any_mouse_down(cx.listener(|this, _: &gpui::MouseDownEvent, _, cx| {
                this.context_menu = None;
                cx.notify();
                for pane in this.panes.values() {
                    if let Pane::Web(h) = pane {
                        h.release_focus();
                    }
                }
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key.eq_ignore_ascii_case("escape") {
                    this.context_menu = None;
                    cx.notify();
                }
            }))
            .when(cfg!(target_os = "linux"), |root| {
                root.on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, w, cx| {
                    this.forward_key(&e.keystroke, true, w, cx);
                }))
                .on_key_up(cx.listener(|this, e: &gpui::KeyUpEvent, w, cx| {
                    this.forward_key(&e.keystroke, false, w, cx);
                }))
            })
            .on_action(cx.listener(|this, _: &crate::NewTab, _w, cx| {
                this.store.update(cx, |s, cx| {
                    s.open_tab("hifi://newtab", None, None, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &crate::CloseTab, _w, cx| {
                this.store.update(cx, |s, cx| {
                    if let Some(id) = s.active_tab_id() {
                        s.close_tab(&id, cx);
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &crate::CommandBar, _w, cx| {
                this.store.update(cx, |s, cx| s.toggle_command_bar(cx));
            }))
            .on_action(cx.listener(|this, _: &crate::FocusAddress, _w, cx| {
                if this.store.read(cx).command_bar_open {
                    this.store.update(cx, |s, cx| s.toggle_command_bar(cx));
                } else {
                    this.open_address(None, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &crate::ReloadPage, _w, cx| {
                this.store.update(cx, |s, cx| {
                    if let Some(id) = s.active_tab_id() {
                        s.pending_reload.push(id);
                        cx.notify();
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &crate::GoBack, _w, cx| {
                this.store.update(cx, |s, cx| {
                    if let Some(id) = s.active_tab_id() {
                        s.pending_back.push(id);
                        cx.notify();
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &crate::GoForward, _w, cx| {
                this.store.update(cx, |s, cx| {
                    if let Some(id) = s.active_tab_id() {
                        s.pending_forward.push(id);
                        cx.notify();
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &crate::NextTab, _w, cx| {
                this.store.update(cx, |s, cx| s.cycle_tab(1, cx));
            }))
            .on_action(cx.listener(|this, _: &crate::PrevTab, _w, cx| {
                this.store.update(cx, |s, cx| s.cycle_tab(-1, cx));
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleSidebar, _w, cx| {
                this.store.update(cx, |s, cx| {
                    s.sidebar_collapsed = !s.sidebar_collapsed;
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|this, _: &crate::NewTerminal, _w, cx| {
                this.store.update(cx, |s, cx| {
                    s.open_tab("hifi://terminal", None, None, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &crate::NewAgent, _w, cx| {
                this.store.update(cx, |s, cx| {
                    s.open_tab("hifi://agent", None, None, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &crate::NewDiff, _w, cx| {
                this.store.update(cx, |s, cx| {
                    s.open_tab("hifi://diff", None, None, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &crate::OpenSettings, _w, cx| {
                this.store.update(cx, |s, cx| {
                    let existing = s
                        .state
                        .active_space()
                        .into_iter()
                        .flat_map(|sp| &sp.groups)
                        .flat_map(|g| g.tab_ids().into_iter().chain(g.dock.tabs.clone()))
                        .find(|id| s.state.tab(id).is_some_and(|t| t.kind == TabKind::Settings));
                    match existing {
                        Some(id) => s.focus_tab(&id, cx),
                        None => {
                            s.open_tab("hifi://settings", None, None, cx);
                        }
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &crate::NewGroup, _w, cx| {
                this.store.update(cx, |s, cx| {
                    s.create_group("Group", None, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &crate::FindInPage, _w, cx| {
                this.store.update(cx, |s, cx| {
                    s.find_bar_open = !s.find_bar_open;
                    cx.notify();
                });
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleDock, _w, cx| {
                this.store.update(cx, |s, cx| s.toggle_dock(cx));
            }))
            .on_action(cx.listener(|this, _: &crate::NewNotes, _w, cx| {
                this.store.update(cx, |s, cx| {
                    s.dock_open("hifi://notes", cx);
                });
            }))
            // cosmos sidebar tone: a slightly lighter column spanning the
            // full window height, hairlined on its right edge.
            .when(sidebar_now > 0., |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(px(sidebar_now))
                        .bg(p.wash(0.03))
                        .border_r_1()
                        .border_color(p.border),
                )
            })
            .child({
                // Body row: sidebar · content · dock. The two resize handles
                // float over this row; its bounds drive the width math
                // (cosmos's on_drag_move idiom).
                let store_sb = self.store.clone();
                let store_dk = self.store.clone();
                let mut row = div()
                    .id("bodyrow")
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.))
                    .relative()
                    .on_drag_move::<SidebarResize>(move |event, w, cx| {
                        if event.event.pressed_button != Some(gpui::MouseButton::Left) {
                            cx.stop_active_drag(w);
                            return;
                        }
                        let w = f32::from(event.event.position.x - event.bounds.left());
                        store_sb.update(cx, |s, cx| s.set_sidebar_width(w, cx));
                    })
                    .on_drag_move::<DockResize>(move |event, w, cx| {
                        if event.event.pressed_button != Some(gpui::MouseButton::Left) {
                            cx.stop_active_drag(w);
                            return;
                        }
                        let w = f32::from(event.bounds.right() - event.event.position.x);
                        let max = (f32::from(event.bounds.size.width) - 300.).max(360.);
                        store_dk.update(cx, |s, cx| {
                            s.dock_set_width(w.clamp(360., max), cx);
                        });
                    });
                if !collapsed {
                    row = row.child(
                        div()
                            .h_full()
                            .pt(px(Theme::SIDEBAR_TOP_PAD))
                            .child(self.sidebar.clone()),
                    );
                }
                if !collapsed && !sb_compact {
                    row = row.child(
                        div()
                            .id("sidebar-resize")
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(px(sb_width - 10.))
                            .w(px(20.))
                            .occlude()
                            .cursor_col_resize()
                            .on_drag(SidebarResize, |_, _point, _, cx| {
                                cx.stop_propagation();
                                cx.new(|_| DragGhost)
                            }),
                    );
                }
                row = row.child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .h_full()
                        .pt(px(Theme::TITLEBAR_HEIGHT))
                        .flex()
                        .child(content),
                );
                if dock_open {
                    row = row.child(
                        div()
                            .h_full()
                            .pt(px(Theme::TITLEBAR_HEIGHT))
                            .child(self.render_dock(dock_width, window, cx)),
                    );
                    row = row.child(
                        div()
                            .id("dock-resize")
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(dock_width - DOCK_GUTTER - 4.))
                            .w(px(DOCK_GUTTER + 8.))
                            .occlude()
                            .cursor_col_resize()
                            .on_drag(DockResize, |_, _point, _, cx| {
                                cx.stop_propagation();
                                cx.new(|_| DragGhost)
                            }),
                    );
                }
                row
            });
        root = root.child(self.render_title_bar(window, cx));
        if let Some((x, y, items)) = &self.context_menu {
            let shell = cx.entity().downgrade();
            let dismiss: Rc<dyn Fn(&mut App)> = Rc::new(move |app| {
                let _ = shell.update(app, |this, cx| {
                    this.context_menu = None;
                    cx.notify();
                });
            });
            root = root.child(crate::menu::context_menu(
                items.clone(),
                point(px(*x), px(*y)),
                window.viewport_size(),
                dismiss,
                cx,
            ));
        }

        // Find bar — deferred overlay top-right so it floats over webviews.
        if self.store.read(cx).find_bar_open
            && let Some(input) = &self.find_input
        {
            root = root.child(
                deferred(
                    div()
                        .absolute()
                        .top(px(Theme::TITLEBAR_HEIGHT + 44.))
                        .right(px(16.))
                        .child(
                            div()
                                .w(px(280.))
                                .h(px(30.))
                                .rounded(px(10.))
                                .bg(p.card)
                                .border_1()
                                .border_color(p.border)
                                .flex()
                                .items_center()
                                .px(px(8.))
                                .gap(px(6.))
                                .child(glyph(icons::MAGNIFER, 12., p.faint))
                                .child(div().flex_1().child(input.clone()))
                                .child({
                                    let store = self.store.clone();
                                    div()
                                        .id("findbar-x")
                                        .size(px(18.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(4.))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(p.raised))
                                        .child(glyph(icons::CLOSE, 10., p.faint))
                                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                            store.update(cx, |s, cx| {
                                                s.find_bar_open = false;
                                                cx.notify();
                                            });
                                        })
                                }),
                        ),
                )
                .into_any(),
            );
            window.focus(&input.read(cx).focus_handle(cx), cx);
        }

        // Command bar overlay (deferred = above native webviews).
        if self.store.read(cx).command_bar_open {
            root = root.child(
                deferred(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .justify_center()
                        .pt(px(120.))
                        .bg(gpui::black().opacity(0.3))
                        .child(div().child(self.command_bar.clone())),
                )
                .into_any(),
            );
            // Focus the input once opened.
            if let Some(f) = Some(self.command_bar.read(cx).input.clone()) {
                window.focus(&f.read(cx).focus_handle(cx), cx);
            }
        }

        root
    }
}
