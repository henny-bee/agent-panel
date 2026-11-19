use super::super::*;

impl Shell {
    /// App.tsx's `insetRightPanelShown`: open (or sliding shut), over a
    /// project, and not covered by an app view.
    pub(in crate::app::shell) fn right_panel_shown(&self, cx: &App) -> bool {
        (self.workspace.right_panel_open || self.right_slide.is_some())
            && (!self.workspace.classic_layout || self.mono_active(cx))
            && !self.app_view_open()
            && self.settings.is_none()
            && !self.search_page()
            && (self.mono_active(cx) || self.git.read(cx).available())
            && !self.split_panels(cx)
    }

    pub(in crate::app::shell) fn sync_right_panel(&mut self, cx: &mut Context<Self>) {
        let opening = self.right_slide.as_ref().is_some_and(PaneSlide::opening);
        let shown = !self.mono_active(cx)
            && !self.workspace.classic_layout
            && self.workspace.right_panel_open
            && !self.app_view_open()
            && self.settings.is_none()
            && !self.search_page()
            && !self.split_panels(cx)
            && !opening;
        if let Some(panel) = &self.right_panel {
            let sidebar_tab = (self.workspace.classic_layout
                && self.session_sidebar_available(cx)
                && self.workspace.sidebar_visible
                && self.settings.is_none())
            .then(|| self.classic_sidebar_tab.tool_tab())
            .flatten();
            let terminal_tab = self.terminal_in_panel(cx);
            panel.update(cx, |p, cx| {
                p.set_shown(shown, cx);
                p.set_sidebar_tab(sidebar_tab, cx);
                p.set_terminal_tab(terminal_tab, cx);
            });
        }
    }

    pub(in crate::app::shell) fn select_classic_sidebar_tab(
        &mut self,
        tab: sidebar::classic::ClassicSidebarTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_right_panel(window, cx);
        self.leave_app_views(cx);
        self.close_app_search(window, cx);
        if let Some(monos) = self.monos.clone() {
            monos.update(cx, |monos, cx| monos.close(cx));
        }
        self.classic_sidebar_tab = tab;
        self.workspace
            .classic_sidebar_tabs
            .insert(self.workspace.project.clone(), tab.id().into());
        self.close_preview(cx);
        self.sidebar_auto_hidden = false;
        self.set_sidebar_visible(true, cx);
        self.sync_right_panel(cx);
        self.notify_sidebar(cx);
        self.persist(cx);
        cx.notify();
    }

