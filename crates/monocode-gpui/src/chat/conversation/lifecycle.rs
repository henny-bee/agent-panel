use super::*;

impl Conversation {
    /// Seed workspace metadata before an unloaded blank can render or submit.
    /// Loaded/running conversations keep their current agent settings.
    pub(crate) fn restore_worktree(
        &mut self,
        path: Option<String>,
        branch: Option<String>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if !self.loaded && self.settings.revision == 0 && self.settings.worktree_cwd != path {
            self.settings.worktree_cwd = path;
            self.git = self
                .settings
                .worktree_cwd
                .as_deref()
                .and_then(crate::runtime::git_cache::get);
            self.branch = branch.or_else(|| self.git.as_ref().and_then(|s| s.current.clone()));
            self.sync_tool_paths(cx);
            cx.notify();
        } else if !self.loaded && branch.is_some() && self.branch != branch {
            self.branch = branch;
            cx.notify();
        }
        self.settings.worktree_cwd.clone()
    }

    pub(crate) fn set_classic_layout(&mut self, classic: bool, cx: &mut Context<Self>) {
        if self.classic_layout != classic {
            self.classic_layout = classic;
            cx.notify();
        }
    }

    pub(crate) fn set_beside_pane(&mut self, beside: bool, cx: &mut Context<Self>) {
        if self.beside_pane != beside {
            self.beside_pane = beside;
            cx.notify();
        }
    }

    pub(crate) fn set_project_appearance(
        &mut self,
        appearance: monocode_core::project::appearance::ProjectAppearance,
        cx: &mut Context<Self>,
    ) {
        if self.project_appearance != appearance {
            self.project_appearance = appearance;
            cx.notify();
        }
    }

    /// SessionCardPreview.tsx's compact model/status header and workspace details.
    /// Its latest message is saved as a draft, not sent (`session.draft`).
    pub(crate) fn has_saved_draft(&self) -> bool {
        if self.loaded {
            self.follow_ups.state.draft.is_some()
        } else {
            self.draft_hint
        }
    }

    pub(crate) fn archive_shortcut_blocked(&self, cx: &gpui::App) -> bool {
        self.menu.is_some()
            || self.response_menu.is_some()
            || self.git_picker.is_some()
            || self.needs_approval()
            || self.input.read(cx).is_composing()
    }

    pub(crate) fn needs_approval(&self) -> bool {
        !self.approvals.is_empty() || !self.questions.is_empty()
    }

    pub(crate) fn responding(&self) -> bool {
        self.busy && !self.stopping
    }

    pub(crate) fn sidebar_details(&self) -> Option<(SharedString, Option<String>)> {
        self.loaded
            .then(|| (self.model_label(), self.branch.clone()))
    }

