use super::*;

impl Shell {
    pub(super) fn update_chat_visibility(&mut self, cx: &mut Context<Self>) {
        if let Some(inbox) = &self.inbox {
            let active = self.active_app_view == Some(glue::app_views::AppView::Inbox)
                && self.settings.is_none()
                && !self.search_page();
            inbox.update(cx, |inbox, cx| inbox.set_active(active, cx));
        }
        // Every pane of the active tab is on screen, not just the focused one.
        let mut shown: HashSet<i64> = match self.workspace.active_tab() {
            Some(tab)
                if !self.app_view_open()
                    && self.settings.is_none()
                    && !self.search_page()
                    && !self.mono_active(cx) =>
            {
                self.workspace.layout(tab).leaf_ids().into_iter().collect()
            }
            _ => HashSet::new(),
        };
        // A session opened in the Mono details panel is on screen too. Left
        // out, it was trimmed as inactive and the panel showed it empty.
        if self.mono_active(cx)
            && self.workspace.right_panel_open
            && !self.app_view_open()
            && self.settings.is_none()
            && !self.search_page()
        {
            shown.extend(
                self.monos
                    .as_ref()
                    .and_then(|m| m.read(cx).routed_session()),
            );
        }
        // Hidden tabs keep their prepared image and any in-flight load. Only
        // closing a tab/pane should discard its background.
        let open: HashSet<i64> = self.workspace.open_sessions().into_iter().collect();
        self.backgrounds.retain(|id, _| open.contains(id));
        for &id in &shown {
            self.ensure_session(id, cx);
            self.history_clock += 1;
            self.history_recency.insert(id, self.history_clock);
        }
        self.shown = shown.clone();
        let shown = self.shown_anywhere(shown, cx);
        for (&id, view) in &self.sessions {
            view.update(cx, |v, cx| {
                if shown.contains(&id) {
                    v.load(cx);
                }
                v.transcript
                    .update(cx, |t, cx| t.visible(shown.contains(&id), cx))
            });
        }
        self.trim_session_histories(&shown, cx);
    }

    /// SessionPane's reviewUndoLocked: another running session shares the checkout.
    pub(super) fn sync_review_locks(&mut self, cx: &mut Context<Self>) {
        let running: Vec<(i64, String)> = self
            .sessions
            .iter()
            .filter(|(_, view)| view.read(cx).busy)
            .map(|(&id, view)| (id, view.read(cx).review.read(cx).session().1.to_owned()))
            .collect();
        for (&id, view) in &self.sessions {
            let review = view.read(cx).review.clone();
            let cwd = review.read(cx).session().1.to_owned();
            let locked = running
                .iter()
                .any(|(other, path)| *other != id && *path == cwd);
            review.update(cx, |review, cx| review.set_undo_locked(locked, cx));
        }
    }

    pub(super) fn refresh_session_mark(&mut self, id: i64, cx: &mut Context<Self>) {
        let id = self.workspace.session_for_pane(id);
        let Some(view) = self.sessions.get(&id) else {
            return;
        };
        let session = view.read(cx);
        let busy = session.responding();
        let focused = !self.app_view_open() && self.workspace.active == Some(id);
        if busy {
            self.busy_sessions.insert(id);
            self.unseen_finished.remove(&id);
        } else if session.busy {
            // Explicit cancellation is already idle in the UI. The worker is
            // only finishing its save, so it must not announce a completed turn.
            self.busy_sessions.remove(&id);
            self.unseen_finished.remove(&id);
        } else if self.busy_sessions.remove(&id) && !focused && !session.summary.archived {
            self.unseen_finished.insert(id);
        }
        if focused || session.summary.archived {
            self.unseen_finished.remove(&id);
        }
        let state = if busy {
            MarkState::Working
        } else if self.unseen_finished.contains(&id) {
            MarkState::Done
        } else {
            MarkState::Idle
        };
        // A blocked turn is still busy, but the tab asks for attention first.
        let attention = session.needs_approval();
        let tab_state = if attention {
            MarkState::Attention
        } else {
            state
        };
        let provider = session.summary.provider.clone();
        self.session_marks[&id].update(cx, |mark, cx| mark.set(&provider, state, cx));
        self.tab_marks[&id].update(cx, |mark, cx| mark.set(&provider, tab_state, cx));
    }

