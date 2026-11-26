use super::*;

impl Shell {
    /// Only Unix opens another window onto the running app.
    #[cfg(any(unix, test))]
    pub(crate) fn new(
        store: Database,
        summaries: Vec<Session>,
        cwd: String,
        saved: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let paths = summaries
            .iter()
            .map(|s| (s.id, s.cwd.clone()))
            .collect::<Vec<_>>();
        let workspace = Workspace::restore(saved.as_deref(), &paths, &cwd);
        let mut projects = workspace.projects;
        if !projects.contains(&workspace.project) {
            projects.push(workspace.project);
        }
        let metadata = locked(&store)
            .map(|db| db.session_sidebar_metadata(&projects))
            .unwrap_or_default();
        Self::new_with_metadata(store, summaries, cwd, saved, metadata, cx)
    }

    pub(crate) fn new_with_metadata(
        store: Database,
        summaries: Vec<Session>,
        cwd: String,
        saved: Option<String>,
        sidebar_metadata: monocode_core::store::SessionSidebarMetadata,
        cx: &mut Context<Self>,
    ) -> Self {
        crate::accounts::install(store.clone(), cx);
        let main = Self::join_windows(&store, cx);
        // Later windows share the first window's connections.
        let remote = match monocode_core::integrations::remote::default_connections()
            .ok()
            .filter(|_| !main)
        {
            Some(remote) => remote,
            None => {
                let remote_dir = locked(&store)
                    .ok()
                    .and_then(|store| store.data_dir().map(std::path::Path::to_path_buf))
                    .unwrap_or_else(|| {
                        std::env::temp_dir().join(format!("monocode-remote-{}", std::process::id()))
                    });
                let remote = Arc::new(monocode_core::integrations::remote::Connections::new(
                    remote_dir,
                ));
                monocode_core::integrations::remote::set_default_connections(remote.clone());
                remote
            }
        };
        let quit_remote = remote.clone();
        cx.on_app_quit(move |_, cx| {
            let remote = quit_remote.clone();
            cx.background_executor().spawn(async move {
                remote.shutdown();
            })
        })
        .detach();
        let remote_clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(3))
                    .await;
                if this
                    .update(cx, |this, cx| this.refresh_remote_projects(cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let metadata: Vec<_> = summaries.iter().map(|s| (s.id, s.cwd.clone())).collect();
        let mut workspace = Workspace::restore(saved.as_deref(), &metadata, &cwd);
        if !main {
            Self::adopt_app_workspace(&mut workspace, &store, cx);
        }
        monocode_core::provider::cli::initialize(&workspace.preferences.binary_paths);
        // One quit-time save for the app, written by whichever window runs the
        // services, so a window that took them over is the one restored.
        let latest_state = match Self::app_state(&store, cx).filter(|_| !main) {
            Some(state) => state,
            None => {
                let latest_state = Arc::new(Mutex::new(serde_json::to_string(&workspace).unwrap()));
                let quit_state = latest_state.clone();
                let quit_store = store.clone();
                // This application-level hook survives the last window/entity being released.
                let app: &mut App = cx;
                app.on_app_quit(move |cx| {
                    let state = quit_state.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    let store = quit_store.clone();
                    cx.background_executor().spawn(async move {
                        let _ = locked(&store).and_then(|s| s.save_ui_state(&state));
                    })
                })
                .detach();
                Self::set_app_state(&store, latest_state.clone(), cx);
                latest_state
            }
        };
        let owner = cx.weak_entity();
        let sidebar_frame = cx.new(|_| Chrome {
            owner: owner.clone(),
            kind: ChromeKind::Sidebar,
            renders: 0,
        });
        let tabs_frame = cx.new(|_| Chrome {
            owner: owner.clone(),
            kind: ChromeKind::Tabs,
            renders: 0,
        });
        let projects_frame = cx.new(|_| Chrome {
            owner,
            kind: ChromeKind::Projects,
            renders: 0,
        });
        let active_kind = summaries
            .iter()
            .find(|s| Some(s.id) == workspace.active)
            .map(|s| Kind::from_id(&s.provider))
            .unwrap_or(Kind::Codex);
        let session_clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(30))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        if this.main {
                            this.poll_reminders(cx);
                            this.poll_automations(cx);
                        }
                        this.update_chat_visibility(cx);
                        if this.workspace.sidebar_visible {
                            this.notify_sidebar(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let git_project = workspace.project.clone();
        let git = cx.new(|cx| crate::runtime::git_status::ProjectGit::new(git_project, cx));
        let git_observation = cx.observe(&git, |this, git, cx| {
            let cwd = git.read(cx).root.clone();
            for view in this.sessions.values() {
                if view.read(cx).review.read(cx).session().1 == cwd {
                    view.update(cx, |view, cx| view.refresh_git_actions(cx));
                }
            }
            // The title bar's dot and the panel's tab label follow its stats.
            this.sync_project_rail_git_stats(cx);
            if this.workspace.classic_layout {
                this.sidebar_frame.update(cx, |_, cx| cx.notify());
            }
            cx.notify();
        });
        // A real change, such as a discard from the Changes panel: chats in
        // this checkout reread their recorded changes, which drops reverted
        // files from the changes pill and its card.
        let git_changes = cx.subscribe(
            &git,
            |this, git, _: &crate::runtime::git_status::GitChanged, cx| {
                let cwd = git.read(cx).root.clone();
                let branch = git.read(cx).index.as_ref().and_then(|i| i.branch.clone());
                for view in this.sessions.values() {
                    if view.read(cx).review.read(cx).session().1 == cwd {
                        view.update(cx, |view, cx| {
                            view.review.update(cx, |review, cx| review.refresh(cx));
                            view.follow_branch(branch.as_deref(), cx);
                        });
                    }
                }
            },
        );
        let resources = cx.new(ResourceMonitor::new);
        let resource_observation = cx.observe(&resources, |_, _, cx| cx.notify());
        let resource_stops = cx.subscribe(&resources, |this, _, StopSession(id), cx| {
            if let Some(session) = this.sessions.get(id) {
                session.update(cx, |session, cx| session.stop_if_busy(cx));
            } else if let Some(m) = this.monos.clone() {
                m.update(cx, |m, cx| m.stop_session(*id, cx));
            }
        });
        let worktree_focus = workspace.worktree_focus.clone();
        let session_worktrees = sidebar_metadata.worktrees;
        let notification_state = glue::notifications::State::new(&workspace, store.clone(), cx);
        let classic_session_search = sidebar::classic::session_search(cx);
        let classic_search_subscription =
            sidebar::classic::bind_session_search(&classic_session_search, cx);
        let classic_sidebar_tab = workspace
            .classic_sidebar_tabs
            .get(&workspace.project)
            .map(|id| sidebar::classic::ClassicSidebarTab::from_id(id))
            .unwrap_or_default();
        let mut this = Self {
            monos: None,
            quick_open: None,
            image_lightbox: None,
            app_bridge: None,
            mobile: glue::mobile_host::Mobile::default(),
            app_results: HashMap::new(),
            app_pending: HashSet::new(),
            orchestrations: HashMap::new(),
            orchestration_summaries: sidebar_metadata.orchestrations,
            started_by: sidebar_metadata.started_by,
            rails: Default::default(),
            usage_footer: cx.new(|cx| UsageFooter::new(active_kind, cx)),
            resources,
            sidebar_frame,
            projects_frame,
            project_rail_list: sidebar::project_rail::ProjectRailList::new(cx),
            project_rail_drag: None,
            space_reorder: Default::default(),
            project_rail_stats: Default::default(),
            project_resize: None,
            classic_sidebar_tab,
            classic_tabs: Default::default(),
            classic_session_search,
            classic_worktree_picker: None,
            sidebar_list: sidebar::list::SidebarList::new(cx),
            sidebar_slide: None,
            sidebar_fraction: if workspace.sidebar_visible { 1. } else { 0. },
            settings_nav_visible: true,
            sidebar_peek: None,
            peek_animating: false,
            peek_close: None,
            peek_toggle_bounds: AnchorSlot::default(),
            fullscreen: false,
            window_controls: 0.,
            right_panel_min: crate::ui::panel::WIDTH_MIN,
            #[cfg(target_os = "linux")]
            client_decorations: false,
            tabs_frame,
            store,
            remote,
            remote_project_picker: None,
            remote_refreshing: false,
            remote_marks: HashMap::new(),
            _remote_clock: remote_clock,
            sessions: HashMap::new(),
            session_summaries: HashMap::new(),
            draft_sessions: sidebar_metadata.drafts,
            history_recency: HashMap::new(),
            history_clock: 0,
            session_marks: HashMap::new(),
            tab_marks: HashMap::new(),
            live_marks: HashMap::new(),
            live_agents_expanded: false,
            live_tick: None,
            live_next_change: 1000,
            busy_sessions: HashSet::new(),
            notification_state,
            github_star: Default::default(),
            quick: Default::default(),
            unseen_finished: HashSet::new(),
            archiving: HashSet::new(),
            archive_shortcut_held: false,
            external_editors: None,
            groups: sidebar_metadata.groups,
            reminders: sidebar_metadata.reminders,
            renaming: None,
            link_dialog: None,
            action_dialog: None,
            actions_anchor: AnchorSlot::default(),
            new_tab_anchor: AnchorSlot::default(),
            action_terminals: HashMap::new(),
            deleting: HashSet::new(),
            group_writes: None,
            selected_sessions: HashSet::new(),
            selection_anchor: None,
            context_selection: false,
            sidebar_target: None,
            session_row_bounds: HashMap::new(),
            folder_bounds: HashMap::new(),
            sidebar_drop: None,
            folder_drag: None,
            skip_folder_click: false,
            filter_anchor: AnchorSlot::default(),
            folder_picker: None,
            pending_folder: None,
            _session_clock: session_clock,
            notes: None,
            note_count: None,
            inbox: None,
            automations: None,
            active_app_view: None,
            app_view_events: Vec::new(),
            automation_state: Default::default(),
            settings: None,
            settings_events: None,
            settings_overlay_watch: None,
            app_search: None,
            saved_search: None,
            order: Vec::new(),
            workspace,
            focus: cx.focus_handle(),
            sidebar_focus: cx.focus_handle(),
            query: String::new(),
            sidebar_hover: None,
            tab_hover: None,
            pin_fan: Default::default(),
            pin_deck: Default::default(),
            pin_card_hover: None,
            title_tab_motion: Default::default(),
            title_tab_selection: (None, None, std::time::Instant::now()),
            session_hover: None,
            session_preview: None,
            preview_close: None,
            preview_open: None,
            sidebar_last_scroll: None,
            window_observed: false,
            shortcuts: None,
            window_active: false,
            window_glass: None,
            theme_project: None,
            quit_prompted: false,
            menu: None,
            background_dialog: None,
            backgrounds: HashMap::new(),
            backdrop: None,
            background_card: AnchorSlot::default(),
            project_anchor: AnchorSlot::default(),
            sidebar_options_anchor: AnchorSlot::default(),
            history: VisitHistory::default(),
            navigation_revision: 0,
            pending_project_blanks: HashSet::new(),
            new_tab_task: None,
            worktree_focus,
            session_worktrees,
            session_details: sidebar_metadata.details,
            sidebar_hidden_sessions: sidebar_metadata.sidebar_hidden,
            worktree_returns: HashMap::new(),
            tab_drag: None,
            tab_shifts: HashMap::new(),
            pane_ticks: HashMap::new(),
            skip_tab_click: false,
            spaces_start: 0,
            spaces_nudge: None,
            hovered_space: None,
            space_anchor: AnchorSlot::default(),
            error: None,
            tab_scroll: ScrollHandle::new(),
            tab_scroll_task: None,
            resize: None,
            git,
            right_panel: None,
            pane_panels: HashMap::new(),
            pane_gits: HashMap::new(),
            pane_panel_resize: None,
            right_resize: None,
            right_resize_hovered: false,
            right_slide: None,
            reviews: HashMap::new(),
            review_slide: None,
            pending_review: None,
            pending_composer_focus: None,
            review_focused: false,
            review_drag: false,
            split_bounds: HashMap::new(),
            pane_bounds: AnchorSlot::default(),
            mono_bounds: AnchorSlot::default(),
            pane_drag: None,
            pane_sash: None,
            pane_motion: None,
            skip_session_click: false,
            sidebar_auto_hidden: false,
            sidebar_fitted_room: None,
            terminals: HashMap::new(),
            terminal_pane_views: HashMap::new(),
            dock_slide: None,
            dock_resize: None,
            dock_hover: None,
            dock_tab_motion: Default::default(),
            dock_drag: None,
            right_drawn: 0.,
            corner_reveal: None,
            dock_shifts: HashMap::new(),
            dock_skip_click: false,
            dock_tab_scroll: ScrollHandle::new(),
            move_terminal_anchor: AnchorSlot::default(),
            terminal_chip_anchor: AnchorSlot::default(),
            live_mark: None,
            viewport_width: 1200.,
            viewport_height: 800.,
            observations: vec![
                git_observation,
                git_changes,
                resource_observation,
                resource_stops,
                classic_search_subscription,
            ],
            generation: Arc::new(AtomicU64::new(0)),
            latest_state,
            render_passes: 0,
            main,
            shown: HashSet::new(),
        };
        if !cfg!(test) {
            this.scan_external_editors(cx);
        }
        this.observe_accounts(cx);
        this.load_note_count(cx);
        this.observe_picker_preferences(cx);
        if this.main {
            this.start_monos(cx);
        }
        this.bind_notices(cx);
        if this.main {
            this.init_app_bridge(cx);
        }
        // A window without Monos of its own still leaves their sessions out.
        let monos = this.app_monos(cx);
        for summary in summaries {
            if monos
                .as_ref()
                .is_some_and(|m| m.read(cx).is_mono_session(summary.id))
            {
                continue;
            }
            this.order.push(summary.id);
            this.session_summaries.insert(summary.id, summary);
        }
        if this.main {
            this.init_mobile_host(cx);
        }
        if let Some(id) = this.workspace.active {
            this.ensure_session(this.workspace.session_for_pane(id), cx);
            this.record_worktree_session(id, cx);
        }
        if let Some(id) = this.workspace.active {
            this.history.record(id);
        }
        this.reveal_active_tab(cx);
        this.update_chat_visibility(cx);
        this.update_usage_provider(cx);
        this.apply_preferences(cx);
        if this.main {
            this.poll_reminders(cx);
            this.initialize_automations(cx);
        }
        cx.on_release(|this, cx| {
            if this.main {
                let store = this.store.clone();
                cx.defer(move |cx| Shell::hand_over_services(&store, cx));
            }
        })
        .detach();
        if this.workspace.active.is_none() {
            cx.spawn(async move |this, cx| {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.ensure_project_session(window, cx);
                    this.persist(cx);
                });
            })
            .detach();
        }
        this
    }