    pub(crate) fn preview(&self, done: bool) -> AnyElement {
        let kind = Kind::from_id(&self.summary.provider);
        let model = self.resolved_model.as_deref().or(self.model.as_deref());
        let name = self
            .catalog
            .iter()
            .find(|entry| Some(entry.id.as_str()) == model)
            .map(|entry| model_display_name(kind, &entry.id, &entry.name))
            .unwrap_or_else(|| model_name(kind, model));
        let cwd = self
            .settings
            .worktree_cwd
            .as_deref()
            .unwrap_or(&self.summary.cwd);
        let directory = std::env::var_os("HOME")
            .and_then(|home| {
                Path::new(cwd).strip_prefix(home).ok().map(|path| {
                    if path.as_os_str().is_empty() {
                        "~".to_owned()
                    } else {
                        format!("~/{}", path.display())
                    }
                })
            })
            .unwrap_or_else(|| cwd.to_owned());
        let linked =
            self.settings.worktree_cwd.is_some() || self.git.as_ref().is_some_and(|git| git.linked);
        let detail = |label: &'static str, value: String| {
            div()
                .flex()
                .gap_2()
                .child(
                    div()
                        .w(px(72.))
                        .flex_shrink_0()
                        .text_color(content(0.45))
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(content(0.8))
                        .child(value),
                )
        };
        div()
            .px_3()
            .py_2p5()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .text_size(px(11.))
                            .text_color(content(0.55))
                            .child(provider_icon(&self.summary.provider, 14.))
                            .child(div().min_w_0().truncate().child(name)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(if self.needs_approval() {
                                gpui::rgb(0xfbbf24).into()
                            } else if self.responding() {
                                accent_color()
                            } else if done {
                                gpui::rgb(0x34d399).into()
                            } else {
                                content(0.45)
                            })
                            .child(if self.needs_approval() {
                                "Needs approval".to_owned()
                            } else if self.busy {
                                "Working…".to_owned()
                            } else if done {
                                "Done".to_owned()
                            } else if self.loading {
                                "Loading…".to_owned()
                            } else {
                                monocode_core::sys::time::relative(
                                    self.summary.updated_at,
                                    monocode_core::sys::time::now_ms(),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .mt_1p5()
                    .text_size(px(13.))
                    .line_height(px(20.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(content(1.))
                    .child(self.display_title()),
            )
            .child(
                div()
                    .mt_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(content(0.08))
                    .text_size(px(11.))
                    .line_height(px(16.))
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .when_some(self.branch.clone(), |d, branch| {
                        d.child(detail("Branch", branch))
                    })
                    .child(detail(
                        if linked { "Worktree" } else { "Directory" },
                        directory,
                    )),
            )
            .into_any_element()
    }

    pub(crate) fn new(summary: Session, store: Database, cx: &mut Context<Self>) -> Self {
        let input = cx.new(input::Input::session_composer);
        // A new tab shows its folder's checkout from the first frame; the
        // load below confirms it rather than popping it in.
        let git = (!summary.has_messages)
            .then(|| crate::runtime::git_cache::get(&summary.cwd))
            .flatten();
        let completion = cx.new(|cx| {
            crate::chat::composer::completion::Completion::new(
                input.clone(),
                summary.cwd.clone(),
                cx,
            )
            .with_store(store.clone())
        });
        completion.update(cx, |completion, cx| {
            completion.set_provider(&summary.provider, cx)
        });
        let completion_event = cx.subscribe(
            &completion,
            |this, _, event: &crate::chat::composer::completion::ModePicked, cx| {
                if event.0 == "add-to-folder" {
                    this.request_folder(cx);
                } else {
                    this.enable_mode(event.0, cx);
                }
            },
        );
        let command_removal = cx.subscribe(&input, |this, _, event: &input::CommandRemoved, cx| {
            this.clear_mode(event.0, cx)
        });
        let draft_observation = cx.observe(&input, |this, input, cx| {
            this.sync_resume(cx);
            let content = input.read(cx).content.clone();
            let folder =
                monocode_core::session::composer::consume_command(&content, "add-to-folder");
            if folder.is_none() {
                this.folder_selected = false;
            }
            if !this.folder_selected
                && folder == Some("")
                && content.ends_with(' ')
                && !input.read(cx).is_composing()
            {
                input.update(cx, |input, cx| {
                    input.set_text(content.trim_end().into(), cx)
                });
                this.request_folder(cx);
            }
            let has_value = input.read(cx).has_message_text();
            if this.draft_has_value != has_value {
                this.draft_has_value = has_value;
                cx.notify();
            }
            this.open_typed_btw(cx);
        });
        input.update(cx, |input, _| input.set_alternate_submit(true));
        let submission = cx.subscribe(&input, |this, _, _: &input::Submitted, cx| this.send(cx));
        let alternate_submission = cx
            .subscribe(&input, |this, _, _: &input::AlternateSubmitted, cx| {
                this.send_follow_up(true, cx)
            });
        let review = cx.new(|_| {
            crate::git::session_review::SessionReview::new(summary.id, summary.cwd.clone())
        });
        let review_events = cx.subscribe(&review, |this, review, event, cx| match event {
            crate::git::session_review::SessionReviewEvent::Open { path } => {
                let (session, cwd) = review.read(cx).session();
                cx.emit(OpenSessionChanges(crate::git::review::ReviewTab::Session {
                    cwd: cwd.to_owned(),
                    session,
                    path: path.clone(),
                }));
            }
            // Undo and Keep may rewrite files; the checkout's views reread.
            crate::git::session_review::SessionReviewEvent::Changed => {
                this.refresh_git(cx);
                cx.emit(SessionChanged);
            }
        });
        // The changes pill and its ledge card read the review's files. Files
        // the recorded turns changed, also through shell commands, count as
        // this session's edits for the commit pill.
        let review_observation = cx.observe(&review, |this, review, cx| {
            let paths = review.read(cx).paths();
            let known = this.session_edits.len();
            this.remember_session_edits(paths);
            if this.session_edits.len() != known {
                this.refresh_git_actions(cx);
            }
            cx.notify();
        });
        let transcript = cx.new(|_| {
            let mut view = TranscriptView::new();
            view.identity(&summary.provider, Kind::from_id(&summary.provider).label());
            view.file_link_base = Some(summary.cwd.clone());
            view
        });
        let account = crate::accounts::get(cx)
            .map(|center| {
                center
                    .read(cx)
                    .selected(Kind::from_id(&summary.provider), &summary.cwd)
            })
            .unwrap_or_default();
        let usage_clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(30))
                    .await;
                if this.update(cx, |this, cx| this.tick_usage(cx)).is_err() {
                    break;
                }
            }
        });
        let account_usage = crate::accounts::get(cx).map(|center| {
            cx.observe(&center, |this, _, cx| {
                if this.settings.usage_limit.is_some() {
                    this.tick_usage(cx);
                }
            })
        });
        perf_prompt(cx);
        Self {
            editing_last_turn: Default::default(),
            recall: None,
            btw: None,
            btw_home: (AnchorSlot::default(), 1.),
            _btw_observer: None,
            store,
            mono_turn: 0,
            submission_generation: 0,
            mono_ui: mono::ChatState::default(),
            transcript,
            review,
            _review_events: review_events,
            _review_observation: review_observation,
            input,
            completion,
            _completion_event: completion_event,
            _command_removal: command_removal,
            _draft_observation: draft_observation,
            _input_focus: None,
            draft_has_value: false,
            folder_selected: false,
            follow_ups: Default::default(),
            background: Default::default(),
            changes: Default::default(),
            attachments: Vec::new(),
            pending_attachments: Vec::new(),
            attachment_photos: Default::default(),
            attaching: false,
            file_drag: false,
            attachment_error: None,
            app_environment: Default::default(),
            app_read_only: false,
            planning_orchestration: false,
            orchestration_plan: None,
            orchestration_error: None,
            orchestration_progress: None,
            status: "Loading conversation…".into(),
            loaded: false,
            history_unloaded: false,
            history_actions: Default::default(),
            pending_search: None,
            loading: false,
            busy: false,
            stopping: false,
            notification_cancelled: false,
            draft_hint: false,
            model: std::env::var(format!(
                "MONOCODE_{}_MODEL",
                summary.provider.to_uppercase()
            ))
            .ok(),
            resolved_model: None,
            settings: Settings {
                account,
                access: default_access(cx),
                effort: preferred_effort(&summary.provider, cx),
                ..Default::default()
            },
            project_appearance: Default::default(),
            classic_layout: false,
            catalog: Vec::new(),
            catalog_epoch: 0,
            account_switching: false,
            _usage_clock: usage_clock,
            _account_usage: account_usage,
            approvals: Default::default(),
            approval_ui: Default::default(),
            questions: Default::default(),
            question_overlay: Default::default(),
            controls: None,
            branch: git.as_ref().and_then(|s| s.current.clone()),
            git,
            git_picker: None,
            git_events: None,
            preparing_worktree: false,
            context: None,
            context_hovered: false,
            send_morph: Default::default(),
            send_alternate: false,
            menu: None,
            add_anchor: AnchorSlot::default(),
            model_anchor: AnchorSlot::default(),
            access_anchor: AnchorSlot::default(),
            account_anchor: AnchorSlot::default(),
            workspace_anchor: AnchorSlot::default(),
            git_action_anchor: AnchorSlot::default(),
            session_edits: Vec::new(),
            session_git: Default::default(),
            session_unpushed: false,
            session_edits_generation: 0,
            branch_anchor: AnchorSlot::default(),
            context_anchor: AnchorSlot::default(),
            composer_anchor: AnchorSlot::default(),
            composer_motion: ComposerMotion::default(),
            narrow: false,
            beside_pane: false,
            arcade: None,
            ink: None,
            runner: None,
            cancel: Arc::new(AtomicBool::new(false)),
            title_cancel: None,
            claude_terminal: None,
            claude_start: None,
            switching_mode: false,
            resume: None,
            claude_activity: None,
            worktree_name_cancel: None,
            _submission: submission,
            _alternate_submission: alternate_submission,
            response_events: None,
            response_menu: None,
            response_request: None,
            summary,
            remote_state: remote::State::default(),
            remote_plans: remote_plans::State::default(),
        }
    }