    pub(super) fn toggle_archive(&mut self, id: i64, cx: &mut Context<Self>) {
        if !self.archiving.insert(id) {
            return;
        }
        self.close_preview(cx);
        let archived = !self.session_metadata(id, cx).summary.archived;
        let store = self.store.clone();
        let work = cx
            .background_executor()
            .spawn(async move { locked(&store)?.set_archived(id, archived) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.archiving.remove(&id);
                match result {
                    Ok(()) => {
                        if let Some(summary) = this.session_summaries.get_mut(&id) {
                            summary.archived = archived;
                        }
                        if let Some(view) = this.sessions.get(&id) {
                            view.update(cx, |session, cx| {
                                session.summary.archived = archived;
                                if archived {
                                    session.stop_if_busy(cx);
                                }
                            });
                        }
                        this.refresh_session_mark(id, cx);
                        if archived && this.workspace.tab_of(id).is_some() {
                            this.close_pane(id, window, cx);
                        } else {
                            this.invalidate_chrome(cx);
                            cx.notify();
                        }
                    }
                    Err(error) => {
                        this.error = Some(format!("Could not update archive: {error}"));
                        this.invalidate_chrome(cx);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        self.invalidate_chrome(cx);
    }

    pub(super) fn transfer_response(&mut self, transfer: Transfer, cx: &mut Context<Self>) {
        use monocode_core::session::{
            agent::{Handoff, Settings},
            response::{self, TransferKind},
        };
        let navigation_revision = self.navigation_revision;
        let store = self.store.clone();
        let target_kind = Kind::from_id(&transfer.target.provider);
        let account = crate::accounts::get(cx)
            .map(|center| center.read(cx).selected(target_kind, &transfer.cwd))
            .unwrap_or_default();
        let work = cx.background_executor().spawn(async move {
            let mut settings = Settings {
                account,
                revision: response::now_ms() * 1_000_000,
                model: transfer.target.model.clone(),
                effort: transfer.target.effort.clone(),
                model_settings: transfer.target.settings.clone(),
                access: transfer.settings.access,
                worktree_cwd: transfer.settings.worktree_cwd.clone(),
                ..Settings::default()
            };
            let prompt = if transfer.kind == TransferKind::SecondOpinion {
                Some(response::second_opinion(&transfer.from, &transfer.turn))
            } else {
                settings.handoff = Some(Handoff {
                    from: transfer.from.clone(),
                    brief: response::handoff(&transfer.through),
                });
                None
            };
            let mut summary = locked(&store)?.create(&transfer.target.provider, &transfer.cwd)?;
            summary.title = if transfer.kind == TransferKind::SecondOpinion {
                "Second opinion".into()
            } else {
                transfer.title
            };
            locked(&store)?.title(summary.id, &summary.title)?;
            locked(&store)?.configure_agent(summary.id, &summary.provider, &settings)?;
            Ok::<_, String>((summary, settings, prompt))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok((summary, settings, prompt)) => {
                    let id = summary.id;
                    this.add(summary, cx);
                    this.sessions[&id]
                        .update(cx, |view, cx| view.install_transfer(settings, prompt, cx));
                    this.order.retain(|s| *s != id);
                    this.order.insert(0, id);
                    if this.navigation_revision == navigation_revision {
                        this.activate_session(id, window, cx);
                    }
                }
                Err(error) => {
                    this.error = Some(error);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn notify_sidebar(&self, cx: &mut Context<Self>) {
        self.sidebar_list.invalidate_structure();
        self.sidebar_frame.update(cx, |_, cx| cx.notify());
    }

    pub(super) fn invalidate_chrome(&self, cx: &mut Context<Self>) {
        self.notify_sidebar(cx);
        self.tabs_frame.update(cx, |_, cx| cx.notify());
        if self.workspace.classic_layout {
            self.projects_frame.update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn reveal_active_tab(&self, cx: &App) {
        if self.workspace.classic_layout {
            self.reveal_active_project_rail();
        }
        if let Some(ix) = self
            .visible_tabs(cx)
            .iter()
            .position(|tab| Some(tab) == self.active_title_tab(cx).as_ref())
        {
            self.tab_scroll.scroll_to_item(ix);
        }
    }

    /// Sidebar.tsx's `gitRoot`: the active session's worktree or folder.
    pub(super) fn sync_git(&mut self, cx: &mut Context<Self>) {
        self.classic_sidebar_tab = self
            .workspace
            .classic_sidebar_tabs
            .get(&self.workspace.project)
            .map(|id| sidebar::classic::ClassicSidebarTab::from_id(id))
            .unwrap_or_default();
        let session = self
            .workspace
            .active
            .and_then(|id| self.sessions.get(&self.workspace.session_for_pane(id)))
            .map(|view| {
                let view = view.read(cx);
                (
                    view.settings
                        .worktree_cwd
                        .clone()
                        .unwrap_or_else(|| view.summary.cwd.clone()),
                    Kind::from_id(&view.summary.provider),
                )
            });
        let (project, provider) = match session {
            Some((cwd, kind)) => (cwd, Some(kind)),
            None => (self.working_copy().to_owned(), None),
        };
        let project = self
            .workspace
            .active
            .and_then(|id| self.workspace.panel_folders.get(&id).cloned())
            .unwrap_or(project);
        self.git.update(cx, |git, cx| git.set_project(project, cx));
        if let (Some(panel), Some(kind)) = (&self.right_panel, provider) {
            let changes = panel.read(cx).changes.clone();
            changes.update(cx, |c, _| c.text_provider = kind);
        }
        self.sync_right_panel(cx);
    }

    pub(crate) fn focus_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keep_visible_idle_agents(cx);
        if let Some(search) = &self.app_search {
            window.focus(&search.view.focus_handle(cx), cx);
            return;
        }
        if self.focus_app_view(window, cx) {
            return;
        }
        if let Some(monos) = self.monos.clone().filter(|m| m.read(cx).active.is_some()) {
            monos.update(cx, |m, cx| m.focus_active(window, cx));
            return;
        }
        if self.focus_mono_pane(window, cx) || self.focus_terminal_pane(window, cx) {
            return;
        }
        if let Some(review) = self
            .active_review(cx)
            .filter(|review| review.read(cx).state.chat_hidden)
        {
            review.update(cx, |r, cx| r.focus_active(window, cx));
            return;
        }
        if let Some(view) = self.workspace.active.and_then(|id| self.sessions.get(&id)) {
            let focus = view.read(cx).session_focus(cx);
            window.focus(&focus, cx);
        } else {
            window.focus(&self.focus, cx);
        }
    }

    pub(crate) fn open(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.review_owners.contains_key(&id) {
            self.open_file_pane(id, window, cx);
            return;
        }
        if self.workspace.mono_panes.contains_key(&id) {
            self.open_mono_pane(id, window, cx);
            return;
        }
        if self.workspace.terminal_panes.contains_key(&id) {
            self.open_terminal_pane(id, window, cx);
            return;
        }
        if !self.session_summaries.contains_key(&id) || self.deleting.contains(&id) {
            return;
        }
        self.navigation_revision += 1;
        self.replace_blank_pane(id, cx);
        self.activate_session(id, window, cx);
    }

    /// Opening a conversation that has no tab takes over a blank pane in the
    /// active tab instead of adding a tab, as in the desktop app.
    pub(super) fn replace_blank_pane(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.workspace.tab_of(id).is_some()
            || !self.session_metadata(id, cx).summary.has_messages
        {
            return;
        }
        let Some(tab) = self.workspace.active_tab() else {
            return;
        };
        let cwd = &self.session_metadata(id, cx).summary.cwd;
        let working_copy = monocode_core::files::path_key(self.session_working_copy(id, cx));
        let blank = |pane: i64| {
            if self.workspace.review_owners.contains_key(&pane)
                || !self.session_summaries.contains_key(&pane)
            {
                return false;
            }
            let s = self.session_metadata(pane, cx);
            &s.summary.cwd == cwd
                && monocode_core::files::path_key(self.session_working_copy(pane, cx))
                    == working_copy
                && !s.summary.has_messages
                && !s.busy
                && !s.has_saved_draft()
                && self
                    .sessions
                    .get(&pane)
                    .is_none_or(|v| v.read(cx).input.read(cx).content.is_empty())
        };
        let focused = self.workspace.tab_focus(tab);
        let Some(pane) = std::iter::once(focused)
            .chain(self.workspace.layout(tab).leaf_ids())
            .find(|pane| blank(*pane))
        else {
            return;
        };
        self.workspace.swap_session(pane, id);
        if let Some(next) = self.workspace.tab_of(id) {
            self.replace_title_tab_key(tab, next);
        }
    }

    pub(super) fn activate_session(
        &mut self,
        id: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace.review_owners.contains_key(&id) {
            self.open_file_pane(id, window, cx);
            return;
        }
        if self.workspace.mono_panes.contains_key(&id) {
            self.open_mono_pane(id, window, cx);
            return;
        }
        if self.workspace.terminal_panes.contains_key(&id) {
            self.open_terminal_pane(id, window, cx);
            return;
        }
        if let Some(monos) = &self.monos {
            monos.update(cx, |m, cx| m.close(cx));
        }
        self.leave_app_views(cx);
        self.leave_app_search(cx);
        self.close_preview(cx);
        self.dismiss_due_reminder(id, cx);
        let Some(view) = self.ensure_session(id, cx) else {
            return;
        };
        self.history.record(id);
        self.workspace.open(id);
        self.workspace.project = view.read(cx).summary.cwd.clone();
        self.workspace
            .project_focus
            .insert(self.workspace.project.clone(), id);
        self.workspace.collapsed.remove(&self.workspace.project);
        view.update(cx, |v, cx| v.load(cx));
        self.record_worktree_session(id, cx);
        self.remember_title_tab(TitleTab::Session(self.workspace.tab_of(id).unwrap()));
        self.reveal_active_tab(cx);
        self.review_focused = self
            .workspace
            .reviews
            .get(&id)
            .is_some_and(|review| review.chat_hidden);
        self.focus_active(window, cx);
        self.persist(cx);
        cx.notify();
    }

    pub(crate) fn close(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tab_can_close(id, cx) {
            return;
        }
        self.close_menu(cx);
        self.close_tabs(&[id], None, window, cx);
    }

    /// Apply a close operation to one snapshot. Intermediate neighbours must
    /// never change the project used for the final selection or blank tab.
    pub(super) fn close_tabs(
        &mut self,
        ids: &[i64],
        preferred: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tabs = ids
            .iter()
            .filter_map(|id| self.workspace.tab_of(*id))
            .map(TitleTab::Session)
            .collect::<Vec<_>>();
        let preferred = preferred
            .and_then(|id| self.workspace.tab_of(id))
            .map(TitleTab::Session);
        self.close_title_tabs(&tabs, preferred, window, cx);
    }

    pub(super) fn visit(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        // Settings sit over the history: Back leaves them, and Forward
        // leaves them on its way to the next visit.
        if self.settings.is_some() {
            self.close_settings(window, cx);
            if step < 0 {
                return;
            }
        }
        if let Some(id) = self.history.step(step) {
            self.open(id, window, cx);
        }
    }

    pub(super) fn cycle(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let tabs = self.visible_tabs(cx);
        if tabs.is_empty() {
            return;
        }
        let active = self.active_title_tab(cx);
        let current = tabs.iter().position(|tab| Some(tab) == active.as_ref());
        let next = current.map_or(if step < 0 { tabs.len() - 1 } else { 0 }, |index| {
            (index as isize + step).rem_euclid(tabs.len() as isize) as usize
        });
        self.open_title_tab(&tabs[next], window, cx);
    }

    pub(super) fn focus_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(settings) = &self.settings {
            window.focus(&settings.focus_handle(cx), cx);
            return;
        }
        window.focus(&self.focus, cx);
        if self.focus_app_view(window, cx) {
            return;
        }
        if self.review_focused
            && let Some(review) = self.active_review(cx)
        {
            review.update(cx, |r, cx| r.focus_active(window, cx));
        } else {
            self.focus_active(window, cx);
        }
    }

    pub(super) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_app_search(window, cx);
    }

    pub(crate) fn select_project(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_sessions.clear();
        self.selection_anchor = None;
        self.sidebar_target = None;
        self.navigation_revision += 1;
        self.leave_app_views(cx);
        self.leave_app_search(cx);
        self.close_preview(cx);
        if let Some(monos) = self.monos.clone() {
            monos.update(cx, |m, cx| m.close(cx));
        }
        self.workspace.project = path.clone();
        self.workspace.active = None;
        if let Some(ix) = self.workspace.projects.iter().position(|p| p == &path) {
            if ix < self.spaces_start {
                self.spaces_start = ix;
            } else if ix >= self.spaces_start + 5 {
                self.spaces_start = ix - 4;
            }
        }
        self.workspace.collapsed.remove(&path);
        // Original-layout project buttons leave the standalone Mono/app view.
        // The new layout restores each project's selected tab, including Monos.
        let remembered_session = self
            .workspace
            .scope_focus
            .get(&self.tab_scope())
            .is_some_and(|tab| matches!(tab, TitleTab::Session(_)));
        if (!self.workspace.classic_layout || remembered_session)
            && self.restore_scope_tab(window, cx)
        {
            return;
        }
        self.workspace.active = self
            .workspace
            .project_focus
            .get(&path)
            .copied()
            .filter(|id| {
                self.workspace.tab_of(*id).is_some()
                    && self
                        .session_summaries
                        .contains_key(&self.workspace.session_for_pane(*id))
                    && self
                        .session_metadata(self.workspace.session_for_pane(*id), cx)
                        .summary
                        .cwd
                        == path
                    && self.in_worktree_scope(*id, cx)
            })
            .or_else(|| {
                self.workspace
                    .tabs
                    .iter()
                    .rev()
                    .find(|id| {
                        self.session_summaries.contains_key(
                            &self
                                .workspace
                                .session_for_pane(self.workspace.scope_pane(**id)),
                        ) && self
                            .session_metadata(
                                self.workspace
                                    .session_for_pane(self.workspace.scope_pane(**id)),
                                cx,
                            )
                            .summary
                            .cwd
                            == path
                            && self.in_worktree_scope(self.workspace.scope_pane(**id), cx)
                    })
                    .map(|tab| self.workspace.tab_focus(*tab))
            });
        if let Some(id) = self.workspace.active {
            self.activate_session(id, window, cx);
        } else {
            self.ensure_project_session(window, cx);
        }
        self.focus_active(window, cx);
        self.persist(cx);
        cx.notify();
    }

    pub(super) fn ensure_project_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.active.is_some() || self.mono_active(cx) {
            return;
        }
        if let Some(id) = self.order.iter().copied().find(|id| {
            let s = self.session_metadata(*id, cx);
            s.summary.cwd == self.workspace.project
                && monocode_core::files::path_key(self.session_working_copy(*id, cx))
                    == monocode_core::files::path_key(self.working_copy())
                && !s.summary.archived
                && !s.summary.has_messages
                && !s.busy
                && !s.has_saved_draft()
                && self
                    .sessions
                    .get(id)
                    .is_none_or(|v| v.read(cx).input.read(cx).content.is_empty())
                && self.workspace.tab_of(*id).is_none()
                && self
                    .workspace
                    .reviews
                    .get(id)
                    .is_none_or(|r| r.tabs.is_empty())
        }) {
            self.activate_session(id, window, cx);
        } else {
            self.create_session(self.default_choice(), true, cx);
        }
    }

    pub(super) fn open_project(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open project".into()),
        });
        cx.spawn(async move |this, cx| match picker.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.first() {
                    let path = path.to_string_lossy().into_owned();
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.workspace.add_project(&path);
                        this.select_project(path, window, cx);
                    });
                }
            }
            Ok(Ok(None)) => {}
            error => {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(format!("Could not open folder: {error:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn new_chat(&mut self, cx: &mut Context<Self>) {
        self.create_session(self.default_choice(), false, cx);
    }

    pub(super) fn create(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.create_session(self.explicit_choice(kind), false, cx);
    }

    pub(super) fn create_session(
        &mut self,
        choice: glue::new_chat_choice::NewChatChoice,
        only_if_empty: bool,
        cx: &mut Context<Self>,
    ) {
        self.create_session_target(choice, only_if_empty, None, cx);
    }

    pub(super) fn create_in_folder(&mut self, folder: String, cx: &mut Context<Self>) {
        self.create_session_target(self.default_choice(), false, Some(folder), cx);
    }

    pub(super) fn create_session_target(
        &mut self,
        choice: glue::new_chat_choice::NewChatChoice,
        only_if_empty: bool,
        folder: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let store = self.store.clone();
        let cwd = self.workspace.project.clone();
        let focus = self.worktree_focus.get(&cwd).cloned();
        // The project folder as a selected checkout is not a separate worktree.
        let copy = focus.clone().filter(|f| {
            monocode_core::files::path_key(&f.path) != monocode_core::files::path_key(&cwd)
        });
        let blank_key = format!("{cwd}\0{}", focus.as_ref().map_or("", |f| f.path.as_str()));
        if only_if_empty && !self.pending_project_blanks.insert(blank_key.clone()) {
            return;
        }
        let navigation_revision = self.navigation_revision;
        let defaults = sessions::loading::NewSessionDefaults::read(choice, &cwd, cx);
        let work = cx.background_executor().spawn({
            let cwd = cwd.clone();
            let worktree_cwd = copy.as_ref().map(|f| f.path.clone());
            async move { defaults.create(&*locked(&store)?, &cwd, worktree_cwd) }
        });
        let previous = self.new_tab_task.take();
        self.new_tab_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            if let Some(previous) = previous {
                previous.await;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                if only_if_empty {
                    this.pending_project_blanks.remove(&blank_key);
                }
                match result {
                    Ok((summary, settings)) => {
                        let id = summary.id;
                        this.add(summary, cx);
                        if let Some(folder) = &folder {
                            let groups = this.groups.entry(cwd.clone()).or_default();
                            groups.add_to_folder(folder, id);
                            let groups = groups.clone();
                            let project = cwd.clone();
                            this.store_write("save folders", cx, move |store| {
                                store.save_groups(&project, &groups)
                            });
                        }
                        if let Some(view) = this.sessions.get(&id) {
                            view.update(cx, |view, cx| view.seed_new_settings(settings, cx));
                        }
                        this.seed_worktree(id, copy.as_ref(), cx);
                        // A pending blank belongs to the copy that requested it;
                        // navigation to another copy must never redirect that request.
                        if this.workspace.project != cwd
                            || this.worktree_focus.get(&cwd).map(|f| &f.path)
                                != focus.as_ref().map(|f| &f.path)
                            || if only_if_empty {
                                this.workspace.active.is_some()
                                    || this.mono_active(cx)
                                    || this.app_view_open()
                                    || this.settings.is_some()
                            } else {
                                this.navigation_revision != navigation_revision
                            }
                        {
                            this.invalidate_chrome(cx);
                            cx.notify();
                            return;
                        }
                        this.order.retain(|s| *s != id);
                        this.order.insert(0, id);
                        this.activate_session(id, window, cx);
                    }
                    Err(error) => {
                        this.error = Some(error);
                        cx.notify();
                    }
                }
            });
        }));
        cx.notify();
    }

    /// The project's sessions as the sidebar lists them, top to bottom.
    pub(super) fn sidebar_session_ids(&self, cx: &App) -> Vec<i64> {
        let start = self
            .workspace
            .session_filters
            .time
            .start(monocode_core::sys::time::now_ms());
        // No selected worktree means the whole project, as in the desktop
        // sidebar. Tab restoration still uses the stricter execution scope.
        let listable = |id: i64| {
            let s = self.session_metadata(id, cx);
            !self
                .monos
                .as_ref()
                .is_some_and(|m| m.read(cx).hidden_session(id))
                && s.summary.cwd == self.workspace.project
                && s.summary.has_messages
                && !self.orchestration_worker(id)
                && self.in_sidebar_worktree_scope(id, cx)
        };
        let shown: HashSet<_> = self
            .order
            .iter()
            .copied()
            .filter(|id| !self.sidebar_hidden_sessions.contains(id) && listable(*id))
            .collect();
        let mut ids: Vec<_> = self
            .order
            .iter()
            .copied()
            .filter(|id| {
                let s = self.session_metadata(*id, cx);
                listable(*id)
                    && (!self.sidebar_hidden_sessions.contains(id)
                        || self
                            .started_by
                            .get(id)
                            .is_some_and(|caller| shown.contains(caller)))
                    && self.filter_session(*id, start, cx)
                    && (self.query.is_empty()
                        || if self.workspace.classic_layout {
                            self.classic_session_matches_query(*id, cx)
                        } else {
                            format!("{} {}", s.summary.title, s.summary.provider)
                                .to_lowercase()
                                .contains(&self.query)
                        })
            })
            .collect();
        ids.sort_by_key(|id| {
            std::cmp::Reverse((self.session_metadata(*id, cx).summary.updated_at, *id))
        });
        ids
    }

    /// `Session: Previous/Next` opens the sidebar's neighbour of the active
    /// session; `in_tab` swaps it into the current tab instead.
    pub(super) fn step_session(
        &mut self,
        step: isize,
        in_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu.is_some() {
            return;
        }
        let ids = self.sidebar_navigation_ids(cx);
        let current = self.workspace.active;
        let Some(next) = adjacent(&ids, current.as_ref(), step) else {
            return;
        };
        if Some(next) == current {
            return;
        }
        if in_tab
            && self.workspace.tab_of(next).is_none()
            && let Some(active) = current
        {
            let previous_tab = self.workspace.tab_of(active);
            self.workspace.swap_session(active, next);
            if let Some((from, to)) = previous_tab.zip(self.workspace.tab_of(next)) {
                self.replace_title_tab_key(from, to);
            }
        }
        self.open(next, window, cx);
    }

    /// `Project: Move Back/Forward` moves the current project one place.
    pub(super) fn move_current_project(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            return;
        }
        let path = self.workspace.project.clone();
        if self.workspace.step_project_order(&path, step) {
            self.project_order_changed(&path, cx);
        }
    }

