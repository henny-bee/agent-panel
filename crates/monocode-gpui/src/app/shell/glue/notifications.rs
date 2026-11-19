//! Hosts the same notification stack as App.tsx, and routes native clicks.
mod linked_activity;
mod overlay;
use super::super::*;
use crate::notifications::{
    app_update::{self, AppUpdateEvent, InstalledUpdateNotice, WhatsNew, WhatsNewEvent},
    approvals::{self, ApprovalNotices, SessionInputs},
    delivery::{self, Permission, Target},
    errors,
    file_error::{FileActionError, FileActionErrorEvent},
    github_activity::{self, GithubActivity, GithubActivityEvent},
    harness_updates::{HarnessUpdates, HarnessUpdatesEvent},
    layout_notice::{LayoutNotice, LayoutNoticeEvent},
    linked_activity::{Cleanup, LinkedActivity, LinkedActivityEvent},
    platform, reminders,
    updater::{self, Event as UpdaterEvent, Updater},
};
use std::{sync::mpsc::Receiver, time::Duration};

pub(crate) struct State {
    pub region: AnchorSlot,
    pub harness: Entity<HarnessUpdates>,
    pub approvals: Entity<ApprovalNotices>,
    pub reminders: Entity<reminders::View>,
    pub layout: Entity<LayoutNotice>,
    pub installed: Entity<InstalledUpdateNotice>,
    pub updater: Entity<Updater>,
    pub file_error: Entity<FileActionError>,
    pub reminder_error: Option<String>,
    pub github: Entity<GithubActivity>,
    linked: HashMap<i64, Entity<LinkedActivity>>,
    linked_cleanup: HashMap<i64, Cleanup>,
    linked_seen_ready: bool,
    linked_last_poll: Option<std::time::Instant>,
    linked_revisions: HashMap<String, i64>,
    linked_primed: bool,
    linked_started_at: u64,
    linked_seen_writes: Option<gpui::Task<()>>,
    pub(crate) whats_new: Option<(Entity<WhatsNew>, Subscription, FocusHandle)>,
    permission: Permission,
    permission_probe: Option<Receiver<Permission>>,
    announcements: approvals::InputAnnouncements,
    responding: HashSet<i64>,
    sequence: u64,
    badge: usize,
    sounds_enabled: bool,
    sounds_enabled_at: u64,
    poll: Option<gpui::Task<()>>,
    /// How many notification mute and resume times had passed at the last poll.
    rule_times_passed: usize,
}

impl State {
    pub(crate) fn new(workspace: &Workspace, store: Database, cx: &mut Context<Shell>) -> Self {
        Self {
            region: AnchorSlot::default(),
            harness: cx
                .new(|cx| HarnessUpdates::new(workspace.preferences.hidden_providers.clone(), cx)),
            approvals: cx.new(|_| ApprovalNotices::default()),
            reminders: cx.new(|_| reminders::View::default()),
            layout: cx.new(|_| {
                LayoutNotice::new(
                    !workspace.classic_layout,
                    workspace.new_layout_prompt_dismissed,
                )
            }),
            installed: cx.new(|_| InstalledUpdateNotice::new(None)),
            updater: updater::shared(store, cx),
            file_error: cx.new(|_| FileActionError::new()),
            reminder_error: None,
            github: cx.new(|_| GithubActivity::new(HashMap::new())),
            linked: HashMap::new(),
            linked_cleanup: HashMap::new(),
            linked_seen_ready: false,
            linked_last_poll: None,
            linked_revisions: HashMap::new(),
            linked_primed: false,
            linked_started_at: monocode_core::sys::time::now_ms(),
            linked_seen_writes: None,
            whats_new: None,
            permission: Permission::Unsupported,
            permission_probe: None,
            announcements: Default::default(),
            responding: HashSet::new(),
            sequence: 0,
            badge: 0,
            sounds_enabled: workspace.preferences.sounds,
            sounds_enabled_at: 0,
            poll: None,
            rule_times_passed: 0,
        }
    }
}

