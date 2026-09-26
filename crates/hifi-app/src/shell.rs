//! `Shell` — the root view: frosted sidebar, split content area, right dock,
//! deferred overlays (command bar, find bar), and the native-pane registry.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};

use gpui::{
    AnyElement, App, Context, Entity, Focusable, MouseButton, SharedString, Window,
    WindowControlArea, deferred, div, prelude::*, px, relative,
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
use crate::webview::{WebEvent, WebPaneHost};
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

/// The empty drag preview — a 1px chip; the live resize is the feedback.
struct DragGhost;
impl gpui::Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(px(1.))
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
    /// Find-bar input, created on first ⌘F.
    find_input: Option<Entity<TextField>>,
    /// WebEvent source; drained each render.
    web_rx: Receiver<WebEvent>,
    web_tx: Sender<WebEvent>,
    ipc_rx: Receiver<IpcJob>,
    /// Events drained off the channels by the wake loop, applied on render.
    inbox_web: Vec<WebEvent>,
    inbox_ipc: Vec<IpcJob>,
    /// Root focus target so window-level key bindings dispatch when no
    /// input or terminal holds focus.
    focus: gpui::FocusHandle,
}

impl Shell {
    pub fn new(
        store: Entity<Store>,
        ipc_rx: Receiver<IpcJob>,
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
        let (web_tx, web_rx) = channel::<WebEvent>();
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        // Webview events, IPC requests and PTY output arrive off-thread. Poll
        // the channels at frame rate but only repaint when something landed,
        // so an idle window costs no layout or paint work.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
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
            find_input: Some(find_input),
            web_rx,
            web_tx,
            ipc_rx,
            inbox_web: Vec::new(),
            inbox_ipc: Vec::new(),
            focus: cx.focus_handle(),
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
                    self.store.update(cx, |s, cx| {
                        s.tab_meta_update(&tab, Some(title), None, None, None, None, cx);
                    });
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
                WebEvent::Keystroke { combo } => {
                    let (key, _tab) = combo.rsplit_once('|').unwrap_or((&combo, ""));
                    if let Ok(ks) = gpui::Keystroke::parse(key) {
                        window.dispatch_keystroke(ks, cx);
                    }
                }
                WebEvent::NotesSave { tab, html } => {
                    let page = serde_json::from_str::<crate::notes::PageSave>(&html).ok();
                    self.store.update(cx, |s, cx| match page {
                        Some(page) => {
                            s.notes_save(&tab, &page.html);
                            s.set_title(&tab, page.title, cx);
                        }
                        None => s.notes_save(&tab, &html),
                    });
                }
                WebEvent::Error { tab, message } => {
                    self.store.update(cx, |s, cx| {
                        s.tab_meta_update(
                            &tab,
                            Some(format!("⚠ {message}")),
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
            m::TAB_CLICK => format!(
                "(()=>{{const el=document.querySelector({});el&&el.click();return JSON.stringify({{ok:!!el}})}})()",
                serde_json::to_string(&get("target").unwrap_or_default()).unwrap()
            ),
            m::TAB_TYPE => format!(
                "(()=>{{const el=document.querySelector({});if(el){{el.focus();el.value={};el.dispatchEvent(new Event('input',{{bubbles:true}}))}}return JSON.stringify({{ok:!!el}})}})()",
                serde_json::to_string(&get("target").unwrap_or_default()).unwrap(),
                serde_json::to_string(&get("text").unwrap_or_default()).unwrap()
            ),
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
        let ipc = matches!(tab.kind, TabKind::Notes);
        // hifi:// urls never reach the network — the nav delegate would
        // cancel them into WebEvent::Url and open a duplicate tab.
        let load_url = if tab.url.starts_with("hifi://") {
            ""
        } else {
            &tab.url
        };
        match WebPaneHost::new(window, tab.id.clone(), load_url, self.web_tx.clone(), ipc) {
            Ok(host) => {
                let host = Rc::new(host);
                if tab.kind == TabKind::Notes {
                    let body = self.store.read(cx).notes_body(&tab.id);
                    host.load_html(&crate::notes::editor_html(&body, &tab.title, is_dark));
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
        let id = job.request.id.clone();
        let p = &job.request.params;
        let get = |k: &str| p.get(k).and_then(|v| v.as_str()).map(String::from);
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
                    .map(|t| {
                        json!({"id": t.id, "kind": t.kind.as_str(), "title": t.title, "url": t.url, "pinned": t.pinned, "groupId": t.group_id})
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
            m::TAB_EXEC | m::TAB_SNAPSHOT | m::TAB_CLICK | m::TAB_TYPE => {
                self.ipc_eval_async(&job.request, job.reply.clone());
                return;
            }
            m::TAB_SCREENSHOT => {
                let id = get("id");
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
        let response = match result {
            Ok(v) => hifi_core::IpcResponse::ok(id, v),
            Err(e) => hifi_core::IpcResponse::err(id, e),
        };
        let _ = job.reply.send(response);
    }

    // ----- layout -----

    /// A webview canvas bound to `host` — syncs the native clip to the
    /// painted bounds each frame. Shared by leaves and the dock.
    fn web_canvas(host: &Rc<WebPaneHost>) -> gpui::Canvas<()> {
        let host = host.clone();
        gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, _cx| {
                let host = Rc::downgrade(&host);
                window.on_present(move || {
                    if let Some(host) = host.upgrade() {
                        host.sync_bounds(bounds, true);
                    }
                });
            },
        )
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
        match tab.kind {
            TabKind::Web | TabKind::Notes => match self.web_host(tab, p.is_dark, window, cx) {
                Some(host) => div()
                    .size_full()
                    .relative()
                    .child(Self::web_canvas(&host).absolute().inset_0())
                    .into_any(),
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
                        .child(format!("⚠ {err}"))
                        .into_any()
                }
            },
            TabKind::Terminal | TabKind::Agent => {
                let view = match self.panes.entry(tab.id.clone()) {
                    std::collections::hash_map::Entry::Occupied(e) => match e.get() {
                        Pane::Term(v) => Some(v.clone()),
                        _ => None,
                    },
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let cmd = if tab.kind == TabKind::Agent {
                            Some(if tab.command.is_empty() {
                                "claude".to_string()
                            } else {
                                tab.command.clone()
                            })
                        } else {
                            (!tab.command.is_empty()).then(|| tab.command.clone())
                        };
                        let cwd = (!tab.cwd.is_empty()).then(|| tab.cwd.clone());
                        match TerminalPane::spawn_pty(cmd.as_deref(), cwd.as_deref()) {
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
                            "⚠ {}",
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
                let view = self.view_pane(
                    tab.id.clone(),
                    |tab_id, w, cx| {
                        let path = std::env::var("HOME").unwrap_or_default() + "/repos";
                        DiffView::new(path, tab_id, w, cx)
                    },
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

        if tab.kind == TabKind::Web {
            let nav = |name: &'static str,
                       icon: &'static str,
                       enabled: bool,
                       op: fn(&mut Store, TabId)| {
                let store = store.clone();
                let id = id.clone();
                toolbar_button(name, icon, enabled, p, move |cx| {
                    store.update(cx, |s, cx| {
                        op(s, id.clone());
                        cx.notify();
                    });
                })
            };
            header = header
                .child(nav(
                    "nav-back",
                    icons::ARROW_LEFT,
                    tab.can_go_back,
                    |s, id| s.pending_back.push(id),
                ))
                .child(nav(
                    "nav-fwd",
                    icons::ARROW_RIGHT,
                    tab.can_go_forward,
                    |s, id| s.pending_forward.push(id),
                ))
                .child(nav("nav-reload", icons::REFRESH, true, |s, id| {
                    s.pending_reload.push(id)
                }));
        }

        let basename = |path: &str| {
            path.trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string()
        };
        let (lead_icon, primary, secondary): (&'static str, String, String) = match tab.kind {
            TabKind::Web => {
                let rest = tab
                    .url
                    .split_once("://")
                    .map_or(tab.url.as_str(), |(_, r)| r);
                let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
                let host = host.strip_prefix("www.").unwrap_or(host).to_string();
                let path = if path == "/" { "" } else { path };
                let icon = if tab.url.starts_with("https://") {
                    icons::LOCK
                } else {
                    icons::GLOBE
                };
                (icon, host, path.to_string())
            }
            TabKind::Terminal => (
                sidebar::kind_icon(tab),
                "Terminal".into(),
                basename(&tab.cwd),
            ),
            TabKind::Agent => (sidebar::kind_icon(tab), "Agent".into(), basename(&tab.cwd)),
            TabKind::Diff => (icons::GIT_BRANCH, "Changes".into(), basename(&tab.cwd)),
            TabKind::Preview => (icons::EYE, "Preview".into(), basename(&tab.url)),
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
                    tab.title.clone()
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

        let split = |name: &'static str, icon: &'static str, side: hifi_core::SplitSide| {
            let store = store.clone();
            let id = id.clone();
            toolbar_button(name, icon, true, p, move |cx| {
                store.update(cx, |s, cx| {
                    s.open_tab("hifi://newtab", None, Some((id.clone(), side)), cx);
                });
            })
        };
        if !docked && focused {
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
                ));
        }
        if focused {
            header = header
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
                        true,
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
                        true,
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
        }
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
                Pane::View(v) => return v.clone(),
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
                let handle = div()
                    .id(handle_id)
                    .group("split-handle")
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(horizontal, |d| d.w(px(6.)).h_full().cursor_col_resize())
                    .when(!horizontal, |d| d.h(px(6.)).w_full().cursor_row_resize())
                    .child(
                        div()
                            .rounded_full()
                            .when(horizontal, |d| d.w(px(2.)).h(px(32.)))
                            .when(!horizontal, |d| d.h(px(2.)).w(px(32.)))
                            .group_hover("split-handle", |s| s.bg(p.accent.opacity(0.7))),
                    )
                    .occlude()
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
                row = row.on_drag_move::<SplitDrag>(move |event, _window, cx| {
                    let (path, horizontal) = {
                        let drag = event.drag(cx);
                        (drag.path.clone(), drag.horizontal)
                    };
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
                        store.update(cx, |s, cx| {
                            s.resize_split(&path, frac / span, cx);
                        });
                    }
                });
                row.child(
                    div()
                        .flex_basis(relative(f))
                        .flex_grow(1.)
                        .min_h(px(0.))
                        .min_w(px(0.))
                        .flex()
                        .child(first_el),
                )
                .child(handle)
                .child(
                    div()
                        .flex_basis(relative(1. - f))
                        .flex_grow(1.)
                        .min_h(px(0.))
                        .min_w(px(0.))
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
    fn render_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = Theme::of(cx).palette;
        let store_e = self.store.clone();
        let (width, tabs, active) = self
            .store
            .read(cx)
            .state
            .active_space()
            .and_then(|s| {
                let gid = s
                    .active_group
                    .clone()
                    .or_else(|| s.groups.first().map(|g| g.id.clone()))?;
                s.groups
                    .iter()
                    .find(|g| g.id == gid)
                    .map(|g| (g.dock.width, g.dock.tabs.clone(), g.dock.active.clone()))
            })
            .unwrap_or((460., vec![], None));
        let width = width.max(360.);

        let mut strip = surface_chrome::toolbar(&p).overflow_hidden();
        for id in &tabs {
            let Some(tab) = self.store.read(cx).state.tab(id).cloned() else {
                continue;
            };
            let is_active = active.as_deref() == Some(id.as_str());
            let cid: SharedString = format!("dockchip:{id}").into();
            let kind_icon = sidebar::kind_icon(&tab);
            let title = tab.display_title();
            let store_c = store_e.clone();
            let id_c = id.clone();
            let store_x = store_e.clone();
            let id_x = id.clone();
            strip = strip.child(
                div()
                    .id(cid)
                    .group("dock-chip")
                    .h(px(surface_chrome::CONTROL_SIZE))
                    .max_w(px(160.))
                    .min_w(px(0.))
                    .flex_shrink(1.)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pl(px(8.))
                    .pr(px(4.))
                    .rounded(px(surface_chrome::CONTROL_RADIUS))
                    .cursor_pointer()
                    .when(is_active, |d| d.bg(p.selected()))
                    .when(!is_active, |d| d.hover(|s| s.bg(p.glass_hover())))
                    .child(glyph(
                        kind_icon,
                        12.,
                        if is_active { p.text } else { p.faint },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(11.5))
                            .text_color(if is_active { p.text } else { p.muted })
                            .child(title),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("dockchip-x:{id}")))
                            .size(px(16.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .opacity(if is_active { 1. } else { 0. })
                            .group_hover("dock-chip", |s| s.opacity(1.))
                            .hover(|s| s.bg(p.wash(0.14)))
                            .child(glyph(icons::CLOSE, 9., p.faint))
                            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                cx.stop_propagation();
                                store_x.update(cx, |s, cx| s.close_tab(&id_x, cx));
                            }),
                    )
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        store_c.update(cx, |s, cx| s.dock_focus(&id_c, cx));
                    }),
            );
        }
        let store_p = store_e.clone();
        let store_close = store_e.clone();
        strip = strip
            .child(toolbar_button(
                "dock-plus",
                icons::PLUS,
                true,
                p,
                move |cx| {
                    store_p.update(cx, |s, cx| {
                        s.dock_menu_open = !s.dock_menu_open;
                        cx.notify();
                    });
                },
            ))
            .child(div().flex_1())
            .child(toolbar_button(
                "dock-close",
                icons::CLOSE,
                true,
                p,
                move |cx| store_close.update(cx, |s, cx| s.toggle_dock(cx)),
            ));

        let body: AnyElement = match active
            .as_ref()
            .and_then(|id| self.store.read(cx).state.tab(id).cloned())
        {
            Some(tab) => {
                let header =
                    (tab.kind == TabKind::Web).then(|| self.pane_header(&tab, true, true, cx));
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
                    .child(strip)
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

    /// The unified window titlebar (cosmos `render_title_bar` +
    /// `render_titlebar_cluster`): past the traffic lights, the sidebar
    /// toggle, history and new-tab controls, then the active tab's identity,
    /// then the trailing actions — all on the glass shell.
    fn render_title_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = Theme::of(cx).palette;
        let store = self.store.clone();
        let (tab, dock_open) = {
            let s = self.store.read(cx);
            let tab = s.active_tab_id().and_then(|id| s.state.tab(&id).cloned());
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
            (tab, dock_open)
        };
        let (can_back, can_fwd) = tab
            .as_ref()
            .map_or((false, false), |t| (t.can_go_back, t.can_go_forward));
        let nav =
            |id: &'static str, icon: &'static str, enabled: bool, op: fn(&mut Store, TabId)| {
                let store = store.clone();
                nav_history_button(id, icon, enabled, p, move |cx| {
                    store.update(cx, |s, cx| {
                        if let Some(id) = s.active_tab_id() {
                            op(s, id);
                            cx.notify();
                        }
                    });
                })
            };
        let spacer = if cfg!(target_os = "macos") {
            TITLEBAR_CLUSTER_START - TITLEBAR_CLUSTER_PAD
        } else {
            0.
        };
        let store_sb = store.clone();
        let store_nt = store.clone();
        let cluster = div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .child(div().flex_none().w(px(spacer)))
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
                    .ml(px(Theme::SPACE_SM))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.))
                    .child(nav("nav-back", icons::ARROW_LEFT, can_back, |s, id| {
                        s.pending_back.push(id)
                    }))
                    .child(nav("nav-forward", icons::ARROW_RIGHT, can_fwd, |s, id| {
                        s.pending_forward.push(id)
                    })),
            )
            .child(div().ml(px(Theme::SPACE_SM)).child(window_control_button(
                "titlebar-new-tab",
                icons::PLUS,
                p,
                move |cx| {
                    store_nt.update(cx, |s, cx| {
                        s.open_tab("hifi://newtab", None, None, cx);
                    });
                },
            )));

        let identity = tab.map(|t| {
            let title = t.display_title();
            div()
                .ml(px(Theme::SPACE_MD))
                .min_w(px(0.))
                .flex_shrink(1.)
                .flex()
                .items_center()
                .gap(px(8.))
                .child(sidebar::tab_badge(&t, 13., &p))
                .child(
                    div()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(p.text)
                        .child(title),
                )
        });

        let store_ag = store.clone();
        let store_pg = store.clone();
        let store_dk = store.clone();
        let trailing = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.))
            .pr(px(TITLEBAR_ACTION_EDGE_INSET))
            .child(window_control_button(
                "tb-agent",
                icons::BOT,
                p,
                move |cx| {
                    store_ag.update(cx, |s, cx| {
                        s.dock_open("hifi://agent", cx);
                    });
                },
            ))
            .child(window_control_button(
                "tb-page",
                icons::DOCUMENT_ADD,
                p,
                move |cx| {
                    store_pg.update(cx, |s, cx| {
                        s.open_tab("hifi://notes", None, None, cx);
                    });
                },
            ))
            .child(window_control_button(
                "tb-dock",
                if dock_open {
                    icons::SIDEBAR_RIGHT
                } else {
                    icons::SIDEBAR
                },
                p,
                move |cx| store_dk.update(cx, |s, cx| s.toggle_dock(cx)),
            ));

        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(Theme::TITLEBAR_HEIGHT))
            .pt(px(Theme::TITLEBAR_TOP_PAD))
            .pl(px(TITLEBAR_CLUSTER_PAD))
            .flex()
            .flex_row()
            .items_center()
            .child(cluster)
            .children(identity)
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(trailing)
            .into_any()
    }
}

/// Where the titlebar control cluster starts on macOS: past the traffic
/// lights at {14,14} (cosmos `titlebar_cluster_start`).
const TITLEBAR_CLUSTER_START: f32 = 88.0;
/// Horizontal inset owned by the titlebar control row itself.
const TITLEBAR_CLUSTER_PAD: f32 = 10.0;
const TITLEBAR_ACTION_EDGE_INSET: f32 = 6.0;

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

/// cosmos `nav_history_button`: a disabled history control still occludes
/// the strip but dims its glyph.
fn nav_history_button(
    id: &'static str,
    icon_path: &'static str,
    enabled: bool,
    p: crate::theme::Palette,
    on_click: impl Fn(&mut App) + 'static,
) -> AnyElement {
    if !enabled {
        return div()
            .size(px(24.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .child(glyph(icon_path, 16.0, p.muted.opacity(0.35)))
            .into_any_element();
    }
    window_control_button(id, icon_path, p, on_click).into_any_element()
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
                    if g.dock.open {
                        for t in &g.dock.tabs {
                            v.insert(t.clone());
                        }
                    }
                    (v, g.dock.open)
                }
                None => (HashSet::new(), false),
            }
        };
        for (id, pane) in &self.panes {
            if let Pane::Web(h) = pane
                && !visible_tabs.contains(id)
            {
                h.hide();
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
            Some(node) => {
                let active = self.store.read(cx).active_tab_id();
                self.render_node(&node, active.as_ref(), &[], window, cx)
            }
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(p.faint)
                .text_size(px(13.))
                .child("No tabs — ⌘T to open one")
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
            .unwrap_or(460.)
            .max(360.);

        let sidebar_now = if collapsed {
            0.
        } else if sb_compact {
            sidebar::COMPACT_WIDTH
        } else {
            sb_width
        };
        let mut root = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.glass())
            .text_color(p.text)
            .key_context("Shell")
            .track_focus(&self.focus)
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
                    s.open_tab("hifi://settings", None, None, cx);
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
                    .pt(px(Theme::TITLEBAR_HEIGHT))
                    .relative()
                    .on_drag_move::<SidebarResize>(move |event, _w, cx| {
                        let w = f32::from(event.event.position.x - event.bounds.left());
                        store_sb.update(cx, |s, cx| s.set_sidebar_width(w, cx));
                    })
                    .on_drag_move::<DockResize>(move |event, _w, cx| {
                        let w = f32::from(event.bounds.right() - event.event.position.x);
                        let max = (f32::from(event.bounds.size.width) - 300.).max(360.);
                        store_dk.update(cx, |s, cx| {
                            s.dock_set_width(w.clamp(360., max), cx);
                        });
                    });
                if !collapsed {
                    row = row.child(div().h_full().child(self.sidebar.clone()));
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
                row = row.child(div().flex_1().min_w(px(0.)).h_full().flex().child(content));
                if dock_open {
                    row = row.child(self.render_dock(window, cx));
                    row = row.child(
                        div()
                            .id("dock-resize")
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .right(px(dock_width - 10.))
                            .w(px(20.))
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
        root = root.child(self.render_title_bar(cx));

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
