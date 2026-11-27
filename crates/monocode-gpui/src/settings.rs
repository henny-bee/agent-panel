//! Port of src/features/settings/ui/SettingsView.tsx and
//! src/app/shell/SettingsRail.tsx: the Settings pages, their rail and the
//! breadcrumb, search and actions the inset layout puts in its title bar.
mod data;
mod keymap;
mod management;
mod model;
mod notification_preferences;
mod pages;
mod skill_document;
#[cfg(test)]
mod tests;
pub(crate) mod view;
pub(crate) mod widgets;

pub(crate) use crate::ui::preferences::Preferences;
pub(crate) use keymap::apply as apply_keybindings;
pub(crate) use model::Section;
pub(crate) use pages::command_shortcut;

use crate::ui::{
    input::{self, Input},
    popover::{Align, Anchor, AnchorSlot, Side, popover},
    theme::*,
};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, MouseButton,
    MouseMoveEvent, Role, ScrollHandle, SharedString, Subscription, Task, Window, div, prelude::*,
    px,
};
use std::{cell::RefCell, collections::HashMap, time::Duration};

/// A session the Archive page lists.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ArchivedSession {
    pub id: i64,
    pub title: String,
    pub provider: String,
    pub updated_at: u64,
}

/// What Settings shows about the workspace around it.
#[derive(Clone, Debug, Default)]
pub(crate) struct SettingsContext {
    /// The desktop's shared remote connection and SSH setup service.
    pub remote: Option<std::sync::Arc<monocode_core::integrations::remote::Connections>>,
    pub monos: Vec<MonoSetting>,
    pub classic: bool,
    pub new_layout_boxy: bool,
    /// The selected project, or empty outside one.
    pub cwd: String,
    pub project_label: String,
    pub archived: Vec<ArchivedSession>,
    /// Projects archived from the rail, as (path, label).
    pub archived_projects: Vec<(String, String)>,
    /// Projects the Worktrees, Providers and Inbox pages offer.
    pub projects: Vec<(String, String)>,
    pub sessions: Vec<(i64, String, bool)>,
    pub quick_error: Option<String>,
    pub catalogs: HashMap<String, Vec<(String, String)>>,
    pub inbox_connections: HashMap<String, (bool, String)>,
    /// Mobile access, or `None` when this desktop cannot offer it.
    pub mobile: Option<monocode_core::integrations::mobile::Status>,
    /// The pairing code on show.
    pub mobile_invite: Option<monocode_core::integrations::mobile::Invite>,
}
impl PartialEq for SettingsContext {
    fn eq(&self, other: &Self) -> bool {
        match (&self.remote, &other.remote) {
            (Some(a), Some(b)) if !std::sync::Arc::ptr_eq(a, b) => return false,
            (None, Some(_)) | (Some(_), None) => return false,
            _ => {}
        }
        self.monos == other.monos
            && self.classic == other.classic
            && self.new_layout_boxy == other.new_layout_boxy
            && self.cwd == other.cwd
            && self.project_label == other.project_label
            && self.archived == other.archived
            && self.archived_projects == other.archived_projects
            && self.projects == other.projects
            && self.sessions == other.sessions
            && self.quick_error == other.quick_error
            && self.catalogs == other.catalogs
            && self.inbox_connections == other.inbox_connections
            && self.mobile == other.mobile
            && self.mobile_invite == other.mobile_invite
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MonoSetting {
    pub id: String,
    pub name: String,
    pub mascot: String,
    pub color: u32,
    pub projects: Vec<String>,
    pub show: bool,
    pub folders: bool,
}

pub(crate) enum SettingsEvent {
    WhatsNew,
    CheckAppUpdates,
    CheckHarnessUpdates,
    ManageInboxProvider(String),
    Changed(Box<Preferences>),
    /// The Appearance page's Layout picker: true for Focused, false for Classic.
    NewLayout(bool),
    NewLayoutBoxy(bool),
    Section(Section),
    OpenSession(i64),
    Archive(i64, bool),
    Delete(i64),
    UnarchiveProject(String),
    DeleteProject(String),
    ChooseBackground,
    RemoveWorktree {
        cwd: String,
        path: String,
        delete_sessions: bool,
    },
    ResetMono(String),
    MonoSidebar(String, bool, bool),
    /// Read mobile access again: the router and pairing change on their own.
    MobileRefresh,
    MobileConfigure {
        enabled: bool,
        open_router: bool,
        remote_address: String,
    },
    /// Save what the firewall switch set.
    MobileFirewall(bool),
    /// Start listening when needed, then make a new pairing code.
    MobilePair,
    MobileCancelInvite,
    MobileRevoke(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SliderKind {
    Hue,
    Saturation,
    DarkLightness,
    SidebarOpacity,
    BackgroundEmpty,
    BackgroundSession,
    BackgroundBlur,
}
impl SliderKind {
    fn id(self) -> &'static str {
        match self {
            SliderKind::Hue => "hue",
            SliderKind::Saturation => "saturation",
            SliderKind::DarkLightness => "dark-lightness",
            SliderKind::SidebarOpacity => "sidebar-opacity",
            SliderKind::BackgroundEmpty => "background-empty",
            SliderKind::BackgroundSession => "background-session",
            SliderKind::BackgroundBlur => "background-blur",
        }
    }
    fn label(self) -> &'static str {
        match self {
            SliderKind::Hue => "Hue",
            SliderKind::Saturation => "Saturation",
            SliderKind::DarkLightness => "Dark-mode lightness",
            SliderKind::SidebarOpacity => "Sidebar opacity",
            SliderKind::BackgroundEmpty => "Empty chat background visibility",
            SliderKind::BackgroundSession => "Session background visibility",
            SliderKind::BackgroundBlur => "Chat background blur",
        }
    }
    fn range(self) -> (u32, u32) {
        match self {
            SliderKind::Hue => (0, 360),
            SliderKind::Saturation => (0, 100),
            SliderKind::DarkLightness => (0, 30),
            SliderKind::SidebarOpacity => (15, 100),
            SliderKind::BackgroundEmpty | SliderKind::BackgroundSession => (5, 65),
            SliderKind::BackgroundBlur => (0, 100),
        }
    }
    fn get(self, p: &Preferences) -> u32 {
        match self {
            SliderKind::Hue => p.theme_hue,
            SliderKind::Saturation => p.theme_saturation,
            SliderKind::DarkLightness => p.theme_dark_lightness,
            SliderKind::SidebarOpacity => p.sidebar_opacity,
            SliderKind::BackgroundEmpty => p.chat_background_empty,
            SliderKind::BackgroundSession => p.chat_background_session,
            SliderKind::BackgroundBlur => p.chat_background_blur,
        }
    }
    fn set(self, p: &mut Preferences, value: u32) {
        match self {
            SliderKind::Hue => p.theme_hue = value,
            SliderKind::Saturation => p.theme_saturation = value,
            SliderKind::DarkLightness => p.theme_dark_lightness = value,
            SliderKind::SidebarOpacity => p.sidebar_opacity = value,
            SliderKind::BackgroundEmpty => p.chat_background_empty = value,
            SliderKind::BackgroundSession => p.chat_background_session = value,
            SliderKind::BackgroundBlur => p.chat_background_blur = value,
        }
    }
}

pub(crate) struct SettingsView {
    pub(crate) section: Section,
    prefs: Preferences,
    notification_state: notification_preferences::State,
    ctx: SettingsContext,
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// The row or group search just landed on, flashing for 1.8s.
    revealed: Option<String>,
    pending_scroll: bool,
    _reveal: Option<Task<()>>,
    targets: RefCell<HashMap<&'static str, AnchorSlot>>,
    slider_tracks: RefCell<HashMap<SliderKind, AnchorSlot>>,
    slider_drag: Option<SliderKind>,
    select_open: Option<&'static str>,
    select_active: usize,
    select_focus: FocusHandle,
    /// Filter field atop long selects, and the open select's list scroll.
    select_filter: Entity<Input>,
    select_query: String,
    select_scroll: ScrollHandle,
    select_anchors: RefCell<HashMap<&'static str, AnchorSlot>>,
    search: Entity<Input>,
    search_query: String,
    search_active: usize,
    search_anchor: AnchorSlot,
    /// Where the search last drew, so its text follows the nav rows' size.
    search_in_nav: Option<bool>,
    keybinding_filter: Entity<Input>,
    keybinding_query: String,
    /// The Keybindings row recording new keys.
    recording: Option<String>,
    recording_preview: String,
    recorder_focus: FocusHandle,
    recording_error: Option<String>,
    management: Option<management::Editor>,
    management_generation: u64,
    binary_anchors: RefCell<HashMap<monocode_core::provider::Kind, AnchorSlot>>,
    mcp_show_all: bool,
    mcp_remove_scopes: HashMap<String, String>,
    mcp_busy: bool,
    mcp_health: Option<Task<()>>,
    discovery: data::Discovery,
    discovery_loading: bool,
    discovery_task: Option<Task<()>>,
    selected_project: String,
    provider_scope: String,
    mcp_filter: String,
    fields: HashMap<&'static str, Entity<Input>>,
    connections: pages::ConnectionsState,
    skill_preview: Option<std::path::PathBuf>,
    skill_preview_text: Option<Result<std::sync::Arc<skill_document::DocumentData>, String>>,
    skill_preview_generation: u64,
    skill_preview_task: Option<Task<()>>,
    skill_documents: HashMap<(std::path::PathBuf, bool), Entity<skill_document::SkillDocument>>,
    skill_surface: AnchorSlot,
    skill_filter: Entity<Input>,
    skill_query: String,
    accent_custom: Entity<Input>,
    accent_custom_open: bool,
    accent_anchor: AnchorSlot,
    mobile: pages::MobileState,
    /// Chat background preview, drawn with the chosen effect like the chat panes.
    background_preview: Entity<crate::project::background::ProjectBackground>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<SettingsEvent> for SettingsView {}
impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl SettingsView {
    pub(crate) fn new(
        section: Section,
        prefs: Preferences,
        ctx: SettingsContext,
        cx: &mut Context<Self>,
    ) -> Self {
        if let Some(center) = crate::accounts::get(cx) {
            center.update(cx, |center, _| center.hide_emails());
        }
        let search = cx.new(|cx| {
            let mut input = Input::field("Search settings", "Search settings", content(0.35), cx);
            input.set_font_size(12.);
            input
        });
        let keybinding_filter = cx.new(|cx| {
            let mut input = Input::field("Filter keybindings", "Filter", content(0.35), cx);
            input.set_font_size(12.);
            input
        });
        let accent_custom = cx.new(|cx| {
            let mut input = Input::field("Custom accent color", "#4da3f5", content(0.35), cx);
            input.set_font_size(12.);
            input
        });
        let skill_filter = cx.new(|cx| {
            let mut input = Input::field("Filter skills", "Filter", content(0.35), cx);
            input.set_font_size(12.);
            input
        });
        let select_filter = cx.new(|cx| {
            let mut input = Input::field("Filter options", "Search", content(0.35), cx);
            input.set_font_size(12.);
            input
        });
        let mut subscriptions = vec![
            cx.subscribe(&select_filter, |this, input, _: &input::Changed, cx| {
                this.select_query = input.read(cx).content.to_string();
                this.select_active = 0;
                this.select_scroll.scroll_to_item(0);
                cx.notify();
            }),
            cx.subscribe(&skill_filter, |this, input, _: &input::Changed, cx| {
                this.skill_query = input.read(cx).content.to_string();
                cx.notify();
            }),
            cx.subscribe(&search, |this, input, _: &input::Changed, cx| {
                this.search_query = input.read(cx).content.to_string();
                this.search_active = 0;
                cx.notify();
            }),
            cx.subscribe(&search, |this, _, _: &input::Submitted, cx| {
                let results = model::search(&this.search_query, cx.compositor_name());
                if let Some(result) = results.get(this.search_active).cloned() {
                    this.go(result, cx);
                }
            }),
            cx.subscribe(&keybinding_filter, |this, input, _: &input::Changed, cx| {
                this.keybinding_query = input.read(cx).content.to_string();
                cx.notify();
            }),
            cx.subscribe(&accent_custom, |this, input, _: &input::Changed, cx| {
                let text = input.read(cx).content.trim().to_owned();
                if pages::parse_hex(&text).is_some() {
                    this.update(cx, |p| p.accent_color = Some(text.to_lowercase()));
                }
            }),
        ];
        if let Some(center) = crate::accounts::get(cx) {
            subscriptions.push(cx.observe(&center, |_, _, cx| cx.notify()));
        }
        let background_preview =
            cx.new(|_| crate::project::background::ProjectBackground::preview());
        subscriptions.push(cx.observe(&background_preview, |_, _, cx| cx.notify()));
        let recorder = cx.weak_entity();
        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            let _ = recorder.update(cx, |this, cx| {
                if this.recording.is_some() && this.recorder_focus.is_focused(window) {
                    this.record_key(&event.keystroke, cx);
                    if this.recording.is_none() {
                        window.focus(&this.focus, cx);
                    }
                }
            });
        }));
        let selected_project = ctx.cwd.clone();
        let mut this = Self {
            section,
            prefs,
            notification_state: notification_preferences::State::default(),
            ctx,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            revealed: None,
            pending_scroll: false,
            _reveal: None,
            targets: Default::default(),
            slider_tracks: Default::default(),
            slider_drag: None,
            select_open: None,
            select_active: 0,
            select_focus: cx.focus_handle(),
            select_filter,
            select_query: String::new(),
            select_scroll: ScrollHandle::new(),
            select_anchors: Default::default(),
            search,
            search_query: String::new(),
            search_active: 0,
            search_anchor: AnchorSlot::default(),
            search_in_nav: None,
            keybinding_filter,
            keybinding_query: String::new(),
            recording: None,
            recording_preview: String::new(),
            recorder_focus: cx.focus_handle(),
            recording_error: None,
            management: None,
            management_generation: 0,
            binary_anchors: RefCell::default(),
            mcp_show_all: false,
            mcp_remove_scopes: HashMap::new(),
            mcp_busy: false,
            mcp_health: None,
            discovery: data::Discovery::default(),
            discovery_loading: false,
            discovery_task: None,
            selected_project,
            provider_scope: "global".into(),
            mcp_filter: "all".into(),
            fields: HashMap::new(),
            connections: pages::ConnectionsState::default(),
            skill_preview: None,
            skill_preview_text: None,
            skill_preview_generation: 0,
            skill_preview_task: None,
            skill_documents: HashMap::new(),
            skill_surface: AnchorSlot::default(),
            skill_filter,
            skill_query: String::new(),
            accent_custom,
            accent_custom_open: false,
            accent_anchor: AnchorSlot::default(),
            mobile: pages::MobileState::default(),
            background_preview,
            _subscriptions: subscriptions,
        };
        this.watch_mobile(cx);
        this.watch_connections(cx);
        if this.prefs.notifications {
            this.probe_notification_permission(false, cx);
        }
        this.notification_deadline(cx);
        this.refresh(cx);
        this
    }
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.section == Section::Providers
            && let Some(center) = crate::accounts::get(cx)
        {
            center.update(cx, |center, cx| {
                for kind in monocode_core::provider::accounts::PROVIDERS {
                    center.load_provider(kind, true, cx);
                }
            });
        }
        let cwd = self.selected_project.clone();
        self.discovery_loading = true;
        let work = cx
            .background_executor()
            .spawn(async move { data::discover(&cwd) });
        self.discovery_task = Some(cx.spawn(async move |this, cx| {
            let data = work.await;
            let _ = this.update(cx, |this, cx| {
                this.skill_documents.clear();
                this.discovery = data;
                this.discovery_loading = false;
                if this.section == Section::Mcp {
                    this.refresh_mcp_health(cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(crate) fn set_context(&mut self, ctx: SettingsContext, cx: &mut Context<Self>) {
        if self.ctx != ctx {
            let remote_changed = match (&self.ctx.remote, &ctx.remote) {
                (Some(a), Some(b)) => !std::sync::Arc::ptr_eq(a, b),
                (None, None) => false,
                _ => true,
            };
            self.ctx = ctx;
            self.mobile_context_changed(cx);
            if remote_changed {
                self.leave_connections(cx);
                self.watch_connections(cx);
            }
            cx.notify();
        }
    }
    pub(crate) fn set_preferences(&mut self, prefs: &Preferences, cx: &mut Context<Self>) {
        if &self.prefs != prefs {
            self.prefs = prefs.clone();
            self.notification_deadline(cx);
            cx.notify();
        }
    }
    pub(crate) fn select_section(&mut self, section: Section, cx: &mut Context<Self>) {
        if self.section != section {
            if let Some(center) = crate::accounts::get(cx) {
                center.update(cx, |center, _| center.hide_emails());
            }
            self.management = None;
            self.management_generation += 1;
            self.leave_mobile(cx);
            self.leave_connections(cx);
            self.section = section;
            self.watch_mobile(cx);
            self.watch_connections(cx);
            if section == Section::Mcp {
                self.refresh_mcp_health(cx);
            }
            self.select_open = None;
            self.recording = None;
            self.scroll.set_offset(gpui::point(px(0.), px(0.)));
            cx.notify();
        }
    }
    /// Consume Escape at the innermost Settings surface before the shell closes Settings.
    pub(crate) fn dismiss_transient(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.dismiss_notification_transient(window, cx) {
            return true;
        }
        if self.dismiss_mobile(cx) {
            return true;
        }
        if self.dismiss_connections(cx) {
            return true;
        }
        if self.recording.take().is_some() {
            self.recording_preview.clear();
            window.focus(&self.focus, cx);
            cx.notify();
            return true;
        }
        if self.select_open.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
            return true;
        }
        if !self.search_query.is_empty() {
            self.search_query.clear();
            self.search
                .update(cx, |i, cx| i.set_text(String::new(), cx));
            window.focus(&self.focus, cx);
            cx.notify();
            return true;
        }
        if self.management.is_some() {
            self.close_management(window, cx);
            return true;
        }
        if self.skill_preview.take().is_some() {
            window.focus(&self.skill_filter.read(cx).focus_handle(cx), cx);
            cx.notify();
            return true;
        }
        false
    }
    /// Change preferences and tell the shell to keep them.
    fn update(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Preferences)) {
        change(&mut self.prefs);
        self.notification_deadline(cx);
        cx.emit(SettingsEvent::Changed(Box::new(self.prefs.clone())));
        cx.notify();
    }
    fn restore_defaults(&mut self, cx: &mut Context<Self>) {
        self.accent_custom_open = false;
        self.update(cx, |p| p.restore_appearance());
        // onInsetLayout(INSET_LAYOUT_DEFAULT): React's default is the classic layout.
        cx.emit(SettingsEvent::NewLayout(false));
    }
    /// Jump to a search result, on this page or another.
    fn go(&mut self, result: model::SearchResult, cx: &mut Context<Self>) {
        if result.section != self.section {
            self.select_section(result.section, cx);
            cx.emit(SettingsEvent::Section(result.section));
        }
        self.revealed = result.setting.map(str::to_owned);
        self.pending_scroll = self.revealed.is_some();
        self.search_query.clear();
        self.search
            .update(cx, |input, cx| input.set_text(String::new(), cx));
        self._reveal = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1800))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.revealed = None;
                this.pending_scroll = false;
                cx.notify();
            });
        }));
        cx.notify();
    }
    /// Centre the revealed row in the page once it has been laid out.
    fn scroll_to_revealed(&mut self) {
        let Some(id) = self.revealed.clone() else {
            self.pending_scroll = false;
            return;
        };
        let target = self.targets.borrow().get(id.as_str()).and_then(|s| s.get());
        let viewport = self.scroll.bounds();
        self.pending_scroll = false;
        let Some(target) = target else {
            return;
        };
        let offset = self.scroll.offset();
        let delta = target.center().y - viewport.center().y;
        let max = self.scroll.max_offset().y;
        let y = (offset.y - delta).clamp(-max, px(0.));
        self.scroll.set_offset(gpui::point(offset.x, y));
    }

    /// Moves the highlighted search result; false while no results show.
    fn step_search(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        if self.search_query.trim().is_empty() {
            return false;
        }
        let count = model::search(&self.search_query, cx.compositor_name()).len();
        let last = count.saturating_sub(1);
        self.search_active = self.search_active.saturating_add_signed(delta).min(last);
        cx.notify();
        true
    }

    /// What the search field shows, so a cached rail can tell it changed.
    pub(crate) fn search_state(&self) -> (String, usize) {
        (self.search_query.clone(), self.search_active)
    }
}

trait FiniteAnd {
    fn finite_and(self, f: impl FnOnce(f32) -> bool) -> bool;
}
impl FiniteAnd for f32 {
    fn finite_and(self, f: impl FnOnce(f32) -> bool) -> bool {
        self.is_finite() && f(self)
    }
}