    pub(crate) fn reveal_search(&mut self, row: usize, query: String, cx: &mut Context<Self>) {
        if !self.loaded || self.history_unloaded {
            self.pending_search = Some((row, query));
            self.load(cx);
            return;
        }
        self.transcript
            .update(cx, |view, cx| view.reveal_saved_search(row, &query, cx));
    }

    /// Reads this conversation's older turns for the transcript as it scrolls.
    pub(crate) fn history_pager(&self) -> crate::chat::transcript::HistoryPager {
        let store = self.store.clone();
        let id = self.summary.id;
        Arc::new(move |before, all| locked(&store)?.load_before(id, before, all))
    }

    pub(crate) fn load(&mut self, cx: &mut Context<Self>) {
        if self.is_remote() {
            self.load_remote(cx);
            return;
        }
        if self.history_unloaded {
            self.reload_history(cx);
            return;
        }
        if self.loaded || self.loading {
            return;
        }
        self.loading = true;
        self.refresh_catalog(cx);
        let store = self.store.clone();
        let id = self.summary.id;
        let mono = self.settings.mono.is_some();
        let work = cx.background_executor().spawn(async move {
            let settings = locked(&store).and_then(|s| s.agent_settings(id));
            (
                locked(&store)
                    .and_then(|s| {
                        let history = s.load(id)?;
                        let edits = if mono {
                            Vec::new()
                        } else {
                            git_actions::restored_session_edits(&s, id, &history.rows)?
                        };
                        Ok((history, edits))
                    })
                    .map(|(history, edits)| {
                        (
                            crate::chat::transcript::PreparedHistory::new(history.rows, mono),
                            edits,
                        )
                    }),
                settings,
                locked(&store).and_then(|s| s.context_usage(id)),
                locked(&store).and_then(|s| s.follow_ups(id)),
                locked(&store).and_then(|s| s.history_before(id)),
            )
        });
        cx.spawn(async move |this, cx| {
            let (result, settings, context, follow_ups, before) = work.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.context = context.ok().flatten();
                match follow_ups {
                    Ok(state) => {
                        this.follow_ups.persisted = state.revision;
                        this.follow_ups.state = state;
                        let draft = this.follow_ups.state.draft.clone();
                        if draft.is_some() {
                            this.summary.has_messages = true;
                        }
                        this.transcript
                            .update(cx, |view, cx| view.set_saved_draft(draft, cx));
                    }
                    Err(error) => {
                        this.status = error;
                        this.load_git(cx);
                        return;
                    }
                }
                // The composer may have a newer revision while history loads;
                // historical turn provenance still belongs to the loaded rows.
                if let Ok(Some(settings)) = &settings {
                    this.transcript
                        .update(cx, |view, _| view.set_turns(settings.turns.clone()));
                    this.settings.turns = settings.turns.clone();
                }
                if let Ok(Some(mut settings)) = settings
                    && settings.revision >= this.settings.revision
                {
                    if !this.summary.has_messages
                        && settings.revision == 0
                        && settings.account.id == monocode_core::session::profile::DEFAULT
                    {
                        settings.account = crate::accounts::get(cx)
                            .map(|center| {
                                center.read(cx).selected(
                                    Kind::from_id(&this.summary.provider),
                                    &this.summary.cwd,
                                )
                            })
                            .unwrap_or_default();
                    }
                    this.model = settings.model.clone();
                    let mono = this.settings.mono.take();
                    let environment = std::mem::take(&mut this.settings.environment);
                    this.settings = settings;
                    this.attach_account(cx);
                    this.settings.mono = mono;
                    this.settings.environment = environment;
                }
                match result {
                    Ok((history, edits)) => {
                        this.remember_session_edits(edits);
                        let pager = this.history_pager();
                        this.transcript.update(cx, |v, cx| {
                            v.install_prepared_history(history, cx);
                            v.set_history_pager(before.ok().flatten(), Some(pager));
                        });
                        this.status = "Ready".into();
                        this.loaded = true;
                        if let Some((row, query)) = this.pending_search.take() {
                            this.reveal_search(row, query, cx);
                        }
                        this.sync_review(cx);
                        this.run_history_actions(cx);
                        this.pump_follow_ups(cx);
                    }
                    Err(e) => this.status = e,
                }
                // Git can take tens of milliseconds; history shows first.
                if let Some(git) = crate::runtime::git_cache::get(this.execution_cwd()) {
                    this.branch = git.current.clone();
                    this.git = Some(git);
                }
                this.load_git(cx);
                this.mono_flush_external(cx);
                cx.emit(SessionChanged);
                cx.notify();
            });
        })
        .detach();
    }
}