impl Shell {
    pub(crate) fn app_update_footer(&self, cx: &App) -> AnyElement {
        let state = &self.notification_state;
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .when(state.installed.read(cx).is_visible(), |d| {
                d.child(div().mx_2().mt_2().child(state.installed.clone()))
            })
            .when(state.updater.read(cx).actionable(), |d| {
                d.child(
                    div()
                        .mx_2()
                        .mt_2()
                        .child(updater::row(state.updater.clone(), cx)),
                )
            })
            .into_any_element()
    }
    pub(crate) fn refresh_notification_permission(&mut self) {
        self.notification_state.permission_probe = Some(platform::permission());
    }

    /// Call from SessionChanged, so a turn beginning and ending between render
    /// frames still produces exactly one completion. Render sync shares the set.
    pub(crate) fn notification_session_changed(&mut self, id: i64, cx: &mut Context<Self>) {
        self.process_notification_inputs(cx);
        let Some(view) = self.sessions.get(&id) else {
            return;
        };
        let chat = view.read(cx);
        if chat.responding() {
            self.notification_state.responding.insert(id);
            return;
        }
        let ended = self.notification_state.responding.remove(&id) && !chat.busy;
        if !ended
            || chat.notification_was_cancelled()
            || chat.summary.archived
            || self
                .monos
                .as_ref()
                .is_some_and(|monos| monos.read(cx).is_mono_session(id))
        {
            // Ended with no finish to send, as when stopped.
            if ended {
                self.mobile_turn_ended(id);
            }
            return;
        }
        let summary = chat.summary.clone();
        let body = chat.notification_reply_body(cx);
        self.deliver_session_notice(
            &summary,
            "agentFinished",
            &body,
            Some(crate::ui::sound::Cue::Finished),
            cx,
        );
    }

