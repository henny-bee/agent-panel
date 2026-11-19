use super::*;

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.keep_visible_idle_agents(cx);
        self.sync_ui_scale(window, cx);
        if self.mono_panes_unsettled(cx) {
            let this = cx.entity();
            window.defer(cx, move |window, cx| {
                this.update(cx, |this, cx| this.settle_mono_panes(window, cx));
            });
        }
        if let Some(id) = self.pending_composer_focus.take()
            && self
                .workspace
                .active
                .map(|pane| self.workspace.session_for_pane(pane))
                == Some(id)
            && let Some(view) = self.sessions.get(&id)
        {
            self.review_focused = false;
            window.focus(&view.read(cx).session_focus(cx), cx);
        }
        if let Some(m) = &self.monos {
            m.read(cx).hide_html(cx);
            m.update(cx, |m, cx| m.bind_window(window, cx));
        }
        self.render_passes += 1;
        if let Some((id, tab)) = self.pending_review.take()
            && self.workspace.tab_of(id).is_some()
        {
            self.focus_pane(id, window, cx);
            self.open_review(tab, window, cx);
        }
        // Native fullscreen transitions resize the window, which redraws it.
        self.fullscreen = window.is_fullscreen();
        self.window_controls = corner_controls_width(window);
        // The corner's tools share the panel's header with client-drawn
        // window controls, so the panel keeps room for both. A Mono's icon
        // tabs need 252px (`corner_room`).
        let tools = if self.window_controls > 0. && self.mono_active(cx) {
            252.
        } else {
            crate::ui::panel::WIDTH_MIN
        };
        self.right_panel_min = tools + self.window_controls;
        #[cfg(target_os = "linux")]
        {
            let client = matches!(
                window.window_decorations(),
                gpui::Decorations::Client { .. }
            );
            if self.client_decorations != client {
                self.client_decorations = client;
                self.invalidate_chrome(cx);
            }
        }
        if cx.reduce_motion()
            && (self.sidebar_slide.is_some()
                || self.right_slide.is_some()
                || self.review_slide.is_some())
        {
            self.sidebar_slide = None;
            self.right_slide = None;
            self.review_slide = None;
            self.sync_right_panel(cx);
        }
        let p = &self.workspace.preferences;
        let system_light = matches!(
            cx.window_appearance(),
            gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight
        );
        let project = &self.workspace.project;
        let animate = !cx.reduce_motion()
            && self
                .theme_project
                .as_ref()
                .is_some_and(|previous| previous != project);
        let project_theme = self
            .workspace
            .project_appearance
            .get(project)
            .and_then(|a| a.theme.as_ref());
        // Repainting bypasses cached views, so every surface follows the blend.
        if configure_project(p, project_theme, system_light, animate) {
            cx.refresh_windows();
        }
        if tweening() {
            window.request_animation_frame();
        }
        if self.theme_project.as_ref() != Some(project) {
            self.theme_project = Some(project.clone());
        }
        let glass = window_glass(p, system_light);
        if self.window_glass != Some(glass) {
            self.window_glass = Some(glass);
            window.set_background_appearance(crate::ui::theme::window_backdrop(glass));
        }
        self.sync_settings(cx);
        self.sidebar_fraction = match &self.sidebar_slide {
            Some(slide) => {
                let (fraction, done) = slide.at(std::time::Instant::now());
                if done {
                    self.sidebar_slide = None;
                } else {
                    window.request_animation_frame();
                }
                fraction
            }
            None => {
                let visible = if self.settings.is_some() {
                    self.settings_nav_visible
                } else {
                    self.workspace.sidebar_visible
                };
                if visible { 1. } else { 0. }
            }
        };
        // The classic layout's settings nav is the project rail, which stays.
        if self.settings.is_some() && self.workspace.classic_layout {
            self.sidebar_fraction = 1.;
        }
        let sidebar_width = self.workspace.sidebar_width * self.sidebar_fraction;
        if !self.window_observed {
            self.window_observed = true;
            self.shortcuts = Some(commands::shortcuts::install(cx.weak_entity(), window, cx));
            self.observations
                .push(cx.on_focus_lost(window, |this, window, cx| {
                    // Cached editors and previews can outlive their rendered pane.
                    // Recover as soon as its focus node leaves the tree, including
                    // close paths without a Window in their original event.
                    let target = window
                        .focus_lost_restore_target(cx)
                        .unwrap_or_else(|| this.focus.clone());
                    window.focus(&target, cx);
                }));
            self.window_active = window.is_window_active();
            if self.window_active {
                self.window_used(cx);
            }
            self.observations
                .push(cx.observe_window_activation(window, |this, window, cx| {
                    this.window_active = window.is_window_active();
                    if this.window_active {
                        this.window_used(cx);
                    }
                    this.refresh_notification_permission();
                    cx.notify();
                }));
            if self.main {
                self.start_native_notifications(cx);
            }
            self.check_github_star(false, cx);
            let shell = cx.entity().downgrade();
            window.on_window_should_close(cx, move |window, cx| {
                shell
                    .update(cx, |shell, cx| {
                        shell.save_window_state(window, cx);
                        #[cfg(all(target_os = "macos", not(test)))]
                        if shell.workspace.preferences.close_to_tray
                            && shell.workspace.preferences.mono_menu_bar_icon
                            && shell
                                .monos
                                .as_ref()
                                .is_some_and(|m| !m.read(cx).roster.monos.is_empty())
                            && !shell.unsaved_files(cx)
                        {
                            crate::ui::traffic_lights::hide(window);
                            return false;
                        }
                        if shell.unsaved_files(cx) {
                            shell.request_quit(window, cx);
                            false
                        } else {
                            true
                        }
                    })
                    .unwrap_or(true)
            });
            self.observations
                .push(cx.observe_window_activation(window, |this, window, cx| {
                    if !window.is_window_active() {
                        this.close_preview(cx);
                    } else {
                        this.resume_git(cx);
                        this.check_github_star(true, cx);
                        if let Some(center) = crate::accounts::get(cx) {
                            center.update(cx, |center, cx| center.app_reactivated(cx));
                        }
                    }
                }));
            self.observations
                .push(cx.observe_window_bounds(window, |this, window, cx| {
                    this.save_window_state(window, cx);
                    this.close_preview(cx);
                }));
        }
        self.viewport_width = window.viewport_size().width.as_f32();
        self.viewport_height = window.viewport_size().height.as_f32();
        self.refit_sidebar_to_room(window, cx);
        self.ensure_right_panel(window, cx);
        let full = self.right_panel_width();
        let right_width = match &self.right_slide {
            Some(slide) => {
                let (width, done) = slide.at(std::time::Instant::now());
                if done {
                    self.right_slide = None;
                    self.sync_right_panel(cx);
                    if self.workspace.right_panel_open
                        && !self.mono_active(cx)
                        && let Some(panel) = self.right_panel.clone()
                    {
                        panel.update(cx, |p, cx| p.finish_opening(window, cx));
                    }
                } else {
                    window.request_animation_frame();
                }
                width
            }
            None => full,
        };
        self.sync_pane_panels(cx);
        self.sync_right_panel(cx);
        let right_shown = self.right_panel_shown(cx);
        self.right_drawn = if right_shown { right_width } else { 0. };
        let poll_git = self.window_active && !self.mono_active(cx);
        self.git.update(cx, |git, cx| {
            git.set_polling(poll_git && git.available(), cx)
        });
        if let Some(panel) = &self.right_panel {
            let classic = self.workspace.boxy_layout();
            let tab_width = self.project_tabs_width(cx);
            panel.update(cx, |p, cx| {
                p.set_top_inset(if classic { TITLE_BAR_HEIGHT } else { 0. }, cx);
                p.set_tab_width(tab_width, cx);
                p.set_show_excluded(self.workspace.preferences.show_excluded_files, cx);
            });
        }
        if let Some(search) = &self.app_search {
            let original = self.workspace.classic_layout.then(|| {
                if self.workspace.project_rail_open {
                    12.
                } else {
                    self.strip_lead(cx) + 6.
                }
            });
            search
                .view
                .update(cx, |search, cx| search.set_original_header(original, cx));
        }
        if let Some(monos) = &self.monos {
            monos.update(cx, |monos, cx| {
                monos.set_original_chrome(self.workspace.classic_layout, cx)
            });
            let details = monos.read(cx).details.clone();
            let inset = if self.workspace.boxy_layout() && !self.workspace.classic_layout {
                TITLE_BAR_HEIGHT
            } else {
                0.
            };
            details.update(cx, |view, cx| {
                view.set_top_inset(inset, cx);
            });
        }
        self.sync_notices(window, cx);
        div()
            .id("monocode-workspace")
            .relative()
            .role(Role::Application)
            .aria_label("MCode")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .capture_key_up(cx.listener(|this, _, _, _| this.archive_shortcut_held = false))
            .on_action(cx.listener(|this, _: &ArchiveSession, window, cx| {
                this.archive_active_session(window, cx)
            }))
            .on_action(
                cx.listener(|this, open: &crate::ui::image_lightbox::Open, window, cx| {
                    let view = cx.new(|cx| {
                        crate::ui::image_lightbox::ImageLightbox::new(open.clone(), window, cx)
                    });
                    let closed = cx.subscribe(
                        &view,
                        |this, _, _: &crate::ui::image_lightbox::Closed, cx| {
                            this.image_lightbox = None;
                            cx.notify();
                        },
                    );
                    this.image_lightbox = Some((view, closed));
                    cx.notify();
                }),
            )
            .flex()
            .flex_col()
            .size_full()
            .bg(shell_fill())
            .text_color(content(1.))
            .font_family(crate::ui::fonts::UI)
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                if this.session_preview.as_ref().is_some_and(|preview| {
                    !preview
                        .surface
                        .get()
                        .is_some_and(|bounds| bounds.contains(&event.position))
                }) {
                    this.close_preview(cx);
                }
            }))
            .on_action(
                cx.listener(|this, event: &crate::ui::files::OpenPath, window, cx| {
                    let cwd = this.git.read(cx).project().to_owned();
                    let (path, location) = crate::ui::files::file_location(&event.0, &cwd);
                    this.open_review(
                        crate::git::review::ReviewTab::File { cwd, path },
                        window,
                        cx,
                    );
                    // Monos render above the session pane tree. Give its file
                    // an independent visible tab using the normal navigation
                    // path, which retains the Mono tab and live conversation.
                    if this.mono_active(cx)
                        && let Some(owner) = this.workspace.active
                        && let Some(pane) = this.independent_file_pane(owner, cx)
                    {
                        this.open_file_pane(pane, window, cx);
                    }
                    if let Some((line, column)) = location
                        && let Some(pane) = this.active_review(cx)
                    {
                        pane.update(cx, |pane, cx| pane.navigate_file(line, column, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::Quit, window, cx| this.request_quit(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &GoToFile, window, cx| {
                    this.open_quick_open(false, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &OpenCommandPalette, window, cx| {
                this.open_quick_open(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.open_settings(crate::settings::Section::General, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenInbox, window, cx| this.open_inbox(window, cx)))
            .on_action(cx.listener(|this, _: &OpenNotes, window, cx| this.open_notes(window, cx)))
            .on_action(cx.listener(|this, _: &CheckForUpdates, _, cx| {
                this.notification_state
                    .updater
                    .update(cx, |updater, cx| updater.check(true, cx));
            }))
            .on_action(|_: &MinimizeWindow, window, _| window.minimize_window())
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.step_ui_scale(1, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.step_ui_scale(-1, cx)))
            .on_action(cx.listener(|this, _: &ResetZoom, _, cx| this.step_ui_scale(0, cx)))
            .on_action(cx.listener(|this, _: &Reload, window, cx| this.request_reload(window, cx)))
            .on_action(cx.listener(|this, _: &SidebarAppearance, window, cx| {
                this.open_settings(crate::settings::Section::Appearance, window, cx)
            }))
            .on_action(|_: &About, window, cx| {
                let detail = format!("Version {}", crate::notifications::updater::version());
                drop(window.prompt(gpui::PromptLevel::Info, "MCode", Some(&detail), &["OK"], cx));
            })
            .on_action(|_: &ZoomWindow, window, _| window.zoom_window())
            .on_action(cx.listener(
                |this, _: &sessions::groups::RenameSidebarItem, window, cx| {
                    this.rename_sidebar_item(window, cx)
                },
            ))
            .on_action(cx.listener(|this, _: &NewCodex, window, cx| {
                // ⌘T in the focused dock opens another terminal there.
                if this.dock_focused(window, cx).is_some() {
                    this.new_terminal(window, cx);
                } else {
                    this.new_chat(cx);
                }
            }))
            .on_action(cx.listener(
                |this, _: &crate::editor::file_editor::ToggleAutosave, _, cx| {
                    this.workspace.preferences.autosave = !this.workspace.preferences.autosave;
                    this.apply_preferences(cx);
                    this.persist(cx);
                    cx.notify();
                },
            ))
            .on_action(cx.listener(|this, _: &NewClaude, _, cx| this.create(Kind::Claude, cx)))
            .on_action(cx.listener(|this, _: &OpenProject, _, cx| this.open_project(cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                // A pinned tab stays put: ⌘W only closes what sits inside
                // it, a dock terminal, a review file or one pane of a split.
                let pinned = this.active_title_tab(cx).filter(|tab| this.tab_pinned(tab));
                if pinned
                    .as_ref()
                    .is_some_and(|tab| !matches!(tab, TitleTab::Session(_)))
                {
                    return;
                }
                if let Some(view) = this.active_app_view.filter(|_| this.settings.is_none()) {
                    if this.workspace.classic_layout {
                        this.close_app_view(window, cx);
                    } else {
                        this.close_title_tabs(&[TitleTab::App(view)], None, window, cx);
                    }
                    return;
                }
                if this.search_page() {
                    this.close_search_tab(window, cx);
                    return;
                }
                if this.app_search.is_some() {
                    this.close_app_search(window, cx);
                    return;
                }
                if let Some(TitleTab::Mono { id, .. }) = this.active_title_tab(cx) {
                    this.close_mono_tab(&id, window, cx);
                    return;
                }
                // ⌘W in the focused dock closes its terminal.
                if let Some(ix) = this.dock_focused(window, cx) {
                    this.close_terminal(ix, window, cx);
                    return;
                }
                // A focused review closes its tab first, as a focused file pane does.
                if this.review_focused
                    && let Some(review) = this.active_review(cx)
                {
                    review.update(cx, |r, cx| r.close_active(cx));
                    return;
                }
                if let Some(id) = this.workspace.active {
                    if pinned.is_some()
                        && !this
                            .workspace
                            .tab_of(id)
                            .is_some_and(|tab| this.workspace.layout(tab).is_split())
                    {
                        return;
                    }
                    // Close Pane (⌘W) in a split; the whole tab otherwise.
                    this.close_pane(id, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| this.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &VisitBack, window, cx| this.visit(-1, window, cx)))
            .on_action(cx.listener(|this, _: &VisitForward, window, cx| this.visit(1, window, cx)))
            .on_action(cx.listener(|this, _: &CloseAllTabs, window, cx| {
                // An empty strip can still have a new-tab request in flight.
                this.navigation_revision += 1;
                // Pins survive the bulk closes, as in the tab menu.
                this.close_title_tabs(&this.strip_tabs(cx), None, window, cx);
                if !this.mono_active(cx) {
                    this.ensure_project_session(window, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &CloseOtherTabs, window, cx| {
                    this.close_other_tabs(window, cx)
                }),
            )
            .on_action(cx.listener(|this, action: &ActivateTab, window, cx| {
                this.activate_tab(action.0, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousSession, window, cx| {
                this.step_session(-1, false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextSession, window, cx| {
                this.step_session(1, false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousSessionInTab, window, cx| {
                this.step_session(-1, true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextSessionInTab, window, cx| {
                this.step_session(1, true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousProject, window, cx| {
                this.step_project(-1, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &NextProject, window, cx| this.step_project(1, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &MoveProjectBack, _, cx| this.move_current_project(-1, cx)),
            )
            .on_action(
                cx.listener(|this, _: &MoveProjectForward, _, cx| this.move_current_project(1, cx)),
            )
            .on_action(cx.listener(|this, _: &SwitchModel, window, cx| {
                if let Some(view) = this.workspace.active.and_then(|id| this.sessions.get(&id)) {
                    view.update(cx, |v, cx| v.toggle_recent_models(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &DismissMenu, window, cx| {
                if this.cancel_project_rail_drag(cx) || this.cancel_space_drag(cx) {
                    return;
                }
                if this.cancel_classic_sidebar_tab(cx) {
                    return;
                }
                if this.remote_project_picker.is_some() {
                    this.close_remote_project(window, cx);
                    return;
                }
                if this.quick_open.is_some() {
                    this.close_quick_open(window, cx);
                    return;
                }
                if this.app_search.is_some() {
                    this.close_app_search(window, cx);
                    return;
                }
                if this.link_dialog.is_some() {
                    this.close_link_dialog(window, cx);
                    return;
                }
                if this.action_dialog.is_some() {
                    this.close_action_dialog(window, cx);
                    return;
                }
                if let Some(settings) = this.settings.clone() {
                    if !settings.update(cx, |s, cx| s.dismiss_transient(window, cx)) {
                        this.close_settings(window, cx);
                    }
                    return;
                }
                if let Some(monos) = this.monos.clone()
                    && monos.update(cx, |m, cx| m.dismiss_settings(window, cx))
                {
                    return;
                }
                if this.app_view_open() {
                    this.close_app_view(window, cx);
                    return;
                }
                if this.session_preview.is_some() {
                    this.close_preview(cx);
                    return;
                }
                if this.peek_open() && this.menu.is_none() {
                    this.set_peek(false, cx);
                    return;
                }
                if this.menu.is_none()
                    && this.right_panel_shown(cx)
                    && !this.mono_active(cx)
                    && let Some(panel) = &this.right_panel
                    && panel.read(cx).tab() == crate::ui::panel::RightTab::Files
                {
                    let explorer = panel.read(cx).explorer.clone();
                    if explorer.update(cx, |explorer, cx| explorer.close_search(window, cx)) {
                        cx.stop_propagation();
                        return;
                    }
                }
                // `Session: Stop` when no menu is open to close.
                if this.menu.is_none()
                    && let Some(view) = this.workspace.active.and_then(|id| this.sessions.get(&id))
                {
                    view.update(cx, |v, cx| v.stop_if_busy(cx));
                }
                this.close_menu(cx);
                this.tab_drag = None;
                this.folder_drag = None;
                this.cancel_pane_drag(cx);
                this.pane_sash = None;
                this.invalidate_chrome(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| {
                this.toggle_primary_sidebar(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleRightPanel, window, cx| {
                if this.workspace.classic_layout {
                    this.toggle_sidebar(window, cx)
                } else {
                    this.toggle_right_panel(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &SearchFiles, window, cx| {
                if this.app_view_open() || this.settings.is_some() || this.mono_active(cx) {
                    return;
                }
                this.close_app_search(window, cx);
                this.ensure_right_panel(window, cx);
                if this.workspace.classic_layout {
                    this.select_classic_sidebar_tab(
                        sidebar::classic::ClassicSidebarTab::Files,
                        window,
                        cx,
                    );
                    let explorer = this.right_panel.as_ref().unwrap().read(cx).explorer.clone();
                    explorer.update(cx, |explorer, cx| explorer.open_search(window, cx));
                    return;
                }
                this.set_right_panel_open(true, window, cx);
                if let Some(panel) = &this.right_panel {
                    let explorer = panel.read(cx).explorer.clone();
                    panel.update(cx, |panel, cx| {
                        panel.select(crate::ui::panel::RightTab::Files, cx)
                    });
                    explorer.update(cx, |explorer, cx| explorer.open_search(window, cx));
                }
            }))
            .on_action(
                cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleTerminal, window, cx| {
                // Title bar terminals open a new tab each time.
                if this.terminal_in_title() {
                    this.new_title_terminal(window, cx)
                } else {
                    this.toggle_terminal(window, cx)
                }
            }))
            .on_action(
                cx.listener(|this, _: &NewTerminal, window, cx| this.new_terminal(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SplitTerminal, window, cx| {
                // Title bar terminals split beside the focused pane; the dock
                // and panel placements have no pane to split, so add a tab.
                if this.terminal_in_title() {
                    this.split_title_terminal(window, cx)
                } else {
                    this.new_terminal(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleLayout, _, cx| this.toggle_layout(cx)))
            .on_action(
                cx.listener(|this, _: &ToggleWorkspaceStyle, _, cx| {
                    this.toggle_workspace_style(cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &SplitRight, _, cx| this.split_session(SplitDir::Right, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SplitDown, _, cx| this.split_session(SplitDir::Down, cx)),
            )
            .on_action(cx.listener(|this, _: &FocusPaneLeft, window, cx| {
                this.focus_neighbor(Edge::Left, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FocusPaneRight, window, cx| {
                this.focus_neighbor(Edge::Right, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FocusPaneUp, window, cx| {
                this.focus_neighbor(Edge::Top, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FocusPaneDown, window, cx| {
                this.focus_neighbor(Edge::Bottom, window, cx)
            }))
            .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
            .on_action(|_: &FocusPrevious, window, cx| window.focus_prev(cx))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if let Some(monos) = &this.monos {
                    monos.update(cx, |m, cx| m.move_rail_drag(event, cx));
                }
                this.move_tab_drag(event, cx);
                this.move_dock_drag(event, cx);
                this.move_pane_drag(event, cx);
                this.move_folder_drag(event, cx);
                this.move_project_rail_drag(event, cx);
                this.move_space_drag(event, window, cx);
                this.move_classic_sidebar_tab(event, cx);
                this.drag_sash(event, cx);
                this.drag_pane_panel(event, cx);
                this.drag_dock(event, cx);
                this.drag_review(event, cx);
                if let Some((start, width)) = this.project_resize {
                    if event.pressed_button == Some(MouseButton::Left) {
                        this.drag_resize(
                            ResizeTarget::Projects,
                            width + (event.position.x - start).as_f32(),
                            cx,
                        );
                    } else {
                        this.release_resizes(cx);
                    }
                }
                if let Some((start, width)) = this.resize {
                    if event.pressed_button == Some(MouseButton::Left) {
                        let width = width + (event.position.x - start).as_f32();
                        this.drag_resize(ResizeTarget::Sidebar, width, cx);
                    } else {
                        this.release_resizes(cx);
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.finish_tab_drag(window, cx);
                    this.finish_dock_drag(cx);
                    this.finish_pane_drag(window, cx);
                    this.finish_folder_drag(cx);
                    this.finish_project_rail_drag(cx);
                    this.finish_space_drag(cx);
                    this.end_classic_sidebar_tab(cx);
                    if let Some(m) = this.monos.clone() {
                        m.update(cx, |m, cx| m.finish_reorder(cx));
                    }
                    this.release_resizes(cx);
                    if std::mem::take(&mut this.review_drag)
                        && let Some(review) = this.active_review(cx)
                    {
                        review.update(cx, |_, cx| {
                            cx.emit(crate::git::review::ReviewEvent::Changed)
                        });
                    }
                }),
            )
            .relative()
            .child(self.right_panel_resize_watch(cx))
            .children(self.layout_body(sidebar_width, right_shown, right_width, full, window, cx))
            .children(self.peek_watch(window, cx))
            .child(self.notices_overlay(cx))
            .children(self.menu.as_ref().map(|open| {
                let point = matches!(open.anchor, Anchor::Point(_));
                let beside = matches!(open.target, MenuTarget::Project { .. });
                // Context menus open at the pointer, as ExplorerMenu
                // does; TabGroupMenu hangs its top-left corner there.
                menu_popover(&open.menu, open.anchor.clone(), cx)
                    .when(
                        self.workspace.classic_layout && open.target == MenuTarget::SessionFilter,
                        |p| p.align(Align::End).gap(2.),
                    )
                    .when(point, |p| p.gap(0.))
                    .when(beside, |p| p.side(Side::Right))
                    .when(open.target == MenuTarget::SidebarOptions, |p| {
                        p.align(Align::Start)
                    })
                    .when(open.target == MenuTarget::Actions, |p| {
                        p.side(Side::Top).align(crate::ui::popover::Align::End)
                    })
                    .when(open.target == MenuTarget::RunningTerminals, |p| {
                        p.side(Side::Top).align(crate::ui::popover::Align::End)
                    })
            }))
            .children(self.drag_ghost(cx))
            .children(self.background_modal(window, cx))
            .children(self.link_modal(window, cx))
            .children(self.action_modal(window, cx))
            .children(self.resource_modal(window, cx))
            .children(self.quick_open_modal(window, cx))
            .children(self.search_modal(window, cx))
            .children(self.remote_project_modal(window, cx))
            .children(self.notice_modals(window, cx))
            .children(self.folder_picker_popover(window, cx))
            .children(crate::accounts::get(cx))
            .children(
                self.settings
                    .clone()
                    .and_then(|view| view.update(cx, |view, cx| view.mobile_overlay(window, cx))),
            )
            .children(
                self.image_lightbox
                    .as_ref()
                    .map(|(view, _)| crate::ui::image_lightbox::overlay(view)),
            )
            .map(|root| {
                #[cfg(target_os = "linux")]
                let caption = self.client_decorations;
                // `resize_handles` stays empty on Windows: GPUI reports server
                // decorations and the native frame does the resizing.
                #[cfg(target_os = "windows")]
                let caption = WINDOWS_CONTROLS;
                #[cfg(any(target_os = "linux", target_os = "windows"))]
                let root = root
                    .when(caption && !self.fullscreen, |root| {
                        root.child(chrome::window_chrome::controls(
                            window,
                            cx.listener(|this, _, window, cx| this.request_quit(window, cx)),
                        ))
                    })
                    .children(chrome::window_chrome::resize_handles(window));
                root
            })
    }
}