impl Conversation {
    pub(crate) fn worktree_deleted(
        &mut self,
        summary: Session,
        settings: Settings,
        cx: &mut Context<Self>,
    ) {
        self.summary = summary;
        self.settings = settings;
        self.context = None;
        self.status = "Worktree deleted".into();
        self.refresh_git(cx);
        cx.emit(SessionChanged);
        cx.notify();
    }

    pub(super) fn execution_cwd(&self) -> &str {
        if self.settings.worktree_removed {
            return &self.summary.cwd;
        }
        self.settings
            .worktree_cwd
            .as_deref()
            .unwrap_or(&self.summary.cwd)
    }

    pub(super) fn sync_tool_paths(&mut self, cx: &mut Context<Self>) {
        let cwd = self.execution_cwd().to_owned();
        self.transcript.update(cx, |view, cx| {
            if view.file_link_base.as_deref() != Some(cwd.as_str()) {
                view.file_link_base = Some(cwd);
                cx.notify();
            }
        });
    }

    /// The review card follows the checkout and rereads its recorded turn.
    pub(crate) fn sync_review(&mut self, cx: &mut Context<Self>) {
        self.sync_tool_paths(cx);
        if self.settings.mono.is_some() || self.is_remote() {
            return;
        }
        self.collect_session_edits(cx);
        let cwd = self.execution_cwd().to_owned();
        self.review.update(cx, |review, cx| {
            if review.session().1 == cwd {
                review.refresh(cx);
            } else {
                review.set_cwd(cwd, cx);
            }
        });
    }

