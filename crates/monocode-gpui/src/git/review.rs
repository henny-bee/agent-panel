//! The file pane a review opens beside the chat (FilePane.tsx with
//! SurfaceTabs.tsx), holding files, Changes and commit tabs. WorkingTreeDiff.tsx and
//! CommitDiff.tsx load their files four at a time off the UI thread and
//! build each diff there too; the view only lays out rows.
mod loading;
mod tabs;
use crate::{
    editor::{
        diff_view::{DiffEvent, DiffFile, DiffView},
        file_editor::{FileEditor, FileEvent},
    },
    runtime::git_status::{GitChanged, ProjectGit},
    ui::{
        motion::pane::PaneReveal,
        panel::{selection, spinner, stroke},
        popover::{Anchor, Menu, MenuAction, MenuEvent, menu_popover},
        theme::{RED_400, *},
    },
};
use gpui::{
    AnyElement, Bounds, Context, CursorStyle, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Role, ScrollHandle,
    SharedString, Subscription, Task, Window, div, prelude::*, px, rgb,
};
use monocode_core::vcs::{
    diff,
    git::scm::{self, ChangedFile, FileDiff, FileDiffVersion},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

const CONCURRENCY: usize = 4;
const MAX_CACHED_SOURCES: usize = 8;
const MAX_CACHED_TEXT_BYTES: usize = 32 * 1024 * 1024;

/// A review tab as the workspace saves it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum ReviewTab {
    File {
        cwd: String,
        path: String,
    },
    /// Working-tree changes, one side when opened from a sidebar section.
    Changes {
        cwd: String,
        path: Option<String>,
        staged: Option<bool>,
    },
    Commit {
        cwd: String,
        sha: String,
        short_sha: String,
        subject: String,
    },
    /// SessionChangesDiff: a session's recorded turn, focused on one file.
    Session {
        cwd: String,
        session: i64,
        path: Option<String>,
    },
}
impl ReviewTab {
    /// SurfaceTabs's `surfaceTabPresentation`.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::File { path, .. } => std::path::Path::new(path)
                .file_name()
                .map_or_else(|| path.clone(), |p| p.to_string_lossy().into_owned()),
            Self::Changes {
                path: Some(path), ..
            } => std::path::Path::new(path)
                .file_name()
                .map_or_else(|| path.clone(), |p| p.to_string_lossy().into_owned()),
            Self::Changes { staged, .. } => {
                if *staged == Some(true) {
                    "Staged Changes".into()
                } else {
                    "Changes".into()
                }
            }
            Self::Commit {
                short_sha, subject, ..
            } => {
                let subject = subject.trim();
                if subject.is_empty() {
                    short_sha.clone()
                } else {
                    subject.into()
                }
            }
            Self::Session { .. } => "Session Changes".into(),
        }
    }
    /// `editorTabKey`: one Changes tab per checkout and side, one per commit.
    pub(crate) fn same_tab(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::File { path: a, .. }, Self::File { path: b, .. }) => a == b,
            (
                Self::Changes {
                    cwd: a,
                    staged: x,
                    path: p,
                },
                Self::Changes {
                    cwd: b,
                    staged: y,
                    path: q,
                },
            ) => a == b && p == q && (x == y || x.is_none() || y.is_none()),
            (Self::Commit { cwd: a, sha: x, .. }, Self::Commit { cwd: b, sha: y, .. }) => {
                a == b && x == y
            }
            (
                Self::Session {
                    cwd: a, session: x, ..
                },
                Self::Session {
                    cwd: b, session: y, ..
                },
            ) => a == b && x == y,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ReviewState {
    pub tabs: Vec<ReviewTab>,
    pub active: usize,
    /// The pane's share of the split.
    pub ratio: f32,
    /// The chat beside it was closed; the review fills the card.
    pub chat_hidden: bool,
}

pub(crate) enum ReviewEvent {
    Changed,
    Comment(String),
    DragPane(Point<Pixels>),
    /// The last tab closed.
    Empty,
    WordWrap(bool),
}

#[derive(Clone)]
struct Loaded {
    version: Option<FileDiffVersion>,
    diff: Arc<FileDiff>,
    unified: Option<Arc<Vec<diff::Block>>>,
    additions: u32,
    deletions: u32,
    error: Option<String>,
}