    pub(crate) fn bind_notices(&mut self, cx: &mut Context<Self>) {
        self.bind_linked_activity(cx);
        self.observations.push(cx.subscribe(
            &self.notification_state.updater,
            |this, _, event: &UpdaterEvent, cx| {
                if let UpdaterEvent::Error(error) = event {
                    this.error = Some(error.clone());
                }
                this.invalidate_chrome(cx);
                if let Some(settings) = &this.settings {
                    settings.update(cx, |_, cx| cx.notify());
                }
                cx.notify();
            },
        ));
        self.observations.push(cx.subscribe(
            &self.notification_state.harness,
            |this, _, event, cx| {
                if let HarnessUpdatesEvent::Updated(kind) = event {
                    for view in this.notification_chats(cx) {
                        if Kind::from_id(&view.read(cx).summary.provider) == *kind {
                            view.update(cx, |view, cx| {
                                view.refresh_models_after_harness_update(cx)
                            });
                        }
                    }
                    this.sync_settings(cx);
                }
                cx.notify();
            },
        ));
        self.observations.push(cx.subscribe(
            &self.notification_state.approvals,
            |this, _, event, cx| match event {
                approvals::Action::OpenSession { session_id } => {
                    if let Ok(id) = session_id.parse() {
                        this.open_notice_target(Target::Session(id), cx);
                    }
                }
                approvals::Action::Decide {
                    session_id,
                    request_id,
                    decision,
                } => {
                    if let Ok(id) = session_id.parse()
                        && let Some(view) = this.notification_chat(id, cx)
                    {
                        view.update(cx, |view, cx| {
                            view.decide_notification_request(request_id, decision.clone(), cx);
                        });
                    }
                    cx.notify();
                }
            },
        ));
        self.observations.push(cx.subscribe(
            &self.notification_state.layout,
            |this, _, event, cx| {
                if matches!(event, LayoutNoticeEvent::Try) && this.workspace.classic_layout {
                    // Dismisses the notice and picks the default frame.
                    this.toggle_layout(cx);
                } else {
                    this.workspace.new_layout_prompt_dismissed = true;
                    this.persist(cx);
                    cx.notify();
                }
            },
        ));
        self.observations.push(cx.subscribe(
            &self.notification_state.reminders,
            |this, _, event, cx| match event {
                reminders::Action::Open { session_id, due_at } => this.open_notice_target(
                    Target::Reminder {
                        session_id: *session_id,
                        due_at: *due_at,
                    },
                    cx,
                ),
                reminders::Action::Dismiss { session_id, due_at } => {
                    this.clear_notification_reminder(*session_id, *due_at, cx)
                }
                reminders::Action::Snooze {
                    session_id,
                    expected_due_at,
                    due_at,
                } => this.snooze_notification_reminder(*session_id, *expected_due_at, *due_at, cx),
                reminders::Action::Retry => this.reload_notification_reminders(cx),
                reminders::Action::OpenSettings => {
                    cx.spawn(async move |this, cx| {
                        let _ = this.update_in(cx, |this, window, cx| {
                            this.open_settings(crate::settings::Section::General, window, cx)
                        });
                    })
                    .detach();
                }
            },
        ));
        self.observations.push(cx.subscribe(
            &self.notification_state.installed,
            |this, _, event, cx| match event {
                AppUpdateEvent::Open(version) => this.open_whats_new(version.clone(), cx),
                AppUpdateEvent::Dismiss => {
                    this.notification_state
                        .installed
                        .update(cx, |notice, cx| notice.set_version(None, cx));
                    this.invalidate_chrome(cx);
                    cx.notify();
                }
            },
        ));
        self.observations.push(cx.subscribe(
            &self.notification_state.file_error,
            |this, _, _: &FileActionErrorEvent, cx| {
                this.notification_state
                    .file_error
                    .update(cx, |notice, cx| notice.set_message(None, cx));
                cx.notify();
            },
        ));
        if let Some(monos) = self.monos.clone() {
            self.observations.push(cx.subscribe(
                &monos,
                |this, _, _: &crate::monos::NotificationChanged, cx| {
                    this.process_notification_inputs(cx);
                    cx.notify()
                },
            ));
            self.observations.push(cx.subscribe(
                &monos,
                |this, _, event: &crate::monos::NotificationPosted, cx| {
                    this.notify_mobile_post(event.session, &event.body, cx)
                },
            ));
            self.observations.push(cx.subscribe(
                &monos,
                |this, _, event: &crate::monos::NotificationFinished, cx| {
                    if !event.was_cancelled && !event.habit && !event.summary.archived {
                        this.deliver_session_notice(
                            &event.summary,
                            "agentFinished",
                            &event.body,
                            Some(crate::ui::sound::Cue::Finished),
                            cx,
                        );
                    }
                },
            ));
        }
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            locked(&store).and_then(|store| app_update::consume_installed_update(&store))
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Some(version)) = work.await {
                let _ = this.update(cx, |this, cx| {
                    this.notification_state
                        .installed
                        .update(cx, |notice, cx| notice.set_version(Some(version), cx));
                    this.invalidate_chrome(cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn start_native_notifications(&mut self, cx: &mut Context<Self>) {
        platform::set_dock_badge(0);
        if platform::initialize() {
            self.notification_state.permission = Permission::Prompt;
            self.notification_state.permission_probe = Some(platform::permission());
        }
        self.notification_state.badge = 0;
        self.process_notification_inputs(cx);
        self.notification_state.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this
                    .update(cx, |this, cx| {
                        let permission = this
                            .notification_state
                            .permission_probe
                            .as_ref()
                            .and_then(|rx| rx.try_recv().ok());
                        if let Some(permission) = permission {
                            this.notification_state.permission = permission;
                            this.notification_state.permission_probe = None;
                            cx.notify();
                        }
                        for target in platform::drain_clicks() {
                            this.open_notice_target(target, cx);
                        }
                        this.poll_reminders(cx);
                        this.poll_linked_activity(cx);
                        // Scheduled mutes expire without a preference change;
                        // redraw when one does, not on every tick.
                        let passed = this
                            .workspace
                            .preferences
                            .notification_times_passed(monocode_core::sys::time::now_ms());
                        if passed != this.notification_state.rule_times_passed {
                            this.notification_state.rule_times_passed = passed;
                            if !this.notification_state.approvals.read(cx).is_empty()
                                || this
                                    .notification_chats(cx)
                                    .iter()
                                    .any(|v| v.read(cx).needs_approval())
                            {
                                cx.notify();
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn notification_chats(&self, cx: &App) -> Vec<Entity<Conversation>> {
        let mut views: Vec<_> = self.sessions.values().cloned().collect();
        if let Some(monos) = &self.monos {
            views.extend(monos.read(cx).all_notification_chats(cx));
        }
        let mut seen = HashSet::new();
        views.retain(|view| seen.insert(view.read(cx).summary.id));
        views.sort_by_key(|view| view.read(cx).summary.id);
        views
    }

    pub(crate) fn notification_chat(&self, id: i64, cx: &App) -> Option<Entity<Conversation>> {
        self.sessions
            .get(&id)
            .cloned()
            .or_else(|| self.monos.as_ref()?.read(cx).notification_entity(id, cx))
    }

    /// Advance banners and the Dock badge while an occluded window skips render.
    fn process_notification_inputs(&mut self, cx: &mut Context<Self>) {
        let snapshots: Vec<_> = self
            .notification_chats(cx)
            .iter()
            .map(|view| {
                let chat = view.read(cx);
                SessionInputs {
                    session_id: chat.summary.id.to_string(),
                    title: chat.display_title().to_owned(),
                    provider: chat.summary.provider.clone(),
                    harness: Kind::from_id(&chat.summary.provider).label().to_owned(),
                    pending: chat.pending_notification_inputs(cx),
                    inbox_ask: false,
                    orchestration_worker: self.orchestration_worker(chat.summary.id),
                    conversation_focused: false,
                }
            })
            .collect();
        self.notification_state.approvals.update(cx, |notices, _| {
            notices.track_pending(&snapshots);
        });
        let badge = delivery::dock_badge_count(
            snapshots
                .iter()
                .filter_map(|s| Some((s.session_id.parse().ok()?, !s.pending.is_empty()))),
        );
        if badge != self.notification_state.badge {
            self.notification_state.badge = badge;
            platform::set_dock_badge(badge);
        }
        for (session, input) in self.notification_state.announcements.reconcile(&snapshots) {
            if let Ok(id) = session.session_id.parse()
                && let Some(view) = self.notification_chat(id, cx)
            {
                let summary = view.read(cx).summary.clone();
                let body = if input.kind == approvals::InputKind::Approval {
                    format!("Approve: {}", input.label)
                } else {
                    input.label
                };
                self.deliver_session_notice(
                    &summary,
                    "agentInput",
                    &body,
                    Some(crate::ui::sound::Cue::Input),
                    cx,
                );
            }
        }
    }

    pub(crate) fn notice_session_visible(&self, id: i64, cx: &App) -> bool {
        if self.app_view_open() || self.settings.is_some() || self.search_page() {
            return false;
        }
        if self.mono_active(cx) {
            return self
                .monos
                .as_ref()
                .and_then(|m| m.read(cx).active_notification_session(cx))
                == Some(id);
        }
        self.workspace.active_tab().is_some_and(|tab| {
            self.workspace
                .layout(tab)
                .leaf_ids()
                .into_iter()
                .any(|pane| {
                    self.workspace.session_for_pane(pane) == id
                        && !self.workspace.is_detached_pane(pane)
                })
        })
    }

    pub(crate) fn sync_notices(&mut self, window: &Window, cx: &mut Context<Self>) {
        let now = monocode_core::sys::time::now_ms();
        // This runs on every Shell frame; borrow the preferences below instead
        // of cloning their JSON maps each time.
        let sounds = self.workspace.preferences.sounds;
        if self.notification_state.sounds_enabled != sounds {
            self.notification_state.sounds_enabled = sounds;
            if sounds {
                self.notification_state.sounds_enabled_at = now;
            }
        }
        let views = self.notification_chats(cx);
        let mut snapshots = Vec::new();
        let mut directories = HashMap::new();
        let mut completed = Vec::new();
        let mut quiet = Vec::new();
        let mut live_ids = HashSet::new();
        for view in views {
            let chat = view.read(cx);
            let id = chat.summary.id;
            live_ids.insert(id);
            if chat.responding() {
                self.notification_state.responding.insert(id);
            } else if self.notification_state.responding.remove(&id) && !chat.busy {
                if !chat.notification_was_cancelled()
                    && !chat.summary.archived
                    && self.sessions.contains_key(&id)
                    && !self
                        .monos
                        .as_ref()
                        .is_some_and(|monos| monos.read(cx).is_mono_session(id))
                {
                    completed.push((chat.summary.clone(), chat.notification_reply_body(cx)));
                } else {
                    // Ended with no finish to send, as when stopped.
                    quiet.push(id);
                }
            }
            directories.insert(id.to_string(), chat.summary.cwd.clone());
            let mono_visible = self.mono_active(cx) && self.notice_session_visible(id, cx);
            snapshots.push(SessionInputs {
                session_id: id.to_string(),
                title: chat.display_title().to_owned(),
                provider: chat.summary.provider.clone(),
                harness: Kind::from_id(&chat.summary.provider).label().to_owned(),
                pending: chat.pending_notification_inputs(cx),
                inbox_ask: false,
                orchestration_worker: self.orchestration_worker(id),
                conversation_focused: self.window_active
                    && (self.workspace.active == Some(id) || mono_visible)
                    && self.notice_session_visible(id, cx)
                    && chat.composer_focused(window, cx),
            });
        }
        self.notification_state
            .responding
            .retain(|id| live_ids.contains(id));
        for id in quiet {
            self.mobile_turn_ended(id);
        }
        let badge = delivery::dock_badge_count(
            snapshots
                .iter()
                .filter_map(|s| Some((s.session_id.parse().ok()?, !s.pending.is_empty()))),
        );
        if self.notification_state.badge != badge {
            self.notification_state.badge = badge;
            platform::set_dock_badge(badge);
        }
        let announcements = self.notification_state.announcements.reconcile(&snapshots);
        for (session, input) in announcements {
            if let Ok(id) = session.session_id.parse()
                && let Some(view) = self.notification_chat(id, cx)
            {
                let summary = view.read(cx).summary.clone();
                self.deliver_session_notice(
                    &summary,
                    "agentInput",
                    &input.label,
                    Some(crate::ui::sound::Cue::Input),
                    cx,
                );
            }
        }
        for (summary, body) in completed {
            self.deliver_session_notice(
                &summary,
                "agentFinished",
                &body,
                Some(crate::ui::sound::Cue::Finished),
                cx,
            );
        }
        let prefs = &self.workspace.preferences;
        self.notification_state.approvals.update(cx, |notices, cx| {
            notices.sync(
                &snapshots,
                |session, at| {
                    let occurred = at
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                        .min(u64::MAX as u128) as u64;
                    directories.get(&session.session_id).is_some_and(|cwd| {
                        prefs.allows_project_notification(cwd, "agentInput", Some(occurred), now)
                    })
                },
                cx,
            )
        });
        let due = self
            .reminders
            .iter()
            .filter(|r| r.due_at <= now)
            .filter_map(|r| {
                let summary = self.session_summaries.get(&r.session_id)?;
                prefs
                    .allows_project_notification(&summary.cwd, "reminders", Some(r.due_at), now)
                    .then(|| reminders::Snapshot {
                        session_id: r.session_id,
                        due_at: r.due_at,
                        title: summary.title.clone(),
                        project: project_name(&summary.cwd),
                        time_label: local_reminder_label(r.due_at),
                    })
            })
            .collect();
        let error = self.notification_state.reminder_error.clone();
        self.notification_state.reminders.update(cx, |notices, cx| {
            notices.sync(due, error, prefs.notifications, cx)
        });
        self.notification_state.layout.update(cx, |notice, cx| {
            notice.set_snapshot(
                !self.workspace.classic_layout,
                self.workspace.new_layout_prompt_dismissed,
                cx,
            )
        });
    }

    pub(crate) fn deliver_session_notice(
        &mut self,
        summary: &Session,
        category: &str,
        body: &str,
        cue: Option<crate::ui::sound::Cue>,
        cx: &mut Context<Self>,
    ) {
        self.notify_mobile(summary, category, body, cx);
        self.dispatch_notice(
            summary,
            category,
            Target::Session(summary.id),
            body,
            cue,
            cx,
        );
    }

    pub(crate) fn deliver_reminder_notice(&mut self, id: i64, due_at: u64, cx: &mut Context<Self>) {
        if let Some(summary) = self.session_summaries.get(&id).cloned() {
            self.dispatch_notice(
                &summary,
                "reminders",
                Target::Reminder {
                    session_id: id,
                    due_at,
                },
                &summary.title,
                None,
                cx,
            );
        }
    }

    fn dispatch_notice(
        &mut self,
        summary: &Session,
        category: &str,
        target: Target,
        body: &str,
        cue: Option<crate::ui::sound::Cue>,
        cx: &mut Context<Self>,
    ) {
        let now = monocode_core::sys::time::now_ms();
        let occurred = match target {
            Target::Session(_) => now,
            Target::Reminder { due_at, .. } => due_at,
        };
        let prefs = &self.workspace.preferences;
        if !prefs.allows_project_notification(&summary.cwd, category, Some(occurred), now) {
            return;
        }
        let notify = delivery::should_notify(
            prefs.notifications,
            self.notification_state.permission,
            self.window_active,
            category != "reminders" && self.notice_session_visible(summary.id, cx),
        );
        if !notify {
            if prefs.sounds
                && occurred >= self.notification_state.sounds_enabled_at
                && let Some(cue) = cue
            {
                crate::ui::sound::play(cue);
            }
            return;
        }
        self.notification_state.sequence = self.notification_state.sequence.wrapping_add(1);
        let body = if body.trim().is_empty() && category == "agentFinished" {
            format!("{} finished", Kind::from_id(&summary.provider).label())
        } else {
            body.to_owned()
        };
        let receiver = platform::show(platform::Notification {
            tag: target.tag(self.notification_state.sequence),
            subtitle: if category == "reminders" {
                "Session reminder".into()
            } else {
                summary.title.clone()
            },
            body: delivery::clip_body(&body),
            sound: prefs.sounds,
        });
        let directory = summary.cwd.clone();
        let category = category.to_owned();
        let work = cx.background_executor().spawn(async move {
            receiver
                .recv_timeout(Duration::from_secs(5))
                .ok()
                .is_some_and(|result| result.is_ok())
        });
        cx.spawn(async move |this, cx| {
            let accepted = work.await;
            let _ = this.update(cx, |this, _| {
                let now = monocode_core::sys::time::now_ms();
                let prefs = &this.workspace.preferences;
                if !accepted
                    && prefs.sounds
                    && occurred >= this.notification_state.sounds_enabled_at
                    && prefs.allows_project_notification(&directory, &category, Some(occurred), now)
                    && let Some(cue) = cue
                {
                    crate::ui::sound::play(cue);
                }
            });
        })
        .detach();
    }

    fn open_notice_target(&mut self, target: Target, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let _ = this.update_in(cx, |this, window, cx| {
                let id = match target {
                    Target::Session(id) => id,
                    Target::Reminder { session_id, due_at } => {
                        this.clear_notification_reminder(session_id, due_at, cx);
                        session_id
                    }
                };
                this.close_settings(window, cx);
                this.leave_app_views(cx);
                this.leave_app_search(cx);
                if let Some(monos) = this.monos.clone()
                    && monos.update(cx, |m, cx| m.open_notification_session(id, window, cx))
                {
                    window.activate_window();
                    return;
                }
                // Habit runs share their live Conversation with the Mono
                // manager. Adopt that entity so controls answer the running
                // provider, rather than constructing a disconnected history.
                if !this.sessions.contains_key(&id)
                    && let Some(view) = this.notification_chat(id, cx)
                {
                    let summary = view.read(cx).summary.clone();
                    this.session_summaries.insert(id, summary.clone());
                    this.session_marks
                        .insert(id, cx.new(|_| SessionMark::new(summary.provider.clone())));
                    this.tab_marks
                        .insert(id, cx.new(|_| SessionMark::tab(summary.provider.clone())));
                    this.observations.push(cx.subscribe(
                        &view,
                        move |this, _, _: &SessionChanged, cx| {
                            this.refresh_session_mark(id, cx);
                            this.notification_session_changed(id, cx);
                            this.invalidate_chrome(cx);
                            cx.notify();
                        },
                    ));
                    this.sessions.insert(id, view);
                    if !this.order.contains(&id) {
                        this.order.push(id);
                    }
                }
                if this.session_summaries.contains_key(&id) {
                    // `open` normally dismisses today's due reminder. A stale
                    // OS target must preserve a newer occurrence instead.
                    let newer = match target {
                        Target::Reminder { due_at, .. } => this
                            .reminders
                            .iter()
                            .find(|r| r.session_id == id && r.due_at != due_at)
                            .cloned(),
                        _ => None,
                    };
                    if newer.is_some() {
                        this.reminders.retain(|r| r.session_id != id);
                    }
                    this.close_settings(window, cx);
                    this.open(id, window, cx);
                    if let Some(newer) = newer {
                        this.reminders.push(newer);
                    }
                    window.activate_window();
                }
            });
        })
        .detach();
    }
}

impl Shell {}

#[cfg(test)]
mod tests;
