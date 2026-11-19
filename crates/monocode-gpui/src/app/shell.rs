pub(crate) mod chrome;
pub(crate) mod commands;
pub(crate) mod glue;
mod lifecycle;
mod menu;
mod navigation;
pub(crate) mod panes;
mod review;
pub(crate) mod right_panel;
pub(crate) mod sessions;
pub(crate) mod sidebar;
mod terminal_dock;
mod view;
#[cfg(target_os = "windows")]
use crate::ui::chrome::WINDOWS_CONTROLS;
use crate::ui::chrome::{client_controls_width, corner_controls_width, titlebar_drag_area};
use crate::ui::popover::DismissMenu;
use panes::terminal_panes::terminal_drag_ghost;

use crate::{
    app::{
        shell::right_panel::{PanelEvent, RightPanel},
        workspace::{DockSide, TitleTab, VisitHistory, Workspace},
    },
    chat::{
        composer::pickers::{ProjectPicked, ProjectPicker},
        conversation::{Conversation, SessionChanged},
        response_actions::Transfer,
    },
    footer::{
        resource_pressure::{ResourceMonitor, StopSession},
        usage::UsageFooter,
    },
    notes::Notes,
    runtime::{
        Database, locked,
        pane_layout::{Edge, LayoutLeaf, Rect, SplitDir},
    },
    terminal::{Terminal, TerminalEvent},
    ui::{
        input,
        motion::pane::PaneSlide,
        popover::{
            Align, Anchor, AnchorSlot, Flyout, Menu, MenuAction, MenuEvent, MenuItem, MenuStyle,
            Side, menu_popover, popover,
        },
        project_badge::project_name,
        session_mark::{SessionMark, State as MarkState},
        tabs::tile::{Face, NARROW_TAB, TAB_TILE, TAB_TILE_GAP, mark_tile},
        theme::*,
    },
};
use gpui::{
    AnyElement, App, Bounds, BoxShadow, ClipboardItem, Context, CursorStyle, DispatchPhase, Entity,
    FocusHandle, Focusable, FontWeight, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PathPromptOptions, Pixels, Point, Role, ScrollHandle, SharedString,
    StyleRefinement, Subscription, Window, actions, canvas, div, prelude::*, px, rgb, rgba,
};
use monocode_core::{provider::Kind, store::Session};
use sessions::groups::local_reminder_label;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

actions!(
    workspace,
    [
        NewCodex,
        NewClaude,
        OpenProject,
        CloseTab,
        CloseAllTabs,
        ArchiveSession,
        VisitBack,
        VisitForward,
        NextTab,
        PreviousTab,
        ToggleSidebar,
        FocusSearch,
        SearchFiles,
        FocusNext,
        FocusPrevious,
        CloseOtherTabs,
        PreviousSession,
        NextSession,
        PreviousSessionInTab,
        NextSessionInTab,
        PreviousProject,
        NextProject,
        MoveProjectBack,
        MoveProjectForward,
        SwitchModel,
        ToggleRightPanel,
        SplitRight,
        SplitDown,
        FocusPaneLeft,
        FocusPaneRight,
        FocusPaneUp,
        FocusPaneDown,
        ToggleTerminal,
        NewTerminal,
        SplitTerminal,
        ToggleLayout,
        ToggleWorkspaceStyle,
        OpenSettings,
        GoToFile,
        OpenCommandPalette,
        OpenInbox,
        OpenNotes,
        CheckForUpdates,
        MinimizeWindow,
        ZoomWindow,
        ZoomIn,
        ZoomOut,
        ResetZoom,
        Reload,
        SidebarAppearance,
        About
    ]
);

#[derive(Clone, Copy)]
enum ChromeKind {
    Sidebar,
    Projects,
    Tabs,
}
struct Chrome {
    owner: gpui::WeakEntity<Shell>,
    kind: ChromeKind,
    renders: usize,
}
impl Render for Chrome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let kind = self.kind;
        self.owner
            .update(cx, |shell, cx| match kind {
                ChromeKind::Sidebar => {
                    let sidebar = shell.sidebar(cx);
                    if shell.spaces_nudge.is_some() || shell.classic_tabs.animating() {
                        window.request_animation_frame();
                    }
                    sidebar
                }
                ChromeKind::Tabs => shell.tabs(window, cx),
                ChromeKind::Projects => {
                    let rail = shell.project_rail(cx);
                    if shell.project_rail_animating() {
                        window.request_animation_frame();
                    }
                    rail
                }
            })
            .unwrap_or_else(|_| div().into_any())
    }
}