    pub(in crate::app::shell) fn classic_tool_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(panel) = &self.right_panel else {
            return div().into_any_element();
        };
        let panel = panel.read(cx);
        match self.classic_sidebar_tab {
            sidebar::classic::ClassicSidebarTab::Files => panel
                .explorer
                .clone()
                .cached(StyleRefinement::default().size_full())
                .into_any_element(),
            sidebar::classic::ClassicSidebarTab::Changes => panel
                .changes
                .clone()
                .cached(StyleRefinement::default().size_full())
                .into_any_element(),
            sidebar::classic::ClassicSidebarTab::Sessions => div().into_any_element(),
        }
    }

    pub(in crate::app::shell) fn toggle_right_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.split_panels(cx)
            && let Some(id) = self.workspace.active
        {
            return self.toggle_pane_panel(id, window, cx);
        }
        self.set_right_panel_open(!self.workspace.right_panel_open, window, cx);
    }

    pub(in crate::app::shell) fn set_right_panel_open(
        &mut self,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.app_view_open()
            || (!self.mono_active(cx) && !self.git.read(cx).available())
            || self.workspace.right_panel_open == open
        {
            return;
        }
        let details_focus =
            if let Some(monos) = self.monos.as_ref().filter(|m| m.read(cx).active.is_some()) {
                Some(monos.read(cx).details.focus_handle(cx))
            } else {
                self.right_panel.as_ref().map(|p| p.focus_handle(cx))
            };
        if !open
            && details_focus
                .as_ref()
                .is_some_and(|focus| focus.contains_focused(window, cx))
        {
            self.focus_active(window, cx);
        }
        let full = self.right_panel_width();
        let to = if open { full } else { 0. };
        self.workspace.right_panel_open = open;
        let previous = self.right_slide.take();
        self.right_slide = (!cx.reduce_motion()).then(|| match previous {
            Some(slide) => slide.retarget(to, std::time::Instant::now()),
            None => PaneSlide::new(if open { 0. } else { full }, to, full),
        });
        self.sync_right_panel(cx);
        if open
            && !self.mono_active(cx)
            && let Some(panel) = self.right_panel.clone()
        {
            panel.update(cx, |p, cx| p.focus_pane(window, cx));
        }
        self.fit_sidebar(cx);
        self.persist(cx);
        cx.notify();
    }

    pub(in crate::app::shell) fn ensure_right_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.right_panel.is_some() {
            return;
        }
        let terminal_tab = self.terminal_in_panel(cx);
        let tab = self.workspace.right_panel_tab;
        let panel = self.new_project_panel(tab, None, self.git.clone(), window, cx);
        panel.update(cx, |panel, cx| panel.set_terminal_tab(terminal_tab, cx));
        self.right_panel = Some(panel);
        self.sync_git(cx);
    }

    /// The project tools, for the window (`pane` None) or one split pane.
    /// Choices made in it are saved for whichever it serves.
    pub(in crate::app::shell) fn new_project_panel(
        &mut self,
        tab: crate::ui::panel::RightTab,
        pane: Option<i64>,
        git: Entity<crate::runtime::git_status::ProjectGit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<RightPanel> {
        let panel_git = git.clone();
        let (order, view) = (
            self.workspace.right_panel_tabs.clone(),
            self.workspace.changes_view,
        );
        let panel = cx.new(|cx| RightPanel::new(git, tab, order, window, cx));
        let explorer = panel.read(cx).explorer.clone();
        let claude = match pane {
            Some(id) => self.workspace.pane_panels.get(&id).and_then(|p| p.claude),
            None => self.workspace.right_panel_claude,
        };
        explorer.update(cx, |e, cx| e.set_claude_tab(claude, cx));
        self.observations.push(cx.subscribe(
            &explorer,
            move |this, _, event: &crate::project::explorer::ClaudeTabChanged, cx| {
                match pane {
                    Some(id) => this.workspace.pane_panels.entry(id).or_default().claude = event.0,
                    None => this.workspace.right_panel_claude = event.0,
                }
                this.persist(cx);
            },
        ));
        self.observations.push(cx.subscribe(
            &explorer,
            move |this, _, event: &crate::project::explorer::FolderChosen, cx| {
                let Some(id) = pane.or(this.workspace.active) else {
                    return;
                };
                match &event.0 {
                    Some(folder) => this.workspace.panel_folders.insert(id, folder.clone()),
                    None => this.workspace.panel_folders.remove(&id),
                };
                if pane.is_none() {
                    this.sync_git(cx);
                }
                this.persist(cx);
                cx.notify();
            },
        ));
        self.observations.push(cx.subscribe(
            &explorer,
            |this, _, event: &crate::project::explorer::FileActionError, cx| {
                this.notification_state
                    .file_error
                    .update(cx, |notice, cx| notice.set_message(event.0.clone(), cx));
                cx.notify();
            },
        ));
        self.observations.push(cx.subscribe_in(
            &explorer,
            window,
            move |this, _, event: &crate::project::explorer::OpenFile, window, cx| {
                this.open_review(
                    crate::git::review::ReviewTab::File {
                        cwd: panel_git.read(cx).project().into(),
                        path: event.0.clone(),
                    },
                    window,
                    cx,
                );
            },
        ));
        self.observations.push(cx.subscribe_in(
            &explorer,
            window,
            |this, _, event: &crate::project::explorer::OpenSearchMatch, window, cx| {
                this.open_review_with_preview(
                    crate::git::review::ReviewTab::File {
                        cwd: event.cwd.clone(),
                        path: event.matched.path.clone(),
                    },
                    Some(event.pin),
                    window,
                    cx,
                );
                if let Some(pane) = this.active_review(cx) {
                    pane.update(cx, |pane, cx| {
                        pane.navigate_file(
                            event.matched.line as usize,
                            event.matched.column as usize,
                            cx,
                        )
                    });
                }
            },
        ));
        let changes = panel.read(cx).changes.clone();
        changes.update(cx, |c, cx| c.set_view(view, cx));
        let code = self.workspace.graph_code;
        let graph = panel.read(cx).graph.clone();
        graph.update(cx, |g, cx| g.set_code(code, cx));
        self.observations.push(
            cx.subscribe(&panel, move |this, _, event: &PanelEvent, cx| {
                match event {
                    PanelEvent::Tab(tab) => match pane {
                        Some(id) => this.workspace.pane_panels.entry(id).or_default().tab = *tab,
                        None => {
                            this.workspace.right_panel_tab = *tab;
                            if *tab == crate::ui::panel::RightTab::Terminal {
                                this.ensure_panel_terminal(cx);
                            }
                        }
                    },
                    PanelEvent::Reordered(order) => this.workspace.right_panel_tabs = order.clone(),
                }
                this.persist(cx);
            }),
        );
        self.observations.push(cx.subscribe_in(
            &changes,
            window,
            |this, _, event: &crate::git::changes::ChangesEvent, window, cx| match event {
                crate::git::changes::ChangesEvent::View(view) => {
                    this.workspace.changes_view = *view;
                    this.persist(cx);
                }
                crate::git::changes::ChangesEvent::OpenDiff { cwd, path, staged } => {
                    this.open_review(
                        crate::git::review::ReviewTab::Changes {
                            cwd: cwd.clone(),
                            path: path.clone(),
                            staged: Some(*staged),
                        },
                        window,
                        cx,
                    );
                }
            },
        ));
        let histories = {
            let panel = panel.read(cx);
            [panel.history.clone(), panel.graph.clone()]
        };
        for history in histories {
            self.observations.push(cx.subscribe_in(
                &history,
                window,
                |this, _, event: &crate::git::history::HistoryEvent, window, cx| {
                    let (cwd, sha, short_sha, subject) = match event {
                        crate::git::history::HistoryEvent::Code(code) => {
                            this.workspace.graph_code = *code;
                            this.persist(cx);
                            return;
                        }
                        crate::git::history::HistoryEvent::OpenCommit {
                            cwd,
                            sha,
                            short_sha,
                            subject,
                        } => (cwd, sha, short_sha, subject),
                    };
                    this.open_review(
                        crate::git::review::ReviewTab::Commit {
                            cwd: cwd.clone(),
                            sha: sha.clone(),
                            short_sha: short_sha.clone(),
                            subject: subject.clone(),
                        },
                        window,
                        cx,
                    );
                },
            ));
        }
        panel
    }

    /// Part of the card: the project tools beside the workspace, behind a
    /// hairline. Its contents keep their full width while it slides.
    pub(in crate::app::shell) fn right_panel_frame(
        &mut self,
        panel: Entity<RightPanel>,
        width: f32,
        full: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let content =
            if let Some(monos) = self.monos.as_ref().filter(|m| m.read(cx).active.is_some()) {
                let manager = monos.read(cx);
                let details = manager.details.clone();
                let html = manager.native_detail_content(cx);
                // Native artifact views must update their frames and clipping
                // on every paint; replaying cached GPU quads skips that work.
                if html {
                    details.into_any_element()
                } else {
                    details
                        .cached(StyleRefinement::default().size_full())
                        .into_any_element()
                }
            } else if self.terminal_in_panel(cx)
                && panel.read(cx).tab() == crate::ui::panel::RightTab::Terminal
            {
                self.panel_terminal(window, cx)
            } else {
                panel
                    .cached(StyleRefinement::default().size_full())
                    .into_any_element()
            };
        div()
            .id("right-panel-frame")
            .debug_selector(|| "right-panel-frame".into())
            .relative()
            .h_full()
            .flex_shrink_0()
            .w(px(width))
            .border_l_1()
            .border_color(crate::ui::panel::stroke())
            // The full-width contents travel with the leading edge instead of
            // reflowing into the growing clip on every animation frame.
            .child(
                div().relative().size_full().overflow_hidden().child(
                    div()
                        .id("right-panel-content")
                        .debug_selector(|| "right-panel-content".into())
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(px(full - 1.))
                        .child(content),
                ),
            )
            .child(
                self.right_panel_resize_handle("right-panel-resize", cx)
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(-3.5))
                    .child(resize_border(
                        true,
                        self.right_resize.is_some() || self.right_resize_hovered,
                    )),
            )
            .into_any_element()
    }

    pub(in crate::app::shell) fn right_panel_resize_handle(
        &self,
        id: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .group("resize-handle")
            .role(Role::Splitter)
            .aria_label("Resize right sidebar")
            .w(px(6.))
            .cursor(CursorStyle::ResizeLeftRight)
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if this.right_resize_hovered != *hovered {
                    this.right_resize_hovered = *hovered;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.right_resize = Some((event.position.x, this.right_panel_width()));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                if event.click_count() == 2 {
                    this.right_resize = None;
                    this.workspace.right_panel_width = crate::ui::panel::WIDTH_DEFAULT;
                    this.persist(cx);
                    cx.notify();
                }
            }))
    }

    /// Mono detail layers occlude the shell's ordinary mouse handlers. An
    /// active resize must keep receiving moves and release over those layers.
    pub(in crate::app::shell) fn right_panel_resize_watch(&self, cx: &Context<Self>) -> AnyElement {
        let shell = cx.entity().downgrade();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let moves = shell.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = moves.update(cx, |this, cx| {
                            if this.drag_right_panel(event, cx) {
                                cx.stop_propagation();
                            }
                        });
                    }
                });
                let release = shell.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        let _ = release.update(cx, |this, cx| {
                            if this.right_resize.is_some() {
                                this.release_resizes(cx);
                            }
                        });
                    }
                });
            },
        )
        .absolute()
        .size_0()
        .into_any_element()
    }

    pub(in crate::app::shell) fn drag_right_panel(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((start, width)) = self.right_resize else {
            return false;
        };
        if event.pressed_button == Some(MouseButton::Left) {
            let width = width - (event.position.x - start).as_f32();
            self.drag_resize(ResizeTarget::RightPanel, width, cx);
        } else {
            self.release_resizes(cx);
        }
        true
    }

    /// Client-decorated Linux: the bar ends left of the window controls.
    /// The panel's width as drawn: the user's width, widened while Windows'
    /// controls need room beside the corner's tools. The stored width stays
    /// as the user set it, so closing a Mono gives it back.
    pub(crate) fn right_panel_width(&self) -> f32 {
        self.workspace.right_panel_width.max(self.right_panel_min)
    }

    /// TitleBar.tsx's right panel toggle, `pl-1.5 pr-3` clear of the edge.
    pub(in crate::app::shell) fn panel_toggle(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Split panes carry these controls in their own headers.
        if self.split_panels(cx) {
            return titlebar_drag_area()
                .w(px(self.toggle_room(cx)))
                .flex_shrink_0()
                .into_any_element();
        }
        let mono = self.mono_active(cx);
        if self.workspace.classic_layout && !mono {
            return div().into_any_element();
        }
        if self.workspace.boxy_layout() && !mono {
            return self.classic_corner(window, cx);
        }
        let toggle = mono || self.git.read(cx).available();
        let panel_off = self.app_view_open();
        let dot = !mono && !panel_off && self.git.read(cx).has_changes();
        let panel_active = self.workspace.right_panel_open && !panel_off;
        let tabs = if mono && panel_active {
            self.monos.clone().map(|m| {
                m.update(cx, |m, cx| {
                    m.overview_controls(
                        self.right_panel_width() >= 312. + self.window_controls,
                        window,
                        cx,
                    )
                })
            })
        } else {
            self.right_panel
                .as_ref()
                .filter(|_| panel_active && !mono)
                .map(|panel| {
                    panel
                        .read(cx)
                        .tabs
                        .clone()
                        .cached(
                            StyleRefinement::default()
                                .w(px(self.project_tabs_width(cx)))
                                .h(px(TITLE_BAR_HEIGHT)),
                        )
                        .into_any_element()
                })
        };
        let show_tabs = tabs.is_some();
        div()
            .relative()
            .w(px(self.toggle_room(cx)))
            .h(px(TITLE_BAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .map(|d| {
                if !toggle {
                    return d
                        .pl_1p5()
                        .pr(px(12. + self.window_controls))
                        .child(self.file_search_button(cx));
                }
                let ink = if panel_off {
                    content(0.25)
                } else if panel_active && !self.workspace.boxy_layout() {
                    content(1.)
                } else {
                    content(0.5)
                };
                d.child(
                    div()
                        .h_full()
                        .w_full()
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .pl(px(if show_tabs && !self.workspace.boxy_layout() {
                            CORNER_TABS_LEAD
                        } else {
                            6.
                        }))
                        .pr(px(12. + self.window_controls))
                        .when_some(tabs, |d, tabs| {
                            d.child(tabs)
                                .when(!mono, |d| d.child(titlebar_drag_area().flex_1().min_w_0()))
                        })
                        .children(self.project_diff_stat(cx))
                        .child(self.file_search_button(cx).mx(px(CORNER_CONTROL_GAP)))
                        .child(
                            div()
                                .id("toggle-right-panel")
                                .debug_selector(|| "toggle-right-panel".into())
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .role(Role::Button)
                                .aria_label(if dot {
                                    TOGGLE_RIGHT_PANEL_DOT_LABEL
                                } else {
                                    TOGGLE_RIGHT_PANEL_LABEL
                                })
                                .aria_toggled(if panel_active {
                                    gpui::Toggled::True
                                } else {
                                    gpui::Toggled::False
                                })
                                .group("toggle-right-panel")
                                .cursor_pointer()
                                .size(px(24.))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_md()
                                .when(!panel_off, |d| d.hover(|s| s.bg(content(0.1))))
                                .child(
                                    div()
                                        .relative()
                                        .size(px(14.))
                                        .child(icon("sidebar-right", 14.).text_color(ink).when(
                                            !panel_off,
                                            |i| {
                                                i.group_hover("toggle-right-panel", |s| {
                                                    s.text_color(content(1.))
                                                })
                                            },
                                        ))
                                        .when(dot, |d| {
                                            d.child(
                                                div()
                                                    .absolute()
                                                    .top(px(-2.))
                                                    .right(px(-2.))
                                                    .size(px(6.))
                                                    .rounded_full()
                                                    .bg(accent_color()),
                                            )
                                        }),
                                )
                                .when(!panel_off, |d| {
                                    d.on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_right_panel(window, cx)
                                    }))
                                }),
                        ),
                )
            })
            .when(show_tabs && !self.workspace.boxy_layout(), |d| {
                // The card has 6px of outer padding and a 1px right border.
                // Keep the rule on its divider even during the opening slide.
                let offset = self.toggle_room(cx) - (self.right_drawn + 7.);
                d.child(
                    self.right_panel_resize_handle("titlebar-right-panel-resize", cx)
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(offset - 2.5))
                        .child(
                            div()
                                .id("project-tabs-separator")
                                .debug_selector(|| "project-tabs-separator".into())
                                .absolute()
                                .left(px(2.5))
                                .top(px((TITLE_BAR_HEIGHT - 16.) / 2.))
                                .w(px(1.))
                                .h(px(16.))
                                .bg(crate::ui::panel::stroke())
                                .when(
                                    self.right_resize.is_some() || self.right_resize_hovered,
                                    |d| d.bg(content(0.16)),
                                )
                                .group_hover("resize-handle", |s| s.bg(content(0.16))),
                        ),
                )
            })
            .into_any()
    }

    pub(in crate::app::shell) fn project_diff_stat(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let width = self.diff_stat_room(cx);
        // With no Changes tab, the stat marks the open Changes pane.
        let selected = self.workspace.right_panel_open
            && !self.app_view_open()
            && self
                .right_panel
                .as_ref()
                .is_some_and(|p| p.read(cx).tab() == crate::ui::panel::RightTab::Changes);
        (width > 0.).then(|| {
            crate::ui::panel::diff_stat(self.git.read(cx).stats.as_ref(), width)
                .when(selected, |d| d.bg(content(0.1)))
                .when(!self.app_view_open(), |d| {
                    d.cursor_pointer()
                        .hover(|s| s.bg(content(0.1)))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.ensure_right_panel(window, cx);
                            let panel = this.right_panel.clone().unwrap();
                            let previous = panel.read(cx).tab();
                            let changes = crate::ui::panel::RightTab::Changes;
                            if previous != changes {
                                this.corner_reveal =
                                    Some(crate::project::tabs::TabReveal::new(changes, previous));
                            }
                            panel.update(cx, |panel, cx| panel.select(changes, cx));
                            this.set_right_panel_open(true, window, cx);
                            panel.update(cx, |panel, cx| panel.focus_pane(window, cx));
                            this.invalidate_chrome(cx);
                            cx.notify();
                        }))
                })
                .into_any_element()
        })
    }

    pub(in crate::app::shell) fn file_search_button(
        &self,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let label = if cfg!(target_os = "macos") {
            "Search files (⌘P)"
        } else {
            "Search files (Ctrl+P)"
        };
        icon_button("titlebar-file-search", label, "search")
            .debug_selector(|| "titlebar-file-search".into())
            .flex_shrink_0()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.open_quick_open(false, window, cx)))
    }

    pub(in crate::app::shell) fn diff_stat_room(&self, cx: &App) -> f32 {
        if self.mono_active(cx) || !self.git.read(cx).available() {
            return 0.;
        }
        let natural = crate::ui::panel::diff_stat_width(self.git.read(cx).stats.as_ref());
        if !self.workspace.right_panel_open || self.app_view_open() {
            return natural;
        }
        let controls = crate::ui::panel::tab_icons_width(self.right_tab_count(cx))
            + 6.
            + CORNER_BUTTONS_WIDTH
            + 12.
            + self.window_controls;
        natural.min((self.right_panel_width() - controls).max(48.))
    }

    pub(in crate::app::shell) fn project_tabs_width(&self, cx: &App) -> f32 {
        let padding = if self.workspace.boxy_layout() {
            24.
        } else {
            12.
        };
        (self.right_panel_width()
            - self.diff_stat_room(cx)
            - CORNER_BUTTONS_WIDTH
            - padding
            - self.window_controls)
            .clamp(
                crate::ui::panel::tab_icons_width(self.right_tab_count(cx)),
                crate::app::shell::right_panel::tabs_width(self.right_tab_count(cx)),
            )
    }

    /// Explorer and History, and Terminal when it lives in the panel. Changes
    /// opens from the diff stat instead.
    pub(in crate::app::shell) fn right_tab_count(&self, cx: &App) -> usize {
        2 + usize::from(self.terminal_in_panel(cx))
    }

    /// The toggle's room at the end of the strip: `pl-1.5`, the button, `pr-3`.
    pub(in crate::app::shell) fn toggle_room(&self, cx: &App) -> f32 {
        if self.workspace.boxy_layout() {
            return self.corner_room(cx);
        }
        // Client-drawn window controls hold the corner's right end.
        let controls = self.window_controls;
        if self.split_panels(cx) {
            return 12. + controls;
        }
        if self.mono_active(cx) || self.git.read(cx).available() {
            if self.workspace.right_panel_open && !self.app_view_open() {
                if self.mono_active(cx) {
                    let minimum = if self.right_panel_width() >= 312. + controls {
                        312.
                    } else {
                        252.
                    };
                    return (self.right_drawn + 7.).max(minimum + controls);
                }
                // Follow the rounded panel's left edge, including the card's
                // right inset, while keeping room for all controls when narrow.
                (self.right_drawn + 7.).max(
                    CORNER_TABS_LEAD
                        + self.project_tabs_width(cx)
                        + self.diff_stat_room(cx)
                        + CORNER_BUTTONS_WIDTH
                        + 12.
                        + controls,
                )
            } else {
                6. + self.diff_stat_room(cx) + CORNER_BUTTONS_WIDTH + 12. + controls
            }
        } else {
            6. + 24. + 12. + controls
        }
    }
}