    /// The Mono agents and their heartbeats, run by the services window.
    pub(super) fn start_monos(&mut self, cx: &mut Context<Self>) {
        let this = self;
        let mono_owner = cx.weak_entity();
        let mono_active =
            this.workspace
                .scope_focus
                .get(&this.tab_scope())
                .and_then(|tab| match tab {
                    TitleTab::Mono { id, .. } => Some(id.clone()),
                    _ => None,
                });
        let monos = cx.new(|cx| {
            crate::monos::Monos::new(
                this.store.clone(),
                mono_owner,
                this.workspace.preferences.clone(),
                mono_active,
                this.notification_state.region.clone(),
                cx,
            )
        });
        this.observations.push(
            cx.subscribe(&monos, |this, _, _: &crate::monos::Changed, cx| {
                this.sync_mono_search(cx);
                this.update_usage_provider(cx);
                this.invalidate_chrome(cx);
                cx.notify();
            }),
        );
        this.observations.push(cx.subscribe(
            &monos,
            |this, _, opened: &crate::monos::TabOpened, cx| {
                this.mono_opened(opened.0.clone(), cx);
            },
        ));
        this.observations.push(cx.subscribe(
            &monos,
            |_, _, request: &crate::monos::ShowDetails, cx| {
                let id = request.0.clone();
                cx.spawn(async move |this, cx| {
                    let _ = this.update_in(cx, |this, window, cx| {
                        if this
                            .monos
                            .as_ref()
                            .is_some_and(|m| m.read(cx).active.as_ref() == Some(&id))
                        {
                            this.set_right_panel_open(true, window, cx);
                        }
                    });
                })
                .detach();
            },
        ));
        let mut mono_chrome = monos.read(cx).overview_chrome();
        let mut mono_error = monos.read(cx).error.clone();
        let mut mono_session = monos.read(cx).routed_session();
        this.observations
            .push(cx.observe(&monos, move |this, monos, cx| {
                let session = monos.read(cx).routed_session();
                if session != mono_session {
                    mono_session = session;
                    this.update_chat_visibility(cx);
                }
                let chrome = monos.read(cx).overview_chrome();
                if chrome != mono_chrome {
                    mono_chrome = chrome;
                    this.invalidate_chrome(cx);
                    cx.notify();
                }
                let error = &monos.read(cx).error;
                if *error != mono_error {
                    mono_error = error.clone();
                    cx.notify();
                }
            }));
        this.monos = Some(monos);
    }

