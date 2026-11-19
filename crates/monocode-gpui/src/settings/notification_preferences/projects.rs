use super::*;

impl SettingsView {
    pub(super) fn notification_projects(&self) -> Vec<Project> {
        let mut projects = Vec::new();
        let mut ids = HashSet::new();
        let paths: Vec<_> = self
            .ctx
            .projects
            .iter()
            .map(|(path, _)| path.as_str())
            .chain(std::iter::once(self.ctx.cwd.as_str()))
            .filter(|path| looks_like_project(path))
            .collect();
        for p in self
            .prefs
            .notification_projects
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| valid_project(p))
        {
            if p["paths"].as_array().is_some_and(|entries| {
                entries.is_empty()
                    || entries.iter().any(|entry| {
                        entry.as_str().is_some_and(|entry| {
                            paths.iter().any(|path| path_key(entry) == path_key(path))
                        })
                    })
            }) {
                let id = p["id"].as_str().unwrap().to_owned();
                ids.insert(id.clone());
                projects.push(Project {
                    id,
                    name: p["name"].as_str().unwrap().into(),
                    detail: p["detail"].as_str().unwrap().into(),
                    kind: p["kind"].as_str().unwrap().into(),
                });
            }
        }
        for path in paths {
            let Some(id) = self.prefs.known_notification_project_id(path) else {
                continue;
            };
            if ids.insert(id.clone()) {
                projects.push(Project {
                    id,
                    name: self
                        .ctx
                        .projects
                        .iter()
                        .find(|(p, _)| path_key(p) == path_key(path))
                        .map(|(_, n)| n.clone())
                        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).into()),
                    detail: path.into(),
                    kind: "local".into(),
                });
            }
        }
        projects.sort_by_key(|p| p.name.to_lowercase());
        projects
    }

    pub(in crate::settings) fn project_notification_settings(
        &mut self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let projects = self.notification_projects();
        let all_ids: Vec<_> = projects.iter().map(|p| p.id.clone()).collect();
        self.notification_state
            .selected
            .retain(|id| all_ids.contains(id));
        let selecting = self.notification_state.selecting;
        let selected: Vec<_> = all_ids
            .iter()
            .filter(|id| self.notification_state.selected.contains(*id))
            .cloned()
            .collect();
        let all_selected = !all_ids.is_empty() && selected.len() == all_ids.len();
        let mut rows = Vec::new();
        if selecting {
            let select_ids = all_ids.clone();
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_4()
                    .py(px(14.))
                    .border_b_1()
                    .border_color(content(0.05))
                    .child(
                        secondary_button(
                            "notification-select-all",
                            "Select all projects",
                            false,
                            false,
                        )
                        .child(if selected.is_empty() {
                            "Select all projects".into()
                        } else {
                            format!("{} selected", selected.len())
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if all_selected {
                                this.notification_state.selected.clear();
                            } else {
                                this.notification_state
                                    .selected
                                    .extend(select_ids.iter().cloned());
                            }
                            cx.notify();
                        })),
                    )
                    .when(!selected.is_empty(), |d| {
                        d.child(self.mute_controls(selected.clone(), "selected".into(), cx))
                    })
                    .into_any_element(),
            );
        }
        for (ix, project) in projects.iter().enumerate() {
            let id = project.id.clone();
            let expand_id = id.clone();
            let select_id = id.clone();
            let expanded = self.notification_state.expanded.as_ref() == Some(&id);
            let muted = self
                .prefs
                .project_muted(&id, monocode_core::sys::time::now_ms());
            let categories: Vec<_> = CATEGORIES
                .iter()
                .filter(|(cat, _)| match project.kind.as_str() {
                    "linear" | "jira" => *cat == "issues",
                    "local" => !["pullRequests", "issues"].contains(cat),
                    _ => true,
                })
                .collect();
            let enabled_count = categories
                .iter()
                .filter(|(cat, _)| {
                    !self.prefs.project_notifications[&id]["disabled"]
                        .as_array()
                        .is_some_and(|values| values.iter().any(|v| v.as_str() == Some(cat)))
                })
                .count();
            let status = if muted {
                "All notifications paused".into()
            } else if enabled_count == categories.len() {
                "All categories enabled".into()
            } else {
                format!("{enabled_count} of {} enabled", categories.len())
            };
            let mut category_rows = Vec::new();
            if muted {
                category_rows.push(div().id("project-notification-accessible-2").role(Role::Status).py_3().text_size(px(12.)).text_color(content(0.45)).child("Your category choices apply when notifications resume. You can edit them while muted.").into_any_element());
            }
            for (cat, label) in categories {
                let category = (*cat).to_owned();
                let project_id = id.clone();
                let enabled = !self.prefs.project_notifications[&id]["disabled"]
                    .as_array()
                    .is_some_and(|values| values.iter().any(|v| v.as_str() == Some(cat)));
                category_rows.push(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .py_3()
                        .text_size(px(12.))
                        .text_color(content(0.7))
                        .child(*label)
                        .child(
                            div()
                                .id(gpui::SharedString::from(format!(
                                    "notification-category-{id}-{cat}"
                                )))
                                .role(Role::Switch)
                                .aria_label(format!("{label} for {}", project.name))
                                .aria_toggled(if enabled {
                                    gpui::Toggled::True
                                } else {
                                    gpui::Toggled::False
                                })
                                .cursor_pointer()
                                .w(px(36.))
                                .h(px(20.))
                                .relative()
                                .rounded_full()
                                .bg(if enabled {
                                    accent_color()
                                } else {
                                    content(0.2)
                                })
                                .child(
                                    div()
                                        .absolute()
                                        .top(px(2.))
                                        .left(px(if enabled { 18. } else { 2. }))
                                        .size(px(16.))
                                        .rounded_full()
                                        .bg(gpui::rgb(0xffffff)),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.update(cx, |p| {
                                        p.set_category(
                                            &project_id,
                                            &category,
                                            !enabled,
                                            monocode_core::sys::time::now_ms(),
                                        )
                                    })
                                })),
                        )
                        .into_any_element(),
                );
            }
            rows.push(
                div()
                    .id(("notification-project", ix))
                    .border_b_1()
                    .border_color(content(0.05))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_3()
                            .px_4()
                            .py(px(14.))
                            .when(selecting, |d| {
                                d.child(
                                    secondary_button(
                                        format!("select-notification-{id}"),
                                        format!("Select {}", project.name),
                                        false,
                                        false,
                                    )
                                    .child(if self.notification_state.selected.contains(&id) {
                                        "✓"
                                    } else {
                                        "□"
                                    })
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            if !this.notification_state.selected.remove(&select_id)
                                            {
                                                this.notification_state
                                                    .selected
                                                    .insert(select_id.clone());
                                            }
                                            cx.notify();
                                        },
                                    )),
                                )
                            })
                            .child(
                                div()
                                    .id(gpui::SharedString::from(format!(
                                        "expand-notification-{id}"
                                    )))
                                    .role(Role::Button)
                                    .aria_label(format!(
                                        "Notification categories for {} ({})",
                                        project.name, project.detail
                                    ))
                                    .cursor_pointer()
                                    .flex_1()
                                    .min_w(px(180.))
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(icon("folder", 16.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(13.))
                                                    .font_weight(gpui::FontWeight::MEDIUM)
                                                    .child(project.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .mt_1()
                                                    .text_size(px(12.))
                                                    .text_color(content(0.45))
                                                    .child(format!(
                                                        "{}{status}",
                                                        if project.kind == "local" {
                                                            "Local project · "
                                                        } else {
                                                            ""
                                                        }
                                                    )),
                                            ),
                                    )
                                    .child(icon(
                                        if expanded {
                                            "chevron-down"
                                        } else {
                                            "chevron-right"
                                        },
                                        14.,
                                    ))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.notification_state.expanded = if expanded {
                                            None
                                        } else {
                                            Some(expand_id.clone())
                                        };
                                        cx.notify();
                                    })),
                            )
                            .child(self.mute_controls(vec![id], format!("project-{ix}"), cx)),
                    )
                    .when(expanded, |d| {
                        d.child(
                            div()
                                .border_t_1()
                                .border_color(content(0.05))
                                .px_4()
                                .children(category_rows),
                        )
                    })
                    .into_any_element(),
            );
        }
        if projects.is_empty() {
            rows.push(div().id("project-notification-accessible-3").role(Role::Status).px_4().py(px(14.)).text_size(px(12.)).text_color(content(0.45)).child("Open a project or connect an Inbox provider to configure its notifications.").into_any_element());
        }
        self.group(Some("project-notifications"),true,"Project notifications",Some("Choose sounds, banners and sidebar indicators by category. Mute pauses them without changing your choices. Unread items stay marked in Inbox.".into()),
            Some(secondary_button("notification-select-projects",if selecting {"Done"} else {"Select projects"},false,projects.is_empty()).child(if selecting {"Done"} else {"Select projects"})
                .on_click(cx.listener(|this,_,_,cx| {this.notification_state.selecting = !this.notification_state.selecting;this.notification_state.selected.clear();cx.notify();})).into_any_element()),rows)
    }
}
