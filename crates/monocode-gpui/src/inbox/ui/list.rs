use super::*;

impl Inbox {
    pub(super) fn project_label(&self, path: &str) -> String {
        self.appearances.get(path).map_or_else(
            || project_name(std::path::Path::new(path)),
            |appearance| appearance.label(path),
        )
    }

    pub(super) fn project_mark(&self, path: &str, side: f32) -> AnyElement {
        crate::ui::project_badge::project_icon(
            &self.appearances.get(path).cloned().unwrap_or_default(),
            path,
            side,
        )
    }

    pub(super) fn list_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let connected = model::PROVIDERS
            .into_iter()
            .filter(|provider| {
                self.snapshot
                    .connections
                    .get(*provider)
                    .is_some_and(|s| s.connected)
            })
            .collect::<Vec<_>>();
        let items = self.visible_items(cx);
        let sources = div()
            .h(px(36.))
            .px(px(8.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(1.))
            .border_b_1()
            .border_color(content(0.07))
            .children(connected.iter().map(|provider| {
                let source = provider.to_string();
                div()
                    .id(SharedString::from(format!("inbox-source-{provider}")))
                    .role(Role::Tab)
                    .aria_label(model::provider_label(provider))
                    .flex_1()
                    .min_w_0()
                    .h(px(24.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .rounded(px(6.))
                    .text_size(px(12.))
                    .text_color(content(if self.state.source == *provider {
                        1.
                    } else {
                        0.5
                    }))
                    .when(self.state.source == *provider, |d| d.bg(content(0.1)))
                    .hover(|s| s.bg(content(0.05)))
                    .child(provider_mark(provider))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(model::provider_label(provider)),
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.change_source(source.clone(), cx)),
                    )
            }))
            .child(
                div()
                    .id("inbox-add-connection")
                    .role(Role::Button)
                    .aria_label("Add connection")
                    .relative()
                    .h(px(24.))
                    .flex_1()
                    .min_w_0()
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .rounded(px(6.))
                    .text_size(px(12.))
                    .text_color(content(0.4))
                    .hover(|s| s.bg(content(0.05)))
                    .child(self.connection_anchor.probe())
                    .child(icon("plus", 14.))
                    .when(connected.len() < 2, |d| {
                        d.child(div().truncate().child("Add connection"))
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.connections_open = !this.connections_open;
                        this.filters_open = false;
                        cx.notify();
                    })),
            );
        let tools = div()
            .h(px(36.))
            .px(px(8.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(1.))
            .border_b_1()
            .border_color(content(0.07))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(28.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(icon("search", 12.))
                    .child(self.search.clone()),
            )
            .child(
                icon_button("inbox-filter", "Filter inbox", "filter")
                    .relative()
                    .child(self.filter_anchor.probe())
                    .when(self.filters_open || self.filters_active(), |d| {
                        d.bg(content(0.1))
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.filters_open = !this.filters_open;
                        this.connections_open = false;
                        cx.notify();
                    })),
            )
            .child(
                icon_button("inbox-read-all", "Mark all as read", "check-2").on_click(cx.listener(
                    |this, _, _, cx| {
                        for item in &this.snapshot.items {
                            if item.provider == this.state.source {
                                this.state.seen.mark(item);
                            }
                        }
                        this.save();
                        cx.notify();
                    },
                )),
            )
            .child(
                icon_button(
                    "inbox-refresh",
                    "Refresh",
                    if self.loading { "loader" } else { "refresh-cw" },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.refresh(cx);
                    this.load_detail(false, cx);
                })),
            );
        let empty = if connected.is_empty() && !self.loading {
            "Add a connection to start using the Inbox.".into()
        } else if self.loading && self.snapshot.items.is_empty() {
            "Loading inbox…".into()
        } else if let Some(error) = self.snapshot.errors.get(&self.state.source) {
            error.clone()
        } else if self.projects.is_empty() && !model::is_tracker(&self.state.source) {
            "Open a project to fill the inbox".into()
        } else if self.filters_active() || !self.search.read(cx).content.is_empty() {
            "No items match these filters".into()
        } else if model::is_tracker(&self.state.source) {
            format!("No {} issues", model::provider_label(&self.state.source))
        } else {
            "No matching issues or pull requests".into()
        };
        let rows = items
            .into_iter()
            .map(|item| {
                let key = item.key();
                let active = self.selected.as_ref() == Some(&key);
                let unread = self.state.seen.unseen(&item);
                let sessions = self.related_sessions(&item).len();
                let tracker = model::is_tracker(&item.provider);
                let kind = if item.kind == "pr" {
                    if item.provider == "gitlab" {
                        "Merge request"
                    } else {
                        "Pull request"
                    }
                } else {
                    "Issue"
                };
                let source = if tracker && !item.team_name.is_empty() {
                    item.team_name.clone()
                } else if !item.repo.is_empty() {
                    item.repo.clone()
                } else {
                    self.project_label(&item.project_path)
                };
                let attention = attention_label(&item.attention_reason);
                div()
                    .id(SharedString::from(format!("inbox-item-{key}")))
                    .role(Role::Button)
                    .aria_label(item.title.clone())
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(content(0.))
                    .cursor_pointer()
                    .when(active, |d| d.bg(content(0.1)))
                    .when(!active, |d| d.hover(|s| s.bg(content(0.05))))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(11.))
                            .text_color(content(0.5))
                            .child(provider_mark(&item.provider))
                            .child(icon(status_icon(&item), 12.).text_color(status_color(&item)))
                            .child(div().min_w_0().flex_1().truncate().child(format!(
                                "{kind} · {}{}",
                                item.reference(),
                                if attention.is_empty() {
                                    String::new()
                                } else {
                                    format!(" · {attention}")
                                }
                            )))
                            .when(sessions > 0, |d| {
                                d.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(2.))
                                        .text_color(accent_color())
                                        .child(
                                            icon("message-multiple", 12.)
                                                .text_color(accent_color()),
                                        )
                                        .child(sessions.to_string()),
                                )
                            })
                            .child(
                                div()
                                    .text_color(content(0.45))
                                    .child(relative_time(&item.updated_at)),
                            )
                            .when(unread, |d| {
                                d.child(div().size(px(6.)).rounded_full().bg(accent_color()))
                            }),
                    )
                    .child(
                        div()
                            .mt(px(4.))
                            .truncate()
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(1.))
                            .child(item.title.clone()),
                    )
                    .child(
                        div()
                            .mt(px(4.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(11.))
                                    .text_color(content(0.45))
                                    .when(!tracker && !item.project_path.is_empty(), |d| {
                                        d.child(self.project_mark(&item.project_path, 12.))
                                    })
                                    .child(div().min_w_0().truncate().child(source)),
                            )
                            .children(
                                item.labels
                                    .iter()
                                    .take(2)
                                    .map(|label| label_badge(label, true)),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select(key.clone(), cx)))
            })
            .collect::<Vec<_>>();
        div()
            .relative()
            .w(px(self.state.list_width.clamp(240., 420.)))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(content(0.07))
            .flex()
            .flex_col()
            .child(sources)
            .when(!connected.is_empty(), |d| d.child(tools))
            .child(
                div()
                    .id("inbox-list-scroll")
                    .role(Role::ListBox)
                    .aria_label("Inbox items")
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .p(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .when(rows.is_empty(), |d| d.child(message(empty)))
                    .children(rows),
            )
            .child(
                div()
                    .id("inbox-list-resize")
                    .role(Role::Splitter)
                    .aria_label("Resize inbox list")
                    .absolute()
                    .right(px(-3.))
                    .top_0()
                    .bottom_0()
                    .w(px(6.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            this.resize_start =
                                Some((f32::from(event.position.x), this.state.list_width));
                            cx.stop_propagation();
                        }),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn related_sessions(&self, item: &Item) -> Vec<Session> {
        let work_item = item.work_item();
        self.sessions
            .iter()
            .filter(|s| {
                s.work_item.as_ref().is_some_and(|work| {
                    (!work.url.is_empty() && work.url == work_item.url)
                        || (work.kind == work_item.kind
                            && work.number == work_item.number
                            && work.repo.eq_ignore_ascii_case(&work_item.repo))
                })
            })
            .cloned()
            .collect()
    }

    pub(super) fn filters_active(&self) -> bool {
        let f = &self.state.filters;
        f.assigned_to_me
            || !f.hidden_projects.is_empty()
            || !f.hidden_linear_projects.is_empty()
            || !f.hidden_kinds.is_empty()
            || f.time != "all"
            || f.status.open
            || f.status.draft
            || f.status.closed
            || f.status.merged
            || (self.state.source == "linear" && !self.state.hidden_linear_teams.is_empty())
            || (self.state.source == "jira" && !self.state.hidden_jira_projects.is_empty())
    }

    pub(super) fn filters_changed(&mut self, refetch: bool, cx: &mut Context<Self>) {
        self.save();
        if let Some(item) = self.visible_items(cx).first() {
            self.select(item.key(), cx);
        } else {
            self.selected = None;
        }
        if refetch {
            self.refresh(cx);
        }
        cx.notify();
    }

    pub(super) fn filters_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let f = &self.state.filters;
        let tracker = model::is_tracker(&self.state.source);
        let mut rows = vec![
            caption("FILTER INBOX").into_any_element(),
            menu_row(
                "inbox-assigned",
                if matches!(self.state.source.as_str(), "gitlab" | "azuredevops") {
                    "Needs my attention"
                } else {
                    "Assigned to me"
                },
                f.assigned_to_me,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.state.filters.assigned_to_me = !this.state.filters.assigned_to_me;
                this.filters_changed(true, cx);
            }))
            .into_any_element(),
            caption("UPDATED").into_any_element(),
        ];
        for (time, label) in [
            ("all", "Any time"),
            ("today", "Today"),
            ("7d", "Last 7 days"),
            ("30d", "Last 30 days"),
        ] {
            rows.push(
                menu_row(format!("inbox-time-{time}"), label, f.time == time)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.filters.time = time.into();
                        this.filters_changed(false, cx);
                    }))
                    .into_any_element(),
            );
        }
        rows.push(caption("STATUS").into_any_element());
        for status in ["Open", "Draft", "Closed", "Merged"]
            .into_iter()
            .filter(|s| !tracker || matches!(*s, "Open" | "Closed"))
        {
            let checked = match status {
                "Open" => f.status.open,
                "Draft" => f.status.draft,
                "Closed" => f.status.closed,
                _ => f.status.merged,
            };
            rows.push(
                menu_row(format!("inbox-status-{status}"), status, checked)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let s = &mut this.state.filters.status;
                        match status {
                            "Open" => s.open = !s.open,
                            "Draft" => s.draft = !s.draft,
                            "Closed" => s.closed = !s.closed,
                            _ => s.merged = !s.merged,
                        }
                        this.filters_changed(true, cx);
                    }))
                    .into_any_element(),
            );
        }
        if !tracker {
            rows.push(caption("TYPE").into_any_element());
            for (kind, label) in [
                ("issue", "Issues"),
                (
                    "pr",
                    if self.state.source == "gitlab" {
                        "Merge requests"
                    } else {
                        "Pull requests"
                    },
                ),
            ] {
                rows.push(
                    menu_row(
                        format!("inbox-kind-{kind}"),
                        label,
                        !f.hidden_kinds.iter().any(|k| k == kind),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        toggle(&mut this.state.filters.hidden_kinds, kind.into());
                        this.filters_changed(false, cx);
                    }))
                    .into_any_element(),
                );
            }
            rows.push(caption("PROJECTS").into_any_element());
            for (ix, project) in self.projects.iter().enumerate() {
                let key = project.to_string_lossy().into_owned();
                rows.push(
                    menu_row(
                        format!("inbox-project-filter-{ix}"),
                        self.project_label(&key),
                        !f.hidden_projects.contains(&key),
                    )
                    .child(self.project_mark(&key, 12.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        toggle(&mut this.state.filters.hidden_projects, key.clone());
                        this.filters_changed(false, cx);
                    }))
                    .into_any_element(),
                );
            }
        }
        let provider_options = if self.state.source == "linear" {
            &self.snapshot.linear_teams
        } else {
            &self.snapshot.jira_projects
        };
        if tracker && !provider_options.is_empty() {
            rows.push(
                caption(if self.state.source == "linear" {
                    "TEAMS"
                } else {
                    "PROJECTS"
                })
                .into_any_element(),
            );
            for (ix, option) in provider_options.iter().enumerate() {
                let id = option.id.clone();
                let hidden = if self.state.source == "linear" {
                    &self.state.hidden_linear_teams
                } else {
                    &self.state.hidden_jira_projects
                };
                rows.push(
                    menu_row(
                        format!("inbox-tracker-filter-{ix}"),
                        option.name.clone(),
                        !hidden.contains(&id),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.state.source == "linear" {
                            toggle(&mut this.state.hidden_linear_teams, id.clone());
                        } else {
                            toggle(&mut this.state.hidden_jira_projects, id.clone());
                        }
                        this.filters_changed(true, cx);
                    }))
                    .into_any_element(),
                );
            }
        }
        if self.state.source == "linear" {
            let mut projects = std::collections::BTreeMap::new();
            for item in self
                .snapshot
                .items
                .iter()
                .filter(|i| i.provider == "linear")
            {
                projects
                    .entry(if item.project_id.is_empty() {
                        "~none".into()
                    } else {
                        item.project_id.clone()
                    })
                    .or_insert_with(|| {
                        if item.project_name.is_empty() {
                            "No project".into()
                        } else {
                            item.project_name.clone()
                        }
                    });
            }
            rows.push(caption("LINEAR PROJECTS").into_any_element());
            for (ix, (id, name)) in projects.into_iter().enumerate() {
                rows.push(
                    menu_row(
                        format!("inbox-linear-project-{ix}"),
                        name,
                        !f.hidden_linear_projects.contains(&id),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        toggle(&mut this.state.filters.hidden_linear_projects, id.clone());
                        this.filters_changed(false, cx);
                    }))
                    .into_any_element(),
                );
            }
        }
        rows.push(
            action("inbox-clear-filters", "Clear filters")
                .w_full()
                .justify_start()
                .mt(px(8.))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.state.filters = model::state::Filters::default();
                    this.state.hidden_linear_teams.clear();
                    this.state.hidden_jira_projects.clear();
                    this.filters_changed(true, cx);
                }))
                .into_any_element(),
        );
        popover(
            "inbox-filter-menu",
            Anchor::Slot(self.filter_anchor.clone()),
        )
        .align(Align::End)
        .width(252.)
        .max_height(620.)
        .padding(4.)
        .on_dismiss(move |_, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.filters_open = false;
                cx.notify();
            });
        })
        .child(
            div()
                .id("inbox-filter-scroll")
                .overflow_y_scroll()
                .children(rows),
        )
        .into_any_element()
    }

    pub(super) fn project_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        popover(
            "inbox-project-menu",
            Anchor::Slot(self.project_anchor.clone()),
        )
        .width(240.)
        .padding(4.)
        .on_dismiss(move |_, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.project_picker = false;
                cx.notify();
            });
        })
        .children(self.projects.iter().enumerate().map(|(ix, project)| {
            let path = project.clone();
            menu_row(
                format!("inbox-start-project-{ix}"),
                self.project_label(&project.to_string_lossy()),
                *project == self.start_project,
            )
            .child(self.project_mark(&project.to_string_lossy(), 14.))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.start_project = path.clone();
                this.project_picker = false;
                cx.notify();
            }))
        }))
        .into_any_element()
    }
}
