//! Sidebar surfaces of the shell.

pub(crate) mod classic;
pub(crate) mod classic_surfaces;
pub(crate) mod folders;
pub(crate) mod list;
pub(crate) mod peek;
pub(crate) mod pinned_tabs;
pub(crate) mod project_rail;
pub(crate) mod rails;
mod session_rows;
pub(crate) mod space_reorder;
use super::*;

impl Shell {
    /// The narrowest chat column the active tab would have, with or without
    /// the sidebar: each pane's share of the card, less its review.
    pub(super) fn narrowest_chat(&self, sidebar: bool, cx: &App) -> f32 {
        // The card's inset and border.
        let mut main = self.viewport_width - 10. - self.project_rail_drawn_width();
        if sidebar && self.session_sidebar_available(cx) {
            main -= self.workspace.sidebar_width;
        }
        if self.right_panel_shown(cx) {
            main -= self.right_panel_width();
        }
        if self.mono_active(cx) {
            return main;
        }
        if let Some(dock) = self.workspace.docks.get(&self.workspace.project)
            && dock.open
            && !dock.side.vertical()
        {
            main -= dock.size;
        }
        let Some(tab) = self
            .workspace
            .active_tab()
            .filter(|_| !self.app_view_open())
        else {
            return main;
        };
        self.workspace
            .layout(tab)
            .leaves()
            .iter()
            .map(|leaf| {
                let width = leaf.rect.w * main;
                let review = self
                    .reviews
                    .get(&leaf.id)
                    .map(|r| r.read(cx).state.clone())
                    .or_else(|| self.workspace.reviews.get(&leaf.id).cloned())
                    .filter(|r| !r.tabs.is_empty());
                match review {
                    Some(r) if r.chat_hidden => f32::INFINITY,
                    Some(r) => width * (1. - r.ratio),
                    None => width,
                }
            })
            .fold(f32::INFINITY, f32::min)
    }

    /// A panel or pane that would squeeze a chat below a readable width
    /// collapses the sidebar as it opens. A sidebar collapsed that way comes
    /// back once there is room again; one the user hid stays hidden.
    pub(super) fn fit_sidebar(&mut self, cx: &mut Context<Self>) {
        if !self.session_sidebar_available(cx) {
            return;
        }
        if self.workspace.sidebar_visible {
            if self.narrowest_chat(true, cx) < MIN_CHAT_WIDTH {
                self.set_sidebar_visible(false, cx);
                self.sidebar_auto_hidden = true;
            }
        } else if self.sidebar_auto_hidden && self.narrowest_chat(true, cx) >= MIN_CHAT_WIDTH {
            self.set_sidebar_visible(true, cx);
            self.sidebar_auto_hidden = false;
        }
    }

    /// Refit the sidebar whenever the room around the chats changes, however
    /// it changed: a window resize, a tab or project switch, a review or pane
    /// closed from inside, a panel's slide settling. Room is measured without
    /// the sidebar so showing or hiding it by hand never counts as a change,
    /// and drags refit once they let go.
    pub(super) fn refit_sidebar_to_room(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.resize.is_some()
            || self.right_resize.is_some()
            || self.review_drag
            || self.pane_sash.is_some()
            || self.dock_resize.is_some()
            || self.project_resize.is_some()
        {
            return;
        }
        let room = (
            self.narrowest_chat(false, cx).round() as i32,
            self.session_sidebar_available(cx),
        );
        if self.sidebar_fitted_room.replace(room) == Some(room) {
            return;
        }
        let this = cx.entity();
        window.defer(cx, move |_, cx| {
            this.update(cx, |this, cx| {
                let visible = this.workspace.sidebar_visible;
                this.fit_sidebar(cx);
                if visible != this.workspace.sidebar_visible {
                    this.persist(cx);
                    cx.notify();
                }
            });
        });
    }

