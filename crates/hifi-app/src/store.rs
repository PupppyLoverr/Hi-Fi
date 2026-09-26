//! `Store` — the app's central state entity. Wraps `WorkspaceState`, performs
//! all mutations, and persists to `state.json` on every change.

use gpui::{App, AppContext, Context, Entity};
use hifi_core::{
    Group, GroupId, HifiPaths, RoutedUrl, Settings, Space, SplitSide, Tab, TabId, TabKind,
    WorkspaceState, route,
};

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub title: String,
    pub url: String,
    /// Written for future history UI.
    #[allow(dead_code)]
    pub visited_at: i64,
}

pub struct Store {
    pub state: WorkspaceState,
    pub paths: HifiPaths,
    pub command_bar_open: bool,
    pub find_bar_open: bool,
    pub sidebar_collapsed: bool,
    pub history: Vec<HistoryEntry>,
    /// Pending navigations the webview layer picks up (drained by WebPaneView).
    pub pending_navs: Vec<(TabId, String)>,
    /// Tabs that must reload on next frame.
    pub pending_reload: Vec<TabId>,
    pub pending_back: Vec<TabId>,
    pub pending_forward: Vec<TabId>,
    /// Raised on IPC open — shell scrolls new tab into view.
    pub last_opened: Option<TabId>,
}

impl Store {
    pub fn load(cx: &mut App) -> Entity<Store> {
        let paths = HifiPaths::detect();
        let _ = paths.ensure_dirs();
        let file = hifi_core::StateFile::new(paths.state_file());
        let mut state = file.load();
        if state.spaces.is_empty() {
            let mut space = Space::new("Personal");
            let mut group = Group::new("Tabs");
            group.color = "#8b7cf6".into();
            let tab = Tab::new(TabKind::NewTab, "hifi://newtab", group.id.clone());
            state.tabs.push(tab.clone());
            group.add_tab(tab.id.clone());
            space.groups.push(group.clone());
            space.active_group = Some(group.id);
            state.spaces.push(space);
            state.active_space = state.spaces.first().map(|s| s.id.clone());
        }
        cx.new(|_| Store {
            state,
            paths,
            command_bar_open: false,
            find_bar_open: false,
            sidebar_collapsed: false,
            history: Vec::new(),
            pending_navs: Vec::new(),
            pending_reload: Vec::new(),
            pending_back: Vec::new(),
            pending_forward: Vec::new(),
            last_opened: None,
        })
    }

    pub fn save(&self) {
        let _ = hifi_core::StateFile::new(self.paths.state_file()).save(&self.state);
    }

    #[allow(dead_code)] // part of the store API; views read settings directly
    pub fn settings(&self) -> &Settings {
        &self.state.settings
    }

    // ----- tab ops -----

    pub fn open_tab(
        &mut self,
        url_or_kind: &str,
        group_id: Option<GroupId>,
        split_anchor: Option<(TabId, SplitSide)>,
        cx: &mut Context<Self>,
    ) -> TabId {
        let (kind, url) = match route(url_or_kind) {
            RoutedUrl::Internal(kind) => (kind, url_or_kind.to_string()),
            RoutedUrl::Preview(path) => (TabKind::Preview, path),
            RoutedUrl::External(url) => (TabKind::Web, url),
        };
        let group_id = group_id
            .or_else(|| {
                self.state
                    .active_space()
                    .and_then(|s| s.active_group.clone().or_else(|| s.groups.first().map(|g| g.id.clone())))
            })
            .or_else(|| Some(self.create_group("Tabs", None, cx)));
        let Some(group_id) = group_id else {
            return String::new();
        };
        let tab = Tab::new(kind, url, group_id.clone());
        let id = tab.id.clone();
        self.state.tabs.push(tab);
        if let Some(group) = self.state.group_mut(&group_id) {
            match split_anchor {
                Some((anchor, side)) => {
                    if let Some(root) = group.root.as_mut() {
                        root.split(&anchor, id.clone(), side);
                    } else {
                        group.add_tab(id.clone());
                    }
                    group.active_tab = Some(id.clone());
                }
                None => group.add_tab(id.clone()),
            }
        }
        self.last_opened = Some(id.clone());
        self.save();
        cx.notify();
        id
    }

