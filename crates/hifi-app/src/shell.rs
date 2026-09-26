//! `Shell` — the root view: titlebar, frosted sidebar, split content area,
//! deferred overlays (command bar, find bar), and the native-pane registry.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};

use gpui::{
    AnyElement, App, Context, Entity, Focusable, MouseButton, SharedString,
    Window, WindowControlArea, deferred, div, prelude::*, px, relative,
};

use crate::assets::icons;
use crate::command_bar::CommandBar;
use crate::sidebar::{Sidebar, mark_icon};
use crate::store::Store;
use crate::terminal::TerminalPane;
use crate::theme::{Theme, radius};
use crate::views::{DiffView, NewTabView, PreviewView, SettingsView, glyph};
use crate::webview::{WebEvent, WebPaneHost};
use hifi_core::{SplitNode, TabId, TabKind};

pub enum Pane {
    Web(Rc<WebPaneHost>),
    Term(Entity<TerminalPane>),
    View(gpui::AnyView),
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
    /// WebEvent source; drained each render.
    web_rx: Receiver<WebEvent>,
    web_tx: Sender<WebEvent>,
    ipc_rx: Receiver<IpcJob>,
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
        let (web_tx, web_rx) = channel::<WebEvent>();
        // Drive the pump even when nothing repaints: webview events and IPC
        // requests arrive off-thread and must not wait for a render.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
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
            web_rx,
            web_tx,
            ipc_rx,
        }
    }

    /// Drain webview + IPC events. Called at the top of render — mutations
    /// land on the store which notifies again.
    fn pump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Ok(ev) = self.web_rx.try_recv() {
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
                    let (key, _tab) = combo.rsplit_once('|').map(|(a, b)| (a, b)).unwrap_or((&combo, ""));
                    if let Ok(ks) = gpui::Keystroke::parse(key) {
                        window.dispatch_keystroke(ks, cx);
                    }
                }
                WebEvent::Error { tab, message } => {
                    self.store.update(cx, |s, cx| {
                        s.tab_meta_update(&tab, Some(format!("⚠ {message}")), None, Some(false), None, None, cx);
                    });
                }
            }
        }
        while let Ok(job) = self.ipc_rx.try_recv() {
            self.handle_ipc(job, window, cx);
        }

        // Apply queued nav ops to live web hosts.
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
                    if let Some(tab) = self.store.read(cx).state.tab(&id).cloned() {
                        let host = self.web_host(&tab, window);
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
        if self.panes.get(&id).is_none() {
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

    fn web_host(&mut self, tab: &hifi_core::Tab, window: &mut Window) -> Rc<WebPaneHost> {
        if let Some(Pane::Web(h)) = self.panes.get(&tab.id) {
            return h.clone();
        }
        let host = WebPaneHost::new(window, tab.id.clone(), &tab.url, self.web_tx.clone())
            .expect("webview");
        let host = Rc::new(host);
        self.panes.insert(tab.id.clone(), Pane::Web(host.clone()));
        host
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
                let size = get("size");
                let anchor = [("rightOf", hifi_core::SplitSide::Right), ("leftOf", hifi_core::SplitSide::Left), ("above", hifi_core::SplitSide::Above), ("below", hifi_core::SplitSide::Below)]
                    .iter()
                    .find_map(|(k, side)| get(k).map(|a| (a, *side)));
                let target_url = if let Some(k) = &kind {
                    format!("hifi://{k}")
                } else {
                    url
                };
                Ok(self.store.update(cx, |s, cx| {
                    let id = s.open_tab(&target_url, group, anchor, cx);
                    if let Some(t) = s.state.tab_mut(&id) {
                        if let Some(c) = &command {
                            t.command = c.clone();
                        }
                        if let Some(pp) = &project_path {
                            t.cwd = pp.clone();
                        }
                    }
                    let _ = size;
                    json!({"id": id})
                }))
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

    fn render_leaf(
        &mut self,
        tab_id: &TabId,
        focused: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab) = self.store.read(cx).state.tab(tab_id).cloned() else {
            return div().into_any();
        };
        let p = Theme::of(cx).palette;

        let header = self.pane_header(&tab, cx);

        let body: AnyElement = match tab.kind {
            TabKind::Web => {
                let host = self.web_host(&tab, window);
                div()
                    .size_full()
                    .relative()
                    .child(
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
                        .absolute()
                        .inset_0(),
                    )
                    .into_any()
            }
            TabKind::Terminal | TabKind::Agent => {
                let view = match self.panes.entry(tab.id.clone()) {
                    std::collections::hash_map::Entry::Occupied(e) => match e.get() {
                        Pane::Term(v) => v.clone(),
                        _ => unreachable!(),
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
                        let v = cx.new(|cx| {
                            TerminalPane::spawn(cmd.as_deref(), cwd.as_deref(), cx)
                                .expect("pty")
                        });
                        e.insert(Pane::Term(v.clone()));
                        v
                    }
                };
                div().size_full().child(view).into_any()
            }
            TabKind::NewTab => {
                let store = self.store.clone();
                let view = self.view_pane(tab.id.clone(), move |tab_id, w, cx| {
                    NewTabView::new(store.clone(), tab_id, w, cx).into()
                }, window, cx);
                div().size_full().child(view).into_any()
            }
            TabKind::Settings => {
                let store = self.store.clone();
                let view = self.view_pane(tab.id.clone(), move |tab_id, w, cx| {
                    SettingsView::new(store.clone(), tab_id, w, cx).into()
                }, window, cx);
                div().size_full().child(view).into_any()
            }
            TabKind::Diff => {
                let view = self.view_pane(tab.id.clone(), |tab_id, w, cx| {
                    let path = std::env::var("HOME").unwrap_or_default() + "/repos";
                    DiffView::new(path, tab_id, w, cx).into()
                }, window, cx);
                div().size_full().child(view).into_any()
            }
            TabKind::Preview => {
                let view = self.view_pane(tab.id.clone(), |tab_id, w, cx| {
                    PreviewView::new(tab.url.clone(), tab_id, w, cx).into()
                }, window, cx);
                div().size_full().child(view).into_any()
            }
        };

        let focused_border = if focused {
            p.border_strong
        } else {
            gpui::transparent_white()
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w(px(120.))
            .min_h(px(80.))
            .overflow_hidden()
            .rounded(radius::CARD)
            .border_1()
            .border_color(focused_border)
            .bg(p.card.opacity(0.92))
            .child(header)
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(body),
            )
            .into_any()
    }

    /// One row of Radius-style chrome above every pane.
    fn pane_header(&mut self, tab: &hifi_core::Tab, cx: &mut Context<Self>) -> AnyElement {
        let p = Theme::of(cx).palette;
        let store = self.store.clone();
        let id = tab.id.clone();
        let id2 = tab.id.clone();
        let id3 = tab.id.clone();
        let id4 = tab.id.clone();
        let id5 = tab.id.clone();

        let kind_icon = match tab.kind {
            TabKind::Web => icons::GLOBE,
            TabKind::NewTab => icons::HOME,
            TabKind::Terminal | TabKind::Agent => mark_icon(&tab.command),
            TabKind::Diff => icons::GIT_BRANCH,
            TabKind::Preview => icons::EYE,
            TabKind::Settings => icons::SETTINGS,
        };

        let mut left = div().flex().items_center().gap(px(6.));
        if tab.kind == TabKind::Web {
            left = left
                .child(header_btn("nav-back", icons::ARROW_LEFT, tab.can_go_back, p, {
                    let store = store.clone();
                    let id = id.clone();
                    move |_, cx| {
                        store.update(cx, |s, cx| {
                            s.pending_back.push(id.clone());
                            cx.notify();
                        });
                    }
                }))
                .child(header_btn("nav-fwd", icons::ARROW_RIGHT, tab.can_go_forward, p, {
                    let store = store.clone();
                    let id = id.clone();
                    move |_, cx| {
                        store.update(cx, |s, cx| {
                            s.pending_forward.push(id.clone());
                            cx.notify();
                        });
                    }
                }))
                .child(header_btn("nav-reload", icons::REFRESH, true, p, {
                    let store = store.clone();
                    let id = id.clone();
                    move |_, cx| {
                        store.update(cx, |s, cx| {
                            s.pending_reload.push(id.clone());
                            cx.notify();
                        });
                    }
                }));
        }
        left = left.child(glyph(kind_icon, 13., p.faint));

        let title_text = match tab.kind {
            TabKind::Web => {
                if tab.title.is_empty() {
                    hifi_core::host_of(&tab.url)
                } else {
                    format!("{} — {}", tab.title, hifi_core::host_of(&tab.url))
                }
            }
            _ => tab.display_title(),
        };

        let mut header = div()
            .h(px(30.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_b_1()
            .border_color(p.border)
            .bg(p.shell.opacity(0.35))
            .child(left)
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(11.5))
                    .text_color(if tab.loading { p.accent } else { p.muted })
                    .child(if tab.loading {
                        format!("{title_text}…")
                    } else {
                        title_text
                    }),
            );

        // Actions: split right, split below, pin, close.
        header = header
            .child(header_btn("split-r", icons::SPLIT_COLUMNS, true, p, {
                let store = store.clone();
                move |_, cx| {
                    store.update(cx, |s, cx| {
                        s.open_tab(
                            "hifi://newtab",
                            None,
                            Some((id.clone(), hifi_core::SplitSide::Right)),
                            cx,
                        );
                    });
                }
            }))
            .child(header_btn("split-b", icons::FOLD_VERTICAL, true, p, {
                let store = store.clone();
                move |_, cx| {
                    store.update(cx, |s, cx| {
                        s.open_tab(
                            "hifi://newtab",
                            None,
                            Some((id2.clone(), hifi_core::SplitSide::Below)),
                            cx,
                        );
                    });
                }
            }))
            .child(header_btn("pin", icons::PIN, true, p, {
                let store = store.clone();
                move |_, cx| {
                    store.update(cx, |s, cx| s.toggle_pin(&id3, cx));
                }
            }))
            .child(header_btn("close", icons::CLOSE, true, p, {
                let store = store.clone();
                move |_, cx| {
                    store.update(cx, |s, cx| s.close_tab(&id4, cx));
                }
            }));
        let _ = id5;
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
                Pane::View(v) => v.clone(),
                _ => unreachable!(),
            },
            std::collections::hash_map::Entry::Vacant(e) => {
                let v: gpui::AnyView = cx.new(|cx| make(id, window, cx)).into();
                e.insert(Pane::View(v.clone()));
                v
            }
        }
    }

    /// Render a split node into nested flexes.
    fn render_node(
        &mut self,
        node: &SplitNode,
        active: Option<&TabId>,
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
                    if Some(t) != shown && let Some(Pane::Web(h)) = self.panes.get(t) {
                        h.hide();
                    }
                }
                match shown {
                    Some(tab_id) => {
                        self.render_leaf(tab_id, active == Some(tab_id), window, cx)
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
                let first_el = self.render_node(first, active, window, cx);
                let second_el = self.render_node(second, active, window, cx);
                let mut row = div()
                    .flex()
                    .size_full()
                    .gap(px(4.))
                    .min_h(px(0.))
                    .min_w(px(0.));
                row = if *direction == hifi_core::SplitDirection::Horizontal {
                    row.flex_row()
                } else {
                    row.flex_col()
                };
                return row
                    .child(
                        div()
                            .flex_basis(relative(f))
                            .flex_grow(1.)
                            .min_h(px(0.))
                            .min_w(px(0.))
                            .flex()
                            .child(first_el),
                    )
                    .child(
                        div()
                            .flex_basis(relative(1. - f))
                            .flex_grow(1.)
                            .min_h(px(0.))
                            .min_w(px(0.))
                            .flex()
                            .child(second_el),
                    )
                    .into_any();
            }
        }
    }

    fn titlebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = Theme::of(cx).palette;
        let store = self.store.clone();
        let active_id = self.store.read(cx).active_tab_id();
        let active_tab = active_id
            .as_ref()
            .and_then(|id| self.store.read(cx).state.tab(id).cloned());
        let collapsed = self.store.read(cx).sidebar_collapsed;

        let mut bar = div()
            .id("titlebar")
            .h(px(38.))
            .pl(px(76.)) // traffic lights
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .window_control_area(WindowControlArea::Drag);

        // Sidebar toggle.
        let store_t = store.clone();
        bar = bar.child(
            div()
                .id("tb-sidebar")
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(radius::ROUND)
                .cursor_pointer()
                .hover(|s| s.bg(p.raised.opacity(0.4)))
                .child(glyph(icons::SIDEBAR_LEFT, 14., p.muted))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    store_t.update(cx, |s, cx| {
                        s.sidebar_collapsed = !s.sidebar_collapsed;
                        cx.notify();
                    });
                }),
        );

        // Address pill (click → command bar).
        let url_label = active_tab
            .as_ref()
            .map(|t| match t.kind {
                TabKind::Web => t.url.clone(),
                _ => t.display_title(),
            })
            .unwrap_or_else(|| "Hi-Fi".into());
        let store_c = store.clone();
        bar = bar.child(
            div()
                .id("tb-url")
                .h(px(26.))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .rounded_full()
                .bg(p.card.opacity(0.6))
                .border_1()
                .border_color(p.border)
                .cursor_pointer()
                .flex_1()
                .max_w(px(560.))
                .mx_auto()
                .child(glyph(icons::LOCK, 11., p.faint))
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(12.))
                        .text_color(p.muted)
                        .child(url_label),
                )
                .child(glyph(icons::MAGNIFER, 12., p.faint))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    store_c.update(cx, |s, cx| s.toggle_command_bar(cx));
                }),
        );

        // Right cluster: new tab, command bar, settings.
        let store_r = store.clone();
        bar = bar
            .child(titlebar_button("tb-newtab", icons::PLUS, p, move |_, cx| {
                store_r.update(cx, |s, cx| {
                    s.open_tab("hifi://newtab", None, None, cx);
                });
            }))
            .child({
                let store = store.clone();
                titlebar_button("tb-palette", icons::COMMAND, p, move |_, cx| {
                    store.update(cx, |s, cx| s.toggle_command_bar(cx));
                })
            })
            .child({
                let store = store.clone();
                titlebar_button("tb-settings", icons::SETTINGS, p, move |_, cx| {
                    store.update(cx, |s, cx| {
                        s.open_tab("hifi://settings", None, None, cx);
                    });
                })
            });
        let _ = collapsed;
        bar.into_any()
    }
}

