//! Browser domain model: tabs, groups, spaces, split trees, settings.

use serde::{Deserialize, Serialize};

pub type TabId = String;
pub type GroupId = String;
pub type SpaceId = String;

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabKind {
    Web,
    #[serde(alias = "newtab")]
    NewTab,
    Terminal,
    Agent,
    Diff,
    Preview,
    Settings,
    Notes,
}

impl TabKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::NewTab => "newtab",
            Self::Terminal => "terminal",
            Self::Agent => "agent",
            Self::Diff => "diff",
            Self::Preview => "preview",
            Self::Settings => "settings",
            Self::Notes => "notes",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    pub id: TabId,
    pub kind: TabKind,
    /// URL for web tabs; working path / label for panes.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub loading: bool,
    /// For terminal/agent tabs: shell/harness command (e.g. "-zsh", "claude").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    /// Working directory for terminal/agent/diff tabs.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    /// Git branch associated with the tab's worktree, if any.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    /// Group this tab belongs to (a tab lives in exactly one group).
    #[serde(default)]
    pub group_id: GroupId,
    #[serde(default)]
    pub can_go_back: bool,
    #[serde(default)]
    pub can_go_forward: bool,
    /// Favicon URL resolved by the webview, for sidebar/speed-dial tiles.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub favicon_url: String,
}

impl Tab {
    pub fn new(kind: TabKind, url: impl Into<String>, group_id: GroupId) -> Self {
        Self {
            id: new_id(),
            kind,
            url: url.into(),
            title: String::new(),
            pinned: false,
            muted: false,
            loading: false,
            command: String::new(),
            cwd: String::new(),
            branch: String::new(),
            group_id,
            can_go_back: false,
            can_go_forward: false,
            favicon_url: String::new(),
        }
    }

    /// Sidebar label: Radius-style `Kind/Title` for pane tabs.
    pub fn display_title(&self) -> String {
        match self.kind {
            TabKind::Web | TabKind::NewTab | TabKind::Settings | TabKind::Notes => {
                if self.title.is_empty() {
                    match self.kind {
                        TabKind::NewTab => "New Tab".into(),
                        TabKind::Settings => "Settings".into(),
                        TabKind::Notes => "Notes".into(),
                        _ => host_of(&self.url),
                    }
                } else {
                    self.title.clone()
                }
            }
            TabKind::Terminal | TabKind::Agent | TabKind::Diff | TabKind::Preview => {
                let kind = self.kind.as_str();
                let mut cap = kind.to_string();
                if let Some(c) = cap.get_mut(0..1) {
                    c.make_ascii_uppercase();
                }
                if self.title.is_empty() || self.title == cap {
                    cap
                } else {
                    format!("{cap}/{}", self.title)
                }
            }
        }
    }
}

pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| url.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitSide {
    Left,
    Right,
    Above,
    Below,
}

impl SplitSide {
    pub fn direction(self) -> SplitDirection {
        match self {
            Self::Left | Self::Right => SplitDirection::Horizontal,
            Self::Above | Self::Below => SplitDirection::Vertical,
        }
    }
    pub fn places_first(self) -> bool {
        matches!(self, Self::Left | Self::Above)
    }
}

