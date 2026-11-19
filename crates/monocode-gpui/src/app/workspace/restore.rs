use super::*;

impl Workspace {
    /// Take the app-wide parts of another window's workspace: settings,
    /// projects, their look and actions. Tabs, panes and the shown project stay
    /// this window's. False when nothing differed.
    pub(crate) fn adopt_shared(&mut self, from: &Workspace) -> bool {
        fn take<T: Clone + PartialEq>(to: &mut T, from: &T) -> bool {
            let changed = to != from;
            if changed {
                *to = from.clone();
            }
            changed
        }
        // `|`, not `||`: every field is taken.
        take(&mut self.preferences, &from.preferences)
            | take(&mut self.projects, &from.projects)
            | take(&mut self.archived_projects, &from.archived_projects)
            | take(&mut self.project_appearance, &from.project_appearance)
            | take(&mut self.actions, &from.actions)
            | take(&mut self.last_actions, &from.last_actions)
            | take(&mut self.project_rail_pinned, &from.project_rail_pinned)
            | take(&mut self.project_rail_groups, &from.project_rail_groups)
            | take(
                &mut self.project_rail_assignments,
                &from.project_rail_assignments,
            )
            | take(&mut self.classic_layout, &from.classic_layout)
            | take(&mut self.new_layout_boxy, &from.new_layout_boxy)
            | take(&mut self.session_filters, &from.session_filters)
            | take(
                &mut self.new_layout_prompt_dismissed,
                &from.new_layout_prompt_dismissed,
            )
            | take(
                &mut self.github_star_prompt_dismissed,
                &from.github_star_prompt_dismissed,
            )
    }

    /// Just the app-wide parts, for handing to another window.
    pub(crate) fn shared_part(&self) -> Workspace {
        let mut part = Workspace::default();
        part.adopt_shared(self);
        part
    }