/// One tab's files and diffs.
struct Source {
    tab: ReviewTab,
    view: Entity<DiffView>,
    editor: Option<Entity<FileEditor>>,
    editor_mode: bool,
    _editor_events: Option<Subscription>,
    files: Option<Vec<ChangedFile>>,
    error: Option<String>,
    loaded: HashMap<String, Arc<Loaded>>,
    generation: Arc<AtomicU64>,
    complete: bool,
    loading: bool,
    refresh_pending: bool,
    _tasks: Vec<Task<()>>,
    _events: Subscription,
}
impl Source {
    fn cancel(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self._tasks.clear();
        self.loading = false;
        self.refresh_pending = false;
    }
}

/// Closed diff tabs retain their models, viewport and completed highlights.
/// Editors are deliberately excluded, so no dirty document is kept here.
#[derive(Default)]
pub(crate) struct ReviewCache {
    sources: VecDeque<CachedSource>,
}
struct CachedSource {
    tab: ReviewTab,
    view: Entity<DiffView>,
    files: Option<Vec<ChangedFile>>,
    loaded: HashMap<String, Arc<Loaded>>,
    complete: bool,
    text_bytes: usize,
}
impl ReviewCache {
    fn matches(a: &ReviewTab, b: &ReviewTab) -> bool {
        match (a, b) {
            (
                ReviewTab::Changes {
                    cwd: a, staged: x, ..
                },
                ReviewTab::Changes {
                    cwd: b, staged: y, ..
                },
            ) => a == b && x == y,
            (
                ReviewTab::Commit { cwd: a, sha: x, .. },
                ReviewTab::Commit { cwd: b, sha: y, .. },
            ) => a == b && x == y,
            _ => false,
        }
    }
    fn take(&mut self, tab: &ReviewTab) -> Option<CachedSource> {
        let ix = self
            .sources
            .iter()
            .position(|s| Self::matches(&s.tab, tab))?;
        self.sources.remove(ix)
    }
    fn remember(&mut self, source: &Source) {
        if source.editor.is_some() || source.files.is_none() || source.error.is_some() {
            return;
        }
        let text_bytes = source
            .loaded
            .values()
            .map(|loaded| {
                loaded.diff.original.len()
                    + loaded.diff.current.len()
                    + loaded.unified.as_ref().map_or(0, |blocks| {
                        blocks
                            .iter()
                            .flat_map(|b| b.lines())
                            .map(|l| l.text.len())
                            .sum::<usize>()
                    })
            })
            .sum();
        self.sources.retain(|s| !Self::matches(&s.tab, &source.tab));
        if text_bytes > MAX_CACHED_TEXT_BYTES {
            return;
        }
        self.sources.push_back(CachedSource {
            tab: source.tab.clone(),
            view: source.view.clone(),
            files: source.files.clone(),
            loaded: source.loaded.clone(),
            complete: source.complete,
            text_bytes,
        });
        while self.sources.len() > MAX_CACHED_SOURCES
            || self.sources.iter().map(|s| s.text_bytes).sum::<usize>() > MAX_CACHED_TEXT_BYTES
        {
            self.sources.pop_front();
        }
    }
}

pub(crate) struct ReviewPane {
    git: Entity<ProjectGit>,
    focus: FocusHandle,
    /// Bodies replaced before the next paint may still own keyboard focus.
    retiring_focus: Vec<FocusHandle>,
    pub(crate) state: ReviewState,
    sources: Vec<Source>,
    /// Search results share one replaceable file tab until it is pinned or edited.
    preview_path: Option<String>,
    visible: bool,
    closed: bool,
    deferred: bool,
    /// The deferred body fades from its first rendered frame.
    revealed: Option<PaneReveal>,
    hovered_tab: Option<usize>,
    pub(crate) pane_draggable: bool,
    tab_scroll: ScrollHandle,
    /// Tab animations: opening and closing file tabs, with each tab's last look.
    tab_motion: crate::ui::tabs::motion::TabMotion<String, ReviewTab>,
    tab_drag: Option<FileTabDrag>,
    skip_tab_click: bool,
    pending_close: Option<(usize, bool)>,
    close_prompted: bool,
    menu: Option<(usize, Point<gpui::Pixels>, Entity<Menu>, Subscription)>,
    _subscriptions: Vec<Subscription>,
}
struct FileTabDrag {
    from: usize,
    to: usize,
    origin: Pixels,
    offset: Pixels,
    bounds: Vec<Bounds<Pixels>>,
    active: bool,
}
impl EventEmitter<ReviewEvent> for ReviewPane {}
impl Drop for ReviewPane {
    fn drop(&mut self) {
        for source in &mut self.sources {
            source.cancel();
        }
    }
}