/// `Tab: Activate 1–8` with the slot, or `Tab: Activate Last` with -1.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = workspace, no_json)]
pub(crate) struct ActivateTab(pub isize);

/// tabKeys.ts's `adjacentItemId`: the next or previous item, wrapping; from
/// outside the list, the first or last.
fn adjacent<T: PartialEq + Clone>(items: &[T], current: Option<&T>, step: isize) -> Option<T> {
    if items.is_empty() {
        return None;
    }
    let len = items.len() as isize;
    match current.and_then(|c| items.iter().position(|item| item == c)) {
        Some(ix) => Some(items[(ix as isize + step.signum()).rem_euclid(len) as usize].clone()),
        None if step < 0 => items.last().cloned(),
        None => items.first().cloned(),
    }
}

#[derive(Clone, PartialEq)]
pub(crate) enum MenuTarget {
    Tab(i64),
    MonoTab(String),
    SearchTab,
    AppTab(glue::app_views::AppView),
    Session(i64),
    Folder(String),
    /// The sidebar project header's menu.
    Header,
    /// The original project rail's local or remote folder menu.
    ProjectRailAdd,
    CompactProjectPicker,
    ProjectRailGroup(String),
    SessionFilter,
    SidebarOptions,
    /// A project's actions; `rail` adds the project rail's grouping and pinning.
    Project {
        path: String,
        rail: bool,
    },
    /// ProjectTerminalDock's Move Terminal menu.
    TerminalDock,
    /// A dock terminal tab's context menu.
    TerminalTab(usize),
    /// The footer chip's list of terminals running a process.
    RunningTerminals,
    /// The footer Actions button's list of saved commands.
    Actions,
    /// The title strip's new-tab button, when terminals open in the title bar.
    NewTab,
    /// The chevron beside it: a new session on a model picked from the list.
    NewTabModel,
}
pub(crate) struct ShellMenu {
    pub(crate) target: MenuTarget,
    anchor: Anchor,
    menu: Entity<Menu>,
    _events: Vec<Subscription>,
}
/// The gap the traffic lights take in the title bar (`w-[78px]`, or `w-2.5`
/// where the window has none, as in macOS fullscreen).
const TRAFFIC_LIGHTS: f32 = if cfg!(target_os = "macos") { 78. } else { 10. };
const NO_TRAFFIC_LIGHTS: f32 = 10.;
// The tab strip's padding; the trailing side also has the 2px gap before the
// strip's drag area.
const STRIP_LEAD: f32 = 6.;
const STRIP_TRAIL: f32 = 6.;
const STRIP_PAD: f32 = STRIP_LEAD + STRIP_TRAIL;
/// The selected title tab shrinks with the others only down to this.
const SELECTED_TAB_MIN_WIDTH: f32 = 112.;
/// Narrower tabs drop their close button.
const TAB_CLOSE_MIN_WIDTH: f32 = 72.;
/// The new-tab button and its leading margin.
const STRIP_NEW_TAB: f32 = TAB_PLATE_HEIGHT + 2. + NEW_TAB_MODEL_WIDTH + 2.;
/// The model chevron after the new-tab button, one strip gap further on.
const NEW_TAB_MODEL_WIDTH: f32 = 18.;
const CORNER_TABS_LEAD: f32 = 7.;
const CORNER_CONTROL_GAP: f32 = 4.;
const CORNER_BUTTONS_WIDTH: f32 = 48. + 2. * CORNER_CONTROL_GAP;
// TitleBar.tsx's labels, with their shortcuts.
const TOGGLE_SIDEBAR_LABEL: &str = if cfg!(target_os = "macos") {
    "Toggle Sidebar (⌘B)"
} else {
    "Toggle Sidebar (Ctrl+B)"
};
const BACK_LABEL: &str = if cfg!(target_os = "macos") {
    "Back (⌘[)"
} else {
    "Back (Ctrl+[)"
};
const FORWARD_LABEL: &str = if cfg!(target_os = "macos") {
    "Forward (⌘])"
} else {
    "Forward (Ctrl+])"
};
const TOGGLE_RIGHT_PANEL_LABEL: &str = if cfg!(target_os = "macos") {
    "Toggle right sidebar (⌘⇧B)"
} else {
    "Toggle right sidebar (Ctrl+Shift+B)"
};
const TOGGLE_RIGHT_PANEL_DOT_LABEL: &str = if cfg!(target_os = "macos") {
    "Toggle right sidebar (⌘⇧B, uncommitted changes)"
} else {
    "Toggle right sidebar (Ctrl+Shift+B, uncommitted changes)"
};
const MOD: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl+"
};
const REVEAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else if cfg!(target_os = "windows") {
    "Reveal in File Explorer"
} else {
    "Open Containing Folder"
};

