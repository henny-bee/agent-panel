use super::super::*;

impl Shell {
    /// The title bar's navigation (`data-inset-title-nav`): as wide as the
    /// open sidebar, with history at its far edge; with the sidebar closed,
    /// just the traffic lights' gap and its three buttons.
    pub(in crate::app::shell) fn traffic_lights(&self) -> f32 {
        if self.fullscreen {
            NO_TRAFFIC_LIGHTS
        } else {
            TRAFFIC_LIGHTS
        }
    }

    pub(in crate::app::shell) fn navigation_width(&self) -> f32 {
        let closed = self.traffic_lights() + 3. * 24.;
        if self.workspace.classic_layout {
            return if self.workspace.project_rail_open || self.settings.is_some() {
                self.project_rail_drawn_width().max(closed + 6.)
            } else {
                closed
            };
        }
        closed + (self.workspace.sidebar_width.max(closed + 6.) - closed) * self.sidebar_fraction
    }

    /// Classic: the strip's room before its tabs, wherever the navigation
    /// overhangs the sidebar.
    pub(in crate::app::shell) fn strip_lead(&self, cx: &App) -> f32 {
        (self.navigation_width() - self.left_sidebar_width(cx)).max(0.)
    }

    /// The traffic lights, sidebar toggle and visit arrows, spanning the
    /// open sidebar so the arrows sit at its edge.
    pub(in crate::app::shell) fn navigation(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .w(px(self.navigation_width()))
            .h(px(TITLE_BAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            // `pr-1.5` only beside the open sidebar; closed, the tab
            // strip's own inset is gap enough after the arrows.
            .pr(px(if self.workspace.classic_layout {
                if self.workspace.project_rail_open || self.settings.is_some() {
                    6.
                } else {
                    0.
                }
            } else {
                6. * self.sidebar_fraction
            }))
            .child(
                // Fixed, so the toggle holds still as the sidebar opens
                // and closes.
                titlebar_drag_area()
                    .w(px(self.traffic_lights()))
                    .flex_shrink_0(),
            )
            .child(self.sidebar_toggle(cx))
            .child(titlebar_drag_area().flex_1())
            .child(
                icon_button_enabled(
                    "visit-back",
                    BACK_LABEL,
                    "chevron-left",
                    self.settings.is_some()
                        || (self.workspace.classic_layout && self.mono_active(cx))
                        || !self.history.back.is_empty(),
                )
                .debug_selector(|| "visit-back".into())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.workspace.classic_layout {
                        this.classic_rail_back(window, cx)
                    } else {
                        this.visit(-1, window, cx)
                    }
                })),
            )
            .child(
                icon_button_enabled(
                    "visit-forward",
                    FORWARD_LABEL,
                    "chevron-right",
                    !self.history.forward.is_empty(),
                )
                .debug_selector(|| "visit-forward".into())
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| this.visit(1, window, cx))),
            )
    }

    /// The classic layout's settings fill the project rail, so its toggle
    /// steps aside. Its slot stays, so nothing beside it moves.
    pub(in crate::app::shell) fn sidebar_toggle(
        &self,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        if self.settings.is_some() && self.workspace.classic_layout {
            return div()
                .id("toggle-sidebar")
                .debug_selector(|| "toggle-sidebar".into())
                .flex_shrink_0()
                .size(px(24.))
                .child(self.peek_toggle_bounds.probe());
        }
        icon_button("toggle-sidebar", TOGGLE_SIDEBAR_LABEL, "sidebar")
            .debug_selector(|| "toggle-sidebar".into())
            .child(self.peek_toggle_bounds.probe())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // Hovering the toggle peeks a collapsed sidebar out.
            .on_hover(cx.listener(|this, hovered, _, cx| {
                if !this.workspace.classic_layout {
                    this.hover_peek_toggle(*hovered, cx)
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_primary_sidebar(window, cx)))
    }

    pub(in crate::app::shell) fn titlebar_controls_space(
        &self,
        bar: AnyElement,
        window: &Window,
    ) -> AnyElement {
        let room = client_controls_width(window) - corner_controls_width(window);
        if room == 0. {
            return bar;
        }
        div()
            .w_full()
            .h(px(TITLE_BAR_HEIGHT))
            .pr(px(room))
            .child(bar)
            .into_any_element()
    }

    pub(in crate::app::shell) fn titlebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let bar = self.titlebar_content(window, cx);
        self.titlebar_controls_space(bar, window)
    }

    pub(in crate::app::shell) fn titlebar_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Same hierarchy as TitleBar.tsx's inset layout: window navigation and
        // session tabs share the outer shell, above the rounded workspace card.
        // Windows' controls sit in the settings header's padding, so its
        // bottom rule still runs beneath them. Session bars leave them to the
        // corner (`toggle_room`, `corner_room`).
        let controls = px(self.window_controls);
        if let Some(settings) = self.settings.clone() {
            let classic = self.workspace.boxy_layout();
            let lead = if classic {
                self.strip_lead(cx)
            } else {
                self.navigation_width() + 6.
            };
            let search = !self.workspace.classic_layout && !self.settings_nav_visible;
            let header = settings.update(cx, |v, cx| v.header(!classic, search, cx));
            return div()
                .h(px(TITLE_BAR_HEIGHT))
                .flex_shrink_0()
                .flex()
                .items_center()
                .pr(controls)
                .when(classic, |d| d.border_b_1().border_color(content(0.1)))
                .child(if classic {
                    titlebar_drag_area()
                        .w(px(lead))
                        .flex_shrink_0()
                        .into_any_element()
                } else {
                    self.navigation(cx).into_any_element()
                })
                .child(header)
                .into_any_element();
        }
        let tabs_width = px(self.tabs_width(window, cx));
        let classic = self.workspace.boxy_layout();
        div()
            .h(px(TITLE_BAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .when(self.workspace.classic_layout, |d| d.items_start())
            // Classic: the strip heads the workspace column, above a rule.
            .when(classic, |d| {
                d.border_b_1().border_color(crate::ui::panel::stroke())
            })
            .child(if classic && !self.classic_titlebar_spans_window(cx) {
                // The navigation floats over the sidebar's header
                // (classic_body); the strip keeps only the part it uncovers.
                titlebar_drag_area()
                    .w(px(self.strip_lead(cx)))
                    .flex_shrink_0()
                    .into_any_element()
            } else {
                self.navigation(cx).into_any_element()
            })
            .child(
                self.tabs_frame.clone().cached(
                    StyleRefinement::default()
                        .w(tabs_width)
                        .h(px(TITLE_BAR_HEIGHT)),
                ),
            )
            .child(if classic {
                // The toggle holds the window's corner (classic_body); the
                // strip keeps only the part an open panel leaves uncovered.
                titlebar_drag_area()
                    .w(px(self.strip_trail(cx)))
                    .flex_shrink_0()
                    .into_any_element()
            } else {
                self.panel_toggle(window, cx)
            })
            .into_any()
    }

    /// Classic: the strip's room after its tabs, wherever the toggle
    /// overhangs an open right panel.
    pub(in crate::app::shell) fn strip_trail(&self, cx: &App) -> f32 {
        (self.toggle_room(cx) - self.right_drawn).max(0.)
    }

    /// App.tsx's toTitleTab with TitleBar's tabCopy: a split tab, or a chat
    /// with a file beside it, heads with the focused pane and names the rest
    /// on a second line.
    pub(in crate::app::shell) fn tab_copy(&self, tab: i64, cx: &App) -> TabCopy {
        let focus = self.workspace.tab_focus(tab);
        let review = |id: &i64| {
            self.workspace
                .reviews
                .get(id)
                .filter(|r| !r.tabs.is_empty())
        };
        let mut leaves = self.workspace.layout(tab).leaf_ids();
        // Focused first, as harnesses and files are ordered.
        if let Some(ix) = leaves.iter().position(|id| *id == focus) {
            let id = leaves.remove(ix);
            leaves.insert(0, id);
        }
        // A chat closed beside its file leaves only the file pane.
        let sessions: Vec<i64> = leaves
            .iter()
            .copied()
            .filter(|id| !review(id).is_some_and(|r| r.chat_hidden))
            .filter(|id| self.session_summaries.contains_key(id))
            .collect();
        let mut files = Vec::new();
        for id in &leaves {
            if let Some(r) = review(id) {
                let mut labels = r.tabs.iter().map(|t| t.label());
                let active = r.tabs.get(r.active).map(|t| t.label());
                for label in active.into_iter().chain(&mut labels) {
                    if !files.contains(&label) {
                        files.push(label);
                    }
                }
            }
        }
        let file_panes = leaves.iter().filter(|id| review(id).is_some()).count();
        let mono_panes = leaves
            .iter()
            .filter(|id| {
                self.workspace.mono_panes.contains_key(id)
                    || self.workspace.terminal_panes.contains_key(id)
            })
            .count();
        let multi_pane = sessions.len() + file_panes + mono_panes > 1;
        let session_focused = sessions.first() == Some(&focus);
        let file_focused =
            !session_focused || (self.tab_review_focused(tab, cx) && review(&focus).is_some());
        let title = |id: &i64| {
            let title = self.session_metadata(*id, cx).display_title();
            if title == "New session" {
                String::new()
            } else {
                title
            }
        };
        let conversation = sessions.first().map(title).unwrap_or_default();
        let file = files.first().cloned().unwrap_or_default();
        let untitled = "New session".to_owned();
        let mono = self
            .mono_pane_title(focus, cx)
            .or_else(|| self.terminal_pane_title(focus, cx));
        let headline = if let Some(mono) = mono {
            mono
        } else if multi_pane && file_focused && !file.is_empty() {
            file
        } else {
            [conversation, file]
                .into_iter()
                .find(|t| !t.is_empty())
                .unwrap_or_else(|| untitled.clone())
        };
        // Layout order, a chat before the file beside it, so moving focus
        // right or down rolls the headline up.
        let mut panes = Vec::new();
        for id in self.workspace.layout(tab).leaf_ids() {
            if let (Some(title), Some(mark)) = (
                self.mono_pane_title(id, cx)
                    .or_else(|| self.terminal_pane_title(id, cx)),
                self.mono_pane_mark(id, cx).or_else(|| {
                    self.workspace
                        .terminal_panes
                        .contains_key(&id)
                        .then_some(PaneMark::Terminal)
                }),
            ) {
                panes.push(TabPane {
                    key: (id, false),
                    title,
                    mark,
                });
            }
            if sessions.contains(&id) {
                let session = self.session_metadata(id, cx);
                let lit = session.responding()
                    || session.needs_approval()
                    || self.unseen_finished.contains(&id);
                let title = title(&id);
                panes.push(TabPane {
                    key: (id, false),
                    title: if title.is_empty() {
                        untitled.clone()
                    } else {
                        title
                    },
                    mark: PaneMark::Session(session.summary.provider.clone(), lit.then_some(id)),
                });
            }
            if let Some(r) = review(&id)
                && let Some(t) = r.tabs.get(r.active)
            {
                panes.push(TabPane {
                    key: (id, true),
                    title: t.label(),
                    mark: PaneMark::File(
                        matches!(t, crate::git::review::ReviewTab::File { .. }).then(|| t.label()),
                    ),
                });
            }
        }
        let focused = (focus, file_focused && review(&focus).is_some());
        let focus = panes.iter().position(|p| p.key == focused).unwrap_or(0);
        TabCopy {
            headline,
            panes,
            focus,
        }
    }
}