/// workingTreeDiffEntries: staged entries first, or just one side.
fn entries(files: &[ChangedFile], staged: Option<bool>) -> Vec<(String, bool, ChangedFile)> {
    let kinds: &[bool] = match staged {
        Some(true) => &[true],
        Some(false) => &[false],
        None => &[true, false],
    };
    let mut out = Vec::new();
    for &kind in kinds {
        for file in files {
            if (kind && file.staged) || (!kind && file.unstaged) {
                let id = format!(
                    "{}:{}",
                    if kind { "staged" } else { "unstaged" },
                    file.relative
                );
                out.push((id, kind, file.clone()));
            }
        }
    }
    out
}

/// SessionChangesDiff's list: the recorded turn's files, as a commit lists them.
type Listed = (Vec<ChangedFile>, Option<HashMap<String, FileDiffVersion>>);
fn session_files(cwd: &str, session: i64) -> Result<Listed, String> {
    let store = monocode_core::vcs::checkpoint::store().ok_or("Session changes are unavailable")?;
    let status = store.exclusive(|s| s.status(&session.to_string(), cwd))?;
    Ok((
        status
            .files
            .into_iter()
            .map(|f| ChangedFile {
                path: f.path,
                relative: f.relative,
                status: f.status,
                additions: f.additions,
                deletions: f.deletions,
                staged: false,
                unstaged: true,
            })
            .collect(),
        None,
    ))
}
fn session_file_diff(cwd: &str, session: i64, relative: &str) -> Result<FileDiff, String> {
    let store = monocode_core::vcs::checkpoint::store().ok_or("Session changes are unavailable")?;
    store.exclusive(|s| s.file_diff(&session.to_string(), cwd, relative))
}

