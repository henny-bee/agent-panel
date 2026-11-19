use super::*;

impl Shell {
    pub(crate) fn assign_project_rail_group(
        &mut self,
        path: &str,
        group: &str,
        cx: &mut Context<Self>,
    ) {
        if group.is_empty() {
            self.workspace.project_rail_assignments.remove(path);
        } else {
            self.workspace
                .project_rail_assignments
                .insert(path.into(), group.into());
        }
        self.persist(cx);
        self.projects_frame.update(cx, |_, cx| cx.notify());
    }

    pub(crate) fn create_project_rail_group(
        &mut self,
        path: Option<&str>,
        cx: &mut Context<Self>,
    ) -> String {
        let id = format!(
            "group-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        self.workspace
            .project_rail_groups
            .push(workspace::ProjectRailGroup {
                id: id.clone(),
                name: "New group".into(),
                ..Default::default()
            });
        if let Some(path) = path {
            self.workspace
                .project_rail_assignments
                .insert(path.into(), id.clone());
        }
        self.persist(cx);
        self.projects_frame.update(cx, |_, cx| cx.notify());
        id
    }

    pub(super) fn project_rail_item_offset(&self, id: &str, group: bool) -> f32 {
        if let Some(drag) = &self.project_rail_drag
            && drag.group == group
            && drag.path == id
            && drag.active
        {
            return drag.offset.as_f32();
        }
        self.project_rail_list
            .shifts
            .get(&rail_shift_key(id, group))
            .map_or(0., |shift| shift.at(std::time::Instant::now()).0)
    }

    pub(super) fn project_rail_project_offset(&self, path: &str) -> f32 {
        let project = self.project_rail_item_offset(path, false);
        if self.workspace.project_rail_pinned.contains(path) {
            return project;
        }
        project
            + self
                .workspace
                .project_rail_assignments
                .get(path)
                .map_or(0., |group| self.project_rail_item_offset(group, true))
    }

    pub(super) fn project_rail_item_raised(&self, id: &str, group: bool) -> bool {
        let key = rail_shift_key(id, group);
        self.project_rail_drag
            .as_ref()
            .is_some_and(|drag| drag.active && drag.group == group && drag.path == id)
            || self.project_rail_list.settling.as_ref() == Some(&key)
                && self
                    .project_rail_list
                    .shifts
                    .get(&key)
                    .is_some_and(|shift| !shift.at(std::time::Instant::now()).1)
    }

    pub(super) fn project_rail_project_raised(&self, path: &str) -> bool {
        self.project_rail_item_raised(path, false)
            || !self.workspace.project_rail_pinned.contains(path)
                && self
                    .workspace
                    .project_rail_assignments
                    .get(path)
                    .is_some_and(|id| self.project_rail_item_raised(id, true))
    }

    pub(super) fn project_rail_reordering(&self) -> bool {
        self.project_rail_drag
            .as_ref()
            .is_some_and(|drag| drag.active)
    }

    pub(crate) fn project_rail_animating(&self) -> bool {
        self.workspace.classic_layout
            && self.workspace.project_rail_open
            && self
                .project_rail_list
                .shifts
                .values()
                .any(|shift| shift.from != shift.to && !shift.at(std::time::Instant::now()).1)
    }

    pub(super) fn start_project_rail_drag(
        &mut self,
        id: String,
        group: bool,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Tauri's row buttons take keyboard focus on press. Keep Escape on the
        // workspace while a native div row is being dragged as well.
        window.focus(&self.focus, cx);
        let now = std::time::Instant::now();
        let items: Vec<(String, Bounds<Pixels>)> = if group {
            self.workspace
                .project_rail_groups
                .iter()
                .filter_map(|candidate| {
                    let mut bounds = self
                        .project_rail_list
                        .group_bounds
                        .get(&candidate.id)?
                        .get()?;
                    if !candidate.collapsed {
                        for path in &self.workspace.projects {
                            if !self.workspace.project_rail_pinned.contains(path)
                                && self.workspace.project_rail_assignments.get(path)
                                    == Some(&candidate.id)
                                && let Some(row) = self
                                    .project_rail_list
                                    .bounds
                                    .get(path)
                                    .and_then(AnchorSlot::get)
                            {
                                bounds.size.height =
                                    bounds.size.height.max(row.bottom() - bounds.top());
                            }
                        }
                    }
                    let offset = self.project_rail_item_offset(&candidate.id, true);
                    bounds.origin.y -= px(offset);
                    Some((candidate.id.clone(), bounds))
                })
                .collect()
        } else {
            self.workspace
                .projects
                .iter()
                .filter(|path| self.workspace.project_rail_peers(path, &id))
                .filter_map(|path| {
                    let mut bounds = self.project_rail_list.bounds.get(path)?.get()?;
                    bounds.origin.y -= px(self.project_rail_project_offset(path));
                    Some((path.clone(), bounds))
                })
                .collect()
        };
        let Some(from) = items.iter().position(|(path, _)| *path == id) else {
            return;
        };
        // A new press finishes any previous settling, as useAnimatedReorder does.
        self.project_rail_list
            .shifts
            .retain(|_, shift| !shift.at(now).1);
        self.project_rail_list.shifts.clear();
        self.project_rail_list.settling = None;
        self.project_rail_list.skip_click = false;
        self.project_rail_drag = Some(ProjectRailDrag {
            path: id,
            group,
            order: items.iter().map(|(id, _)| id.clone()).collect(),
            bounds: items.into_iter().map(|(_, bounds)| bounds).collect(),
            from,
            to: from,
            origin: event.position,
            offset: px(0.),
            active: false,
        });
    }

    pub(crate) fn move_project_rail_drag(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.cancel_project_rail_drag(cx);
            return;
        }
        let Some(drag) = &mut self.project_rail_drag else {
            return;
        };
        let offset = event.position.y - drag.origin.y;
        if !drag.active && offset.abs() < px(5.) {
            return;
        }
        drag.active = true;
        self.project_rail_list.skip_click = true;
        let slots: Vec<_> = (0..drag.order.len())
            .map(|to| drag.layout(to)[drag.from] - drag.bounds[drag.from].top())
            .collect();
        drag.offset = offset.max(slots[0]).min(*slots.last().unwrap());
        let to = slots
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (a.as_f32() - drag.offset.as_f32())
                    .abs()
                    .total_cmp(&(b.as_f32() - drag.offset.as_f32()).abs())
            })
            .map_or(drag.from, |(ix, _)| ix);
        if drag.to != to {
            drag.to = to;
            let starts = drag.layout(to);
            let now = std::time::Instant::now();
            for (ix, id) in drag.order.iter().enumerate() {
                if ix == drag.from {
                    continue;
                }
                let key = rail_shift_key(id, drag.group);
                let target = (starts[ix] - drag.bounds[ix].top()).as_f32();
                let from = self
                    .project_rail_list
                    .shifts
                    .get(&key)
                    .map_or(0., |shift| shift.at(now).0);
                self.project_rail_list.shifts.insert(
                    key,
                    TabShift {
                        from: if cx.reduce_motion() { target } else { from },
                        to: target,
                        start: now,
                    },
                );
            }
        }
        self.projects_frame.update(cx, |_, cx| cx.notify());
    }

    pub(crate) fn finish_project_rail_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.project_rail_drag.take() else {
            return;
        };
        if !drag.active {
            return;
        }
        let mut reordered = drag.order.clone();
        let moved = reordered.remove(drag.from);
        reordered.insert(drag.to, moved);
        let starts = drag.layout(drag.to);
        let now = std::time::Instant::now();
        for (ix, id) in drag.order.iter().enumerate() {
            let key = rail_shift_key(id, drag.group);
            let old_offset = if ix == drag.from {
                drag.offset.as_f32()
            } else {
                self.project_rail_list
                    .shifts
                    .get(&key)
                    .map_or(0., |shift| shift.at(now).0)
            };
            self.project_rail_list.shifts.insert(
                key,
                TabShift {
                    from: (drag.bounds[ix].top() - starts[ix]).as_f32() + old_offset,
                    to: 0.,
                    start: now,
                },
            );
        }
        self.project_rail_list.settling = Some(rail_shift_key(&drag.path, drag.group));
        let subset: HashSet<String> = drag.order.iter().cloned().collect();
        let mut order = reordered.into_iter();
        if drag.group {
            let groups: HashMap<_, _> = self
                .workspace
                .project_rail_groups
                .iter()
                .map(|g| (g.id.clone(), g.clone()))
                .collect();
            for group in &mut self.workspace.project_rail_groups {
                if subset.contains(&group.id)
                    && let Some(id) = order.next()
                    && let Some(next) = groups.get(&id)
                {
                    *group = next.clone();
                }
            }
        } else {
            for path in &mut self.workspace.projects {
                if subset.contains(path.as_str())
                    && let Some(id) = order.next()
                {
                    *path = id;
                }
            }
        }
        self.project_rail_list.skip_click = true;
        if cx.reduce_motion() {
            self.project_rail_list.shifts.clear();
        }
        self.persist(cx);
        self.projects_frame.update(cx, |_, cx| cx.notify());
    }

    pub(crate) fn cancel_project_rail_drag(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.project_rail_drag.take() else {
            return false;
        };
        if drag.active {
            self.project_rail_list.settling = Some(rail_shift_key(&drag.path, drag.group));
            let now = std::time::Instant::now();
            for (ix, id) in drag.order.iter().enumerate() {
                let key = rail_shift_key(id, drag.group);
                let from = if ix == drag.from {
                    drag.offset.as_f32()
                } else {
                    self.project_rail_list
                        .shifts
                        .get(&key)
                        .map_or(0., |shift| shift.at(now).0)
                };
                self.project_rail_list.shifts.insert(
                    key,
                    TabShift {
                        from,
                        to: 0.,
                        start: now,
                    },
                );
            }
            self.project_rail_list.skip_click = true;
            if cx.reduce_motion() {
                self.project_rail_list.shifts.clear();
            }
            self.projects_frame.update(cx, |_, cx| cx.notify());
        }
        true
    }

    pub(super) fn queue_project_rail_stats(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.project_rail_stats.values.contains_key(path)
            || path == "~"
            || path.starts_with("remote://")
        {
            return;
        }
        self.project_rail_stats.queued.insert(path.into());
        if self.project_rail_stats.deferred || self.project_rail_stats.loading {
            return;
        }
        self.project_rail_stats.deferred = true;
        let owner = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = owner.update(cx, |this, cx| {
                this.project_rail_stats.deferred = false;
                this.load_project_rail_stats(cx);
            });
        });
    }

    pub(super) fn load_project_rail_stats(&mut self, cx: &mut Context<Self>) {
        if self.project_rail_stats.loading || self.project_rail_stats.queued.is_empty() {
            return;
        }
        let paths: Vec<_> = self.project_rail_stats.queued.drain().collect();
        self.project_rail_stats.loading = true;
        let work = cx.background_executor().spawn(async move {
            paths
                .into_iter()
                .map(|path| {
                    let stats =
                        monocode_core::vcs::git::scm::diff_stats(std::path::Path::new(&path)).ok();
                    (path, stats)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let values = work.await;
            let _ = this.update(cx, |this, cx| {
                this.project_rail_stats.loading = false;
                this.project_rail_stats.values.extend(values);
                this.projects_frame.update(cx, |_, cx| cx.notify());
                this.load_project_rail_stats(cx);
            });
        })
        .detach();
    }

    pub(crate) fn sync_project_rail_git_stats(&mut self, cx: &mut Context<Self>) {
        // A folder picked in the panel is not the project's own checkout.
        if self
            .workspace
            .active
            .is_some_and(|id| self.workspace.panel_folders.contains_key(&id))
        {
            return;
        }
        if let Some(stats) = self.git.read(cx).stats.clone() {
            self.project_rail_stats
                .values
                .insert(self.workspace.project.clone(), Some(stats));
            if self.workspace.classic_layout {
                self.projects_frame.update(cx, |_, cx| cx.notify());
            }
        }
    }
}