    pub fn close_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        self.state.tabs.retain(|t| t.id != id);
        for space in &mut self.state.spaces {
            for group in &mut space.groups {
                if let Some(root) = &mut group.root {
                    root.remove(&id.to_string());
                    if root.leaves().is_empty() {
                        group.root = None;
                    }
                }
                if group.active_tab.as_deref() == Some(id) {
                    group.active_tab = group
                        .root
                        .as_ref()
                        .and_then(|r| r.leaves().into_iter().next());
                }
            }
        }
        self.save();
        cx.notify();
    }

    pub fn focus_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(tab) = self.state.tab(id) else { return };
        let gid = tab.group_id.clone();
        if let Some(group) = self.state.group_mut(&gid) {
            group.active_tab = Some(id.to_string());
        }
        self.save();
        cx.notify();
    }

    pub fn set_active_group(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(space) = self.state.active_space_mut() {
            space.active_group = Some(id.to_string());
        }
        self.save();
        cx.notify();
    }

    pub fn navigate(&mut self, id: &str, url: &str, cx: &mut Context<Self>) {
        if let Some(tab) = self.state.tab_mut(id) {
            tab.url = url.to_string();
            tab.loading = true;
            if url.starts_with("http://") || url.starts_with("https://") {
                tab.kind = TabKind::Web;
            }
        }
        self.pending_navs.push((id.to_string(), url.to_string()));
        self.save();
        cx.notify();
    }

    pub fn tab_meta_update(
        &mut self,
        id: &str,
        title: Option<String>,
        url: Option<String>,
        loading: Option<bool>,
        can_back: Option<bool>,
        can_fwd: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let mut record_history = None;
        if let Some(tab) = self.state.tab_mut(id) {
            if let Some(t) = title {
                tab.title = t;
            }
            if let Some(u) = url {
                if tab.url != u {
                    record_history = Some((tab.title.clone(), u.clone()));
                }
                tab.url = u;
            }
            if let Some(l) = loading {
                tab.loading = l;
            }
            if let Some(v) = can_back {
                tab.can_go_back = v;
            }
            if let Some(v) = can_fwd {
                tab.can_go_forward = v;
            }
        }
        if let Some((title, url)) = record_history {
            self.history.push(HistoryEntry {
                title,
                url,
                visited_at: chrono::Utc::now().timestamp(),
            });
            if self.history.len() > 500 {
                self.history.drain(0..self.history.len() - 500);
            }
        }
        self.save();
        cx.notify();
    }

    pub fn toggle_pin(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(tab) = self.state.tab_mut(id) {
            tab.pinned = !tab.pinned;
        }
        self.save();
        cx.notify();
    }

    pub fn create_group(
        &mut self,
        name: &str,
        project_path: Option<String>,
        cx: &mut Context<Self>,
    ) -> GroupId {
        let mut group = Group::new(name);
        if let Some(p) = project_path {
            group.project_path = p;
        }
        let id = group.id.clone();
        if let Some(space) = self.state.active_space_mut() {
            space.groups.push(group);
            space.active_group = Some(id.clone());
        }
        self.save();
        cx.notify();
        id
    }

    pub fn create_space(&mut self, name: &str, cx: &mut Context<Self>) {
        let mut space = Space::new(name);
        let mut group = Group::new("Tabs");
        group.color = "#8b7cf6".into();
        space.active_group = Some(group.id.clone());
        space.groups.push(group);
        let id = space.id.clone();
        self.state.spaces.push(space);
        self.state.active_space = Some(id);
        self.save();
        cx.notify();
    }

    pub fn switch_space(&mut self, id: &str, cx: &mut Context<Self>) {
        self.state.active_space = Some(id.to_string());
        self.save();
        cx.notify();
    }

    pub fn update_settings(&mut self, f: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        f(&mut self.state.settings);
        self.save();
        cx.notify();
    }

    pub fn toggle_command_bar(&mut self, cx: &mut Context<Self>) {
        self.command_bar_open = !self.command_bar_open;
        cx.notify();
    }

    /// The tab the content area should show for a group.
    pub fn active_tab_of(&self, group_id: &str) -> Option<TabId> {
        let group = self.state.group(group_id)?;
        group.active_tab.clone().or_else(|| {
            group
                .root
                .as_ref()
                .and_then(|r| r.leaves().into_iter().next())
        })
    }

    /// The active tab of the active group of the active space.
    pub fn active_tab_id(&self) -> Option<TabId> {
        let space = self.state.active_space()?;
        let gid = space
            .active_group
            .clone()
            .or_else(|| space.groups.first().map(|g| g.id.clone()))?;
        self.active_tab_of(&gid)
    }

    /// Cycle the active group's selection by ±1 (wraps around all tabs in the group).
    pub fn cycle_tab(&mut self, dir: i32, cx: &mut Context<Self>) {
        let Some(space) = self.state.active_space() else { return };
        let Some(gid) = space
            .active_group
            .clone()
            .or_else(|| space.groups.first().map(|g| g.id.clone()))
        else {
            return;
        };
        let Some(group) = space.groups.iter().find(|g| g.id == gid) else {
            return;
        };
        let ids = group.tab_ids();
        if ids.is_empty() {
            return;
        }
        let cur = group
            .active_tab
            .as_ref()
            .and_then(|a| ids.iter().position(|i| i == a))
            .unwrap_or(0) as i32;
        let next = (cur + dir).rem_euclid(ids.len() as i32) as usize;
        let id = ids[next].clone();
        self.focus_tab(&id, cx);
    }

    /// Open a typed string: internal schemes open their pane; anything else
    /// becomes a web tab (URL or search via the configured engine).
    pub fn open_tab_or_navigate(&mut self, text: &str, cx: &mut Context<Self>) -> TabId {
        let url = match crate::resolve_for_open(self, text) {
            Some(u) => u,
            None => text.to_string(),
        };
        self.open_tab(&url, None, None, cx)
    }
}