/// What a pane drop carries: a split pane by its header, a title tab
/// pulled down out of the strip, or a sidebar session.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DragSource {
    Pane(i64),
    Review(i64),
    Tab(i64),
    Session(i64),
    /// A Mono tab pulled down out of the strip; it has no pane yet.
    MonoTab,
    /// A dock terminal pulled off the dock's strip; it has no pane yet.
    DockTerminal(usize),
}
impl DragSource {
    fn session(self) -> i64 {
        match self {
            DragSource::Pane(id)
            | DragSource::Review(id)
            | DragSource::Tab(id)
            | DragSource::Session(id) => id,
            DragSource::MonoTab | DragSource::DockTerminal(_) => i64::MIN,
        }
    }
}
/// How close, in pixels, a dragged sash must come to a snap spot to take it.
const SASH_SNAP_PX: f32 = 12.;
struct TabCopy {
    headline: String,
    /// Every pane in layout order; the tab shows one line at a time.
    panes: Vec<TabPane>,
    /// The pane in `panes` that has focus.
    focus: usize,
}
/// One line of a tab's headline: a chat, or the file beside it.
struct TabPane {
    /// The pane's leaf, and whether this is its file side.
    key: (i64, bool),
    title: String,
    mark: PaneMark,
}
pub(crate) enum PaneMark {
    /// The provider, with the session id when its mark is lit.
    Session(String, Option<i64>),
    /// A file's name, or `None` for changes and commits.
    File(Option<String>),
    /// A Mono's mascot and colour.
    Mono(String, u32),
    /// A terminal pulled out of its dock.
    Terminal,
}
/// A split tab's headline rolling to the newly focused pane: the next pane
/// rises from below, the previous one drops from above.
struct PaneTick {
    shown: (i64, bool),
    from: Option<(i64, bool)>,
    /// 1 when the new line rises, -1 when it drops.
    dir: f32,
    start: std::time::Instant,
}
const PANE_TICK_DURATION: f32 = 0.24;
/// The tab tiles' deck shuffle on the same focus change.
const TILE_SHUFFLE_DURATION: f32 = 0.34;
/// useAnimatedReorder: `--motion-reorder-duration` with `--motion-ease-out`.
const REORDER_DURATION: f32 = 0.16;
/// A tab easing from one horizontal offset to another.
#[derive(Clone, Copy)]
struct TabShift {
    from: f32,
    to: f32,
    start: std::time::Instant,
}
impl TabShift {
    fn at(&self, now: std::time::Instant) -> (f32, bool) {
        let t = now.saturating_duration_since(self.start).as_secs_f32() / REORDER_DURATION;
        if t >= 1. {
            return (self.to, true);
        }
        let eased = crate::ui::motion::reveal::bezier(t, 0.22, 1., 0.36, 1.);
        (self.from + (self.to - self.from) * eased, false)
    }
}
/// Below this a chat column stops being comfortable to read.
const MIN_CHAT_WIDTH: f32 = 420.;
/// A sideways tab drag sags; only a deliberate pull below the strip
/// targets a pane (TitleBar's PANE_DROP_PULL).
const PANE_DROP_PULL: f32 = 32.;

struct SessionPreview {
    id: i64,
    data: Option<monocode_core::store::SessionPreviewData>,
    anchor: AnchorSlot,
    surface: AnchorSlot,
}