    /// Saves a new project order and keeps the moved project in view.
    pub(super) fn project_order_changed(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(ix) = self.workspace.projects.iter().position(|p| p == path) {
            if ix < self.spaces_start {
                self.spaces_start = ix;
            } else if ix >= self.spaces_start + 5 {
                self.spaces_start = ix - 4;
            }
        }
        self.persist(cx);
        self.invalidate_chrome(cx);
        self.projects_frame.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    /// `Project: Previous/Next` along the project rail.
    pub(super) fn step_project(
        &mut self,
        step: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu.is_some() {
            return;
        }
        let projects = self.workspace.visible_project_order();
        if let Some(next) = adjacent(&projects, Some(&self.workspace.project), step)
            && next != self.workspace.project
        {
            self.select_project(next, window, cx);
        }
    }

    pub(super) fn close_other_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active_title_tab(cx) else {
            return;
        };
        // Pins survive the bulk closes, as in the tab menu.
        let tabs = self
            .strip_tabs(cx)
            .into_iter()
            .filter(|tab| *tab != active)
            .collect::<Vec<_>>();
        self.close_title_tabs(&tabs, Some(active), window, cx);
    }

    pub(super) fn activate_tab(
        &mut self,
        slot: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tabs = self.visible_tabs(cx);
        let index = if slot < 0 {
            tabs.len().saturating_sub(1)
        } else {
            slot as usize
        };
        if let Some(tab) = tabs.get(index) {
            self.open_title_tab(tab, window, cx);
        }
    }
}