/// A binary tree of leaf tab-ids with fractional sizes. Serializable so the
/// exact split layout restores across launches.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SplitNode {
    /// A pane region holding a stack of tabs; the group's `active_tab`
    /// decides which tab of the stack renders when it lives in this leaf.
    Leaf {
        #[serde(alias = "tab_id")]
        tabs: Vec<TabId>,
    },
    Split {
        direction: SplitDirection,
        /// Fraction [0.05, 0.95] given to `first`.
        fraction: f32,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

impl SplitNode {
    pub fn leaf(id: TabId) -> Self {
        Self::Leaf { tabs: vec![id] }
    }

    pub fn leaves(&self) -> Vec<TabId> {
        match self {
            Self::Leaf { tabs } => tabs.clone(),
            Self::Split { first, second, .. } => {
                let mut v = first.leaves();
                v.extend(second.leaves());
                v
            }
        }
    }

    /// The tab stack of the leaf containing `tab`.
    pub fn leaf_tabs_mut(&mut self, tab: &TabId) -> Option<&mut Vec<TabId>> {
        match self {
            Self::Leaf { tabs } if tabs.iter().any(|t| t == tab) => Some(tabs),
            Self::Leaf { .. } => None,
            Self::Split { first, second, .. } => first
                .leaf_tabs_mut(tab)
                .or_else(|| second.leaf_tabs_mut(tab)),
        }
    }

    /// Split the leaf holding `anchor`, placing `new_tab` as its own leaf on
    /// `side`. The anchor leaf keeps its whole stack. `new_fraction` is the
    /// share of the region the NEW leaf takes (clamped to 5–95%).
    pub fn split(
        &mut self,
        anchor: &TabId,
        new_tab: TabId,
        side: SplitSide,
        new_fraction: f32,
    ) -> bool {
        match self {
            Self::Leaf { tabs } if tabs.iter().any(|t| t == anchor) => {
                let anchor_leaf = Self::Leaf { tabs: tabs.clone() };
                let new_leaf = Self::Leaf {
                    tabs: vec![new_tab],
                };
                let share = new_fraction.clamp(0.05, 0.95);
                let (first, second, fraction) = if side.places_first() {
                    (new_leaf, anchor_leaf, share)
                } else {
                    (anchor_leaf, new_leaf, 1.0 - share)
                };
                *self = Self::Split {
                    direction: side.direction(),
                    fraction,
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            Self::Leaf { .. } => false,
            Self::Split { first, second, .. } => {
                first.split(anchor, new_tab.clone(), side, new_fraction)
                    || second.split(anchor, new_tab, side, new_fraction)
            }
        }
    }

    /// The fraction field of the Split node reached by `path`
    /// (0 = descend into `first`, 1 = into `second`).
    pub fn split_at_path<'a>(&'a mut self, path: &[u8]) -> Option<&'a mut f32> {
        match self {
            Self::Split {
                fraction,
                first,
                second,
                ..
            } if path.is_empty() => Some(fraction),
            Self::Split { first, second, .. } => match path[0] {
                0 => first.split_at_path(&path[1..]),
                _ => second.split_at_path(&path[1..]),
            },
            Self::Leaf { .. } => None,
        }
    }

    /// Remove `id` from its leaf; collapses leaves that become empty.
    /// Returns true if the tab was found.
    pub fn remove(&mut self, id: &TabId) -> bool {
        match self {
            Self::Leaf { tabs } => {
                let n = tabs.len();
                tabs.retain(|t| t != id);
                tabs.len() != n
            }
            Self::Split { first, second, .. } => {
                let removed = first.remove(id) || second.remove(id);
                if first.leaves().is_empty() {
                    *self = std::mem::replace(&mut **second, Self::Leaf { tabs: vec![] });
                } else if second.leaves().is_empty() {
                    *self = std::mem::replace(&mut **first, Self::Leaf { tabs: vec![] });
                }
                removed
            }
        }
    }

    pub fn contains(&self, id: &TabId) -> bool {
        self.leaves().iter().any(|t| t == id)
    }
}

/// A tab collection with a split layout — Radius's "group".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    /// Hex tint, e.g. "#8b7cf6".
    #[serde(default)]
    pub color: String,
    /// Dev-group project path (for terminals/diffs/worktrees).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project_path: String,
    /// Root of the split tree. `None` when empty.
    #[serde(default)]
    pub root: Option<SplitNode>,
    /// The cosmos-style right surface host: its own tab stack behind a chip
    /// strip, hidden by default. Per-group like cosmos's per-session panes.
    #[serde(default)]
    pub dock: RightDock,
    /// The pane tab currently focused in this group.
    #[serde(default)]
    pub active_tab: Option<TabId>,
    /// Permission granted for automation (browser-use) on this group.
    #[serde(default)]
    pub automation_granted: bool,
}