    pub(super) fn add(&mut self, summary: Session, cx: &mut Context<Self>) {
        self.workspace.add_project(&summary.cwd);
        self.add_loaded(summary, cx);
    }

    /// Loading history must not turn every session directory into a rail project.
    pub(super) fn add_loaded(&mut self, summary: Session, cx: &mut Context<Self>) {
        let id = summary.id;
        if self.sessions.contains_key(&id) {
            return;
        }
        self.session_summaries.insert(id, summary.clone());
        self.session_marks
            .insert(id, cx.new(|_| SessionMark::new(summary.provider.clone())));
        self.tab_marks
            .insert(id, cx.new(|_| SessionMark::tab(summary.provider.clone())));
        // Another window's live view, so its agent or CLI keeps running once.
        let view = match self.shared_session(id, cx) {
            Some(view) => view,
            None => {
                let appearance = self.appearance(&summary.cwd);
                let worktree = self.session_worktrees.get(&id).cloned().flatten();
                let environment = self.app_environment(id, cx);
                let view = cx.new(|cx| Conversation::new(summary, self.store.clone(), cx));
                view.update(cx, |view, cx| {
                    // Events may arrive before the first asynchronous agent load. Seed
                    // the indexed working directory so those events cannot erase it.
                    view.restore_worktree(worktree, None, cx);
                    view.draft_hint = self.draft_sessions.contains(&id);
                    if let Some(environment) = environment {
                        view.set_app_environment(environment);
                    }
                    view.set_project_appearance(appearance, cx);
                    view.transcript.update(cx, |transcript, cx| {
                        transcript.apply_preferences(&self.workspace.preferences, cx)
                    });
                });
                self.share_session(id, &view, cx);
                view
            }
        };
        self.observations
            .push(cx.subscribe(&view, move |this, _, _: &SessionChanged, cx| {
                let Some(view) = this.sessions.get(&id) else {
                    return;
                };
                this.session_worktrees
                    .insert(id, view.read(cx).settings.worktree_cwd.clone());
                // A finished turn may have edited files or moved to a worktree.
                if this.workspace.active == Some(id) {
                    this.sync_git(cx);
                    this.git.update(cx, |git, cx| git.reload(cx));
                }
                this.sync_search_session(id, cx);
                this.orchestration_session_changed(id, cx);
                if this.main {
                    this.automation_session_changed(id, cx);
                }
                this.sync_review_locks(cx);
                this.refresh_session_mark(id, cx);
                if this.main {
                    this.notification_session_changed(id, cx);
                }
                this.update_usage_provider(cx);
                this.update_chat_visibility(cx);
                this.invalidate_chrome(cx);
                cx.notify();
            }));
        self.observations.push(cx.subscribe(
            &view,
            |this, _, left: &crate::chat::conversation::SessionLeft, cx| {
                let left = left.0.clone();
                match this.sessions.get(&left.id).cloned() {
                    // An entry that traded places with the tab.
                    Some(view) => {
                        this.session_summaries.insert(left.id, left.clone());
                        view.update(cx, |view, cx| view.claude_terminal_traded(left, cx));
                    }
                    None => this.add(left, cx),
                }
                this.invalidate_chrome(cx);
                cx.notify();
            },
        ));
        self.observations.push(cx.subscribe(
            &view,
            move |this, _, resumed: &crate::chat::conversation::Resumed, cx| {
                if !this.acts_for(id, cx) {
                    return;
                }
                let session = resumed.0.clone();
                if this.sessions.contains_key(&session.id) {
                    let id = session.id;
                    cx.spawn(async move |this, cx| {
                        let _ = this.update_in(cx, |this, window, cx| {
                            this.activate_session(id, window, cx)
                        });
                    })
                    .detach();
                } else {
                    this.open_fork(session, String::new(), Vec::new(), cx);
                }
            },
        ));
        self.observations.push(cx.subscribe(
            &view,
            move |this, _, forked: &crate::chat::conversation::Forked, cx| {
                if this.acts_for(id, cx) {
                    this.open_fork(forked.0.clone(), forked.1.clone(), forked.2.clone(), cx)
                }
            },
        ));
        self.observations.push(
            cx.subscribe(&view, move |this, _, transfer: &Transfer, cx| {
                if this.acts_for(id, cx) {
                    this.transfer_response(transfer.clone(), cx)
                }
            }),
        );
        self.observations.push(cx.subscribe(
            &view,
            move |this, _, event: &crate::chat::conversation::OpenSessionChanges, cx| {
                if !this.acts_for(id, cx) {
                    return;
                }
                let tab = event.0.clone();
                // The card's pane takes focus, so its review opens beside it.
                this.pending_review = Some((id, tab));
                cx.notify();
            },
        ));
        self.observations
            .push(cx.observe(&view, move |this, _, cx| {
                if this
                    .session_preview
                    .as_ref()
                    .is_some_and(|preview| preview.id == id)
                {
                    this.sidebar_frame.update(cx, |_, cx| cx.notify());
                }
            }));
        self.observe_orchestration(&view, id, cx);
        self.observe_session_folders(&view, id, cx);
        self.sessions.insert(id, view);
        if !self.order.contains(&id) {
            self.order.push(id);
        }
    }