    pub(super) fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_preview(cx);
        if self.workspace.sidebar_visible && self.sidebar_focus.contains_focused(window, cx) {
            self.focus_workspace(window, cx);
        }
        if self.settings.is_some() {
            let visible = !self.settings_nav_visible;
            self.slide_sidebar(visible, cx);
            self.settings_nav_visible = visible;
            // Search moves between the nav and the title bar.
            self.invalidate_chrome(cx);
            cx.notify();
            return;
        }
        self.sidebar_auto_hidden = false;
        self.set_sidebar_visible(!self.workspace.sidebar_visible, cx);
        self.persist(cx);
        cx.notify();
    }

    pub(super) fn set_sidebar_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if visible == self.workspace.sidebar_visible {
            return;
        }
        self.slide_sidebar(visible, cx);
        self.workspace.sidebar_visible = visible;
        self.sync_right_panel(cx);
    }

    /// Starts (or retargets) the sidebar's slide toward `visible`.
    pub(super) fn slide_sidebar(&mut self, visible: bool, cx: &mut Context<Self>) {
        let to = if visible { 1. } else { 0. };
        self.sidebar_slide = if cx.reduce_motion() {
            None
        } else {
            Some(match self.sidebar_slide.take() {
                Some(slide) => slide.retarget(to, std::time::Instant::now()),
                None => PaneSlide::new(1. - to, to, self.workspace.sidebar_width),
            })
        };
        self.resize = None;
        self.drop_peek();
    }

    pub(super) fn sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(settings) = self.settings.clone() {
            let section = settings.read(cx).section;
            let search = settings.update(cx, |v, cx| v.search_field(true, cx));
            let select = cx.weak_entity();
            let back = select.clone();
            return crate::settings::view::nav(
                section,
                search,
                move |section, _, cx| {
                    let _ = select.update(cx, |this, cx| this.select_settings(section, cx));
                },
                move |window, cx| {
                    let _ = back.update(cx, |this, cx| this.close_settings(window, cx));
                },
            );
        }
        if self.workspace.classic_layout {
            return self.classic_sidebar(cx);
        }
        self.refresh_sidebar_structure(cx);
        let live_agents = self.live_agents_preview(cx);
        div()
            .id("monocode-sidebar")
            .debug_selector(|| "sidebar-content".into())
            .track_focus(&self.sidebar_focus)
            .key_context("SessionSidebar")
            .on_action(cx.listener(
                |this, _: &sessions::groups::RenameSidebarItem, window, cx| {
                    this.rename_sidebar_item(window, cx)
                },
            ))
            .on_action(cx.listener(
                |this, _: &sessions::groups::DeleteSidebarSelection, window, cx| {
                    this.delete_sidebar_selection(window, cx)
                },
            ))
            .w(px(self.workspace.sidebar_width))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div().flex_1().min_h_0().child(
                    div()
                        .relative()
                        .size_full()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .h(px(39.))
                                .flex_shrink_0()
                                .px_3()
                                .pb_px()
                                .relative()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    div().flex_1().min_w_0().flex().items_center().child(
                                        div()
                                            .id("workspace-project")
                                            .debug_selector(|| "sidebar-project-button".into())
                                            .relative()
                                            .role(Role::Button)
                                            .aria_label(format!(
                                                "Switch project, {}",
                                                self.project_label(&self.workspace.project)
                                            ))
                                            .aria_description(self.workspace.project.clone())
                                            .aria_expanded(self.menu_open(&MenuTarget::Header))
                                            .cursor_pointer()
                                            .h(px(28.))
                                            // The whole picker shares Search's left edge.
                                            .px_1p5()
                                            .ml(px(-4.))
                                            .rounded_md()
                                            .max_w_full()
                                            .hover(|s| s.bg(content(0.08)))
                                            .when(self.menu_open(&MenuTarget::Header), |d| {
                                                d.bg(content(0.08))
                                            })
                                            .min_w_0()
                                            .flex()
                                            .items_center()
                                            .gap_1p5()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(crate::project::settings::header_project_icon(
                                                &self.appearance(&self.workspace.project),
                                                &self.workspace.project,
                                            ))
                                            .child(
                                                div().min_w_0().truncate().child(
                                                    self.project_label(&self.workspace.project),
                                                ),
                                            )
                                            .when_some(
                                                self.worktree_focus.get(&self.workspace.project),
                                                |d, focus| {
                                                    d.child(
                                                        div()
                                                            .min_w_0()
                                                            .truncate()
                                                            .font_weight(FontWeight::NORMAL)
                                                            .text_color(content(0.45))
                                                            .child(
                                                                focus
                                                                    .branch
                                                                    .clone()
                                                                    .unwrap_or_else(|| {
                                                                        "detached".into()
                                                                    }),
                                                            ),
                                                    )
                                                },
                                            )
                                            .child(icon("chevron-down", 14.))
                                            .child(self.project_anchor.probe())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                if this.menu_open(&MenuTarget::Header) {
                                                    this.close_menu(cx);
                                                } else {
                                                    let anchor =
                                                        Anchor::Slot(this.project_anchor.clone());
                                                    this.open_menu(
                                                        MenuTarget::Header,
                                                        anchor,
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            })),
                                    ),
                                )
                                .when(self.workspace.sidebar_search_visible, |d| {
                                    d.child(
                                        icon_button("sidebar-search", "Search", "search")
                                            .debug_selector(|| "sidebar-search-button".into())
                                            .when(self.app_search.is_some(), |d| d.bg(content(0.1)))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                if this.app_search.is_some() {
                                                    this.close_app_search(window, cx)
                                                } else {
                                                    this.focus_search(window, cx)
                                                }
                                            })),
                                    )
                                })
                                .child(
                                    icon_button(
                                        "sidebar-options",
                                        "Sidebar options",
                                        "more-horizontal",
                                    )
                                    .debug_selector(|| "sidebar-options".into())
                                    .aria_expanded(self.menu_open(&MenuTarget::SidebarOptions))
                                    .relative()
                                    .child(self.sidebar_options_anchor.probe())
                                    .on_click(cx.listener(
                                        |this, _, window, cx| {
                                            if this.menu_open(&MenuTarget::SidebarOptions) {
                                                this.close_menu(cx);
                                            } else {
                                                this.open_menu(
                                                    MenuTarget::SidebarOptions,
                                                    Anchor::Slot(
                                                        this.sidebar_options_anchor.clone(),
                                                    ),
                                                    window,
                                                    cx,
                                                );
                                            }
                                        },
                                    )),
                                ),
                        )
                        .child(self.virtual_sidebar(cx))
                        .children(live_agents)
                        .child(self.app_update_footer(cx))
                        // The Focused layout has no project rail footer to hold it.
                        .when(!self.workspace.classic_layout, |d| {
                            d.children(
                                self.github_star_prompt(cx)
                                    .map(|prompt| div().mx_2().my_2().child(prompt)),
                            )
                        }),
                ),
            )
            .when(!self.workspace.classic_layout, |d| {
                d.child(self.project_spaces(cx))
            })
            .when_some(self.session_preview.as_ref(), |d, preview| {
                let id = preview.id;
                d.child(
                    popover("session-preview", Anchor::Slot(preview.anchor.clone()))
                        .side(Side::Right)
                        .gap(8.)
                        .width(260.)
                        .max_height(440.)
                        .track(preview.surface.clone())
                        .child(
                            div()
                                .id("session-preview-content")
                                .role(Role::Tooltip)
                                .max_h(px(438.))
                                .overflow_y_scroll()
                                .on_hover(cx.listener(move |this, hovered, window, cx| {
                                    if this
                                        .session_preview
                                        .as_ref()
                                        .is_none_or(|preview| preview.id != id)
                                    {
                                        return;
                                    }
                                    if *hovered {
                                        this.preview_close = None;
                                    } else {
                                        this.schedule_preview_close(window, cx);
                                    }
                                }))
                                .child(self.sessions.get(&preview.id).map_or_else(
                                    || {
                                        sessions::preview::render(
                                            self.session_metadata(preview.id, cx).summary,
                                            preview.data.as_ref(),
                                            self.unseen_finished.contains(&preview.id),
                                        )
                                    },
                                    |view| {
                                        view.read(cx)
                                            .preview(self.unseen_finished.contains(&preview.id))
                                    },
                                )),
                        ),
                )
            })
            .into_any()
    }

    pub(super) fn project_spaces(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // Sidebar.tsx's ProjectSpacesBar: five equal spaces, paged by chevrons.
        let shown = self.workspace.projects.len().min(5);
        let last = self.workspace.projects.len().saturating_sub(shown);
        self.spaces_start = self.spaces_start.min(last);
        let first = self.spaces_start;
        let projects: Vec<_> = self
            .space_reorder
            .order(&self.workspace.projects)
            .into_iter()
            .enumerate()
            .skip(first)
            .take(shown)
            .collect();
        if self
            .hovered_space
            .as_ref()
            .is_some_and(|hovered| !projects.iter().any(|(_, path)| path == hovered))
        {
            self.hovered_space = None;
        }
        let prefs = &self.workspace.preferences;
        let shortcuts = ["Project: Previous", "Project: Next"]
            .into_iter()
            .filter_map(|command| crate::settings::command_shortcut(prefs, command))
            .collect::<Vec<_>>()
            .join(" / ");
        // A short slide in from the side the new spaces come from.
        let (nudge_x, nudge_opacity) = match self.spaces_nudge {
            Some((direction, start)) => {
                let t = start.elapsed().as_secs_f32() / 0.2;
                if t >= 1. {
                    self.spaces_nudge = None;
                    (0., 1.)
                } else {
                    let eased = crate::ui::motion::reveal::bezier(t, 0.32, 0.72, 0., 1.);
                    (direction * 12. * (1. - eased), 0.4 + 0.6 * eased)
                }
            }
            None => (0., 1.),
        };
        let tooltip = self.hovered_space.clone().map(|path| {
            popover(
                "project-space-tooltip",
                Anchor::Slot(self.space_anchor.clone()),
            )
            .side(Side::Top)
            .align(Align::Center)
            .gap(8.)
            .padding(8.)
            .without_animation()
            .allow_outside_press()
            .child(
                div()
                    .id("project-space-tip")
                    .role(Role::Tooltip)
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .whitespace_nowrap()
                    .text_size(px(12.))
                    .line_height(px(15.))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(1.))
                            .child(self.project_label(&path)),
                    )
                    .when(!shortcuts.is_empty(), |d| {
                        d.child(div().text_color(content(0.55)).child(shortcuts.clone()))
                    }),
            )
        });
        div()
            .h(px(30.))
            .flex_shrink_0()
            .px_2()
            .pb_0p5()
            .flex()
            .items_center()
            .gap_1()
            .child(
                icon_button("previous-projects", "Previous projects", "chevron-left")
                    .size(px(20.))
                    .opacity(if first == 0 { 0.3 } else { 1. })
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.spaces_start > 0 {
                            this.spaces_start -= 1;
                            this.nudge_spaces(-1., cx);
                        }
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .relative()
                    .left(px(nudge_x))
                    .opacity(nudge_opacity)
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(self.space_reorder.strip.probe())
                    .children(projects.into_iter().map(|(ix, path)| {
                        let active = path == self.workspace.project;
                        let hovered = self.hovered_space.as_ref() == Some(&path);
                        let dragging = self.space_reorder.dragging(&path);
                        let offset = self.space_reorder.offset(&path, ix, first);
                        div()
                            .id(("project-space", ix))
                            .debug_selector(move || format!("project-space-{ix}"))
                            .group("project-space")
                            .role(Role::Tab)
                            .aria_label(format!("Project {}", self.project_label(&path)))
                            .aria_description(path.clone())
                            .aria_selected(active)
                            .cursor_pointer()
                            .flex_1()
                            .min_w_0()
                            .h(px(20.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .relative()
                            .left(px(offset))
                            .opacity(if active || dragging { 1. } else { 0.45 })
                            .when(active || dragging, |d| d.bg(content(0.1)))
                            .when(dragging, |d| d.cursor(CursorStyle::ClosedHand))
                            .hover(|s| s.opacity(1.).bg(content(if active { 0.1 } else { 0.05 })))
                            .child(self.project_mark(&path, 12.))
                            .when(hovered, |d| d.child(self.space_anchor.probe()))
                            .on_hover({
                                let path = path.clone();
                                cx.listener(move |this, hovered: &bool, _, cx| {
                                    if *hovered {
                                        this.hovered_space = Some(path.clone());
                                    } else if this.hovered_space.as_ref() == Some(&path) {
                                        this.hovered_space = None;
                                    } else {
                                        return;
                                    }
                                    this.invalidate_chrome(cx);
                                })
                            })
                            .on_mouse_down(MouseButton::Left, {
                                let path = path.clone();
                                cx.listener(move |this, event: &MouseDownEvent, _, _| {
                                    this.start_space_drag(&path, event);
                                })
                            })
                            .on_mouse_down(MouseButton::Right, {
                                let path = path.clone();
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    let anchor = Anchor::Point(event.position);
                                    // Spaces have no pins or groups to offer.
                                    let target = MenuTarget::Project {
                                        path: path.clone(),
                                        rail: false,
                                    };
                                    this.open_menu(target, anchor, window, cx);
                                })
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.space_reorder.take_click() {
                                    return;
                                }
                                // The tip steps aside once a space is clicked.
                                this.hovered_space = None;
                                this.select_project(path.clone(), window, cx)
                            }))
                    })),
            )
            .child(
                icon_button("more-projects", "More projects", "chevron-right")
                    .size(px(20.))
                    .opacity(if first >= last { 0.3 } else { 1. })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.spaces_start < last {
                            this.spaces_start += 1;
                            this.nudge_spaces(1., cx);
                        }
                    })),
            )
            .children(tooltip)
            .into_any()
    }

    pub(super) fn nudge_spaces(&mut self, direction: f32, cx: &mut Context<Self>) {
        if !cx.reduce_motion() {
            self.spaces_nudge = Some((direction, std::time::Instant::now()));
        }
        self.invalidate_chrome(cx);
    }
}