fn titlebar_button(
    id: &'static str,
    icon: &'static str,
    p: crate::theme::Palette,
    on_click: impl Fn(&gpui::MouseDownEvent, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(26.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(radius::ROUND)
        .cursor_pointer()
        .hover(|s| s.bg(p.raised.opacity(0.4)))
        .child(glyph(icon, 13., p.muted))
        .on_mouse_down(MouseButton::Left, move |event, _, cx| on_click(event, cx))
}

fn header_btn(
    id: &'static str,
    icon: &'static str,
    enabled: bool,
    p: crate::theme::Palette,
    on_click: impl Fn(&gpui::MouseDownEvent, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(id.to_string()))
        .size(px(22.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .cursor_pointer()
        .when(enabled, |d| {
            d.hover(|s| s.bg(p.raised.opacity(0.5)))
                .on_mouse_down(MouseButton::Left, move |event, _, cx| on_click(event, cx))
        })
        .child(glyph(icon, 12., if enabled { p.muted } else { p.faint.opacity(0.5) }))
}

impl gpui::Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.pump(window, cx);
        let p = Theme::of(cx).palette;
        let collapsed = self.store.read(cx).sidebar_collapsed;

        // Which web tabs are visible this frame; hide the rest.
        let visible_tabs: HashSet<TabId> = self
            .store
            .read(cx)
            .state
            .active_space()
            .and_then(|s| {
                let gid = s.active_group.clone().or_else(|| s.groups.first().map(|g| g.id.clone()))?;
                s.groups.iter().find(|g| g.id == gid).map(|g| g.tab_ids())
            })
            .unwrap_or_default()
            .into_iter()
            .collect();
        for (id, pane) in &self.panes {
            if let Pane::Web(h) = pane
                && !visible_tabs.contains(id)
            {
                h.hide();
            }
        }

        let root_node = self
            .store
            .read(cx)
            .state
            .active_space()
            .and_then(|s| {
                let gid = s.active_group.clone().or_else(|| s.groups.first().map(|g| g.id.clone()))?;
                s.groups
                    .iter()
                    .find(|g| g.id == gid)
                    .and_then(|g| g.root.clone())
            });

        let content: AnyElement = match root_node {
            Some(node) => {
                let active = self.store.read(cx).active_tab_id();
                self.render_node(&node, active.as_ref(), window, cx)
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

        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .text_color(p.text)
            .key_context("Shell")
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
                this.store.update(cx, |s, cx| s.toggle_command_bar(cx));
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
            .child(self.titlebar(cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.))
                    .child(div().h_full().when(!collapsed, |d| {
                        d.child(self.sidebar.clone())
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .h_full()
                            .p(px(6.))
                            .child(content),
                    ),
            );

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
