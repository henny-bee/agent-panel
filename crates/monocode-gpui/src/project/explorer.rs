//! FileTree.tsx: the Explorer header, project root row and lazily listed
//! tree with Git colours, inline create/rename, clipboard, context menu,
//! dropped files and VS Code-style keyboard navigation. Directory reads and
//! every file operation run off the UI thread.
mod file_ops;
mod tree;
mod view;
use crate::ui::files::DraggedFile;
use crate::{
    chat::claude::panels::{ClaudeActivity, ClaudePanel, ClaudePanelEvent, ClaudeTab},
    project::search::{ProjectSearch, SearchEvent},
    runtime::git_status::{GitChanged, ProjectGit},
    ui::panel::*,
    ui::{
        file_icons::file_type_icon,
        input::{self, Input},
        popover::{Anchor, Menu, MenuAction, MenuEvent, MenuItem, menu_popover},
        theme::*,
    },
};
use gpui::{
    AnyElement, App, ClipboardEntry, ClipboardItem, Context, Entity, EventEmitter, ExternalPaths,
    FocusHandle, Focusable, FontWeight, Hsla, KeyBinding, KeyDownEvent, ListAlignment, ListState,
    MouseButton, MouseDownEvent, Point, PromptLevel, Role, SharedString, StyleRefinement,
    Subscription, Window, actions, div, list, prelude::*, px, rgb,
};
use monocode_core::files::{self, DirEntry, NameIssueKind, Severity};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Instant,
};

const MOD: &str = if cfg!(target_os = "macos") {
    "⌘"
} else {
    "Ctrl+"
};
const SHIFT: &str = if cfg!(target_os = "macos") {
    "⇧"
} else {
    "Shift+"
};
const REVEAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else if cfg!(target_os = "windows") {
    "Reveal in File Explorer"
} else {
    "Open Containing Folder"
};
const ROW: f32 = 30.;
const ENTRY_FONT_SIZE: f32 = 13.;

actions!(
    explorer,
    [
        Down,
        Up,
        PageDown,
        PageUp,
        First,
        Last,
        Expand,
        Collapse,
        Activate,
        OpenSelected,
        Rename,
        Delete,
        ClearCut,
        CopyEntry,
        CutEntry,
        PasteEntry,
        CopyPath,
        CancelName
    ]
);

