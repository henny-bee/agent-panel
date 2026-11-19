//! Viewport state: closing a tab never deletes or stops a session.
//! A title tab is keyed by one of its sessions; a tab holding more than one
//! pane keeps its split tree in `layouts` and its last focused pane in
//! `focused`. `active` is always the focused session.
mod panes;
mod restore;
use crate::runtime::pane_layout::{Edge, PaneNode, SplitDir, leaf};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum AppView {
    Inbox,
    Notes,
    Automations,
}
impl AppView {
    pub fn label(self) -> &'static str {
        match self {
            Self::Inbox => "Inbox",
            Self::Notes => "Notes",
            Self::Automations => "Automations",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Notes => "note",
            Self::Automations => "zap",
        }
    }
}

/// The title strip has one order, including conversations, app views and Monos.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum TitleTab {
    Session(i64),
    Search,
    App(AppView),
    Mono {
        id: String,
        project: String,
        worktree: String,
    },
}

impl TitleTab {
    pub fn session(&self) -> Option<i64> {
        match self {
            Self::Session(id) => Some(*id),
            Self::Mono { .. } | Self::Search | Self::App(_) => None,
        }
    }
}

/// A named section of projects in the original project navigation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct ProjectRailGroup {
    pub id: String,
    pub name: String,
    pub collapsed: bool,
    pub color_index: Option<usize>,
    pub custom_color: Option<String>,
    pub mascot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Workspace {
    pub window: crate::app::window_state::WindowState,
    pub tabs: Vec<i64>,
    pub title_order: Vec<TitleTab>,
    /// Tabs pinned to the strip's left edge as icon cards, in pin order.
    /// Each stays in `title_order` too; this only marks and orders them.
    pub pinned_tabs: Vec<TitleTab>,
    pub scope_focus: HashMap<String, TitleTab>,
    /// Sidebar narrowing belongs to this run. Reopen on the full project,
    /// while saved conversations retain their own execution directories.
    #[serde(skip)]
    pub worktree_focus: HashMap<String, crate::git::worktree_switcher::WorktreeFocus>,
    /// Directly opened chats join the workspace on screen without changing
    /// their execution directory. Like workspace selection, these pins are local
    /// to the current run and disappear when their tabs close.
    #[serde(skip)]
    pub tab_workspaces: HashMap<i64, String>,
    pub active: Option<i64>,
    pub projects: Vec<String>,
    /// Projects archived from the rail, newest first. Their chats stay in
    /// history; adding the project again takes it out of this list.
    pub archived_projects: Vec<String>,
    pub project: String,
    /// Last focused open session in each project, independent of tab order.
    pub project_focus: HashMap<String, i64>,
    pub recent_files: HashMap<String, Vec<String>>,
    pub collapsed: HashSet<String>,
    pub sidebar_visible: bool,
    pub sidebar_width: f32,
    pub compact_session_cards: bool,
    /// The Search field heading the sidebar; ⌘K opens search either way.
    pub sidebar_search_visible: bool,
    pub sidebar_notes_visible: bool,
    pub sidebar_automations_visible: bool,
    pub sidebar_inbox_visible: bool,
    pub sidebar_activity_visible: bool,
    /// The inset right panel: open state, tab, width and tab order.
    pub right_panel_open: bool,
    pub right_panel_tab: crate::ui::panel::RightTab,
    pub right_panel_width: f32,
    pub right_panel_tabs: Vec<crate::ui::panel::RightTab>,
    /// The Claude view the panel's Explorer shows in place of the files.
    pub right_panel_claude: Option<crate::chat::claude::panels::ClaudeTab>,
    /// Each split pane's own project panel, by pane id.
    pub pane_panels: HashMap<i64, PanePanel>,
    /// A folder picked in a chat's panel, shown there instead of the chat's
    /// own (Explorer, Changes, History and the change count), by pane id.
    pub panel_folders: HashMap<i64, String>,
    pub changes_view: crate::git::changes::ChangesView,
    /// The section under Changes shows CODE's languages instead of GRAPH.
    pub graph_code: bool,
    /// Each tab's file/review pane beside its chat.
    pub reviews: std::collections::HashMap<i64, crate::git::review::ReviewState>,
    /// Independent file panes use negative IDs in the layout tree. Their
    /// originating session supplies project/worktree scope and Add to Chat.
    pub review_owners: HashMap<i64, i64>,
    /// Monos dragged into a split, by their negative pane IDs. Each keeps the
    /// Mono tab it came from, which it becomes again once it stands alone.
    pub mono_panes: HashMap<i64, TitleTab>,
    /// Terminals pulled out of a dock into a split or onto the title strip,
    /// by their negative pane IDs. Like file panes, each borrows its owner
    /// session's project and worktree scope.
    pub terminal_panes: HashMap<i64, TerminalPane>,
    /// Split trees of tabs with more than one pane, by tab key.
    pub layouts: HashMap<i64, PaneNode>,
    /// The pane each split tab returns to when it is activated.
    pub focused: HashMap<i64, i64>,
    /// Each project's terminal dock (ProjectTerminalDock), by project path.
    pub docks: HashMap<String, TerminalDock>,
    /// Each project's saved commands, run from the footer's Actions button.
    pub actions: HashMap<String, Vec<ProjectAction>>,
    /// The id of the action each project last ran; the footer shows it.
    pub last_actions: HashMap<String, String>,
    /// Original layout: separate project and session sidebars.
    pub classic_layout: bool,
    /// The new single-sidebar layout can keep its full-height, square frame.
    pub new_layout_boxy: bool,
    pub project_rail_open: bool,
    pub project_rail_monos_pinned: bool,
    pub project_rail_width: f32,
    pub project_rail_pinned: HashSet<String>,
    pub project_rail_groups: Vec<ProjectRailGroup>,
    pub project_rail_assignments: HashMap<String, String>,
    pub classic_sidebar_tabs: HashMap<String, String>,
    pub classic_sidebar_tab_order: Vec<String>,
    pub new_layout_prompt_dismissed: bool,
    /// The rail stops asking for a GitHub star once dismissed.
    pub github_star_prompt_dismissed: bool,
    pub preferences: crate::ui::preferences::Preferences,
    pub session_filters: monocode_core::session::filters::Filters,
    pub project_appearance: HashMap<String, monocode_core::project::appearance::ProjectAppearance>,
    /// Last desktop source values, used to reconcile subsequent desktop changes
    /// while retaining deliberate native project and preference edits.
    pub desktop_project_preferences: Option<serde_json::Value>,
    /// Unmodified desktop viewport metadata, including pane types and explicit
    /// tab-group assignments that do not yet have a native representation.
    pub desktop_workspace_snapshot: Option<serde_json::Value>,
    /// Exact source strings retained for preferences with no native equivalent.
    pub desktop_preferences_metadata: Option<serde_json::Value>,
}