    pub(super) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        self.sync_review(cx);
        self.load_git(cx);
    }

    /// A switch made elsewhere, such as the Changes panel or a terminal,
    /// reloads the composer's branch label.
    pub(crate) fn follow_branch(&mut self, branch: Option<&str>, cx: &mut Context<Self>) {
        if branch.is_some() && self.branch.as_deref() != branch {
            self.load_git(cx);
        }
    }

    pub(super) fn load_git(&mut self, cx: &mut Context<Self>) {
        let cwd = self.execution_cwd().to_owned();
        let work_cwd = cwd.clone();
        let work = cx
            .background_executor()
            .spawn(async move { crate::runtime::git_cache::load(Path::new(&work_cwd)).ok() });
        cx.spawn(async move |this, cx| {
            let git = work.await;
            let _ = this.update(cx, |this, cx| {
                if this.execution_cwd() == cwd {
                    this.branch = git.as_ref().and_then(|s| s.current.clone());
                    this.git = git;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn prepare_worktree(&mut self, cx: &mut Context<Self>) {
        self.preparing_worktree = true;
        self.status = "Creating worktree…".into();
        let cwd = self.summary.cwd.clone();
        let base = self
            .settings
            .worktree_base
            .clone()
            .or(self.branch.clone())
            .unwrap_or_else(|| "HEAD".into());
        let id = self.summary.id;
        let work = cx.background_executor().spawn(async move {
            monocode_core::vcs::git::create_worktree(Path::new(&cwd), &base, id)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.preparing_worktree = false;
                match result {
                    Ok(path) => {
                        this.settings.worktree_cwd = Some(path.to_string_lossy().into_owned());
                        this.settings.new_worktree = false;
                        this.save_settings(cx);
                        this.refresh_git(cx);
                        if this.follow_ups.state.messages.is_empty() {
                            this.send(cx);
                        } else {
                            this.pump_follow_ups(cx);
                        }
                    }
                    Err(error) => {
                        this.status = error.clone();
                        if let Some(message) = this.follow_ups.state.messages.first() {
                            let id = message.id.clone();
                            this.fail_follow_up(&id, error, cx);
                        }
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn open_git(&mut self, mode: GitMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.preparing_worktree {
            return;
        }
        if self.git_picker.take().is_some() {
            self.git_events = None;
            window.focus(&self.input.focus_handle(cx), cx);
            cx.notify();
            return;
        }
        self.menu = None;
        let cwd = self.execution_cwd().into();
        let selected = if mode == GitMode::Base {
            self.settings.worktree_base.clone().or(self.branch.clone())
        } else {
            self.branch.clone()
        };
        let picker = cx.new(|cx| GitPicker::new(cwd, mode, selected, cx));
        self.git_events =
            Some(
                cx.subscribe_in(&picker, window, |this, _, event: &GitPicked, window, cx| {
                    match event {
                        GitPicked::Changed => {
                            if this.settings.worktree_removed {
                                this.settings.worktree_removed = false;
                                this.settings.worktree_cwd = None;
                                this.save_settings(cx);
                            }
                            this.refresh_git(cx);
                        }
                        GitPicked::Base(base) => {
                            this.settings.worktree_base = Some(base.clone());
                            this.save_settings(cx);
                        }
                        GitPicked::Worktree(path) => {
                            if !this.use_existing_worktree(path, cx) {
                                return;
                            }
                        }
                        GitPicked::Dismissed => {}
                    }
                    this.git_picker = None;
                    this.git_events = None;
                    window.focus(&this.input.focus_handle(cx), cx);
                    cx.notify();
                }),
            );
        window.focus(&picker.read(cx).focus(cx), cx);
        self.git_picker = Some(picker);
        cx.notify();
    }

    pub(super) fn use_existing_worktree(&mut self, path: &str, cx: &mut Context<Self>) -> bool {
        if self.summary.has_messages && !self.settings.worktree_removed {
            return false;
        }
        self.settings.worktree_removed = false;
        self.settings.worktree_cwd = (monocode_core::files::path_key(path)
            != monocode_core::files::path_key(&self.summary.cwd))
        .then(|| path.to_owned());
        self.settings.new_worktree = false;
        self.save_settings(cx);
        self.refresh_git(cx);
        true
    }
}