impl ReviewPane {
    pub(crate) fn new(git: Entity<ProjectGit>, state: ReviewState, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.subscribe(&git, |this, _, _: &GitChanged, cx| {
                this.refresh(cx);
            }),
            cx.observe_global::<crate::ui::preferences::Preferences>(|this, cx| {
                let editor_mode =
                    cx.global::<crate::ui::preferences::Preferences>().diff_view == "editor";
                if this.sources.get(this.state.active).is_some_and(|source| {
                    matches!(source.tab, ReviewTab::Changes { path: Some(_), .. })
                        && source.editor_mode != editor_mode
                }) {
                    this.remember_body_focus(cx);
                }
                for source in &mut this.sources {
                    if matches!(source.tab, ReviewTab::Changes { path: Some(_), .. }) {
                        source.editor_mode = editor_mode;
                        if !editor_mode && let Some(editor) = &source.editor {
                            editor.update(cx, |e, cx| e.set_visible(false, cx));
                        }
                    }
                }
                this.load_active(cx);
                cx.notify();
            }),
        ];
        let mut this = Self {
            git,
            focus: cx.focus_handle(),
            retiring_focus: Vec::new(),
            state: ReviewState {
                ratio: if state.ratio > 0.05 { state.ratio } else { 0.5 },
                ..state
            },
            sources: Vec::new(),
            preview_path: None,
            visible: false,
            closed: false,
            deferred: false,
            revealed: None,
            hovered_tab: None,
            pane_draggable: false,
            tab_scroll: ScrollHandle::new(),
            tab_motion: Default::default(),
            tab_drag: None,
            skip_tab_click: false,
            pending_close: None,
            close_prompted: false,
            menu: None,
            _subscriptions: subscriptions,
        };
        let tabs = this.state.tabs.clone();
        for tab in tabs {
            let source = this.source(tab, cx);
            this.sources.push(source);
        }
        this.state.active = this.state.active.min(this.sources.len().saturating_sub(1));
        this
    }
    fn source(&mut self, tab: ReviewTab, cx: &mut Context<Self>) -> Source {
        let cached = self.git.update(cx, |git, _| git.review_cache.take(&tab));
        let view = cached
            .as_ref()
            .map_or_else(|| cx.new(DiffView::new), |s| s.view.clone());
        let events = cx.subscribe(&view, |this, view, event: &DiffEvent, cx| {
            let Some(ix) = this.sources.iter().position(|s| s.view == view) else {
                return;
            };
            this.diff_event(ix, event, cx);
        });
        let editor_mode = matches!(tab, ReviewTab::File { .. })
            || cx
                .try_global::<crate::ui::preferences::Preferences>()
                .is_none_or(|p| p.diff_view == "editor");
        let editor_path = match &tab {
            ReviewTab::File { path, .. } => Some((path, false)),
            ReviewTab::Changes {
                path: Some(path), ..
            } => Some((path, true)),
            _ => None,
        };
        let editor = if let Some((path, show_diff)) = editor_path {
            Some(cx.new(|cx| {
                let mut editor = if self.deferred || !editor_mode {
                    FileEditor::new_deferred(path.clone(), self.git.clone(), cx)
                } else {
                    FileEditor::new(path.clone(), self.git.clone(), cx)
                };
                editor.show_diff = show_diff;
                editor
            }))
        } else {
            None
        };
        let editor_events = editor.as_ref().map(|editor| {
            cx.subscribe(editor, |this, editor, event: &FileEvent, cx| {
                if matches!(event, FileEvent::Dirty) {
                    let editor = editor.read(cx);
                    if editor.dirty && this.preview_path.as_ref() == Some(&editor.path) {
                        this.preview_path = None;
                    }
                }
                if let FileEvent::AddToChat(text) = event {
                    cx.emit(ReviewEvent::Comment(text.clone()));
                }
                if let FileEvent::WordWrap(wrap) = event {
                    cx.emit(ReviewEvent::WordWrap(*wrap));
                }
                if matches!(event, FileEvent::SaveFailed) {
                    this.pending_close = None;
                    this.close_prompted = false;
                }
                if matches!(event, FileEvent::Saved)
                    && this.pending_close.is_some()
                    && this.closing_editors(cx).is_empty()
                {
                    this.approve_close(cx);
                }
                cx.notify();
            })
        });
        Source {
            editor_mode,
            editor,
            _editor_events: editor_events,
            tab,
            view,
            files: cached.as_ref().and_then(|s| s.files.clone()),
            error: None,
            loaded: cached
                .as_ref()
                .map_or_else(HashMap::new, |s| s.loaded.clone()),
            generation: Arc::new(AtomicU64::new(0)),
            complete: cached.is_some_and(|s| s.complete),
            loading: false,
            refresh_pending: false,
            _tasks: Vec::new(),
            _events: events,
        }
    }
    pub(crate) fn unsaved_files(&self, cx: &gpui::App) -> bool {
        self.sources
            .iter()
            .filter_map(|s| s.editor.as_ref())
            .any(|e| e.read(cx).dirty || e.read(cx).saving)
    }
    #[cfg(test)]
    pub(crate) fn active_editor(&self) -> Option<Entity<FileEditor>> {
        self.sources
            .get(self.state.active)
            .and_then(|s| s.editor.clone())
    }
    pub(crate) fn navigate_file(&mut self, line: usize, column: usize, cx: &mut Context<Self>) {
        if let Some(editor) = self
            .sources
            .get(self.state.active)
            .and_then(|s| s.editor.clone())
        {
            editor.update(cx, |editor, cx| editor.navigate(line, column, cx));
        }
    }
    pub(crate) fn focus_active(&self, window: &mut Window, cx: &mut gpui::App) {
        // Keep workspace shortcuts reachable while the active body is loading,
        // deferred or showing an error. Rendering hands focus to a ready body.
        window.focus(&self.focus, cx);
    }
    fn remember_body_focus(&mut self, cx: &gpui::App) {
        if let Some(source) = self.sources.get(self.state.active) {
            for handle in std::iter::once(source.view.focus_handle(cx))
                .chain(source.editor.as_ref().map(|editor| editor.focus_handle(cx)))
            {
                if !self
                    .retiring_focus
                    .iter()
                    .any(|existing| existing == &handle)
                {
                    self.retiring_focus.push(handle);
                }
            }
        }
    }
    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.sources.len() && ix != self.state.active {
            self.remember_body_focus(cx);
            self.state.active = ix;
            self.sync_state();
            self.load_active(cx);
            cx.emit(ReviewEvent::Changed);
            cx.notify();
        }
    }
    fn sync_state(&mut self) {
        self.state.tabs = self.sources.iter().map(|s| s.tab.clone()).collect();
        if self.preview_path.as_ref().is_some_and(|preview| {
            !self.sources.iter().any(
                |source| matches!(&source.tab, ReviewTab::File { path, .. } if path == preview),
            )
        }) {
            self.preview_path = None;
        }
    }
}