/// A saved command the footer runs in a terminal with one click.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ProjectAction {
    pub id: String,
    pub name: String,
    pub command: String,
    /// One of `ACTION_ICONS`.
    pub icon: String,
}
/// The icons an action can wear, in picker order; the first is the default.
pub(crate) const ACTION_ICONS: [&str; 6] =
    ["play", "test-tube", "hammer", "rocket", "bug", "package"];
impl ProjectAction {
    pub fn icon(&self) -> &'static str {
        ACTION_ICONS
            .iter()
            .find(|icon| **icon == self.icon)
            .copied()
            .unwrap_or(ACTION_ICONS[0])
    }
}

/// Port of src/features/projects/model/projectTerminal.ts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DockSide {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}
impl DockSide {
    pub fn vertical(self) -> bool {
        matches!(self, DockSide::Top | DockSide::Bottom)
    }
    /// DOCK_SIZE_DEFAULT.
    pub fn default_size(self) -> f32 {
        if self.vertical() { 220. } else { 360. }
    }
    /// clampDockSize: at least a usable strip, at most 70% of the window.
    pub fn clamp(self, value: f32, viewport: (f32, f32)) -> f32 {
        if !value.is_finite() {
            return self.default_size();
        }
        let (min, max) = self.limits(viewport);
        value.round().clamp(min, max)
    }
    pub fn limits(self, viewport: (f32, f32)) -> (f32, f32) {
        let (min, span) = if self.vertical() {
            (88., viewport.1)
        } else {
            (180., viewport.0)
        };
        (min, (span * 0.7).floor().max(min))
    }
}
/// A terminal living in the pane tree. Shells are not restored; the pane
/// reopens a fresh shell in the same directory, as a dock tab does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TerminalPane {
    pub owner: i64,
    pub cwd: String,
}