pub(crate) fn bind(cx: &mut App) {
    let m = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    let c = Some("Explorer");
    cx.bind_keys([
        KeyBinding::new("down", Down, c),
        KeyBinding::new("up", Up, c),
        KeyBinding::new("pagedown", PageDown, c),
        KeyBinding::new("pageup", PageUp, c),
        KeyBinding::new("home", First, c),
        KeyBinding::new("end", Last, c),
        KeyBinding::new("right", Expand, c),
        KeyBinding::new("left", Collapse, c),
        KeyBinding::new("enter", Activate, c),
        KeyBinding::new("cmd-down", OpenSelected, c),
        KeyBinding::new("f2", Rename, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("backspace", Delete, c),
        KeyBinding::new("escape", ClearCut, c),
        KeyBinding::new(&format!("{m}-c"), CopyEntry, c),
        KeyBinding::new(&format!("{m}-x"), CutEntry, c),
        KeyBinding::new(&format!("{m}-v"), PasteEntry, c),
        KeyBinding::new(&format!("{m}-shift-c"), CopyPath, c),
        KeyBinding::new("escape", CancelName, Some("ExplorerName")),
    ]);
}

#[derive(Clone, PartialEq)]
struct Clip {
    cut: bool,
    path: String,
    is_dir: bool,
}
#[derive(Clone)]
struct Target {
    path: String,
    is_dir: bool,
    root: bool,
}
#[derive(Clone, PartialEq)]
enum NameMode {
    Create {
        id: u64,
        parent: String,
        is_dir: bool,
    },
    Rename {
        path: String,
        is_dir: bool,
    },
}
#[derive(Clone, PartialEq)]
enum Line {
    Note {
        depth: usize,
        text: String,
    },
    Entry {
        entry: DirEntry,
        depth: usize,
        open: bool,
    },
    Name {
        depth: usize,
        is_dir: bool,
    },
    Issue {
        depth: usize,
        text: Vec<(String, bool)>,
        error: bool,
    },
}
struct OpenMenu {
    target: Target,
    anchor: Point<gpui::Pixels>,
    menu: Entity<Menu>,
    _events: Subscription,
}
#[derive(Default, Clone)]
struct Saved {
    expanded: HashSet<String>,
    selected: Option<String>,
}

pub(crate) struct Explorer {
    git: Entity<ProjectGit>,
    cwd: String,
    enabled: bool,
    show_excluded: bool,
    expanded: HashSet<String>,
    selected: Option<String>,
    /// loadExpanded / loadSelected: remembered per project for the session.
    saved: HashMap<String, Saved>,
    /// listCachedDir: the last listing of each folder, or its error.
    dirs: HashMap<String, Result<Vec<DirEntry>, String>>,
    reading: HashSet<String>,
    naming: Option<NameMode>,
    name_input: Entity<Input>,
    name_attempted: bool,
    name_busy: bool,
    name_error: Option<String>,
    clip: Option<Clip>,
    menu: Option<OpenMenu>,
    search: Option<Entity<ProjectSearch>>,
    search_events: Option<Subscription>,
    drag_over: Option<String>,
    op_error: Option<String>,
    lines: Vec<Line>,
    list: ListState,
    focus: FocusHandle,
    keyboard: bool,
    typeahead: (String, Option<Instant>),
    next_id: u64,
    /// The agents or in-this-chat view of the shown Claude session, which the
    /// tree gives way to while `claude_tab` is set. None for other sessions.
    claude: Option<Entity<ClaudePanel>>,
    /// The box above the tree: shows the folder and takes a typed one.
    folder_input: Entity<Input>,
    /// The panel shows a folder picked here instead of its chat's own.
    custom_folder: bool,
    claude_tab: Option<ClaudeTab>,
    _claude_events: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

fn parent_of(path: &str) -> String {
    files::parent_path(path)
}
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub(crate) struct OpenFile(pub String);
impl EventEmitter<OpenFile> for Explorer {}
/// Shared file-action toast state. Name validation remains local to its field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileActionError(pub Option<String>);
impl EventEmitter<FileActionError> for Explorer {}
pub(crate) struct OpenSearchMatch {
    pub cwd: String,
    pub matched: monocode_core::store::search::project::SearchMatch,
    pub pin: bool,
}
impl EventEmitter<OpenSearchMatch> for Explorer {}
/// The user picked a folder for the panel; None goes back to the chat's own.
pub(crate) struct FolderChosen(pub Option<String>);
impl EventEmitter<FolderChosen> for Explorer {}
/// The user switched between the files and a Claude view.
pub(crate) struct ClaudeTabChanged(pub Option<ClaudeTab>);
impl EventEmitter<ClaudeTabChanged> for Explorer {}

impl Explorer {
    pub(crate) fn new(
        git: Entity<ProjectGit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_input = cx.new(|cx| {
            let mut input = Input::field(
                "Type file name. Press Enter to confirm or Escape to cancel.",
                "",
                content(0.35),
                cx,
            );
            input.set_font_size(ENTRY_FONT_SIZE);
            input
        });
        let name_focus = name_input.read(cx).focus_handle(cx);
        let folder_input = cx.new(|cx| {
            let mut input = Input::field("Folder", "Type a folder path", content(0.35), cx);
            input.set_font_size(12.);
            input
        });
        let subscriptions = vec![
            cx.subscribe(&git, |this, _, _: &GitChanged, cx| this.git_changed(cx)),
            cx.observe(&git, |this, _, cx| {
                // Git colours follow every status read.
                if this.enabled {
                    cx.notify();
                }
                let _ = this;
            }),
            cx.subscribe_in(
                &name_input,
                window,
                |this, _, _: &input::Submitted, window, cx| this.finish_name(true, window, cx),
            ),
            cx.subscribe(&name_input, |this, _, _: &input::Changed, cx| {
                this.name_error = None;
                this.rebuild(cx);
            }),
            cx.subscribe(&folder_input, |this, _, _: &input::Submitted, cx| {
                this.submit_folder(cx)
            }),
            cx.on_blur(&name_focus, window, |this, window, cx| {
                if this.naming.is_some() && !this.name_busy {
                    let ok = this.issue(cx).is_none_or(|i| i.severity != Severity::Error);
                    this.finish_name(ok, window, cx);
                }
            }),
        ];
        Self {
            git,
            cwd: String::new(),
            enabled: false,
            show_excluded: false,
            expanded: HashSet::new(),
            selected: None,
            saved: HashMap::new(),
            dirs: HashMap::new(),
            reading: HashSet::new(),
            naming: None,
            name_input,
            name_attempted: false,
            name_busy: false,
            name_error: None,
            clip: None,
            menu: None,
            search: None,
            search_events: None,
            drag_over: None,
            op_error: None,
            lines: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(300.)),
            focus: cx.focus_handle(),
            keyboard: false,
            typeahead: (String::new(), None),
            next_id: 0,
            claude: None,
            folder_input,
            custom_folder: false,
            claude_tab: None,
            _claude_events: None,
            _subscriptions: subscriptions,
        }
    }
    /// The Claude session this Explorer sits beside, or None when it is not
    /// a Claude session; its view buttons show only for one.
    pub(crate) fn set_claude(
        &mut self,
        activity: Option<Entity<ClaudeActivity>>,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .claude
            .as_ref()
            .map(|p| p.read(cx).activity.entity_id());
        if current == activity.as_ref().map(|a| a.entity_id()) {
            return;
        }
        self._claude_events = None;
        self.claude = activity.map(|activity| {
            let tab = self.claude_tab.unwrap_or(ClaudeTab::Agents);
            let panel = cx.new(|cx| ClaudePanel::new(activity, tab, cx));
            self._claude_events = Some(cx.subscribe(
                &panel,
                |this, _, event: &ClaudePanelEvent, cx| match event {
                    ClaudePanelEvent::Hide => this.show_claude(None, cx),
                    ClaudePanelEvent::Open(path) => cx.emit(OpenFile(path.clone())),
                },
            ));
            panel
        });
        cx.notify();
    }
    /// Restores a saved choice without reporting it back.
    pub(crate) fn set_claude_tab(&mut self, tab: Option<ClaudeTab>, cx: &mut Context<Self>) {
        if self.claude_tab != tab {
            self.claude_tab = tab;
            if let (Some(panel), Some(tab)) = (&self.claude, tab) {
                panel.update(cx, |panel, cx| {
                    panel.tab = tab;
                    cx.notify();
                });
            }
            cx.notify();
        }
    }
    pub(crate) fn set_custom_folder(&mut self, custom: bool, cx: &mut Context<Self>) {
        if self.custom_folder != custom {
            self.custom_folder = custom;
            cx.notify();
        }
    }
    /// A typed folder, relative to the shown one unless absolute; `~` is home.
    fn submit_folder(&mut self, cx: &mut Context<Self>) {
        let typed = self.folder_input.read(cx).content.trim().to_owned();
        if typed.is_empty() {
            return cx.emit(FolderChosen(None));
        }
        let home = std::env::var("HOME").unwrap_or_default();
        let path = typed_folder(&typed, &self.cwd, &home);
        if path.is_dir() {
            let path = path.to_string_lossy();
            let path = match path.trim_end_matches('/') {
                "" => "/",
                trimmed => trimmed,
            };
            cx.emit(FolderChosen(Some(path.to_owned())));
        } else {
            cx.emit(FileActionError(Some(format!(
                "{} is not a folder",
                path.display()
            ))));
        }
    }
    /// The system folder window, as New project opens it.
    fn pick_folder(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Show folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let chosen = match picker.await {
                Ok(Ok(Some(paths))) => paths.first().map(|p| Ok(p.to_string_lossy().into_owned())),
                Ok(Ok(None)) => None,
                error => Some(Err(format!("Could not open the folder window: {error:?}"))),
            };
            let _ = this.update(cx, |_, cx| match chosen {
                Some(Ok(path)) => cx.emit(FolderChosen(Some(path))),
                Some(Err(error)) => cx.emit(FileActionError(Some(error))),
                None => {}
            });
        })
        .detach();
    }
    /// Collapse acts on the tree, so bring it back from B or B2 first.
    fn show_tree(&mut self, cx: &mut Context<Self>) {
        if self.claude_tab.is_some() {
            self.show_claude(None, cx);
        }
    }
    fn show_claude(&mut self, tab: Option<ClaudeTab>, cx: &mut Context<Self>) {
        self.set_claude_tab(tab, cx);
        cx.emit(ClaudeTabChanged(tab));
    }
    pub(crate) fn set_show_excluded(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.show_excluded != show {
            self.show_excluded = show;
            self.rebuild(cx);
        }
    }
    pub(crate) fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let was = self.enabled;
        self.enabled = enabled;
        if enabled {
            self.sync_project(cx);
            if !was {
                self.refresh_loaded(cx);
            }
        }
        if let Some(search) = &self.search {
            search.update(cx, |search, cx| search.set_active(enabled, cx));
        }
    }
    pub(crate) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_project(cx);
        self.menu = None;
        if let Some(search) = &self.search {
            search.update(cx, |search, cx| search.focus(window, cx));
            return;
        }
        let search = cx.new(|cx| ProjectSearch::new(self.cwd.clone(), self.enabled, cx));
        self.search_events = Some(cx.subscribe_in(
            &search,
            window,
            |this, _, event: &SearchEvent, window, cx| match event {
                SearchEvent::Close => {
                    this.close_search(window, cx);
                }
                SearchEvent::Open { cwd, matched, pin } => cx.emit(OpenSearchMatch {
                    cwd: cwd.clone(),
                    matched: matched.clone(),
                    pin: *pin,
                }),
            },
        ));
        search.update(cx, |search, cx| search.focus(window, cx));
        self.search = Some(search);
        cx.notify();
    }
    /// Escape also reaches this from the editor after opening a search result.
    pub(crate) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(search) = &self.search else {
            return false;
        };
        if search.read(cx).is_composing(cx) {
            return true;
        }
        self.search = None;
        self.search_events = None;
        window.focus(&self.focus, cx);
        cx.notify();
        true
    }
    /// notifyDirsChanged on window focus.
    pub(crate) fn resume(&mut self, cx: &mut Context<Self>) {
        if self.enabled {
            self.refresh_loaded(cx);
        }
    }
    fn git_changed(&mut self, cx: &mut Context<Self>) {
        if self.enabled {
            if !self.sync_project(cx) {
                self.refresh_loaded(cx);
            }
        } else if self.git.read(cx).root != self.cwd {
            // Keep the hidden tree intact; rebuild it when Files is shown.
        }
    }
    fn sync_project(&mut self, cx: &mut Context<Self>) -> bool {
        let root = self.git.read(cx).root.clone();
        if root == self.cwd {
            return false;
        }
        if !self.cwd.is_empty() {
            self.saved.insert(
                self.cwd.clone(),
                Saved {
                    expanded: std::mem::take(&mut self.expanded),
                    selected: self.selected.take(),
                },
            );
        }
        self.cwd = root.clone();
        self.folder_input
            .update(cx, |input, cx| input.set_text(root.clone(), cx));
        if let Some(search) = &self.search {
            search.update(cx, |search, cx| search.set_root(root.clone(), cx));
        }
        let saved = self.saved.get(&root).cloned().unwrap_or_else(|| Saved {
            expanded: HashSet::from([root.clone()]),
            selected: None,
        });
        self.expanded = saved.expanded;
        self.selected = saved.selected;
        self.naming = None;
        self.clip = None;
        self.menu = None;
        self.set_operation_error(None, cx);
        self.list.scroll_to(gpui::ListOffset::default());
        self.read_dir(root, cx);
        self.rebuild(cx);
        true
    }
    fn available(&self) -> bool {
        !self.cwd.is_empty() && self.cwd != "~"
    }
}

impl Focusable for Explorer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search
            .as_ref()
            .map_or_else(|| self.focus.clone(), |search| search.focus_handle(cx))
    }
}

/// A folder typed in the Explorer's box: `~` is home, and a relative path is
/// under the folder shown.
fn typed_folder(typed: &str, cwd: &str, home: &str) -> PathBuf {
    match typed.strip_prefix('~') {
        Some(rest) => PathBuf::from(format!("{home}{rest}")),
        None => Path::new(cwd).join(typed),
    }
}

/// Smallest `old[start..old_end]` -> `new[start..new_end]` replacement that
/// turns `old` into `new` (common prefix and suffix are left untouched).
fn changed_range<T: PartialEq>(old: &[T], new: &[T]) -> (usize, usize, usize) {
    let start = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[start..]
        .iter()
        .rev()
        .zip(new[start..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (start, old.len() - suffix, new.len() - suffix)
}

#[cfg(test)]
mod tests;
