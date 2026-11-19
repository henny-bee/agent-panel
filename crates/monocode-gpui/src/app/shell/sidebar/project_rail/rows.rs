use super::*;

impl Shell {
    pub(super) fn project_rail_list_row(
        &mut self,
        ix: usize,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(row) = self.project_rail_list.rows.get(ix).cloned() else {
            return div().into_any();
        };
        let row_gap = matches!(
            self.project_rail_list.rows.get(ix + 1),
            Some(ProjectRailRow::Project { .. })
        );
        match row {
            ProjectRailRow::Monos => self
                .monos
                .clone()
                .map(|m| m.update(cx, |m, cx| m.project_navigation_rail(compact, cx)))
                .unwrap_or_else(|| div().into_any()),
            ProjectRailRow::Heading(label) => self.project_rail_heading(label, compact, cx),
            ProjectRailRow::Project { path, grouped } => {
                self.project_rail_row(ix, path, compact, grouped, row_gap, cx)
            }
            ProjectRailRow::Group(id) => self.project_rail_group_header(id, cx),
            ProjectRailRow::Gap => div().h(px(8.)).into_any(),
            ProjectRailRow::Empty => div()
                .px_4()
                .pb_1()
                .text_size(px(11.))
                .line_height(px(13.75))
                .text_color(content(0.4))
                .when(!compact, |d| d.child("No projects yet"))
                .into_any(),
        }
    }