    /// Seed original-rail concepts from the exact desktop preference strings.
    /// Native edits then live in their own persisted fields.
    pub(crate) fn restore_project_rail_metadata(&mut self) {
        let Some(metadata) = self.desktop_preferences_metadata.as_ref() else {
            return;
        };
        let read = |key: &str| -> Option<serde_json::Value> {
            metadata
                .get(key)?
                .as_str()
                .and_then(|s| serde_json::from_str(s).ok())
        };
        if let Some(value) = read("monocode.railMonosPinned") {
            self.project_rail_monos_pinned = value.as_bool().unwrap_or(false);
        }
        if self.classic_sidebar_tab_order.is_empty()
            && let Some(value) = read("monocode.sidebarTabOrder")
        {
            self.classic_sidebar_tab_order = serde_json::from_value::<Vec<String>>(value)
                .unwrap_or_default()
                .into_iter()
                .filter(|tab| ["sessions", "files", "changes"].contains(&tab.as_str()))
                .collect();
        }
        if self.classic_sidebar_tabs.is_empty()
            && let Some(value) = read("monocode.projectSidebarTabs.v1")
        {
            self.classic_sidebar_tabs = serde_json::from_value::<HashMap<String, String>>(value)
                .unwrap_or_default()
                .into_iter()
                .filter(|(_, tab)| ["sessions", "files", "changes"].contains(&tab.as_str()))
                .collect();
        }
        if let Some(value) = read("monocode.projectRailPinned") {
            self.project_rail_pinned = serde_json::from_value(value).unwrap_or_default();
        }
        if let Some(value) = read("monocode.projectGroups") {
            self.project_rail_groups = serde_json::from_value(value).unwrap_or_default();
        }
        if let Some(value) = read("monocode.projectGroupAssignments") {
            self.project_rail_assignments = serde_json::from_value(value).unwrap_or_default();
        }
        if let Some(value) = read("monocode.projectRailOrder")
            && let Ok(order) = serde_json::from_value::<Vec<String>>(value)
        {
            let mut projects: Vec<_> = order
                .into_iter()
                .filter(|p| self.projects.contains(p))
                .collect();
            projects.extend(
                self.projects
                    .iter()
                    .filter(|p| !projects.contains(p))
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            self.projects = projects;
        }
    }

    pub fn boxy_layout(&self) -> bool {
        self.classic_layout || self.new_layout_boxy
    }

    pub fn restore(saved: Option<&str>, sessions: &[(i64, String)], cwd: &str) -> Self {
        let mut state = saved
            .and_then(|s| serde_json::from_str::<Self>(s).ok())
            .unwrap_or_default();
        // Before the project rail returned, "classic" meant the boxy version
        // of the single-sidebar layout. Keep that appearance for existing users.
        if let Some(saved) = saved.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()) {
            if saved.get("new_layout_boxy").is_none() {
                state.new_layout_boxy = state.classic_layout;
                state.classic_layout = false;
            }
            // Keep the native empty-session choice when upgrading from the
            // old games toggle. An explicitly saved new choice always wins.
            let preferences = &saved["preferences"];
            if preferences.get("empty_session").is_none()
                && let Some(games) = preferences["empty_session_games"].as_bool()
            {
                state.preferences.empty_session = if games { "games" } else { "none" }.into();
            }
        }
        let has_rail_state = saved
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .is_some_and(|value| value.get("project_rail_pinned").is_some());
        if !has_rail_state {
            state.restore_project_rail_metadata();
        }
        // Hidden project navigation is retired; collapsed always means icons.
        state.preferences.collapsed_project_rail = "compact".into();
        state.project_rail_width = if state.project_rail_width.is_finite() {
            state.project_rail_width.clamp(176., 360.)
        } else {
            220.
        };
        if state.project.is_empty() {
            state.project = cwd.into();
        }
        for appearance in state.project_appearance.values_mut() {
            appearance.normalize();
        }
        let mut known: HashSet<_> = sessions.iter().map(|s| s.0).collect();
        state.review_owners.retain(|id, owner| {
            *id < 0
                && known.contains(owner)
                && state.reviews.get(id).is_some_and(|r| !r.tabs.is_empty())
        });
        known.extend(state.review_owners.keys().copied());
        let owners = &state.review_owners;
        state.mono_panes.retain(|id, mono| {
            *id < 0 && !owners.contains_key(id) && matches!(mono, TitleTab::Mono { .. })
        });
        known.extend(state.mono_panes.keys().copied());
        let (owners, monos) = (&state.review_owners, &state.mono_panes);
        state.terminal_panes.retain(|id, pane| {
            *id < 0
                && !owners.contains_key(id)
                && !monos.contains_key(id)
                && sessions.iter().any(|(known, _)| *known == pane.owner)
        });
        known.extend(state.terminal_panes.keys().copied());
        let mut seen = HashSet::new();
        let mut layouts = std::mem::take(&mut state.layouts);
        let mut focused = std::mem::take(&mut state.focused);
        for tab in std::mem::take(&mut state.tabs) {
            let layout = layouts.remove(&tab).unwrap_or_else(|| leaf(tab));
            let Some(layout) = layout.retain(&mut |id| known.contains(&id) && seen.insert(id))
            else {
                continue;
            };
            let key = if layout.contains(tab) {
                tab
            } else {
                layout.first_leaf()
            };
            state.tabs.push(key);
            for entry in state
                .title_order
                .iter_mut()
                .chain(state.scope_focus.values_mut())
                .chain(state.pinned_tabs.iter_mut())
            {
                if *entry == TitleTab::Session(tab) {
                    *entry = TitleTab::Session(key);
                }
            }
            if layout.is_split() {
                if let Some(pane) = focused.remove(&tab).filter(|id| layout.contains(*id)) {
                    state.focused.insert(key, pane);
                }
                state.layouts.insert(key, layout);
            }
        }
        state.settle_mono_panes();
        state.prune_terminal_panes();
        state.normalize_title_order();
        if state
            .desktop_project_preferences
            .as_ref()
            .and_then(|source| source.get("projects"))
            .is_some()
            && state.active.is_some_and(|id| {
                sessions.iter().any(|(known, path)| {
                    *known == state.session_for_pane(id) && !state.projects.contains(path)
                })
            })
        {
            // An old native sidecar could focus an internal session folder.
            // It must not restore that folder as a remembered user project.
            state.active = None;
        }
        if state.active.is_none_or(|id| state.tab_of(id).is_none()) {
            state.active = state
                .project_focus
                .get(&state.project)
                .copied()
                .filter(|id| {
                    state.tab_of(*id).is_some()
                        && sessions.iter().any(|(known, path)| {
                            *known == state.session_for_pane(*id) && *path == state.project
                        })
                })
                .or_else(|| {
                    state
                        .tabs
                        .iter()
                        .rev()
                        .map(|tab| state.tab_focus(*tab))
                        .find(|id| {
                            sessions.iter().any(|(known, path)| {
                                *known == state.session_for_pane(*id) && *path == state.project
                            })
                        })
                });
        }
        if saved.is_none()
            && let Some((id, _)) = sessions.iter().find(|(_, path)| path == cwd)
        {
            state.open(*id);
        }
        state.sidebar_width = state.sidebar_width.clamp(200., 560.);
        state.right_panel_width = state
            .right_panel_width
            .clamp(crate::ui::panel::WIDTH_MIN, crate::ui::panel::WIDTH_MAX)
            .round();
        state.right_panel_tabs = crate::ui::panel::RightTab::normalize(&state.right_panel_tabs);
        state
            .reviews
            .retain(|id, review| known.contains(id) && !review.tabs.is_empty());
        state.docks.retain(|_, dock| !dock.terminals.is_empty());
        for dock in state.docks.values_mut() {
            dock.active = dock.active.min(dock.terminals.len() - 1);
            dock.size = dock.side.clamp(dock.size, (4000., 4000.));
        }
        state.add_project(cwd);
        let open: HashSet<_> = state.open_sessions().into_iter().collect();
        let focus = std::mem::take(&mut state.project_focus);
        let owner = |id: i64| state.session_for_pane(id);
        let focus: HashMap<_, _> = focus
            .into_iter()
            .filter(|(project, id)| {
                open.contains(id)
                    && sessions
                        .iter()
                        .any(|(known, path)| *known == owner(*id) && path == project)
            })
            .collect();
        state.project_focus = focus;
        if let Some((_, project)) = sessions.iter().find(|(id, _)| {
            state
                .active
                .is_some_and(|pane| state.session_for_pane(pane) == *id)
        }) {
            state.project = project.clone();
            state
                .project_focus
                .insert(project.clone(), state.active.unwrap());
        }
        state.add_project(&state.project.clone());
        state
    }
}