    /// The active tab's review pane, created from its saved state.
    pub(super) fn unsaved_files(&self, cx: &App) -> bool {
        self.reviews
            .values()
            .any(|pane| pane.read(cx).unsaved_files(cx))
    }

    /// Steps the interface scale one notch up or down, or resets it with 0.
    pub(super) fn step_ui_scale(&mut self, step: i32, cx: &mut Context<Self>) {
        use crate::ui::preferences::UI_SCALE_PERCENTS;
        let current = self.workspace.preferences.ui_scale;
        let next = match step {
            0 => Some(100),
            1 => UI_SCALE_PERCENTS.into_iter().find(|&p| p > current),
            _ => UI_SCALE_PERCENTS.into_iter().rev().find(|&p| p < current),
        }
        .unwrap_or(current);
        if next == current {
            return;
        }
        self.workspace.preferences.ui_scale = next;
        self.apply_preferences(cx);
        self.sync_settings(cx);
        self.persist(cx);
        cx.notify();
    }

    /// The interface scale as a window zoom, applied after the frame that
    /// noticed it so layout never runs against a viewport that changes mid-draw.
    pub(super) fn sync_ui_scale(&self, window: &mut Window, cx: &mut Context<Self>) {
        let zoom = self.workspace.preferences.ui_scale.clamp(50, 200) as f32 / 100.;
        if (window.content_zoom() - zoom).abs() < f32::EPSILON {
            return;
        }
        window.defer(cx, move |window, cx| {
            window.set_content_zoom(zoom, cx);
            #[cfg(target_os = "macos")]
            {
                let height = crate::ui::theme::TITLE_BAR_HEIGHT;
                window
                    .set_traffic_light_position(crate::ui::traffic_lights::position(zoom, height));
                crate::ui::traffic_lights::pin(window, height);
            }
        });
    }