    pub(super) fn project_rail_group_header(
        &mut self,
        id: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(group) = self
            .workspace
            .project_rail_groups
            .iter()
            .find(|g| g.id == id)
        else {
            return div().into_any();
        };
        let name = group.name.clone();
        let collapsed = group.collapsed;
        let context = id.clone();
        let options = id.clone();
        let drag_id = id.clone();
        let bounds = self
            .project_rail_list
            .group_bounds
            .entry(id.clone())
            .or_default()
            .clone();
        let offset = self.project_rail_item_offset(&id, true);
        let dragging = self.project_rail_item_raised(&id, true);
        let color = group
            .custom_color
            .as_ref()
            .and_then(|c| u32::from_str_radix(c.trim_start_matches('#'), 16).ok())
            .or_else(|| {
                group
                    .color_index
                    .and_then(|i| monocode_core::project::appearance::COLORS.get(i).copied())
            });
        let appearance = monocode_core::project::appearance::ProjectAppearance {
            color,
            mascot: group.mascot.clone(),
            ..Default::default()
        };
        let row = div()
            .w_full()
            .px_2()
            .child(
                div()
                    .id(SharedString::from(format!("rail-group-{id}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("rail-group-{id}")
                    })
                    .group("project-rail-group")
                    .role(Role::Button)
                    .aria_label(name.clone())
                    .aria_expanded(!collapsed)
                    .h(px(32.))
                    .px_2()
                    .rounded_md()
                    .relative()
                    .top(px(offset))
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(1.))
                    .opacity(0.65)
                    .when(!collapsed, |d| d.bg(content(0.05)))
                    .hover(|s| s.bg(content(0.08)).opacity(1.))
                    .child(
                        div()
                            .size(px(16.))
                            .flex_shrink_0()
                            .relative()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(collapsed, |d| {
                                d.child(
                                    div()
                                        .group_hover("project-rail-group", |s| s.invisible())
                                        .child(crate::ui::project_badge::project_icon(
                                            &appearance,
                                            &id,
                                            12.,
                                        )),
                                )
                            })
                            .child(
                                icon(
                                    if collapsed {
                                        "chevron-right"
                                    } else {
                                        "chevron-down"
                                    },
                                    14.,
                                )
                                .when(collapsed, |d| {
                                    d.invisible()
                                        .group_hover("project-rail-group", |s| s.visible())
                                })
                                .absolute()
                                .left(px(1.))
                                .top(px(1.)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .group_hover("project-rail-group", |s| s.pr(px(24.)))
                            .child(name),
                    )
                    .child(
                        div()
                            .absolute()
                            .right_1()
                            .top(px(4.))
                            .invisible()
                            .group_hover("project-rail-group", |s| s.visible())
                            .child(
                                icon_button(
                                    "rail-group-options",
                                    "Group options",
                                    "more-horizontal",
                                )
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(
                                    move |this, e: &gpui::ClickEvent, w, cx| {
                                        cx.stop_propagation();
                                        this.open_menu(
                                            MenuTarget::ProjectRailGroup(options.clone()),
                                            Anchor::Point(e.position()),
                                            w,
                                            cx,
                                        );
                                    },
                                )),
                            ),
                    )
                    .child(bounds.probe())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.start_project_rail_drag(drag_id.clone(), true, event, window, cx)
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, w, cx| {
                            cx.stop_propagation();
                            this.open_menu(
                                MenuTarget::ProjectRailGroup(context.clone()),
                                Anchor::Point(e.position),
                                w,
                                cx,
                            );
                        }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if std::mem::take(&mut this.project_rail_list.skip_click) {
                            return;
                        }
                        if let Some(g) = this
                            .workspace
                            .project_rail_groups
                            .iter_mut()
                            .find(|g| g.id == id)
                        {
                            g.collapsed = !g.collapsed;
                        }
                        this.persist(cx);
                        this.projects_frame.update(cx, |_, cx| cx.notify());
                    })),
            )
            .into_any();
        if dragging {
            gpui::deferred(row).with_priority(1).into_any()
        } else {
            row
        }
    }

    pub(crate) fn project_rail_group_controls(
        &self,
        id: &str,
        events: &mut Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<crate::project::settings::AppearanceControls> {
        use crate::project::settings::{AppearanceControls, Event};
        let group = self
            .workspace
            .project_rail_groups
            .iter()
            .find(|g| g.id == id)
            .cloned()
            .unwrap_or_default();
        let color = group
            .custom_color
            .as_ref()
            .and_then(|c| u32::from_str_radix(c.trim_start_matches('#'), 16).ok())
            .or_else(|| {
                group
                    .color_index
                    .and_then(|i| monocode_core::project::appearance::COLORS.get(i).copied())
            });
        let appearance = monocode_core::project::appearance::ProjectAppearance {
            name: group.name,
            color,
            mascot: group.mascot,
            ..Default::default()
        };
        let controls =
            cx.new(|cx| AppearanceControls::new(id.into(), appearance, cx).for_project_group());
        let id = id.to_owned();
        events.push(cx.subscribe_in(
            &controls,
            window,
            move |this, _, event: &Event, window, cx| match event {
                Event::Changed(appearance) => {
                    if let Some(group) = this
                        .workspace
                        .project_rail_groups
                        .iter_mut()
                        .find(|g| g.id == id)
                    {
                        if !appearance.name.trim().is_empty() {
                            group.name = appearance.name.trim().into();
                        }
                        group.mascot = appearance.mascot.clone();
                        group.color_index = appearance.color.and_then(|c| {
                            monocode_core::project::appearance::COLORS
                                .iter()
                                .position(|v| *v == c)
                        });
                        group.custom_color = appearance
                            .color
                            .filter(|_| group.color_index.is_none())
                            .map(|c| format!("#{c:06x}"));
                    }
                    this.persist(cx);
                    this.projects_frame.update(cx, |_, cx| cx.notify());
                }
                Event::Close => {
                    this.close_menu(cx);
                    this.focus_workspace(window, cx);
                }
            },
        ));
        controls
    }

    pub(super) fn project_rail_row(
        &mut self,
        ix: usize,
        path: String,
        compact: bool,
        grouped: bool,
        row_gap: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ix = self
            .workspace
            .projects
            .iter()
            .position(|p| p == &path)
            .unwrap_or(ix);
        let selected =
            self.workspace.project == path && !self.app_view_open() && !self.mono_active(cx);
        let label = self.project_label(&path);
        let tooltip = label.clone();
        let menu_path = path.clone();
        let options_path = path.clone();
        let pin_path = path.clone();
        let drag_path = path.clone();
        let pinned = self.workspace.project_rail_pinned.contains(&path);
        let offset = self.project_rail_project_offset(&path);
        let raised = self.project_rail_project_raised(&path);
        let reordering = self.project_rail_reordering();
        let bounds = self
            .project_rail_list
            .bounds
            .entry(path.clone())
            .or_default()
            .clone();
        if !compact {
            self.queue_project_rail_stats(&path, cx);
        }
        let stats = self
            .project_rail_stats
            .values
            .get(&path)
            .and_then(Option::as_ref)
            .cloned();
        let appearance = self.appearance(&path);
        let seed = crate::ui::project_badge::project_name(&path);
        let mark = if appearance.logo.is_some() {
            self.project_mark(&path, 16.)
        } else {
            crate::ui::project_badge::project_icon(&appearance, &seed, 12.)
        };
        let row = div()
            .id(("rail-project", ix))
            .debug_selector(move || format!("rail-project-{ix}"))
            .h(px(if row_gap { 33. } else { 32. }))
            .pl(px(6.))
            .pr_2()
            .when(row_gap, |d| d.pb_px())
            .when(grouped, |d| d.px_3())
            .child(
                div()
                    .id(("rail-project-button", ix))
                    .debug_selector(move || format!("rail-project-button-{ix}"))
                    .group("project-rail-row")
                    .role(Role::Button)
                    .aria_label(format!("Switch project, {label}"))
                    .aria_description(path.clone())
                    .aria_selected(selected)
                    .cursor_pointer()
                    .size_full()
                    .relative()
                    .top(px(offset))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(1.))
                    .opacity(if selected { 1. } else { 0.65 })
                    .when(selected, |d| d.bg(content(0.1)))
                    .hover(|s| s.bg(content(0.08)).opacity(1.))
                    .child(
                        div()
                            .size(px(16.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(!compact && !reordering, |d| {
                                d.group_hover("project-rail-row", |s| s.invisible())
                            })
                            .child(mark),
                    )
                    .when(!compact, |d| {
                        d.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .when(!reordering, |d| {
                                    d.group_hover("project-rail-row", |s| s.pr(px(24.)))
                                })
                                .child(label),
                        )
                    })
                    .when(!compact, |d| {
                        d.when_some(stats, |d, stats| {
                            d.child(
                                div()
                                    .flex_shrink_0()
                                    .flex()
                                    .gap_1()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .font_features(tabular())
                                    .when(!reordering, |d| {
                                        d.group_hover("project-rail-row", |s| s.invisible())
                                    })
                                    .when(stats.additions > 0, |d| {
                                        d.child(
                                            div().text_color(rgb(DIFF_ADD)).child(format!(
                                                "+{}",
                                                format_integer(stats.additions)
                                            )),
                                        )
                                    })
                                    .when(stats.deletions > 0, |d| {
                                        d.child(
                                            div().text_color(rgb(DIFF_DEL)).child(format!(
                                                "-{}",
                                                format_integer(stats.deletions)
                                            )),
                                        )
                                    }),
                            )
                        })
                    })
                    .when(!compact && !reordering, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left_2()
                                .top(px(8.))
                                .invisible()
                                .group_hover("project-rail-row", |s| s.visible())
                                .child(
                                    icon_button(
                                        "rail-pin-project",
                                        if pinned {
                                            "Unpin project"
                                        } else {
                                            "Pin project"
                                        },
                                        if pinned { "pin-off" } else { "pin" },
                                    )
                                    .size(px(16.))
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.toggle_project_rail_pin(&pin_path, cx);
                                        },
                                    )),
                                ),
                        )
                        .child(
                            div()
                                .absolute()
                                .right_1()
                                .top(px(4.))
                                .invisible()
                                .group_hover("project-rail-row", |s| s.visible())
                                .child(
                                    icon_button(
                                        "rail-project-options",
                                        "Project options",
                                        "more-horizontal",
                                    )
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(
                                        move |this, e: &gpui::ClickEvent, window, cx| {
                                            cx.stop_propagation();
                                            this.open_menu(
                                                MenuTarget::Project {
                                                    path: options_path.clone(),
                                                    rail: true,
                                                },
                                                Anchor::Point(e.position()),
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                                ),
                        )
                    })
                    .child(bounds.probe())
                    .tooltip(move |_, cx| cx.new(|_| ProjectTooltip(tooltip.clone())).into())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if std::mem::take(&mut this.project_rail_list.skip_click) {
                            return;
                        }
                        this.select_project(path.clone(), window, cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            this.start_project_rail_drag(drag_path.clone(), false, e, window, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            this.open_menu(
                                MenuTarget::Project {
                                    path: menu_path.clone(),
                                    rail: true,
                                },
                                Anchor::Point(e.position),
                                window,
                                cx,
                            );
                            cx.stop_propagation();
                        }),
                    ),
            )
            .into_any_element();
        if raised {
            gpui::deferred(row).with_priority(1).into_any()
        } else {
            row
        }
    }
}