/// A split pane's project panel: shown or not, its tab, the Claude view its
/// Explorer shows and its width.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PanePanel {
    pub open: bool,
    pub tab: crate::ui::panel::RightTab,
    pub claude: Option<crate::chat::claude::panels::ClaudeTab>,
    pub width: f32,
}
impl Default for PanePanel {
    fn default() -> Self {
        Self {
            open: false,
            tab: Default::default(),
            claude: None,
            width: crate::ui::panel::WIDTH_DEFAULT,
        }
    }
}

/// A project's dock: its place, size and the terminals it holds. Shells are
/// not restored; the dock reopens the same tabs in their directories.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TerminalDock {
    pub side: DockSide,
    pub size: f32,
    pub open: bool,
    /// Each terminal's working directory, in tab order.
    pub terminals: Vec<String>,
    pub active: usize,
}
impl Default for TerminalDock {
    fn default() -> Self {
        Self {
            side: DockSide::Bottom,
            size: DockSide::Bottom.default_size(),
            open: true,
            terminals: Vec::new(),
            active: 0,
        }
    }
}
impl TerminalDock {
    /// addTerminalToDock: the new terminal is active and the dock opens.
    pub fn add(&mut self, cwd: String) -> usize {
        self.terminals.push(cwd);
        self.active = self.terminals.len() - 1;
        self.open = true;
        self.active
    }
    /// closeTerminalInDock: the next tab takes over a closed active one.
    /// False once the last terminal is gone.
    pub fn close(&mut self, ix: usize) -> bool {
        if ix >= self.terminals.len() {
            return true;
        }
        self.terminals.remove(ix);
        if self.terminals.is_empty() {
            self.active = 0;
            return false;
        }
        if self.active > ix || self.active >= self.terminals.len() {
            self.active = self.active.saturating_sub(1).min(self.terminals.len() - 1);
        }
        true
    }
    /// onReorderTerminals: the active terminal stays active wherever it lands.
    pub fn reorder(&mut self, from: usize, to: usize) {
        let len = self.terminals.len();
        if from >= len || to >= len || from == to {
            return;
        }
        let active = self.active;
        let cwd = self.terminals.remove(from);
        self.terminals.insert(to, cwd);
        self.active = if active == from {
            to
        } else if from < active && active <= to {
            active - 1
        } else if to <= active && active < from {
            active + 1
        } else {
            active
        };
    }
    /// onCloseOtherProjectTerminals: `keep` is left, and active.
    pub fn keep_only(&mut self, keep: usize) {
        if keep < self.terminals.len() {
            self.terminals = vec![self.terminals.swap_remove(keep)];
            self.active = 0;
        }
    }
    pub fn set_side(&mut self, side: DockSide) {
        if side != self.side {
            // A dock moved across axes starts from that axis's default.
            if side.vertical() != self.side.vertical() {
                self.size = side.default_size();
            }
            self.side = side;
        }
    }
}
impl Default for Workspace {
    fn default() -> Self {
        Self {
            window: Default::default(),
            tabs: Vec::new(),
            title_order: Vec::new(),
            pinned_tabs: Vec::new(),
            scope_focus: HashMap::new(),
            worktree_focus: HashMap::new(),
            tab_workspaces: HashMap::new(),
            active: None,
            projects: Vec::new(),
            archived_projects: Vec::new(),
            project: String::new(),
            project_focus: HashMap::new(),
            recent_files: HashMap::new(),
            collapsed: HashSet::new(),
            sidebar_visible: true,
            sidebar_width: 260.,
            compact_session_cards: true,
            sidebar_search_visible: true,
            sidebar_notes_visible: true,
            sidebar_automations_visible: true,
            sidebar_inbox_visible: true,
            sidebar_activity_visible: true,
            right_panel_open: false,
            right_panel_tab: Default::default(),
            right_panel_width: crate::ui::panel::WIDTH_DEFAULT,
            right_panel_tabs: crate::ui::panel::RightTab::normalize(&[]),
            right_panel_claude: None,
            pane_panels: Default::default(),
            panel_folders: Default::default(),
            changes_view: Default::default(),
            graph_code: false,
            reviews: Default::default(),
            review_owners: Default::default(),
            terminal_panes: Default::default(),
            mono_panes: Default::default(),
            layouts: Default::default(),
            focused: Default::default(),
            docks: Default::default(),
            actions: HashMap::new(),
            last_actions: HashMap::new(),
            classic_layout: false,
            // Fresh installs start on the boxy frame; saved choices are kept.
            new_layout_boxy: true,
            project_rail_open: true,
            project_rail_monos_pinned: false,
            project_rail_width: 220.,
            project_rail_pinned: HashSet::new(),
            project_rail_groups: Vec::new(),
            project_rail_assignments: HashMap::new(),
            classic_sidebar_tabs: HashMap::new(),
            classic_sidebar_tab_order: Vec::new(),
            new_layout_prompt_dismissed: false,
            github_star_prompt_dismissed: false,
            preferences: crate::ui::preferences::Preferences::default(),
            session_filters: Default::default(),
            project_appearance: Default::default(),
            desktop_project_preferences: None,
            desktop_workspace_snapshot: None,
            desktop_preferences_metadata: None,
        }
    }
}
impl Workspace {
    pub fn add_project(&mut self, path: &str) {
        self.archived_projects.retain(|p| p != path);
        if !self.projects.iter().any(|p| p == path) {
            self.projects.push(path.into());
        }
    }
    /// The rail group a project is listed under. Assignments to deleted
    /// groups leave the project in the ungrouped list.
    pub fn project_rail_group(&self, path: &str) -> Option<&str> {
        let id = self.project_rail_assignments.get(path)?;
        self.project_rail_groups
            .iter()
            .any(|group| &group.id == id)
            .then_some(id.as_str())
    }
    /// Whether two projects sit in the same rail section and reorder together.
    pub fn project_rail_peers(&self, a: &str, b: &str) -> bool {
        let pinned = self.project_rail_pinned.contains(a);
        pinned == self.project_rail_pinned.contains(b)
            && (pinned || self.project_rail_group(a) == self.project_rail_group(b))
    }
    /// Projects in the order the current layout shows them. The rail lists
    /// pinned projects, then each group's members, then the rest.
    pub fn visible_project_order(&self) -> Vec<String> {
        if !self.classic_layout {
            return self.projects.clone();
        }
        let unpinned = |p: &&String| !self.project_rail_pinned.contains(*p);
        let mut order: Vec<String> = self
            .projects
            .iter()
            .filter(|p| self.project_rail_pinned.contains(*p))
            .cloned()
            .collect();
        for group in &self.project_rail_groups {
            order.extend(
                self.projects
                    .iter()
                    .filter(unpinned)
                    .filter(|p| self.project_rail_group(p) == Some(group.id.as_str()))
                    .cloned(),
            );
        }
        order.extend(
            self.projects
                .iter()
                .filter(unpinned)
                .filter(|p| self.project_rail_group(p).is_none())
                .cloned(),
        );
        order
    }
    /// The projects that reorder alongside `path`, in the order shown: the
    /// whole list, or just its rail section.
    pub fn project_peers(&self, path: &str) -> Vec<String> {
        self.visible_project_order()
            .into_iter()
            .filter(|p| !self.classic_layout || self.project_rail_peers(p, path))
            .collect()
    }
    /// Moves a project to `to` in the saved order. Returns whether it moved.
    pub fn move_project(&mut self, path: &str, to: usize) -> bool {
        let Some(from) = self.projects.iter().position(|p| p == path) else {
            return false;
        };
        let to = to.min(self.projects.len() - 1);
        if from == to {
            return false;
        }
        let project = self.projects.remove(from);
        self.projects.insert(to, project);
        true
    }
    /// Moves a project one place along what the layout shows, staying inside
    /// its rail section. Returns whether it moved.
    pub fn step_project_order(&mut self, path: &str, step: isize) -> bool {
        let peers = self.project_peers(path);
        let Some(ix) = peers.iter().position(|p| p == path) else {
            return false;
        };
        let Some(neighbour) = ix
            .checked_add_signed(step.signum())
            .and_then(|ix| peers.get(ix))
        else {
            return false;
        };
        let to = self.projects.iter().position(|p| p == neighbour).unwrap();
        self.move_project(path, to)
    }
    /// Moves a project to either end of its section: `step < 0` for the start.
    pub fn move_project_to_end(&mut self, path: &str, step: isize) -> bool {
        let peers = self.project_peers(path);
        let target = if step < 0 {
            peers.first()
        } else {
            peers.last()
        };
        let Some(to) = target.and_then(|t| self.projects.iter().position(|p| p == t)) else {
            return false;
        };
        self.move_project(path, to)
    }
    pub fn session_for_pane(&self, id: i64) -> i64 {
        self.review_owners
            .get(&id)
            .copied()
            .or_else(|| self.terminal_panes.get(&id).map(|t| t.owner))
            .unwrap_or(id)
    }
    /// A file or terminal pane standing in for its owner session.
    pub fn is_detached_pane(&self, id: i64) -> bool {
        self.review_owners.contains_key(&id) || self.terminal_panes.contains_key(&id)
    }
    /// The title tab holding a session.
    pub fn tab_of(&self, id: i64) -> Option<i64> {
        self.tabs.iter().copied().find(|tab| {
            *tab == id
                || self
                    .layouts
                    .get(tab)
                    .is_some_and(|layout| layout.contains(id))
        })
    }
    pub fn active_tab(&self) -> Option<i64> {
        self.active.and_then(|id| self.tab_of(id))
    }
    pub fn layout(&self, tab: i64) -> PaneNode {
        self.layouts.get(&tab).cloned().unwrap_or_else(|| leaf(tab))
    }
    /// Every session shown in some tab.
    pub fn open_sessions(&self) -> Vec<i64> {
        self.tabs
            .iter()
            .flat_map(|tab| self.layout(*tab).leaf_ids())
            .collect()
    }
    /// The pane a tab activates.
    pub fn tab_focus(&self, tab: i64) -> i64 {
        self.focused
            .get(&tab)
            .copied()
            .filter(|id| self.layouts.get(&tab).is_some_and(|l| l.contains(*id)))
            .unwrap_or(tab)
    }
    pub fn normalize_title_order(&mut self) {
        self.tab_workspaces.retain(|tab, _| self.tabs.contains(tab));
        let mut seen = HashSet::new();
        self.title_order.retain(|tab| {
            tab.session().is_none_or(|id| self.tabs.contains(&id)) && seen.insert(tab.clone())
        });
        for id in &self.tabs {
            let tab = TitleTab::Session(*id);
            if seen.insert(tab.clone()) {
                self.title_order.push(tab);
            }
        }
        self.prune_pinned_tabs();
    }
    /// Pins follow their tabs: a tab that left the strip is no longer pinned.
    pub fn prune_pinned_tabs(&mut self) {
        let mut seen = HashSet::new();
        let order = &self.title_order;
        self.pinned_tabs
            .retain(|tab| order.contains(tab) && seen.insert(tab.clone()));
    }
}