pub(crate) struct Shell {
    monos: Option<Entity<crate::monos::Monos>>,
    quick_open: Option<glue::quick_open::Open>,
    image_lightbox: Option<(
        Entity<crate::ui::image_lightbox::ImageLightbox>,
        Subscription,
    )>,
    app_bridge: Option<monocode_core::mono::bridge::Server>,
    mobile: glue::mobile_host::Mobile,
    app_results: HashMap<(i64, String), (String, Result<serde_json::Value, String>)>,
    app_pending: HashSet<(i64, String)>,
    orchestrations: HashMap<i64, glue::orchestration::Run>,
    /// Each lead's last orchestration, whose agents hang under it in the sidebar.
    orchestration_summaries: HashMap<i64, monocode_core::session::orchestration::Summary>,
    /// Sessions an /operator thread started, by the session that started them.
    started_by: HashMap<i64, i64>,
    /// The sidebar rails under orchestrators and /operator threads.
    rails: sidebar::rails::Rails,
    usage_footer: Entity<UsageFooter>,
    /// The footer's high-usage chip and its running-work dialog.
    resources: Entity<ResourceMonitor>,
    sidebar_frame: Entity<Chrome>,
    projects_frame: Entity<Chrome>,
    project_rail_list: sidebar::project_rail::ProjectRailList,
    project_rail_drag: Option<sidebar::project_rail::ProjectRailDrag>,
    space_reorder: sidebar::space_reorder::SpaceReorder,
    project_rail_stats: sidebar::project_rail::ProjectRailStats,
    project_resize: Option<(Pixels, f32)>,
    classic_sidebar_tab: sidebar::classic::ClassicSidebarTab,
    classic_tabs: sidebar::classic::WorkspaceTabs,
    classic_session_search: Entity<input::Input>,
    classic_worktree_picker: Option<sidebar::classic::WorktreePicker>,
    sidebar_list: sidebar::list::SidebarList,
    sidebar_slide: Option<PaneSlide>,
    sidebar_fraction: f32,
    /// Settings' own nav collapse, apart from the project sidebar's saved
    /// state; every visit to Settings starts with it open.
    settings_nav_visible: bool,
    /// The collapsed inset sidebar peeking out over the workspace.
    sidebar_peek: Option<sidebar::peek::SidebarPeek>,
    peek_animating: bool,
    peek_close: Option<gpui::Task<()>>,
    peek_toggle_bounds: AnchorSlot,
    /// useTrafficLightsVisible: fullscreen hides the native buttons, so the
    /// title bar takes their room back.
    fullscreen: bool,
    /// `corner_controls_width` as of this render, for the corner's layout.
    window_controls: f32,
    /// The narrowest right panel whose header holds the corner's tools
    /// beside client-drawn window controls.
    right_panel_min: f32,
    #[cfg(target_os = "linux")]
    client_decorations: bool,
    tabs_frame: Entity<Chrome>,
    store: Database,
    remote: Arc<monocode_core::integrations::remote::Connections>,
    remote_project_picker: Option<sessions::remote_projects::Open>,
    remote_refreshing: bool,
    remote_marks: HashMap<i64, (bool, bool)>,
    _remote_clock: gpui::Task<()>,
    sessions: HashMap<i64, Entity<Conversation>>,
    /// Lightweight history index; only opened or previewed chats own GPUI entities.
    session_summaries: HashMap<i64, Session>,
    draft_sessions: HashSet<i64>,
    history_recency: HashMap<i64, u64>,
    history_clock: u64,
    session_marks: HashMap<i64, Entity<SessionMark>>,
    tab_marks: HashMap<i64, Entity<SessionMark>>,
    /// The live agents card's spinners, apart from the sidebar rows' marks.
    live_marks: HashMap<i64, Entity<SessionMark>>,
    live_agents_expanded: bool,
    /// Repaints the live agents card when one of its elapsed times changes.
    live_tick: Option<gpui::Task<()>>,
    /// Milliseconds until the next elapsed label in the live agents card changes.
    live_next_change: u64,
    busy_sessions: HashSet<i64>,
    notification_state: glue::notifications::State,
    github_star: glue::github_star::State,
    quick: crate::chat::quick_composer::Controller,
    unseen_finished: HashSet<i64>,
    archiving: HashSet<i64>,
    archive_shortcut_held: bool,
    /// Editors offered by a project's "Open in editor"; `None` while scanning.
    external_editors: Option<Vec<monocode_core::files::external_editor::ExternalEditor>>,
    groups: HashMap<String, monocode_core::session::groups::Groups>,
    reminders: Vec<monocode_core::store::Reminder>,
    renaming: Option<sessions::groups::Renaming>,
    link_dialog: Option<sessions::groups::LinkDialog>,
    action_dialog: Option<commands::actions::ActionDialog>,
    actions_anchor: AnchorSlot,
    new_tab_anchor: AnchorSlot,
    /// The dock terminal each action last ran in, by project and action id.
    action_terminals: HashMap<(String, String), gpui::WeakEntity<Terminal>>,
    deleting: HashSet<i64>,
    group_writes: Option<gpui::Task<()>>,
    selected_sessions: HashSet<i64>,
    selection_anchor: Option<i64>,
    context_selection: bool,
    sidebar_target: Option<sessions::groups::RenameTarget>,
    session_row_bounds: HashMap<i64, AnchorSlot>,
    folder_bounds: HashMap<String, AnchorSlot>,
    sidebar_drop: Option<sessions::groups::ListDrop>,
    folder_drag: Option<sessions::groups::FolderDrag>,
    skip_folder_click: bool,
    filter_anchor: AnchorSlot,
    folder_picker: Option<sidebar::folders::Open>,
    pending_folder: Option<(i64, AnchorSlot)>,
    _session_clock: gpui::Task<()>,
    notes: Option<Entity<Notes>>,
    /// The sidebar's Notes count; `None` until the store has answered.
    note_count: Option<usize>,
    inbox: Option<Entity<crate::inbox::Inbox>>,
    automations: Option<Entity<crate::automations::Automations>>,
    active_app_view: Option<glue::app_views::AppView>,
    app_view_events: Vec<Subscription>,
    automation_state: glue::automation_runs::State,
    settings: Option<Entity<crate::settings::SettingsView>>,
    settings_events: Option<Subscription>,
    settings_overlay_watch: Option<Subscription>,
    app_search: Option<glue::search::Open>,
    saved_search: Option<(String, monocode_core::store::search::Scope)>,
    order: Vec<i64>,
    pub(crate) workspace: Workspace,
    focus: FocusHandle,
    sidebar_focus: FocusHandle,
    query: String,
    sidebar_hover: Option<&'static str>,
    tab_hover: Option<i64>,
    /// The pinned deck at the strip's left edge: how far it has fanned out,
    /// and the card under the pointer.
    pin_fan: sidebar::pinned_tabs::PinFan,
    pin_deck: sidebar::pinned_tabs::PinDeck,
    pin_card_hover: Option<TitleTab>,
    /// Tab animations: opening and closing title tabs, with each tab's last label.
    title_tab_motion: crate::ui::tabs::motion::TabMotion<TitleTab, ()>,
    /// Tab animations: the selected tab before the last switch, the one now
    /// selected and when it changed, so one eases down as the other widens.
    title_tab_selection: (Option<TitleTab>, Option<TitleTab>, std::time::Instant),
    session_hover: Option<i64>,
    session_preview: Option<SessionPreview>,
    preview_close: Option<gpui::Task<()>>,
    preview_open: Option<gpui::Task<()>>,
    sidebar_last_scroll: Option<std::time::Instant>,
    window_observed: bool,
    shortcuts: Option<commands::shortcuts::Shortcuts>,
    window_active: bool,
    window_glass: Option<bool>,
    /// The project whose colors were last painted; leaving it blends colors.
    theme_project: Option<String>,
    quit_prompted: bool,
    pub(crate) menu: Option<ShellMenu>,
    background_dialog: Option<glue::project_settings::OpenBackground>,
    backgrounds: HashMap<i64, Entity<crate::project::background::ProjectBackground>>,
    /// The last active chat and whether it drew a background, so arriving
    /// from one without fades the next one in.
    backdrop: Option<(i64, bool)>,
    background_card: AnchorSlot,
    project_anchor: AnchorSlot,
    sidebar_options_anchor: AnchorSlot,
    history: VisitHistory,
    /// User navigation invalidates pending requests that would otherwise
    /// focus a new session in a tab or project the user has already left.
    navigation_revision: u64,
    pending_project_blanks: HashSet<String>,
    new_tab_task: Option<gpui::Task<()>>,
    worktree_focus: HashMap<String, crate::git::worktree_switcher::WorktreeFocus>,
    session_worktrees: HashMap<i64, Option<String>>,
    session_details: HashMap<i64, monocode_core::store::MobileSessionMetadata>,
    sidebar_hidden_sessions: HashSet<i64>,
    worktree_returns: HashMap<(String, String), i64>,
    tab_drag: Option<TabDrag>,
    /// Tabs sliding between slots: neighbours making room for a drag and,
    /// after a drop, every tab gliding from where it was drawn into place.
    tab_shifts: HashMap<TitleTab, TabShift>,
    pane_ticks: HashMap<i64, PaneTick>,
    skip_tab_click: bool,
    spaces_start: usize,
    /// The spaces bar's short nudge after a chevron pages it.
    spaces_nudge: Option<(f32, std::time::Instant)>,
    /// The space under the pointer, whose name and shortcuts show above it.
    hovered_space: Option<String>,
    space_anchor: AnchorSlot,
    error: Option<String>,
    tab_scroll: ScrollHandle,
    tab_scroll_task: Option<gpui::Task<()>>,
    resize: Option<(Pixels, f32)>,
    pub(crate) git: Entity<crate::runtime::git_status::ProjectGit>,
    pub(crate) right_panel: Option<Entity<RightPanel>>,
    /// Each split pane's own project panel and the Git status it reads
    /// (pane_panels.rs).
    pane_panels: HashMap<i64, Entity<RightPanel>>,
    pane_gits: HashMap<i64, Entity<crate::runtime::git_status::ProjectGit>>,
    /// A pane panel's edge being dragged: pane, pointer start and width.
    pane_panel_resize: Option<(i64, Pixels, f32)>,
    right_resize: Option<(Pixels, f32)>,
    /// Either right-panel handle is hovered; both of its hairlines light up.
    right_resize_hovered: bool,
    right_slide: Option<PaneSlide>,
    reviews: HashMap<i64, Entity<crate::git::review::ReviewPane>>,
    /// A review sliding in or out: its session and pane.
    review_slide: Option<(i64, PaneSlide, Entity<crate::git::review::ReviewPane>)>,
    /// A session review card's Review, opened beside its pane next frame.
    pending_review: Option<(i64, crate::git::review::ReviewTab)>,
    pending_composer_focus: Option<i64>,
    review_focused: bool,
    review_drag: bool,
    split_bounds: HashMap<i64, AnchorSlot>,
    pane_bounds: AnchorSlot,
    /// The visible Mono's view, a drop target for tabs and panes.
    mono_bounds: AnchorSlot,
    pane_drag: Option<PaneDrag>,
    pane_sash: Option<SashDrag>,
    pane_motion: Option<PaneMotion>,
    skip_session_click: bool,
    /// Live terminals of each project's dock, in the dock's tab order.
    terminals: HashMap<String, Vec<Entity<Terminal>>>,
    /// Live shells of `workspace.terminal_panes`, spawned when first shown.
    terminal_pane_views: HashMap<i64, Entity<Terminal>>,
    /// A project's dock sliding open (toward 1) or shut (toward 0).
    dock_slide: Option<(String, PaneSlide)>,
    dock_resize: Option<(f32, f32)>,
    dock_hover: Option<usize>,
    /// Tab animations for the dock's terminals, with each tab's last title.
    dock_tab_motion: crate::ui::tabs::motion::TabMotion<gpui::EntityId, SharedString>,
    dock_drag: Option<terminal_dock::DockTabDrag>,
    /// The right panel's drawn width this frame, 0 while it is hidden.
    right_drawn: f32,
    /// Classic: the corner tab opening out and the one folding away.
    corner_reveal: Option<crate::project::tabs::TabReveal>,
    dock_shifts: HashMap<gpui::EntityId, TabShift>,
    dock_skip_click: bool,
    dock_tab_scroll: ScrollHandle,
    move_terminal_anchor: AnchorSlot,
    terminal_chip_anchor: AnchorSlot,
    live_mark: Option<Entity<terminal_dock::LiveMark>>,
    /// The sidebar collapsed itself to make room; it returns with the room.
    sidebar_auto_hidden: bool,
    /// The room the sidebar was last fitted to; a change refits it.
    sidebar_fitted_room: Option<(i32, bool)>,
    viewport_width: f32,
    viewport_height: f32,
    observations: Vec<Subscription>,
    generation: Arc<AtomicU64>,
    latest_state: Arc<Mutex<String>>,
    pub(crate) render_passes: usize,
    /// This window runs the app's background services and saves the workspace.
    main: bool,
    /// Sessions on screen in this window, for the others to keep live.
    shown: HashSet<i64>,
}
impl Shell {}
use panes::drag::{
    PaneDrag, PaneMotion, ResizeTarget, SashDrag, TabDrag, resize_border, sash_snap_line,
};
use panes::enter_from;
// Tests across the shell bind the full keymap through `use super::*`.
#[cfg(test)]
use commands::bind_keys;
#[cfg(test)]
mod tests;