fn log_error(error: &str) {
    eprintln!("monocode: review action failed: {error}");
}

/// Polls every future to completion; the workers share one queue.
async fn futures_join_all(workers: Vec<impl std::future::Future<Output = ()>>) {
    let mut workers: Vec<_> = workers.into_iter().map(Box::pin).collect();
    std::future::poll_fn(move |cx| {
        workers.retain_mut(|w| w.as_mut().poll(cx).is_pending());
        if workers.is_empty() {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    })
    .await
}

impl Render for ReviewPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.retiring_focus)
            .iter()
            .any(|handle| handle.contains_focused(window, cx))
        {
            window.focus(&self.focus, cx);
        }
        self.prompt_close(window, cx);
        // PaneTree's reveal: the files fade in once the pane has arrived.
        if cx.reduce_motion() {
            self.revealed = None;
        }
        let fade = match &self.revealed {
            Some(reveal) => {
                let (opacity, done) = reveal.opacity(std::time::Instant::now());
                if !done {
                    window.request_animation_frame();
                } else {
                    self.revealed = None;
                }
                opacity
            }
            None => 1.,
        };
        let body: AnyElement = match self.sources.get(self.state.active) {
            None => div().into_any_element(),
            Some(_) if self.deferred => div().into_any_element(),
            Some(source) if source.editor_mode && source.editor.is_some() => {
                let editor = source.editor.as_ref().unwrap();
                if self.focus.is_focused(window) {
                    window.focus(&editor.focus_handle(cx), cx);
                }
                editor.clone().into_any_element()
            }
            Some(source) => {
                let cwd = match &source.tab {
                    ReviewTab::File { cwd, .. }
                    | ReviewTab::Changes { cwd, .. }
                    | ReviewTab::Commit { cwd, .. }
                    | ReviewTab::Session { cwd, .. } => cwd,
                };
                if cwd.is_empty() || cwd == "~" {
                    centered()
                        .text_size(px(13.))
                        .text_color(content(0.45))
                        .child("No project folder")
                        .into_any_element()
                } else if let Some(error) = &source.error {
                    centered()
                        .p_6()
                        .flex_col()
                        .text_center()
                        .child(icon("circle-alert", 20.).mb_3().text_color(rgb(RED_400)))
                        .child(div().text_size(px(13.)).text_color(content(1.)).child(
                            match source.tab {
                                ReviewTab::File { .. } => "Couldn’t open file",
                                ReviewTab::Changes { .. } | ReviewTab::Session { .. } => {
                                    "Couldn’t load changes"
                                }
                                ReviewTab::Commit { .. } => "Couldn’t load commit",
                            },
                        ))
                        .child(
                            div()
                                .mt_1()
                                .text_size(px(12.))
                                .text_color(content(0.5))
                                .child(error.clone()),
                        )
                        .into_any_element()
                } else if source.files.is_none() {
                    centered()
                        .child(spinner("review-loading", "loader", 16.))
                        .into_any_element()
                } else {
                    if self.focus.is_focused(window) {
                        window.focus(&source.view.focus_handle(cx), cx);
                    }
                    source.view.clone().into_any_element()
                }
            }
        };
        let menu = self
            .menu
            .as_ref()
            .map(|(_, at, menu, _)| menu_popover(menu, Anchor::Point(*at), cx).gap(0.));
        div()
            .id("review-pane")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .on_mouse_move(
                cx.listener(|this, event: &MouseMoveEvent, _, cx| this.move_tab_drag(event, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_tab_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_tab_drag(cx)),
            )
            .font_weight(FontWeight::NORMAL)
            .child(self.tabs(window, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .opacity(fade)
                    .child(body),
            )
            .children(menu)
    }
}

fn centered() -> gpui::Div {
    div().size_full().flex().items_center().justify_center()
}

#[cfg(test)]
mod tests;