/// Port of src/features/workspace/model/tabVisitHistory.ts. Closed tabs are
/// pruned rather than reopened by Back, and a new visit discards Forward.
#[derive(Default)]
pub(crate) struct VisitHistory {
    pub back: Vec<i64>,
    pub forward: Vec<i64>,
    current: Option<i64>,
}
impl VisitHistory {
    fn push(&mut self, id: i64) {
        if self.back.last() != Some(&id) {
            self.back.push(id);
        }
        if self.back.len() > 50 {
            self.back.remove(0);
        }
    }
    pub fn record(&mut self, id: i64) {
        if self.current == Some(id) {
            return;
        }
        if let Some(current) = self.current {
            self.push(current);
        }
        self.current = Some(id);
        self.forward.clear();
    }
    pub fn step(&mut self, direction: isize) -> Option<i64> {
        let target = if direction < 0 {
            let target = self.back.pop()?;
            if let Some(current) = self.current {
                self.forward.insert(0, current);
            }
            target
        } else {
            if self.forward.is_empty() {
                return None;
            }
            let target = self.forward.remove(0);
            if let Some(current) = self.current {
                self.push(current);
            }
            target
        };
        self.current = Some(target);
        Some(target)
    }
    pub fn prune(&mut self, tabs: &[i64], active: Option<i64>) {
        self.back.retain(|id| tabs.contains(id));
        self.forward.retain(|id| tabs.contains(id));
        if self.current.is_none_or(|id| !tabs.contains(&id)) {
            self.current = active;
        }
        while self
            .back
            .last()
            .copied()
            .is_some_and(|id| Some(id) == self.current)
        {
            self.back.pop();
        }
        while self
            .forward
            .first()
            .copied()
            .is_some_and(|id| Some(id) == self.current)
        {
            self.forward.remove(0);
        }
    }
}

#[cfg(test)]
mod tests;
