//! Mono workspaces: one runtime per identity, shared by the rail, title tabs,
//! overview, settings and floating windows.
pub(crate) mod cards;
mod changes;
mod chat;
mod color_picker;
mod dashboard;
mod files;
mod filters;
mod floating;
mod host;
mod memory_ui;
mod mobile;
mod notifications;
mod overview;
mod panel_stack;
mod panels;
mod scheduler;
mod search;
#[cfg(all(target_os = "macos", not(test)))]
mod status;
#[cfg(test)]
mod tests;
mod view;
mod work;
use crate::{
    app::shell::Shell,
    chat::conversation::{Conversation, MonoSubmitted, SessionChanged},
    runtime::{Database, locked},
    ui::{
        input::{self, Input},
        popover::{Anchor, AnchorSlot, Menu, MenuEvent, menu_popover, popover},
        theme::*,
    },
};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, Focusable, MouseButton, Role, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px, rgb,
};
use monocode_core::{
    mono::{
        self,
        files::AgentFiles,
        habits::{self, Habit},
    },
    provider::Kind,
    session::{
        agent::{AccessMode, Settings},
        mono_model::{Message, Mono, Roster},
    },
    store::Session,
};
use panels::{OpenPicker, OverviewTab, PanelView, Route, SettingsSection};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Duration,
};
pub(crate) struct Changed;
pub(crate) struct TabOpened(pub String);
pub(crate) struct ShowDetails(pub String);
pub(crate) use notifications::{NotificationChanged, NotificationFinished, NotificationPosted};
#[derive(Clone, PartialEq)]
pub(crate) struct OverviewChrome {
    active: Option<String>,
    tab: OverviewTab,
    routed: bool,
    counts: (usize, usize, usize, usize),
}
struct RunningHabit {
    mono: String,
    habit: String,
    view: Entity<Conversation>,
    started: u64,
    launched: u64,
    failure: Option<String>,
    waiting_since: Option<u64>,
    cards: Vec<serde_json::Value>,
    mailbox: Vec<String>,
}
pub(crate) struct Monos {
    #[cfg(all(target_os = "macos", not(test)))]
    status: Option<status::Status>,
    parent_window: Option<gpui::AnyWindowHandle>,
    owner: WeakEntity<Shell>,
    store: Database,
    root: PathBuf,
    prefs: crate::ui::preferences::Preferences,
    pub(crate) roster: Roster,
    pub(crate) active: Option<String>,
    chats: HashMap<String, Entity<Conversation>>,
    bridge: Option<mono::bridge::Server>,
    observations: Vec<Subscription>,
    _bridge_task: Option<gpui::Task<()>>,
    _clock: Option<gpui::Task<()>>,
    writes: Option<gpui::Task<()>>,
    loading_chats: HashSet<String>,
    work: work::Cache,
    pub(crate) error: Option<String>,
    notice_region: AnchorSlot,
    pub(crate) details: Entity<overview::Overview>,
    original_chrome: bool,
    overview_tab: OverviewTab,
    overview_previous: OverviewTab,
    /// When the Overview tab last appeared for a Mono, so its graph draws in.
    dashboard_shown: Option<(String, std::time::Instant)>,
    /// Each Mono's contributions as last loaded, and those loading now.
    contributions: HashMap<String, dashboard::Loaded>,
    contributions_loading: HashSet<String>,
    overview_reveal: Option<crate::project::tabs::TabReveal<OverviewTab>>,
    route: Option<Route>,
    /// Each Mono's details-panel tab and open page, restored when it is
    /// opened again.
    panel_views: HashMap<String, PanelView>,
    /// Drops a restored session page once it proves archived or deleted;
    /// explicit navigation cancels it.
    panel_check: Option<gpui::Task<()>>,
    settings: Option<SettingsSection>,
    settings_focus: gpui::FocusHandle,
    fields: HashMap<String, Entity<Input>>,
    memory_edit: Option<String>,
    memory_adding: bool,
    memory_pending: HashSet<String>,
    field_blurs: HashSet<gpui::EntityId>,
    file_versions: HashMap<String, (String, String)>,
    saving_files: HashSet<String>,
    file_load_revision: u64,
    files: Option<AgentFiles>,
    habits: Vec<Habit>,
    habit_ui: scheduler::HabitUi,
    picker: Option<OpenPicker>,
    settings_anchor: AnchorSlot,
    model_anchor: AnchorSlot,
    subsession_model_anchor: AnchorSlot,
    subsession_effort_anchor: AnchorSlot,
    projects_anchor: AnchorSlot,
    access_anchor: AnchorSlot,
    menu: Option<(String, Entity<Menu>, Vec<Subscription>)>,
    menu_anchor: Anchor,
    documents: HashMap<String, Entity<crate::chat::transcript::TranscriptView>>,
    html_documents: HashMap<String, Entity<crate::ui::html::Html>>,
    /// Native visuals the Mono published in its chat, opened as documents.
    visual_documents: HashMap<String, Entity<crate::chat::visuals::VisualView>>,
    project_changes: changes::ProjectChanges,
    review: Option<Entity<crate::git::review::ReviewPane>>,
    activity: Option<(String, Entity<crate::chat::transcript::TranscriptView>)>,
    activity_turn: Option<usize>,
    rail_ids: Vec<String>,
    running: HashMap<i64, RunningHabit>,
    active_run: Option<i64>,
    completion_work: HashSet<(String, u64)>,
    /// The open background dialog, centred in the window it opened in.
    background: Option<(
        Entity<crate::project::settings::BackgroundDialog>,
        Subscription,
        gpui::Point<gpui::Pixels>,
    )>,
    backgrounds: HashMap<String, Entity<crate::project::background::ProjectBackground>>,
    drag: Option<(String, gpui::Point<gpui::Pixels>, Option<String>)>,
    rail_bounds: HashMap<String, AnchorSlot>,
    skip_click: bool,
    floating: Vec<gpui::AnyWindowHandle>,
    scheduling: bool,
    persisted_messages: HashSet<String>,
    failed_reports: HashSet<String>,
    inflight: HashMap<String, Message>,
    card_refresh: HashMap<String, u64>,
    session_filters: filters::Filters,
    session_filter_anchor: AnchorSlot,
    app_results: HashMap<(i64, String), (String, Result<serde_json::Value, String>)>,
    app_pending: HashSet<(i64, String)>,
}
impl EventEmitter<Changed> for Monos {}
impl EventEmitter<TabOpened> for Monos {}
impl EventEmitter<ShowDetails> for Monos {}
impl Monos {
    pub(crate) fn resource_sessions(
        &self,
        cx: &App,
    ) -> Vec<(i64, crate::footer::resource_pressure::SessionInfo)> {
        self.chats
            .values()
            .chain(self.running.values().map(|r| &r.view))
            .map(|v| {
                let s = &v.read(cx).summary;
                (
                    s.id,
                    crate::footer::resource_pressure::SessionInfo {
                        title: s.title.clone().into(),
                        provider: s.provider.clone(),
                    },
                )
            })
            .collect()
    }
    pub(crate) fn stop_session(&mut self, id: i64, cx: &mut Context<Self>) {
        if let Some(view) = self
            .chats
            .values()
            .chain(self.running.values().map(|r| &r.view))
            .find(|v| v.read(cx).summary.id == id)
        {
            view.update(cx, |v, cx| v.stop_if_busy(cx));
        }
    }
    pub(crate) fn bind_window(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.parent_window = Some(window.window_handle());
        self.sync_status(cx);
    }
    fn sync_status(&mut self, _cx: &mut Context<Self>) {
        #[cfg(all(target_os = "macos", not(test)))]
        {
            if !self.prefs.mono_menu_bar_icon {
                self.status = None;
                return;
            }
            if self.status.is_none() {
                self.status = status::Status::new();
            }
            let entity = _cx.weak_entity();
            let app = std::cell::RefCell::new(_cx.to_async());
            let run = std::rc::Rc::new(move |id: &str| {
                let _ = entity.update(&mut *app.borrow_mut(), |this, cx| {
                    if id == "quit" {
                        cx.quit();
                    } else if id == "composer" {
                        // The composer is its own panel: the workspace window
                        // stays where it is, as in the Tauri menu.
                        if let Some(handle) = this.parent_window {
                            let owner = this.owner.clone();
                            let app: &mut App = cx;
                            app.defer(move |cx| {
                                let _ = handle.update(cx, |_, window, cx| {
                                    let _ =
                                        owner.update(cx, |o, cx| o.mono_quick_compose(window, cx));
                                });
                            });
                        }
                    } else if this.roster.get(id).is_some() {
                        // Only the floating chat opens; the workspace keeps
                        // whichever Mono or session it was showing.
                        this.open_floating(id.into(), cx);
                    }
                });
            });
            let monos = self
                .roster
                .monos
                .iter()
                .map(|m| (m.id.clone(), m.label()))
                .collect::<Vec<_>>();
            if let Some(status) = &mut self.status {
                status.update(&monos, run);
            }
        }
    }
    pub(crate) fn sidebar_preferences(
        &mut self,
        id: &str,
        show: bool,
        folders: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(m) = self.roster.get_mut(id) {
            m.show_started_sessions_in_sidebar = show;
            m.use_sidebar_folders = folders;
        }
        let sessions = self
            .roster
            .get(id)
            .map(|m| m.launches.iter().map(|l| l.session_id).collect::<Vec<_>>())
            .unwrap_or_default();
        for session in sessions {
            self.group_launch(id, session, cx);
        }
        self.configure_chat(id, cx);
        self.save(cx);
    }
    pub(crate) fn new(
        store: Database,
        owner: WeakEntity<Shell>,
        prefs: crate::ui::preferences::Preferences,
        active: Option<String>,
        notice_region: AnchorSlot,
        cx: &mut Context<Self>,
    ) -> Self {
        let (roster, root, roster_error) = match locked(&store) {
            Ok(db) => match Roster::load(&db) {
                Ok(roster) => (roster, mono::root(&db), None),
                Err(error) => (Roster::default(), mono::root(&db), Some(error)),
            },
            Err(error) => (
                Roster::default(),
                std::env::temp_dir().join(mono::new_id()),
                Some(error),
            ),
        };
        let (bridge, rx) = if cx.is_test() {
            (None, None)
        } else {
            match mono::bridge::Server::start() {
                Ok((server, rx)) => (Some(server), Some(rx)),
                Err(_) => (None, None),
            }
        };
        let persisted_messages = roster
            .monos
            .iter()
            .flat_map(|m| m.outbox.iter().map(|q| q.id.clone()))
            .collect();
        let session_filters = locked(&store)
            .ok()
            .and_then(|s| s.setting("mono-session-filters").ok().flatten())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let manager = cx.entity();
        let details = cx.new(|cx| overview::Overview::new(manager, cx));
        let mut this = Self {
            #[cfg(all(target_os = "macos", not(test)))]
            status: None,
            parent_window: None,
            owner,
            store,
            root,
            prefs,
            roster,
            active: None,
            chats: HashMap::new(),
            bridge,
            observations: Vec::new(),
            _bridge_task: None,
            _clock: None,
            writes: None,
            loading_chats: HashSet::new(),
            work: work::Cache::default(),
            error: roster_error,
            notice_region,
            details,
            original_chrome: false,
            overview_tab: OverviewTab::Overview,
            overview_previous: OverviewTab::Overview,
            dashboard_shown: None,
            contributions: HashMap::new(),
            contributions_loading: HashSet::new(),
            overview_reveal: None,
            route: None,
            panel_views: HashMap::new(),
            panel_check: None,
            settings: None,
            settings_focus: cx.focus_handle(),
            fields: HashMap::new(),
            memory_edit: None,
            memory_adding: false,
            memory_pending: HashSet::new(),
            field_blurs: HashSet::new(),
            file_versions: HashMap::new(),
            saving_files: HashSet::new(),
            file_load_revision: 0,
            files: None,
            habits: Vec::new(),
            habit_ui: scheduler::HabitUi::default(),
            picker: None,
            settings_anchor: AnchorSlot::default(),
            model_anchor: AnchorSlot::default(),
            subsession_model_anchor: AnchorSlot::default(),
            subsession_effort_anchor: AnchorSlot::default(),
            projects_anchor: AnchorSlot::default(),
            access_anchor: AnchorSlot::default(),
            menu: None,
            menu_anchor: Anchor::Point(gpui::Point::default()),
            documents: HashMap::new(),
            html_documents: HashMap::new(),
            visual_documents: HashMap::new(),
            project_changes: Default::default(),
            review: None,
            activity: None,
            activity_turn: None,
            rail_ids: Vec::new(),
            running: HashMap::new(),
            active_run: None,
            completion_work: HashSet::new(),
            background: None,
            backgrounds: HashMap::new(),
            drag: None,
            rail_bounds: HashMap::new(),
            skip_click: false,
            floating: Vec::new(),
            scheduling: false,
            persisted_messages,
            failed_reports: HashSet::new(),
            inflight: HashMap::new(),
            card_refresh: HashMap::new(),
            session_filters,
            session_filter_anchor: AnchorSlot::default(),
            app_results: HashMap::new(),
            app_pending: HashSet::new(),
        };
        if let Some(rx) = rx {
            this._bridge_task = Some(cx.spawn(async move |this, cx| {
                while let Ok(call) = rx.recv().await {
                    if this
                        .update_in(cx, |this, window, cx| this.app_call(call, window, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
        this._clock = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(10))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        if this.prefs.monos
            && let Some(id) = active.filter(|id| this.roster.get(id).is_some())
        {
            this.active = Some(id.clone());
            this.ensure_chat(&id, cx);
            this.reload_files(&id, cx);
        }
        this
    }
    pub(crate) fn is_mono_session(&self, id: i64) -> bool {
        self.roster.hidden_sessions.contains(&id)
            || self.roster.of_session(id).is_some()
            || self
                .roster
                .monos
                .iter()
                .any(|m| m.habit_sessions.contains(&id))
    }
    pub(crate) fn hidden_session(&self, id: i64) -> bool {
        self.is_mono_session(id)
            || self.roster.monos.iter().any(|m| {
                !m.show_started_sessions_in_sidebar && m.launches.iter().any(|l| l.session_id == id)
            })
    }
    pub(crate) fn set_preferences(
        &mut self,
        prefs: crate::ui::preferences::Preferences,
        cx: &mut Context<Self>,
    ) {
        let was = self.prefs.monos;
        self.prefs = prefs;
        if was && !self.prefs.monos {
            self.active = None;
        }
        cx.notify();
    }
    pub(crate) fn set_original_chrome(&mut self, original: bool, cx: &mut Context<Self>) {
        if self.original_chrome == original {
            return;
        }
        self.original_chrome = original;
        self.details
            .update(cx, |view, cx| view.set_original_chrome(original, cx));
        cx.notify();
    }
    /// The session the details panel shows as a page, if any.
    pub(crate) fn routed_session(&self) -> Option<i64> {
        match self.route {
            Some(Route::Session(id)) => Some(id),
            _ => None,
        }
    }
    fn show_overview(&self, cx: &mut Context<Self>) {
        if let Some(id) = self.active.clone() {
            cx.emit(ShowDetails(id));
        }
    }
    pub(crate) fn open_current_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.active.clone() else {
            return;
        };
        self.settings = Some(SettingsSection::General);
        window.focus(&self.settings_focus, cx);
        self.reload_files(&id, cx);
        cx.notify();
    }
    pub(crate) fn current(&self) -> Option<&Mono> {
        self.active.as_deref().and_then(|id| self.roster.get(id))
    }
    pub(crate) fn account_changed(
        &mut self,
        kind: Kind,
        id: &str,
        removed: bool,
        cx: &mut Context<Self>,
    ) {
        for view in self.chats.values() {
            if view.read(cx).summary.provider == kind.id()
                && view.read(cx).settings.account.id == id
            {
                view.update(cx, |view, cx| {
                    if removed {
                        view.account_unavailable(cx)
                    } else {
                        view.account_authenticated(cx)
                    }
                });
            }
        }
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.sync_status(cx);
        cx.emit(Changed);
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        let roster = self.roster.clone();
        let store = self.store.clone();
        let queued = roster
            .monos
            .iter()
            .map(|m| {
                (
                    m.id.clone(),
                    m.outbox.iter().map(|q| q.id.clone()).collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let previous = self.writes.take();
        self.writes = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let work = cx.background_executor().spawn(async move {
                let db = locked(&store)?;
                roster.save(&db)
            });
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Err(error) => {
                    this.error = Some(error);
                    cx.notify();
                }
                Ok(()) => {
                    for (id, messages) in queued {
                        this.persisted_messages.extend(messages);
                        this.dispatch(&id, cx);
                    }
                }
            });
        }));
        self.changed(cx);
    }
    pub(crate) fn dismiss_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.picker.take().is_some() || self.menu.take().is_some() {
            cx.notify();
            return true;
        }
        if self.settings.take().is_none() {
            return false;
        }
        self.save_open_file(cx);
        if let Some(chat) = self.chat() {
            window.focus(&chat.read(cx).input.focus_handle(cx), cx);
        }
        cx.notify();
        true
    }
    pub(crate) fn create(
        &mut self,
        projects: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.roster.create(projects).id.clone();
        self.save(cx);
        self.open(&id, window, cx);
        self.settings = Some(SettingsSection::General);
        window.focus(&self.settings_focus, cx);
        self.changed(cx);
    }
    fn reload_files(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.active.as_deref() != Some(id) {
            return;
        }
        self.file_load_revision += 1;
        let revision = self.file_load_revision;
        let Some(mono) = self.roster.get(id).cloned() else {
            return;
        };
        let root = self.root.clone();
        let id = id.to_string();
        let work = cx.background_executor().spawn(async move {
            Ok::<_, String>((
                mono::load_files(&root, &mono)?,
                habits::load(&root, &mono.id)?,
            ))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.active.as_deref() == Some(&id) && this.file_load_revision == revision {
                    match result {
                        Ok((files, habits)) => {
                            this.files = Some(files);
                            this.habits = habits;
                        }
                        Err(error) => this.error = Some(error),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }
}
pub(crate) struct MonoSession {
    pub id: i64,
    pub title: String,
    pub cwd: String,
    pub busy: bool,
    pub has_messages: bool,
    pub archived: bool,
    pub updated_at: u64,
    pub needs_input: bool,
    pub branch: Option<String>,
    pub model: SharedString,
}