    /// The native stand-in for the web app's Reload: restart the process.
    /// Unlike a webview reload this stops running agents and terminals, so it
    /// always asks first.
    pub(super) fn request_reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quit_prompted {
            return;
        }
        self.quit_prompted = true;
        let detail = if self.unsaved_files(cx) {
            "Running agents and terminals will stop, and unsaved file edits will be lost."
        } else {
            "Running agents and terminals will stop."
        };
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            "Reload MCode?",
            Some(detail),
            &["Reload", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                this.quit_prompted = false;
                if matches!(answer, Ok(0)) {
                    crate::restart(cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.unsaved_files(cx) {
            crate::quit(cx);
            return;
        }
        if self.quit_prompted {
            return;
        }
        self.quit_prompted = true;
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            "Quit with unsaved file edits?",
            Some("Save your files before quitting to keep your changes."),
            &["Quit Without Saving", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                this.quit_prompted = false;
                if matches!(answer, Ok(0)) {
                    crate::quit(cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn resume_git(&mut self, cx: &mut Context<Self>) {
        self.git.update(cx, |git, cx| git.resume(cx));
        if let Some(panel) = &self.right_panel {
            let panel = panel.read(cx);
            let (explorer, changes, history, graph) = (
                panel.explorer.clone(),
                panel.changes.clone(),
                panel.history.clone(),
                panel.graph.clone(),
            );
            explorer.update(cx, |e, cx| e.resume(cx));
            changes.update(cx, |c, cx| c.resume(cx));
            history.update(cx, |h, cx| h.resume(cx));
            graph.update(cx, |h, cx| h.resume(cx));
        }
    }

    pub(super) fn persist(&mut self, cx: &mut Context<Self>) {
        // A terminal pane closed with its tab or pane stops its shell.
        for pane in self.workspace.prune_terminal_panes() {
            self.terminal_pane_views.remove(&pane);
        }
        if let Some(id) = self.workspace.active {
            self.refresh_session_mark(id, cx);
        }
        self.workspace.worktree_focus = self.worktree_focus.clone();
        self.workspace.normalize_title_order();
        self.sync_git(cx);
        self.update_chat_visibility(cx);
        self.update_usage_provider(cx);
        self.sync_right_panel(cx);
        self.invalidate_chrome(cx);
        self.save_state(cx);
    }

    pub(super) fn save_window_state(&mut self, window: &Window, cx: &mut Context<Self>) {
        let bounds = window.inner_window_bounds();
        // X11 can report Windowed bounds while tracking fullscreen separately.
        let bounds = if window.is_fullscreen() {
            gpui::WindowBounds::Fullscreen(bounds.get_bounds())
        } else {
            bounds
        };
        if self.workspace.window.update(bounds) {
            self.save_state(cx);
        }
    }

    pub(super) fn save_state(&mut self, cx: &mut Context<Self>) {
        self.share_workspace(cx);
        self.write_state(cx);
    }

    /// Only the services window saves; another window's layout lasts while it is open.
    pub(super) fn write_state(&mut self, cx: &mut Context<Self>) {
        if !self.main {
            return;
        }
        let state = serde_json::to_string(&self.workspace).unwrap();
        *self.latest_state.lock().unwrap_or_else(|e| e.into_inner()) = state.clone();
        let revision = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = self.generation.clone();
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let store = locked(&store)?;
            if generation.load(Ordering::SeqCst) == revision {
                store.save_ui_state(&state)?;
            }
            Ok::<_, String>(())
        });
        cx.spawn(async move |this, cx| {
            if let Err(error) = work.await {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(format!("Could not save workspace: {error}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(super) fn update_usage_provider(&self, cx: &mut Context<Self>) {
        let view = self
            .monos
            .as_ref()
            .filter(|_| self.mono_active(cx))
            .and_then(|m| m.read(cx).chat())
            .or_else(|| {
                self.workspace.active.and_then(|id| {
                    self.sessions
                        .get(&self.workspace.session_for_pane(id))
                        .cloned()
                })
            });
        if let Some(view) = view {
            let view = view.read(cx);
            let kind = Kind::from_id(&view.summary.provider);
            let profile = view.settings.account.clone();
            let model = view.account_model().map(str::to_owned);
            let project = view.summary.cwd.clone();
            let session = Some(view.summary.id);
            self.usage_footer.update(cx, |footer, cx| {
                footer.context(kind, profile, model.as_deref(), project, session, cx)
            });
        }
    }
}
