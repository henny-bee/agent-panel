use super::super::*;

impl Shell {
    pub(in crate::app::shell) fn close_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_open = None;
        self.preview_close = None;
        if self.session_preview.take().is_some() {
            self.sidebar_frame.update(cx, |_, cx| cx.notify());
        }
    }

    pub(in crate::app::shell) fn sidebar_scrolled(&mut self, cx: &mut Context<Self>) {
        self.sidebar_last_scroll = Some(std::time::Instant::now());
        self.close_preview(cx);
        if let Some(id) = self.session_hover
            && let Some(anchor) = self.session_row_bounds.get(&id).cloned()
        {
            self.schedule_preview(id, anchor, cx);
        }
    }

    pub(in crate::app::shell) fn schedule_preview(
        &mut self,
        id: i64,
        anchor: AnchorSlot,
        cx: &mut Context<Self>,
    ) {
        if self.menu.is_some() || self.classic_worktree_picker.is_some() {
            return;
        }
        let settle = std::time::Duration::from_millis(180);
        let delay = self
            .sidebar_last_scroll
            .map(|last| settle.saturating_sub(last.elapsed()))
            .unwrap_or_default()
            + std::time::Duration::from_millis(120);
        self.preview_open = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Ok(Some(store)) = this.update_in(cx, |this, window, cx| {
                if this.session_hover != Some(id)
                    || this.menu.is_some()
                    || !this.session_summaries.contains_key(&id)
                    || !anchor
                        .get()
                        .is_some_and(|bounds| bounds.contains(&window.mouse_position()))
                {
                    return None;
                }
                this.session_preview = Some(SessionPreview {
                    id,
                    data: None,
                    anchor,
                    surface: AnchorSlot::default(),
                });
                this.sidebar_frame.update(cx, |_, cx| cx.notify());
                // An already-open conversation has its own live metadata.
                (!this.sessions.contains_key(&id)).then(|| this.store.clone())
            }) else {
                return;
            };
            let data = cx
                .background_executor()
                .spawn(async move { locked(&store).and_then(|store| store.session_preview(id)) })
                .await
                .ok();
            let _ = this.update(cx, |this, cx| {
                if let Some(preview) = this
                    .session_preview
                    .as_mut()
                    .filter(|preview| preview.id == id)
                {
                    preview.data = data;
                    this.sidebar_frame.update(cx, |_, cx| cx.notify());
                }
            });
        }));
    }

    pub(in crate::app::shell) fn hover_session(
        &mut self,
        id: i64,
        anchor: AnchorSlot,
        hovered: bool,
        compact: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.deleting.contains(&id) {
            return;
        }
        let previous = self.session_hover;
        if hovered {
            self.session_hover = Some(id);
        } else if self.session_hover == Some(id) {
            self.session_hover = None;
        }
        if previous != self.session_hover {
            self.sidebar_frame.update(cx, |_, cx| cx.notify());
        }
        if hovered && compact {
            self.preview_close = None;
            if self
                .session_preview
                .as_ref()
                .is_none_or(|preview| preview.id != id)
            {
                self.close_preview(cx);
                self.schedule_preview(id, anchor, cx);
            }
        } else {
            // A row's leave can arrive after the next row's enter. Do not
            // cancel that newer hover, or metadata loading for an open card.
            if self.session_hover.is_none()
                && self
                    .session_preview
                    .as_ref()
                    .is_none_or(|preview| preview.id != id)
            {
                self.preview_open = None;
            }
            if self
                .session_preview
                .as_ref()
                .is_some_and(|preview| preview.id == id)
            {
                self.schedule_preview_close(window, cx);
            }
        }
    }

    pub(in crate::app::shell) fn schedule_preview_close(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        // During the entrance the card can already cover the gap beside its
        // row. The row's leave event must not undo the card's enter event.
        if self.session_preview.as_ref().is_some_and(|preview| {
            preview
                .surface
                .get()
                .is_some_and(|bounds| bounds.contains(&window.mouse_position()))
        }) {
            self.preview_close = None;
            return;
        }
        self.preview_close = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(120))
                .await;
            let _ = this.update(cx, |this, cx| this.close_preview(cx));
        }));
    }

    pub(in crate::app::shell) fn session_row(&self, id: i64, cx: &mut Context<Self>) -> AnyElement {
        self.session_row_with(id, false, None, cx)
    }

    /// SessionCard, dense: `nested` for a session on another's rail; `rail`
    /// for a thread with one under it, and whether it is open.
    pub(in crate::app::shell) fn session_row_with(
        &self,
        id: i64,
        nested: bool,
        rail: Option<(&sidebar::rails::Rail, bool)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(row) = self.rename_row(&sessions::groups::RenameTarget::Session(id), cx) {
            return row;
        }
        // A collapsed thread shows a chevron in its mark's place on hover:
        // selecting it opens the rail below.
        let expandable = rail.is_some_and(|(_, open)| !open);
        let mode = rail.map(|(rail, _)| match rail {
            sidebar::rails::Rail::Agents(summary) => (
                "share",
                // fuchsia-200/90
                gpui::Hsla::from(rgb(0xf5d0fe)).opacity(0.9),
                format!(
                    "Orchestrator, {} {}",
                    summary.tasks.len(),
                    if summary.tasks.len() == 1 {
                        "agent"
                    } else {
                        "agents"
                    }
                ),
            ),
            sidebar::rails::Rail::Started(children) => (
                "cursor-magic",
                // sky-200/90
                gpui::Hsla::from(rgb(0xbae6fd)).opacity(0.9),
                format!(
                    "Operator, started {} {}",
                    children.len(),
                    if children.len() == 1 {
                        "session"
                    } else {
                        "sessions"
                    }
                ),
            ),
        });
        // startedInView: one of its sessions is the one in view or chosen,
        // so the thread keeps its hover fill as the group's head.
        let started_in_view = match rail {
            Some((sidebar::rails::Rail::Started(children), _)) => children.iter().any(|c| {
                self.selected_sessions.contains(c)
                    || (self.workspace.active == Some(*c)
                        && !self.mono_active(cx)
                        && !self.app_view_open())
            }),
            _ => false,
        };
        let compact = nested || self.workspace.compact_session_cards;
        let side = if nested || !compact { 14. } else { 16. };
        if let Some(mark) = self.session_marks.get(&id) {
            mark.update(cx, |mark, cx| mark.set_size(side, cx));
        }
        let work_item = self.work_item_badge(id, compact, cx);
        let has_badges = work_item.is_some()
            || mode.is_some()
            || self.automation_state.session_automations.contains_key(&id);
        let session = self.session_metadata(id, cx);
        let active = self.selected_sessions.contains(&id)
            || (self.workspace.active == Some(id)
                && !self.mono_active(cx)
                && !self.app_view_open());
        let anchor = self
            .session_row_bounds
            .get(&id)
            .cloned()
            .unwrap_or_default();
        self.sidebar_list.track_anchor(anchor.clone());
        let draft = session.has_saved_draft();
        let (model, branch, workspace_label) = if compact {
            (SharedString::default(), None, String::new())
        } else {
            let saved = self.session_details.get(&id);
            let live = self
                .sessions
                .get(&id)
                .and_then(|view| view.read(cx).sidebar_details());
            let (model, branch) = live.unwrap_or_else(|| {
                (
                    crate::chat::composer::pickers::cached_model_name(
                        Kind::from_id(&session.summary.provider),
                        saved.and_then(|data| data.model.as_deref()),
                    ),
                    None,
                )
            });
            let cwd = self
                .session_worktrees
                .get(&id)
                .and_then(|cwd| cwd.as_deref())
                .unwrap_or(&session.summary.cwd);
            let branch = branch
                .or_else(|| saved.and_then(|data| data.branch.clone()))
                .or_else(|| {
                    let git = self.git.read(cx);
                    (git.root == cwd)
                        .then(|| git.index.as_ref().and_then(|index| index.branch.clone()))
                        .flatten()
                });
            let directory = std::path::Path::new(cwd)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(cwd);
            let workspace_label = branch.clone().unwrap_or_else(|| directory.to_owned());
            (model, branch, workspace_label)
        };
        let pinned = session.summary.pinned;
        let mark = {
            let mark = match self.session_marks.get(&id) {
                Some(mark) if self.sidebar_uncached() => div()
                    .size(px(side))
                    .flex_shrink_0()
                    .child(mark.clone())
                    .into_any_element(),
                Some(mark) => mark
                    .clone()
                    .cached(StyleRefinement::default().size(px(side)).flex_shrink_0())
                    .into_any_element(),
                None => provider_icon(&session.summary.provider, side).into_any_element(),
            };
            div()
                .relative()
                .size(px(side))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .when(nested, |d| d.overflow_hidden())
                .child(
                    div()
                        .size_full()
                        .when(expandable, |d| {
                            d.group_hover("session-row", |s| s.opacity(0.))
                        })
                        .child(mark),
                )
                .when(expandable, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(0.)
                            .group_hover("session-row", |s| s.opacity(1.))
                            .child(icon("chevron-down", 14.).text_color(content(0.6))),
                    )
                })
        };
        let title = div()
            .debug_selector(move || format!("session-title-{id}"))
            .flex_1()
            .min_w_0()
            .h(px(20.))
            .when(nested, |d| {
                d.text_size(px(13.))
                    .text_color(content(if active { 1. } else { 0.55 }))
                    .when(!active, |d| {
                        d.group_hover("session-row", |s| s.text_color(content(0.8)))
                    })
            })
            .when(!nested && compact, |d| {
                d.text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(1.))
            })
            .when(!compact, |d| {
                d.text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(content(1.))
            })
            .child(crate::ui::motion::particle_text::particle_text(
                session.display_title(),
            ));
        let badges = div()
            .flex()
            .items_center()
            .gap_1()
            .flex_shrink_0()
            .children(work_item)
            .when(
                self.automation_state.session_automations.contains_key(&id),
                |row| row.child(icon("zap", 12.).text_color(content(0.35))),
            )
            // Marked on the right like a linked issue or PR, in its mode's colour.
            .when_some(mode, |d, (glyph, color, label)| {
                d.child(
                    div()
                        .id(("session-mode", id as usize))
                        .role(Role::Image)
                        .aria_label(label)
                        .size(px(16.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(glyph, 12.).text_color(color)),
                )
            });
        let suffix = self.session_suffix(id, &session, compact, cx);
        let body = if compact {
            div()
                .w_full()
                .flex()
                .items_center()
                .gap(px(if nested { 6. } else { 8. }))
                .child(mark)
                .child(title)
                .when(has_badges, |d| d.child(badges))
                .child(suffix)
                .into_any_element()
        } else {
            div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .h(px(20.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(pinned, |d| {
                            d.child(icon("pin", 12.).text_color(content(0.45)))
                        })
                        .child(title)
                        .child(suffix),
                )
                .child(
                    div()
                        .h(px(20.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_size(px(11.))
                                .text_color(content(0.45))
                                .child(icon(
                                    if branch.is_some() {
                                        "git-branch"
                                    } else {
                                        "folder"
                                    },
                                    12.,
                                ))
                                .child(
                                    div()
                                        .debug_selector(move || format!("session-workspace-{id}"))
                                        .min_w_0()
                                        .truncate()
                                        .child(workspace_label),
                                ),
                        )
                        .when(has_badges, |d| d.child(badges))
                        .child(
                            div()
                                .flex_shrink_0()
                                .min_w_0()
                                .max_w(gpui::relative(0.5))
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .text_size(px(11.))
                                .text_color(content(0.5))
                                .child(
                                    div()
                                        .debug_selector(move || format!("session-model-{id}"))
                                        .min_w_0()
                                        .truncate()
                                        .child(model),
                                )
                                .child(provider_icon(&session.summary.provider, 14.)),
                        ),
                )
                .into_any_element()
        };
        div()
            .id(("sidebar-session", id as usize))
            .debug_selector(move || format!("sidebar-session-{id}"))
            .group("session-row")
            .role(Role::Button)
            .aria_label(session.display_title())
            .aria_description(format!(
                "{} conversation",
                Kind::from_id(&session.summary.provider).label()
            ))
            .cursor_pointer()
            // A nested row's `h-5` line and `px-1.5`, inside the card border.
            .h(px(if compact {
                if nested { 28. } else { 32. }
            } else if nested {
                56.
            } else {
                60.
            }))
            // Sidebar.tsx's `px-2` inside its 1px card border.
            .px(px(if nested { 7. } else { 9. }))
            .rounded_md()
            // The border is dashed for a saved draft. It overlays the row, so
            // the hover anchor still spans the whole card.
            .when(draft, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .rounded_md()
                        .border_1()
                        .border_dashed()
                        .border_color(content(if active { 0.3 } else { 0.25 })),
                )
            })
            .flex()
            .items_center()
            .when(active, |d| d.bg(content(0.1)))
            .when(started_in_view && !active, |d| d.bg(content(0.05)))
            .hover(|s| s.bg(content(if active { 0.1 } else { 0.05 })))
            .when(
                self.sidebar_drop == Some(sessions::groups::ListDrop::Session(id)),
                |d| d.bg(content(0.14)),
            )
            .when(self.deleting.contains(&id), |d| d.opacity(0.5))
            .child(body)
            .child(anchor.probe())
            .relative()
            .on_hover(cx.listener(move |this, hovered, window, cx| {
                this.hover_session(id, anchor.clone(), *hovered, compact, window, cx);
            }))
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    if std::mem::take(&mut this.skip_session_click) {
                        return;
                    }
                    this.select_sidebar_session(id, event.modifiers(), window, cx)
                }),
            )
            // Sidebar.tsx: a session card dragged onto a pane edge opens there.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, _| {
                    this.skip_session_click = false;
                    this.tab_drag = None;
                    this.pane_drag = Some(PaneDrag {
                        source: DragSource::Session(id),
                        start: event.position,
                        position: event.position,
                        active: false,
                        over: None,
                        insert: None,
                    });
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    let anchor = Anchor::Point(event.position);
                    this.context_selection = !this.selected_sessions.contains(&id);
                    if this.context_selection {
                        this.selected_sessions.clear();
                        this.selected_sessions.insert(id);
                    }
                    this.open_menu(MenuTarget::Session(id), anchor, window, cx);
                }),
            )
            .into_any()
    }

    pub(in crate::app::shell) fn session_archive_button(
        &self,
        id: i64,
        session: &sessions::loading::Metadata<'_>,
        cx: &Context<Self>,
    ) -> AnyElement {
        div()
            .id(("archive-session", id as usize))
            .debug_selector(move || format!("archive-session-{id}"))
            .role(Role::Button)
            .aria_label(format!(
                "{} {}",
                if session.summary.archived {
                    "Unarchive"
                } else {
                    "Archive"
                },
                session.display_title()
            ))
            .group("session-archive")
            .size(px(20.))
            .mr(px(-3.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .opacity(if self.archiving.contains(&id) {
                0.4
            } else {
                1.
            })
            .hover(|s| s.bg(content(0.1)))
            .child(
                icon("archive", 12.).group_hover("session-archive", |s| s.text_color(content(1.))),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_archive(id, cx);
            }))
            .into_any()
    }

    pub(in crate::app::shell) fn session_suffix(
        &self,
        id: i64,
        session: &sessions::loading::Metadata<'_>,
        compact: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let detailed = !compact;
        if self.session_hover == Some(id) {
            return self.session_archive_button(id, session, cx);
        }
        div()
            .flex_shrink_0()
            .text_size(px(11.))
            .line_height(px(16.))
            .font_weight(FontWeight::NORMAL)
            .text_color(content(0.4))
            .child(if session.needs_approval() {
                div()
                    .id(("session-approval", id as usize))
                    .role(Role::Image)
                    .aria_label("Needs approval")
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_color(rgb(0xfbbf24))
                    .child(icon("circle-alert", 14.))
                    .when(detailed, |d| d.child("Needs approval"))
                    .into_any()
            } else if detailed && session.responding() {
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_color(content(0.5))
                    .children(self.session_marks.get(&id).cloned())
                    .child("Working…")
                    .into_any()
            } else if detailed && self.unseen_finished.contains(&id) {
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_color(rgb(0x2dd4bf))
                    .child(icon("check-circle-2", 14.).text_color(rgb(0x2dd4bf)))
                    .child("Done")
                    .into_any()
            } else if session.has_saved_draft() {
                div()
                    .id(("session-draft", id as usize))
                    .role(Role::Image)
                    .aria_label("Draft")
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(icon("circle-dashed-hi", 14.).text_color(content(0.4)))
                    .when(detailed, |d| d.child("Draft"))
                    .into_any()
            } else if let Some(reminder) = self.reminders.iter().find(|r| r.session_id == id) {
                div()
                    .id(("session-reminder", id as usize))
                    .aria_label(local_reminder_label(reminder.due_at))
                    .text_color(rgb(0xf59e0b))
                    .child(sessions::groups::reminder_caption(reminder.due_at))
                    .into_any()
            } else {
                div()
                    .child(monocode_core::sys::time::relative(
                        session.summary.updated_at,
                        monocode_core::sys::time::now_ms(),
                    ))
                    .into_any()
            })
            .into_any()
    }
}