impl Group {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            color: String::new(),
            project_path: String::new(),
            root: None,
            dock: RightDock::default(),
            active_tab: None,
            automation_granted: false,
        }
    }

    pub fn tab_ids(&self) -> Vec<TabId> {
        self.root.as_ref().map(|r| r.leaves()).unwrap_or_default()
    }

    /// Stack a tab into the leaf holding the active tab (or the first leaf);
    /// it becomes the group's active tab.
    pub fn add_tab(&mut self, tab_id: TabId) {
        match &mut self.root {
            None => self.root = Some(SplitNode::leaf(tab_id.clone())),
            Some(root) => {
                let anchor = self
                    .active_tab
                    .clone()
                    .or_else(|| root.leaves().first().cloned());
                if let Some(a) = anchor
                    && let Some(tabs) = root.leaf_tabs_mut(&a)
                {
                    tabs.push(tab_id.clone());
                }
            }
        }
        self.active_tab = Some(tab_id);
    }
}

/// Right surface dock — cosmos's RightPanelTabs model: a stack of surface
/// tabs (terminal, notes, diff, …) rendered in a resizable right column.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RightDock {
    #[serde(default)]
    pub open: bool,
    /// Column width in px (clamped at layout to 360..viewport-300).
    #[serde(default = "default_dock_width")]
    pub width: f32,
    /// Surface tab ids, chip order.
    #[serde(default)]
    pub tabs: Vec<TabId>,
    #[serde(default)]
    pub active: Option<TabId>,
}

fn default_dock_width() -> f32 {
    460.0
}

impl Default for RightDock {
    fn default() -> Self {
        Self {
            open: false,
            width: default_dock_width(),
            tabs: Vec::new(),
            active: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub id: SpaceId,
    pub name: String,
    pub icon: String,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub active_group: Option<GroupId>,
}

impl Space {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            icon: "folder".into(),
            groups: Vec::new(),
            active_group: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Appearance {
    Dark,
    Light,
    #[default]
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub appearance: Appearance,
    /// Accent family name from the cosmos palette.
    pub accent: String,
    /// Optional custom new-tab background image (file path).
    #[serde(default)]
    pub background_image: String,
    #[serde(default = "default_blur")]
    pub background_blur: f32,
    #[serde(default)]
    pub background_dim: f32,
    #[serde(default)]
    pub compact_sidebar: bool,
    /// Sidebar width in px (cosmos: 224–400, default 256).
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    /// Restore the previous session on launch.
    #[serde(default = "default_true")]
    pub restore_session: bool,
    /// Search engine query template ("{q}" placeholder).
    #[serde(default = "default_search")]
    pub search_engine: String,
}

fn default_blur() -> f32 {
    24.0
}
fn default_true() -> bool {
    true
}
fn default_sidebar_width() -> f32 {
    256.0
}
fn default_search() -> String {
    "https://duckduckgo.com/?q={q}".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            accent: "violet".into(),
            background_image: String::new(),
            background_blur: default_blur(),
            background_dim: 0.0,
            compact_sidebar: false,
            sidebar_width: default_sidebar_width(),
            restore_session: true,
            search_engine: default_search(),
        }
    }
}

/// Everything persisted to `state.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceState {
    #[serde(default)]
    pub spaces: Vec<Space>,
    #[serde(default)]
    pub active_space: Option<SpaceId>,
    /// Flat tab table; groups reference ids in their split trees.
    #[serde(default)]
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub settings: Settings,
}

impl WorkspaceState {
    pub fn tab(&self, id: &str) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }
    pub fn tab_mut(&mut self, id: &str) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }
    pub fn group(&self, id: &str) -> Option<&Group> {
        self.spaces
            .iter()
            .flat_map(|s| &s.groups)
            .find(|g| g.id == id)
    }
    pub fn group_mut(&mut self, id: &str) -> Option<&mut Group> {
        self.spaces
            .iter_mut()
            .flat_map(|s| &mut s.groups)
            .find(|g| g.id == id)
    }
    pub fn active_space(&self) -> Option<&Space> {
        self.active_space
            .as_ref()
            .and_then(|id| self.spaces.iter().find(|s| &s.id == id))
            .or(self.spaces.first())
    }
    pub fn active_space_mut(&mut self) -> Option<&mut Space> {
        let id = self.active_space.clone();
        if let Some(i) = self.spaces.iter().position(|s| Some(&s.id) == id.as_ref()) {
            return self.spaces.get_mut(i);
        }
        self.spaces.first_mut()
    }
}
