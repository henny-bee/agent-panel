//! Global search, as the original full page (SearchView.tsx) or, when the
//! `search_panel` preference is on, a floating command panel.
mod view;
use crate::{
    runtime::{Database, locked},
    ui::{
        input::{self, Input},
        theme::*,
    },
};
use gpui::{
    AnyElement, App, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, HighlightStyle,
    MouseButton, MouseMoveEvent, Pixels, Point, Render, Role, ScrollHandle, StyledText,
    Subscription, Task, Window, canvas, div, fill, point, prelude::*, px, relative, size,
};
use monocode_core::{
    session::transcript::Row,
    store::{
        Session,
        search::{self, Hit, Scope, Target},
    },
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub(crate) enum Event {
    Open(Target, String),
    Close,
}
#[derive(Default)]
pub(crate) struct Sources {
    pub summaries: Vec<Session>,
    pub live: HashMap<i64, Arc<Vec<Row>>>,
    pub projects: Vec<String>,
    pub recent: Vec<String>,
    pub logos: HashMap<String, String>,
    pub classic: bool,
    pub panel: bool,
}
pub(crate) struct SearchView {
    pub(crate) input: Entity<Input>,
    cwd: String,
    store: Database,
    summaries: Vec<Session>,
    live: HashMap<i64, Arc<Vec<Row>>>,
    projects: Vec<String>,
    recent: Vec<String>,
    logos: HashMap<String, String>,
    classic: bool,
    original_header: Option<f32>,
    /// Floating command panel; otherwise the original full page.
    panel: bool,
    files: Vec<monocode_core::files::ProjectFile>,
    scope: Scope,
    query: String,
    hits: Vec<Hit>,
    remote: Vec<Hit>,
    local: Vec<Hit>,
    live_task: Option<Task<()>>,
    live_revision: u64,
    content: Vec<Hit>,
    active: usize,
    scroll: ScrollHandle,
    pointer: Option<Point<Pixels>>,
    sessions_loading: bool,
    files_loading: bool,
    truncated: (bool, bool),
    error: Option<String>,
    cancel: Arc<AtomicBool>,
    listing_cancel: Arc<AtomicBool>,
    debounce: Option<Task<()>>,
    _changed: Subscription,
}
impl EventEmitter<Event> for SearchView {}
impl Focusable for SearchView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}
impl Drop for SearchView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.listing_cancel.store(true, Ordering::Release);
    }
}
impl SearchView {
    pub(crate) fn new(
        cwd: String,
        store: Database,
        sources: Sources,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            if sources.panel {
                Input::command_field("Search", "Search conversations, files, projects…", cx)
            } else {
                Input::field("Search", "Search everything...", content(0.4), cx)
            }
        });
        let changed = cx.subscribe(&input, |this, _, _: &input::Changed, cx| {
            this.query = this.input.read(cx).content.trim().to_owned();
            this.active = 0;
            this.refresh(cx);
        });
        let listing_cancel = Arc::new(AtomicBool::new(false));
        let flag = listing_cancel.clone();
        let root = cwd.clone();
        let work = cx.background_executor().spawn(async move {
            monocode_core::files::list_project_files_sync_cancellable(&root, Some(&flag))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(files) = result {
                    this.files = files;
                    this.rank(cx);
                }
            });
        })
        .detach();
        let mut view = Self {
            input,
            cwd,
            store,
            summaries: sources.summaries,
            live: sources.live,
            projects: sources.projects,
            recent: sources.recent,
            logos: sources.logos,
            classic: sources.classic,
            original_header: None,
            panel: sources.panel,
            files: vec![],
            scope: Scope::All,
            query: String::new(),
            hits: vec![],
            remote: vec![],
            local: vec![],
            live_task: None,
            live_revision: 0,
            content: vec![],
            active: 0,
            scroll: ScrollHandle::new(),
            pointer: None,
            sessions_loading: false,
            files_loading: false,
            truncated: (false, false),
            error: None,
            cancel: Arc::new(AtomicBool::new(false)),
            listing_cancel,
            debounce: None,
            _changed: changed,
        };
        // Recent conversations show at once; recent files join once listed.
        if view.panel {
            view.hits = view.suggestions();
        }
        view
    }
    pub(crate) fn set_classic(&mut self, classic: bool, cx: &mut Context<Self>) {
        self.classic = classic;
        cx.notify();
    }
    /// Original SearchView owns a native title strip and reserves the leading
    /// space occupied by the shared window navigation above its compact rail.
    pub(crate) fn set_original_header(&mut self, inset: Option<f32>, cx: &mut Context<Self>) {
        if self.original_header == inset {
            return;
        }
        self.original_header = inset;
        cx.notify();
    }
    pub(crate) fn draft(&self, cx: &App) -> (String, Scope) {
        (self.input.read(cx).content.to_string(), self.scope)
    }
    pub(crate) fn restore_draft(&mut self, draft: (String, Scope), cx: &mut Context<Self>) {
        self.scope = draft.1;
        self.input
            .update(cx, |input, cx| input.set_text(draft.0, cx));
    }
    pub(crate) fn select_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.scope = scope;
        self.active = 0;
        self.refresh(cx);
    }
    pub(crate) fn update_session(
        &mut self,
        summary: Session,
        rows: Arc<Vec<Row>>,
        cx: &mut Context<Self>,
    ) {
        if let Some(old) = self.summaries.iter_mut().find(|s| s.id == summary.id) {
            *old = summary.clone();
        } else {
            self.summaries.push(summary.clone());
        }
        self.live.insert(summary.id, rows);
        self.live_revision += 1;
        if !self.query.is_empty() {
            self.rank(cx);
            self.refresh_live(cx);
        }
    }
    fn refresh_live(&mut self, cx: &mut Context<Self>) {
        if self.query.is_empty() || !self.scope.sessions() || self.live_task.is_some() {
            return;
        }
        let check = self.cancel.clone();
        self.live_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let Ok((work, revision)) = this.update(cx, |this, cx| {
                let live = this.live.clone();
                let summaries = this.summaries.clone();
                let query = this.query.clone();
                let flag = this.cancel.clone();
                (
                    cx.background_executor().spawn(async move {
                        summaries
                            .into_iter()
                            .flat_map(|s| {
                                live.get(&s.id)
                                    .map(|rows| search::messages(&s, rows, &query, &flag))
                                    .unwrap_or_default()
                            })
                            .collect::<Vec<_>>()
                    }),
                    this.live_revision,
                )
            }) else {
                return;
            };
            let hits = work.await;
            if check.load(Ordering::Acquire) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.local = hits;
                this.live_task = None;
                this.rank(cx);
                if this.live_revision != revision {
                    this.refresh_live(cx);
                }
            });
        }));
    }
    fn rank(&mut self, cx: &mut Context<Self>) {
        self.hits = if self.query.is_empty() {
            if self.panel {
                self.suggestions()
            } else {
                vec![]
            }
        } else {
            search::grouped(
                search::titles(&self.summaries, &self.query)
                    .into_iter()
                    .chain(self.local.clone())
                    .chain(self.remote.clone())
                    .chain(search::filenames(&self.files, &self.query, &self.recent))
                    .chain(self.content.clone())
                    .chain(search::projects(&self.projects, &self.query)),
                self.scope,
            )
        };
        if self.active >= self.hits.len() {
            self.active = 0;
        }
        self.pointer = None;
        cx.notify();
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Release);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.debounce = None;
        self.live_task = None;
        self.local.clear();
        self.remote.clear();
        self.content.clear();
        self.error = None;
        self.truncated = (false, false);
        self.sessions_loading = !self.query.is_empty() && self.scope.sessions();
        self.files_loading = !self.query.is_empty() && self.scope.files();
        self.rank(cx);
        self.scroll.set_offset(point(px(0.), px(0.)));
        if self.query.is_empty() {
            return;
        }
        self.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let _ = this.update(cx, |this, cx| this.start(cx));
        }));
    }
    fn start(&mut self, cx: &mut Context<Self>) {
        if self.scope.sessions() {
            let store = self.store.clone();
            let query = self.query.clone();
            let flag = self.cancel.clone();
            let check = flag.clone();
            let live = self.live.clone();
            let summaries = self.summaries.clone();
            let revision = self.live_revision;
            let work = cx.background_executor().spawn(async move {
                let mut local = vec![];
                for s in summaries {
                    if let Some(rows) = live.get(&s.id) {
                        local.extend(search::messages(&s, rows, &query, &flag));
                    }
                }
                let saved = locked(&store)
                    .and_then(|db| db.search_reader())
                    .and_then(|reader| match reader {
                        Some(reader) => reader.search_sessions(&query, flag.clone()),
                        None => {
                            locked(&store).and_then(|db| db.search_sessions(&query, flag.clone()))
                        }
                    });
                (local, saved)
            });
            cx.spawn(async move |this, cx| {
                let (local, saved) = work.await;
                if check.load(Ordering::Acquire) {
                    return;
                }
                let _ = this.update(cx, |this, cx| {
                    this.sessions_loading = false;
                    match saved {
                        Ok((hits, truncated)) => {
                            this.remote = hits;
                            this.truncated.0 = truncated;
                        }
                        Err(e) => this.error = Some(e),
                    }
                    if this.live_revision == revision {
                        this.local = local;
                    } else {
                        this.refresh_live(cx);
                    }
                    this.rank(cx);
                });
            })
            .detach();
        }
        if self.scope.files() {
            let cwd = self.cwd.clone();
            let query = self.query.clone();
            let flag = self.cancel.clone();
            let check = flag.clone();
            let work = cx.background_executor().spawn(async move {
                search::project::search_project(
                    &std::path::PathBuf::from(&cwd),
                    &search::project::SearchOptions {
                        cwd: cwd.clone(),
                        query: query.clone(),
                        case_sensitive: false,
                        whole_word: false,
                        regex: false,
                        include: None,
                        exclude: None,
                        search_id: String::new(),
                    },
                    &flag,
                )
                .map(|r| (search::content(r.matches, &query), r.truncated))
            });
            cx.spawn(async move |this, cx| {
                let result = work.await;
                if check.load(Ordering::Acquire) {
                    return;
                }
                let _ = this.update(cx, |this, cx| {
                    this.files_loading = false;
                    match result {
                        Ok((hits, truncated)) => {
                            this.content = hits;
                            this.truncated.1 = truncated;
                        }
                        Err(e) => this.error = Some(e),
                    }
                    this.rank(cx);
                });
            })
            .detach();
        }
    }
    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.input.read(cx).is_composing() || self.hits.is_empty() {
            return;
        }
        self.active = (self.active as isize + delta).rem_euclid(self.hits.len() as isize) as usize;
        self.pointer = None;
        self.scroll.scroll_to_item(self.child_index(self.active));
        cx.notify();
    }
    fn pick(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.input.read(cx).is_composing() {
            return;
        }
        if let Some(hit) = self.hits.get(ix) {
            cx.emit(Event::Open(hit.target.clone(), self.query.clone()));
        }
    }
}
/// The heading a result sits under; consecutive hits share one.
fn section(target: &Target, suggesting: bool) -> &'static str {
    match (target, suggesting) {
        (Target::Conversation(_), true) => "Recent conversations",
        (Target::Conversation(_), false) => "Conversations",
        (Target::Message { .. }, _) => "Messages",
        (Target::File { location: None, .. }, true) => "Recent files",
        (Target::File { location: None, .. }, false) => "Files",
        (Target::File { .. }, _) => "In file contents",
        (Target::Project(_), _) => "Projects",
    }
}
fn keycap(text: impl Into<gpui::SharedString>) -> gpui::Div {
    div()
        .min_w(px(18.))
        .h(px(18.))
        .px(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .bg(content(0.06))
        .border_1()
        .border_color(content(0.08))
        .text_size(px(10.5))
        .text_color(content(0.55))
        .child(text.into())
}
fn hint(keys: &[&'static str], label: &'static str) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .children(keys.iter().map(|k| keycap(*k)))
        .child(div().pl(px(2.)).child(label))
}
impl SearchView {
    /// The list child that holds hit `ix`, counting the notice and headings.
    fn child_index(&self, ix: usize) -> usize {
        let suggesting = self.query.is_empty();
        let mut child = usize::from(self.truncated.0 || self.truncated.1);
        if !self.panel {
            return child + ix;
        }
        let mut last = None;
        for (i, hit) in self.hits.iter().enumerate().take(ix + 1) {
            let heading = section(&hit.target, suggesting);
            if last != Some(heading) {
                child += 1;
                last = Some(heading);
            }
            if i < ix {
                child += 1;
            }
        }
        child
    }
    fn cycle_scope(&mut self, delta: isize, cx: &mut Context<Self>) {
        let ix = Scope::ALL
            .iter()
            .position(|(s, _)| *s == self.scope)
            .unwrap_or(0) as isize;
        let next = (ix + delta).rem_euclid(Scope::ALL.len() as isize) as usize;
        self.select_scope(Scope::ALL[next].0, cx);
    }
}
impl Render for SearchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.panel {
            self.render_panel(cx).into_any_element()
        } else {
            self.render_page(window, cx).into_any_element()
        }
    }
}
impl SearchView {}

#[cfg(test)]
mod tests;
